//! Per-branch measurement and deleted-branch ticket recovery.

use std::collections::{HashMap, HashSet, VecDeque};

use gix::bstr::ByteSlice;
use graduate::promotion::{jira_key_from_branch, JiraIssueState, PromotionBranch, PromotionCommit};

use crate::environment_git::{gitoxide_error, reference_id, unix_date};
use crate::error::CliError;

/// Merge commits made on an environment branch's own first-parent line,
/// keyed by commit id and valued by the environment branch name.
///
/// A feature branch that can reach one of these commits has had that
/// environment branch merged into it. Merges that only pull main into the
/// environment are skipped, so a feature branch that merged main is never
/// flagged.
pub(super) fn environment_merge_markers(
    repository: &gix::Repository,
    prefix: &str,
    names: &[&str],
    main_ancestors: &HashSet<gix::ObjectId>,
) -> Result<HashMap<gix::ObjectId, String>, CliError> {
    let mut markers = HashMap::new();
    for name in names {
        let Ok(tip) = reference_id(repository, &format!("{prefix}{name}")) else {
            continue;
        };
        let mut visited = HashSet::new();
        let mut current = Some(tip);
        while let Some(id) = current {
            if main_ancestors.contains(&id) || !visited.insert(id) {
                break;
            }
            let commit = repository.find_commit(id).map_err(gitoxide_error)?;
            let mut parents = commit.parent_ids().map(|parent| parent.detach());
            let first = parents.next();
            if let Some(second) = parents.next() {
                if !main_ancestors.contains(&second) {
                    markers.entry(id).or_insert_with(|| (*name).to_owned());
                }
            }
            current = first;
        }
    }
    Ok(markers)
}

/// Lowercase merge-subject patterns that reveal an environment branch as the
/// merge target or source.
pub(super) struct EnvironmentSubject {
    /// Environment name reported in `merged_environments`.
    environment: String,
    /// `Merge branch 'X' into qa`: the merge was made on the environment.
    target: String,
    /// `Merge branch 'qa' into X`: the environment was merged into the line.
    source: String,
    /// `Merge remote-tracking branch 'origin/qa'`: the environment on the
    /// configured remote, anchored so a feature branch that merely ends in
    /// an environment name never matches.
    remote_source: String,
}

pub(super) fn environment_subjects(names: &[&str], remote: &str) -> Vec<EnvironmentSubject> {
    let remote = remote.to_ascii_lowercase();
    names
        .iter()
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            EnvironmentSubject {
                environment: (*name).to_owned(),
                target: format!(" into {lower}"),
                source: format!("branch '{lower}'"),
                remote_source: format!("branch '{remote}/{lower}'"),
            }
        })
        .collect()
}

pub(super) fn measure_branch(
    repository: &gix::Repository,
    main_ancestors: &HashSet<gix::ObjectId>,
    environment_markers: &HashMap<gix::ObjectId, String>,
    environment_subjects: &[EnvironmentSubject],
    branch: String,
    tip: gix::ObjectId,
    jira: JiraIssueState,
) -> Result<PromotionBranch, CliError> {
    let tip_commit = repository.find_commit(tip).map_err(gitoxide_error)?;
    let author = tip_commit.author().map_err(gitoxide_error)?;
    let author_time = author.time().map_err(gitoxide_error)?;
    let last = unix_date(author_time.seconds);
    let last_author = author.name.to_str_lossy().into_owned();
    let mut unique = HashSet::new();
    let mut commits = Vec::new();
    let mut merged_environments = Vec::new();
    let mut pending = VecDeque::from([tip]);
    let mut started_seconds = author_time.seconds;
    while let Some(id) = pending.pop_front() {
        if main_ancestors.contains(&id) || !unique.insert(id) {
            continue;
        }
        if let Some(environment) = environment_markers.get(&id) {
            if !merged_environments.contains(environment) {
                merged_environments.push(environment.clone());
            }
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        let parents = commit
            .parent_ids()
            .map(|parent| parent.detach())
            .collect::<Vec<_>>();
        pending.extend(parents.iter().copied());
        if parents.len() > 1 {
            // Merge commits carry no promotable work of their own, but their
            // recorded subjects reveal environment merges that survive even
            // after the environment branch itself is rebuilt.
            let brings_foreign_history = parents
                .get(1)
                .is_some_and(|second| !main_ancestors.contains(second));
            if brings_foreign_history {
                let message = commit.message_raw().map_err(gitoxide_error)?;
                let subject = message
                    .to_str_lossy()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                if subject.starts_with("merge ") {
                    for candidate in environment_subjects {
                        if (subject.ends_with(&candidate.target)
                            || subject.contains(&candidate.source)
                            || subject.contains(&candidate.remote_source))
                            && !merged_environments.contains(&candidate.environment)
                        {
                            merged_environments.push(candidate.environment.clone());
                        }
                    }
                }
            }
            continue;
        }
        let commit_author = commit.author().map_err(gitoxide_error)?;
        let committed_at = commit_author.time().map_err(gitoxide_error)?.seconds;
        started_seconds = started_seconds.min(committed_at);
        let message = commit.message_raw().map_err(gitoxide_error)?;
        let subject = message
            .to_str_lossy()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let author = commit_author.name.to_str_lossy().into_owned();
        let id = id.to_string();
        let short_id = id.chars().take(7).collect();
        commits.push((
            committed_at,
            PromotionCommit {
                id: id.clone(),
                short_id,
                subject,
                author,
                date: unix_date(committed_at),
            },
        ));
    }
    commits.sort_by_key(|commit| std::cmp::Reverse(commit.0));
    merged_environments.sort();
    Ok(PromotionBranch {
        branch,
        started: unix_date(started_seconds),
        last,
        ahead: commits.len(),
        last_author,
        commits: commits.into_iter().map(|(_, commit)| commit).collect(),
        merged_environments,
        jira,
    })
}

/// Work that reached the environment through a branch whose remote ref no
/// longer exists (deleted when its pull request completed) or whose current
/// tip is no longer reachable from the environment.
///
/// Non-merge commits unique to the environment are grouped by the Jira key
/// in their subject, one synthetic row per key, named after the key. Keys in
/// `covered_keys` already have a real branch row and are skipped. Merge
/// commits are skipped entirely: environment sync merges ("master into qa")
/// carry no promotable work, and a real feature merge always brings the
/// branch's own commits into the range.
pub(super) fn recover_deleted_branch_tickets(
    repository: &gix::Repository,
    environment_tip: gix::ObjectId,
    main_ancestors: &HashSet<gix::ObjectId>,
    covered_keys: &HashSet<String>,
    jira_configured: bool,
) -> Result<Vec<PromotionBranch>, CliError> {
    struct TicketWork {
        started: i64,
        last: i64,
        last_author: String,
        commits: Vec<(i64, PromotionCommit)>,
    }
    let mut tickets: HashMap<String, TicketWork> = HashMap::new();
    let mut visited = HashSet::new();
    let mut pending = VecDeque::from([environment_tip]);
    while let Some(id) = pending.pop_front() {
        if main_ancestors.contains(&id) || !visited.insert(id) {
            continue;
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        let parents = commit
            .parent_ids()
            .map(|parent| parent.detach())
            .collect::<Vec<_>>();
        pending.extend(parents.iter().copied());
        if parents.len() > 1 {
            continue;
        }
        let message = commit.message_raw().map_err(gitoxide_error)?;
        let subject = message
            .to_str_lossy()
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let Some(key) = jira_key_from_branch(&subject) else {
            continue;
        };
        if covered_keys.contains(&key) {
            continue;
        }
        let author = commit.author().map_err(gitoxide_error)?;
        let seconds = author.time().map_err(gitoxide_error)?.seconds;
        let author_name = author.name.to_str_lossy().into_owned();
        let id = id.to_string();
        let entry = tickets.entry(key).or_insert_with(|| TicketWork {
            started: seconds,
            last: seconds,
            last_author: author_name.clone(),
            commits: Vec::new(),
        });
        entry.started = entry.started.min(seconds);
        if seconds >= entry.last {
            entry.last = seconds;
            entry.last_author = author_name.clone();
        }
        entry.commits.push((
            seconds,
            PromotionCommit {
                id: id.clone(),
                short_id: id.chars().take(7).collect(),
                subject,
                author: author_name,
                date: unix_date(seconds),
            },
        ));
    }
    let mut rows = tickets
        .into_iter()
        .map(|(key, work)| {
            let mut commits = work.commits;
            commits.sort_by_key(|commit| std::cmp::Reverse(commit.0));
            let jira = if jira_configured {
                JiraIssueState::Loading { key: key.clone() }
            } else {
                JiraIssueState::NotConfigured { key: key.clone() }
            };
            PromotionBranch {
                branch: key,
                started: unix_date(work.started),
                last: unix_date(work.last),
                ahead: commits.len(),
                last_author: work.last_author,
                commits: commits.into_iter().map(|(_, commit)| commit).collect(),
                merged_environments: Vec::new(),
                jira,
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.branch.cmp(&right.branch));
    Ok(rows)
}
