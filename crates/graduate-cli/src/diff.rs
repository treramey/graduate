//! Environment-to-main promotion report orchestration and Git adapters.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use gix::bstr::ByteSlice;
use graduate::jira::JiraCredentials;
use graduate::promotion::{
    jira_key_from_branch, AgeBucket, JiraIssueState, PromotionAgeReport, PromotionBranch,
    PromotionCommit, ReportDate,
};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::browser::SystemBrowserLauncher;
use crate::cli::{DiffArgs, DiffReport, ReportFormat};
use crate::config::Config;
use crate::diff_tui;
use crate::error::CliError;
use crate::jira::JiraClient;

/// Long-lived environment aggregation branches that are never promoted to
/// main and must not appear as feature rows.
const KNOWN_ENVIRONMENTS: [&str; 3] = ["qa", "staging", "cycle"];

#[derive(Clone, Debug)]
pub(crate) enum DiffUpdate {
    Skeleton {
        environment: String,
        main: String,
        branches: Vec<String>,
    },
    Measured(PromotionBranch),
    Jira {
        branch: String,
        state: JiraIssueState,
    },
    Finished,
    Failed(String),
}

pub(crate) struct PromotionReport {
    pub(crate) environment: String,
    pub(crate) main: String,
    pub(crate) branches: Vec<PromotionBranch>,
}

struct ScanOptions {
    repository: PathBuf,
    environment: String,
    main: Option<String>,
    remote: String,
    jira_configured: bool,
    fetch_before_scan: bool,
    selected_branches: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffParams {
    branches: Vec<String>,
}

pub(crate) async fn run(args: DiffArgs, config_path: &Path) -> Result<(), CliError> {
    validate_ref_component("environment", &args.environment)?;
    validate_ref_component("remote", &args.remote)?;
    if let Some(main) = &args.main {
        validate_ref_component("main branch", main)?;
    }
    let selected_branches = parse_selected_branches(args.params.as_deref())?;
    let report_kind = args.report.unwrap_or(DiffReport::Branches);
    let interactive = args.report.is_none()
        && args.params.is_none()
        && args.output_format.is_none()
        && args.output.is_none()
        && io::stdin().is_terminal()
        && io::stderr().is_terminal();
    if !args.no_fetch && !interactive {
        fetch_remote(&args, interactive)?;
    }

    let credentials = if matches!(report_kind, DiffReport::Age) {
        None
    } else {
        Config::load(config_path)?.jira_credentials()?
    };
    let (updates_tx, updates_rx) = mpsc::unbounded_channel();
    let scan = ScanOptions {
        repository: std::env::current_dir()?,
        environment: args.environment.clone(),
        main: args.main.clone(),
        remote: args.remote.clone(),
        jira_configured: credentials.is_some(),
        fetch_before_scan: !args.no_fetch && interactive,
        selected_branches,
    };
    let coordinator = tokio::spawn(coordinate_scan(scan, credentials, updates_tx));

    let report_result = if interactive {
        diff_tui::run(updates_rx, &SystemBrowserLauncher).await
    } else {
        collect_plain(updates_rx).await
    };
    let coordinator_result = coordinator
        .await
        .map_err(|error| CliError::Git(format!("promotion scan task failed: {error}")))?;
    coordinator_result?;
    let report = report_result?;
    if !interactive {
        write_report(
            &report,
            report_kind,
            args.output_format.unwrap_or(ReportFormat::Json),
            args.output.as_deref(),
        )?;
    }
    Ok(())
}

async fn coordinate_scan(
    options: ScanOptions,
    credentials: Option<JiraCredentials>,
    output: mpsc::UnboundedSender<DiffUpdate>,
) -> Result<(), CliError> {
    if options.fetch_before_scan {
        let remote = options.remote.clone();
        let fetch_result = tokio::task::spawn_blocking(move || fetch_remote_name(&remote, true))
            .await
            .map_err(|error| CliError::Git(format!("Git fetch task failed: {error}")))?;
        if let Err(error) = fetch_result {
            let _ = output.send(DiffUpdate::Failed(error.to_string()));
            return Err(error);
        }
    }
    let (scan_tx, mut scan_rx) = mpsc::unbounded_channel();
    let scan_task = tokio::task::spawn_blocking(move || scan_repository(&options, &scan_tx));

    let mut jira_tasks = JoinSet::new();
    let jira = match credentials.as_ref().map(JiraClient::new).transpose() {
        Ok(client) => client.map(Arc::new),
        Err(error) => {
            let _ = output.send(DiffUpdate::Failed(error.to_string()));
            return Err(error);
        }
    };
    let credentials = credentials.map(Arc::new);
    while let Some(update) = scan_rx.recv().await {
        if let DiffUpdate::Measured(row) = &update {
            if let (Some(credentials), Some(jira), Some(key)) =
                (credentials.clone(), jira.clone(), row.jira.key())
            {
                if jira_tasks.len() >= 8 {
                    if let Err(error) = forward_jira_result(&output, jira_tasks.join_next().await) {
                        jira_tasks.abort_all();
                        return if matches!(error, CliError::ReportCancelled) {
                            Ok(())
                        } else {
                            Err(error)
                        };
                    }
                }
                let branch = row.branch.clone();
                let key = key.to_owned();
                jira_tasks.spawn(async move {
                    let result = jira.issue(&credentials, &key).await;
                    let state = jira_issue_state(key, result);
                    (branch, state)
                });
            }
        }
        let failed = matches!(update, DiffUpdate::Failed(_));
        if output.send(update).is_err() || failed {
            jira_tasks.abort_all();
            return Ok(());
        }
    }

    match scan_task.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = output.send(DiffUpdate::Failed(error.to_string()));
            jira_tasks.abort_all();
            return Err(error);
        }
        Err(error) => {
            let message = format!("Git scan task failed: {error}");
            let _ = output.send(DiffUpdate::Failed(message.clone()));
            jira_tasks.abort_all();
            return Err(CliError::Git(message));
        }
    }

    while let Some(result) = jira_tasks.join_next().await {
        if let Err(error) = forward_jira_result(&output, Some(result)) {
            jira_tasks.abort_all();
            return if matches!(error, CliError::ReportCancelled) {
                Ok(())
            } else {
                Err(error)
            };
        }
    }
    let _ = output.send(DiffUpdate::Finished);
    Ok(())
}

fn jira_issue_state(
    key: String,
    result: Result<graduate::promotion::JiraIssueSummary, CliError>,
) -> JiraIssueState {
    match result {
        Ok(issue) => JiraIssueState::Loaded(issue),
        Err(CliError::JiraStatus(404)) => JiraIssueState::NotFound { key },
        Err(error) => JiraIssueState::Failed {
            key,
            message: error.to_string(),
        },
    }
}

fn forward_jira_result(
    output: &mpsc::UnboundedSender<DiffUpdate>,
    result: Option<Result<(String, JiraIssueState), tokio::task::JoinError>>,
) -> Result<(), CliError> {
    match result {
        Some(Ok((branch, state))) => output
            .send(DiffUpdate::Jira { branch, state })
            .map_err(|_| CliError::ReportCancelled),
        Some(Err(error)) => {
            let message = format!("Jira enrichment task failed: {error}");
            let _ = output.send(DiffUpdate::Failed(message.clone()));
            Err(CliError::Git(message))
        }
        None => Err(CliError::Git(
            "Jira enrichment queue ended unexpectedly".to_owned(),
        )),
    }
}

fn scan_repository(
    options: &ScanOptions,
    updates: &mpsc::UnboundedSender<DiffUpdate>,
) -> Result<(), CliError> {
    let repository = gix::discover(&options.repository).map_err(gitoxide_error)?;
    let prefix = format!("refs/remotes/{}/", options.remote);
    let environment_ref = format!("{prefix}{}", options.environment);
    let environment_id = reference_id(&repository, &environment_ref)
        .map_err(|_| CliError::Git(format!("{environment_ref} does not exist after fetching")))?;
    let main = resolve_main_branch(&repository, &prefix, options.main.as_deref())?;
    let main_ref = format!("{prefix}{main}");
    let main_id = reference_id(&repository, &main_ref)?;
    let environment_ancestors = ancestors(&repository, environment_id)?;
    let main_ancestors = ancestors(&repository, main_id)?;
    let names = environment_names(&options.environment, &main);
    let environment_markers =
        environment_merge_markers(&repository, &prefix, &names, &main_ancestors)?;
    let environment_subjects = environment_subjects(&names, &options.remote);

    let mut candidates = promotion_candidates(
        &repository,
        &prefix,
        &options.environment,
        &main,
        &environment_ancestors,
        &main_ancestors,
        options.selected_branches.as_deref(),
    )?;
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    let covered_keys = candidates
        .iter()
        .filter_map(|(branch, _)| jira_key_from_branch(branch))
        .collect::<HashSet<_>>();
    let recovered = if options.selected_branches.is_none() {
        recover_deleted_branch_tickets(
            &repository,
            environment_id,
            &main_ancestors,
            &covered_keys,
            options.jira_configured,
        )?
    } else {
        Vec::new()
    };
    let mut branches = candidates
        .iter()
        .map(|(name, _)| name.clone())
        .chain(recovered.iter().map(|row| row.branch.clone()))
        .collect::<Vec<_>>();
    branches.sort();
    updates
        .send(DiffUpdate::Skeleton {
            environment: options.environment.clone(),
            main: main.clone(),
            branches,
        })
        .map_err(|_| CliError::ReportCancelled)?;

    for (branch, id) in candidates {
        let jira = match jira_key_from_branch(&branch) {
            Some(key) if options.jira_configured => JiraIssueState::Loading { key },
            Some(key) => JiraIssueState::NotConfigured { key },
            None => JiraIssueState::NoTicket,
        };
        let row = measure_branch(
            &repository,
            &main_ancestors,
            &environment_markers,
            &environment_subjects,
            branch,
            id,
            jira,
        )?;
        updates
            .send(DiffUpdate::Measured(row))
            .map_err(|_| CliError::ReportCancelled)?;
    }
    for row in recovered {
        updates
            .send(DiffUpdate::Measured(row))
            .map_err(|_| CliError::ReportCancelled)?;
    }
    Ok(())
}

fn promotion_candidates(
    repository: &gix::Repository,
    prefix: &str,
    environment: &str,
    main: &str,
    environment_ancestors: &HashSet<gix::ObjectId>,
    main_ancestors: &HashSet<gix::ObjectId>,
    selected_branches: Option<&[String]>,
) -> Result<Vec<(String, gix::ObjectId)>, CliError> {
    if let Some(selected_branches) = selected_branches {
        let mut candidates = Vec::with_capacity(selected_branches.len());
        for branch in selected_branches {
            if excluded_branch(branch, environment, main) {
                return Err(CliError::InvalidInput(format!(
                    "--params branch `{branch}` is not a selectable feature branch"
                )));
            }
            let reference = format!("{prefix}{branch}");
            let id = reference_id(repository, &reference).map_err(|_| {
                CliError::InvalidInput(format!(
                    "--params branch `{branch}` does not exist on the selected remote"
                ))
            })?;
            if main_ancestors.contains(&id) {
                return Err(CliError::InvalidInput(format!(
                    "--params branch `{branch}` has already reached `{main}`"
                )));
            }
            if !environment_ancestors.contains(&id) {
                return Err(CliError::InvalidInput(format!(
                    "--params branch `{branch}` has not reached `{environment}`"
                )));
            }
            candidates.push((branch.clone(), id));
        }
        return Ok(candidates);
    }

    let mut candidates = Vec::new();
    let references = repository.references().map_err(gitoxide_error)?;
    let references = references.prefixed(prefix).map_err(gitoxide_error)?;
    for reference in references {
        let mut reference = reference.map_err(gitoxide_error)?;
        let full_name = reference.name().as_bstr().to_str_lossy();
        let Some(branch) = full_name.strip_prefix(prefix).map(str::to_owned) else {
            continue;
        };
        if excluded_branch(&branch, environment, main) {
            continue;
        }
        let id = reference.peel_to_id().map_err(gitoxide_error)?.detach();
        if environment_ancestors.contains(&id) && !main_ancestors.contains(&id) {
            candidates.push((branch, id));
        }
    }
    Ok(candidates)
}

fn resolve_main_branch(
    repository: &gix::Repository,
    prefix: &str,
    explicit: Option<&str>,
) -> Result<String, CliError> {
    if let Some(explicit) = explicit {
        reference_id(repository, &format!("{prefix}{explicit}"))?;
        return Ok(explicit.to_owned());
    }
    if let Ok(reference) = repository.find_reference(&format!("{prefix}HEAD")) {
        if let Some(target) = reference.target().try_name() {
            let target = target.as_bstr().to_str_lossy();
            if let Some(branch) = target.strip_prefix(prefix) {
                if reference_id(repository, &target).is_ok() {
                    return Ok(branch.to_owned());
                }
            }
        }
    }
    for candidate in ["main", "master", "trunk", "develop"] {
        if reference_id(repository, &format!("{prefix}{candidate}")).is_ok() {
            return Ok(candidate.to_owned());
        }
    }
    Err(CliError::Git(
        "could not determine the main branch from the remote default or common names; pass --main <BRANCH>"
            .to_owned(),
    ))
}

fn reference_id(repository: &gix::Repository, name: &str) -> Result<gix::ObjectId, CliError> {
    let mut reference = repository.find_reference(name).map_err(gitoxide_error)?;
    reference
        .peel_to_id()
        .map(|id| id.detach())
        .map_err(gitoxide_error)
}

fn ancestors(
    repository: &gix::Repository,
    start: gix::ObjectId,
) -> Result<HashSet<gix::ObjectId>, CliError> {
    let mut found = HashSet::new();
    let mut pending = VecDeque::from([start]);
    while let Some(id) = pending.pop_front() {
        if !found.insert(id) {
            continue;
        }
        let commit = repository.find_commit(id).map_err(gitoxide_error)?;
        pending.extend(commit.parent_ids().map(|parent| parent.detach()));
    }
    Ok(found)
}

/// Environment branch names to detect merges for: the requested environment
/// first, then the other known environments, never the main branch.
fn environment_names<'a>(environment: &'a str, main: &str) -> Vec<&'a str> {
    let mut names = vec![environment];
    names.extend(
        KNOWN_ENVIRONMENTS
            .iter()
            .copied()
            .filter(|name| *name != environment),
    );
    names.retain(|name| *name != main);
    names
}

/// Merge commits made on an environment branch's own first-parent line,
/// keyed by commit id and valued by the environment branch name.
///
/// A feature branch that can reach one of these commits has had that
/// environment branch merged into it. Merges that only pull main into the
/// environment are skipped, so a feature branch that merged main is never
/// flagged.
fn environment_merge_markers(
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
struct EnvironmentSubject {
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

fn environment_subjects(names: &[&str], remote: &str) -> Vec<EnvironmentSubject> {
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

fn measure_branch(
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
fn recover_deleted_branch_tickets(
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

fn excluded_branch(branch: &str, environment: &str, main: &str) -> bool {
    branch == "HEAD"
        || branch == environment
        || branch == main
        || KNOWN_ENVIRONMENTS.contains(&branch)
        || branch.starts_with("backup/")
}

fn unix_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn fetch_remote(args: &DiffArgs, interactive: bool) -> Result<(), CliError> {
    fetch_remote_name(&args.remote, interactive)
}

fn fetch_remote_name(remote: &str, interactive: bool) -> Result<(), CliError> {
    let pat = std::env::var("GIT_PAT")
        .ok()
        .filter(|value| !value.is_empty());
    if let Some(message) = fetch_status_message(remote, pat.is_some(), interactive) {
        eprintln!("{message}");
    }

    if let Some(pat) = pat {
        let mut authenticated = Command::new("git");
        authenticated
            .args([
                "-c",
                "credential.helper=",
                "-c",
                "credential.helper=!f() { echo username=x-access-token; echo \"password=$GIT_PAT\"; }; f",
                "fetch",
                "--prune",
                remote,
            ])
            .env("GIT_PAT", pat)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        return check_fetch(authenticated.output(), true, !interactive);
    }
    if !interactive {
        let mut unattended = Command::new("git");
        unattended
            .args([
                "-c",
                "credential.interactive=false",
                "fetch",
                "--prune",
                remote,
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        return check_fetch(unattended.output(), false, true);
    }
    let mut command = Command::new("git");
    command
        .args(["fetch", "--prune", remote])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    check_fetch(command.output(), false, false)
}

fn fetch_status_message(remote: &str, has_pat: bool, interactive: bool) -> Option<String> {
    if interactive {
        None
    } else if has_pat {
        Some(format!(
            "Contacting {remote} with the supplied PAT (non-interactive)…"
        ))
    } else {
        Some(format!(
            "Contacting {remote} (unattended; cached credentials only)…"
        ))
    }
}

fn check_fetch(
    result: io::Result<std::process::Output>,
    used_pat: bool,
    unattended: bool,
) -> Result<(), CliError> {
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) if used_pat => Err(CliError::Git(with_git_stderr(
            "could not fetch the remote; the PAT was rejected, expired, or lacks repository read access",
            &output,
        ))),
        Ok(output) if unattended => Err(CliError::Git(with_git_stderr(
            "could not fetch the remote with cached credentials; set GIT_PAT for a headless run",
            &output,
        ))),
        Ok(output) => Err(CliError::Git(with_git_stderr(
            "could not fetch the remote; complete authentication and run the command again",
            &output,
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(CliError::Git(
            "could not run git fetch because the git executable was not found".to_owned(),
        )),
        Err(error) => Err(CliError::Io(error)),
    }
}

fn with_git_stderr(message: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        message.to_owned()
    } else {
        format!("{message}\n{stderr}")
    }
}

fn parse_selected_branches(params: Option<&str>) -> Result<Option<Vec<String>>, CliError> {
    let Some(params) = params else {
        return Ok(None);
    };
    let parsed: DiffParams = serde_json::from_str(params).map_err(|error| {
        CliError::InvalidInput(format!(
            "--params must be a JSON object like {{\"branches\":[\"feature/A\",\"feature/B\"]}}: {error}"
        ))
    })?;
    if parsed.branches.is_empty() {
        return Err(CliError::InvalidInput(
            "--params branches must contain at least one feature branch".to_owned(),
        ));
    }
    for branch in &parsed.branches {
        validate_ref_component("--params branch", branch)?;
    }
    let mut branches = parsed.branches;
    branches.sort();
    branches.dedup();
    Ok(Some(branches))
}

fn validate_ref_component(label: &str, value: &str) -> Result<(), CliError> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || gix::validate::reference::name_partial(value.as_bytes().as_bstr()).is_err()
    {
        return Err(CliError::InvalidInput(format!(
            "{label} must be a non-empty Git branch or remote name"
        )));
    }
    Ok(())
}

fn gitoxide_error(error: impl std::fmt::Display) -> CliError {
    CliError::Git(error.to_string())
}

async fn collect_plain(
    mut updates: mpsc::UnboundedReceiver<DiffUpdate>,
) -> Result<PromotionReport, CliError> {
    let mut rows = HashMap::new();
    let mut environment = String::new();
    let mut main = String::new();
    let mut finished = false;
    while let Some(update) = updates.recv().await {
        match update {
            DiffUpdate::Skeleton {
                environment: next_environment,
                main: next_main,
                ..
            } => {
                environment = next_environment;
                main = next_main;
            }
            DiffUpdate::Measured(row) => {
                rows.insert(row.branch.clone(), row);
            }
            DiffUpdate::Jira { branch, state } => {
                if let Some(row) = rows.get_mut(&branch) {
                    row.jira = state;
                }
            }
            DiffUpdate::Finished => {
                finished = true;
                break;
            }
            DiffUpdate::Failed(message) => return Err(CliError::Git(message)),
        }
    }
    if !finished {
        return Err(CliError::Git(
            "promotion report ended before the scan completed".to_owned(),
        ));
    }
    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.branch.cmp(&right.branch));
    Ok(PromotionReport {
        environment,
        main,
        branches: rows,
    })
}

fn write_report(
    report: &PromotionReport,
    report_kind: DiffReport,
    format: ReportFormat,
    output: Option<&Path>,
) -> Result<(), CliError> {
    let content = match report_kind {
        DiffReport::Branches => match format {
            ReportFormat::Json => {
                format!("{}\n", serde_json::to_string_pretty(&report_value(report))?)
            }
            ReportFormat::Yaml => serde_yaml::to_string(&report_value(report))?,
            ReportFormat::Table => format_table(report),
            ReportFormat::Csv => format_csv(report)?,
        },
        DiffReport::Age => {
            let as_of = current_report_date()?;
            let age = PromotionAgeReport::new(&report.branches, as_of)
                .map_err(|error| CliError::Git(format!("could not build age report: {error}")))?;
            match format {
                ReportFormat::Json => format!(
                    "{}\n",
                    serde_json::to_string_pretty(&age_report_value(report, &age))?
                ),
                ReportFormat::Yaml => serde_yaml::to_string(&age_report_value(report, &age))?,
                ReportFormat::Table => format_age_table(report, &age),
                ReportFormat::Csv => format_age_csv(report, &age)?,
            }
        }
    };
    if let Some(output) = output {
        let output = validate_output_path(output)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".graduate-report-")
            .suffix(".tmp")
            .tempfile_in(&output.parent)?;
        temporary.write_all(content.as_bytes())?;
        temporary.as_file().sync_all()?;
        revalidate_output_parent(&output)?;
        temporary
            .persist(&output.destination)
            .map_err(|error| CliError::Io(error.error))?;
        eprintln!("Wrote {}", output.destination.display());
    } else {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        stdout.write_all(content.as_bytes())?;
    }
    Ok(())
}

pub(crate) fn current_report_date() -> Result<ReportDate, CliError> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CliError::Git(format!("system clock predates the Unix epoch: {error}")))?;
    let seconds = i64::try_from(elapsed.as_secs())
        .map_err(|_| CliError::Git("system clock is outside the supported range".to_owned()))?;
    ReportDate::parse(&unix_date(seconds))
        .map_err(|error| CliError::Git(format!("could not determine report date: {error}")))
}

fn report_value(report: &PromotionReport) -> serde_json::Value {
    let branches = report
        .branches
        .iter()
        .map(|branch| {
            let (issue, issue_key, jira_error) = match &branch.jira {
                JiraIssueState::Loaded(issue) => (
                    serde_json::json!({
                        "key": issue.key,
                        "self": issue.api_url,
                        "browseUrl": issue.url,
                        "fields": {
                            "summary": issue.summary,
                            "status": { "name": issue.status },
                            "assignee": issue.assignee.as_ref().map(|display_name| {
                                serde_json::json!({ "displayName": display_name })
                            }),
                            "fixVersions": issue.fix_versions.iter().map(|name| {
                                serde_json::json!({ "name": name })
                            }).collect::<Vec<_>>()
                        }
                    }),
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                ),
                JiraIssueState::Failed { key, message } => (
                    serde_json::Value::Null,
                    serde_json::json!(key),
                    serde_json::json!(message),
                ),
                JiraIssueState::NotConfigured { key }
                | JiraIssueState::Loading { key }
                | JiraIssueState::NotFound { key } => (
                    serde_json::Value::Null,
                    serde_json::json!(key),
                    serde_json::Value::Null,
                ),
                JiraIssueState::NoTicket => (
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                ),
            };
            serde_json::json!({
                "branch": branch.branch,
                "started": branch.started,
                "last": branch.last,
                "ahead": branch.ahead,
                "lastAuthor": branch.last_author,
                "mergedEnvironments": branch.merged_environments,
                "jiraIssue": issue,
                "jiraIssueKey": issue_key,
                "jiraError": jira_error,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "environment": report.environment,
        "main": report.main,
        "branches": branches,
    })
}

fn age_report_value(report: &PromotionReport, age: &PromotionAgeReport) -> serde_json::Value {
    let buckets = age
        .buckets
        .iter()
        .map(|bucket| {
            let period = serde_json::json!({ "kind": "year", "year": bucket.year });
            let assessment = if bucket.commits == 0 {
                serde_json::json!({ "kind": "noCommits", "summary": "No commits" })
            } else {
                match bucket.year {
                    year if year > age.as_of.year() => serde_json::json!({
                        "kind": "futureDated",
                        "summary": "Future-dated commits"
                    }),
                    year if year == age.as_of.year() => serde_json::json!({
                        "kind": "currentYear",
                        "summary": "Plausibly in flight"
                    }),
                    year if year == age.as_of.year() - 1 => serde_json::json!({
                        "kind": "mostlyOverOneYearOld",
                        "summary": "Mostly over a year old"
                    }),
                    year => {
                        let years = age.as_of.year().saturating_sub(year);
                        serde_json::json!({
                            "kind": "yearsOld",
                            "years": years,
                            "summary": age_bucket_reading(age, bucket)
                        })
                    }
                }
            };
            serde_json::json!({
                "period": period,
                "commits": bucket.commits,
                "sharePercent": share_percent(bucket.commits, age.total_commits),
                "assessment": assessment,
            })
        })
        .collect::<Vec<_>>();
    let oldest_branches = age
        .oldest_branches
        .iter()
        .map(|branch| {
            serde_json::json!({
                "branch": branch.branch,
                "commits": branch.commits,
                "oldestCommit": branch.oldest.to_string(),
                "newestCommit": branch.newest.to_string(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "schemaVersion": 1,
        "report": "age",
        "environment": report.environment,
        "main": report.main,
        "asOf": age.as_of.to_string(),
        "counting": "uniqueCommitsAcrossBranches",
        "totalCommits": age.total_commits,
        "oldestYear": age.oldest_year(),
        "buckets": buckets,
        "thresholds": {
            "last90Days": {
                "since": age.last_90_days.since.to_string(),
                "inclusive": true,
                "commits": age.last_90_days.commits,
                "sharePercent": share_percent(age.last_90_days.commits, age.total_commits),
                "assessment": {
                    "kind": "genuinelyInFlight",
                    "summary": "Genuinely in flight"
                }
            },
            "olderThanOneYear": {
                "before": age.older_than_one_year.before.to_string(),
                "exclusive": true,
                "commits": age.older_than_one_year.commits,
                "sharePercent": share_percent(
                    age.older_than_one_year.commits,
                    age.total_commits
                ),
                "assessment": {
                    "kind": "decisionRequired",
                    "summary": "Will not ship without a decision"
                }
            }
        },
        "oldestBranches": oldest_branches,
    })
}

pub(crate) fn age_bucket_label(year: i32) -> String {
    year.to_string()
}

pub(crate) fn age_bucket_reading(age: &PromotionAgeReport, bucket: &AgeBucket) -> String {
    if bucket.commits == 0 {
        return "No commits".to_owned();
    }
    match bucket.year {
        year if year > age.as_of.year() => "Future-dated commits".to_owned(),
        year if year == age.as_of.year() => "Current year — plausibly in flight".to_owned(),
        year if year == age.as_of.year() - 1 => "Mostly over a year old".to_owned(),
        year => {
            let years = age.as_of.year().saturating_sub(year);
            format!("{years} years old")
        }
    }
}

pub(crate) fn share_percent(commits: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let tenths = ((commits as u128) * 1_000 + (total as u128) / 2) / (total as u128);
    tenths as f64 / 10.0
}

fn format_table(report: &PromotionReport) -> String {
    let mut output = format!(
        "Branches in {} but not {}\n{:<36} {:<10} {:<10} {:>5}  {:<12} {:<14} LAST AUTHOR\n",
        crate::terminal_text::escape(&report.environment),
        crate::terminal_text::escape(&report.main),
        "BRANCH",
        "STARTED",
        "LAST",
        "AHEAD",
        "JIRA",
        "STATUS"
    );
    if report.branches.is_empty() {
        output.push_str("(nothing to promote)\n");
    }
    for row in &report.branches {
        // Only a Jira-validated ticket key may appear in the JIRA column.
        let key = match &row.jira {
            JiraIssueState::Loaded(issue) => crate::terminal_text::escape(&issue.key),
            _ => String::new(),
        };
        let status = match &row.jira {
            JiraIssueState::Loaded(issue) => issue.status.as_str(),
            JiraIssueState::Failed { .. } => "error",
            JiraIssueState::NotFound { .. } | JiraIssueState::NoTicket => "not found",
            JiraIssueState::NotConfigured { .. } => "not configured",
            JiraIssueState::Loading { .. } => "loading",
        };
        output.push_str(&format!(
            "{:<36} {:<10} {:<10} {:>5}  {:<12} {:<14} {}\n",
            crate::terminal_text::escape(&row.branch),
            row.started,
            row.last,
            row.ahead,
            key,
            crate::terminal_text::escape(status),
            crate::terminal_text::escape(&row.last_author)
        ));
    }
    output
}

fn format_age_table(report: &PromotionReport, age: &PromotionAgeReport) -> String {
    let mut output = format!(
        "Age of unshipped work in {} but not {} (as of {})\n\
         All {} unique authored commits across {} branches.\n\
         {:<24} {:>10} {:>8}  READING\n",
        crate::terminal_text::escape(&report.environment),
        crate::terminal_text::escape(&report.main),
        age.as_of,
        age.total_commits,
        report.branches.len(),
        "WRITTEN IN",
        "COMMITS",
        "SHARE"
    );
    for bucket in &age.buckets {
        output.push_str(&format!(
            "{:<24} {:>10} {:>7.1}%  {}\n",
            age_bucket_label(bucket.year),
            bucket.commits,
            share_percent(bucket.commits, age.total_commits),
            age_bucket_reading(age, bucket)
        ));
    }
    output.push_str(&format!(
        "{:<24} {:>10} {:>7.1}%  Genuinely in flight\n",
        "Written in last 90 days",
        age.last_90_days.commits,
        share_percent(age.last_90_days.commits, age.total_commits)
    ));
    output.push_str(&format!(
        "{:<24} {:>10} {:>7.1}%  Will not ship without a decision\n",
        "Older than one year",
        age.older_than_one_year.commits,
        share_percent(age.older_than_one_year.commits, age.total_commits)
    ));

    if let Some(oldest_year) = age.oldest_year() {
        output.push_str(&format!(
            "\nBranches carrying commits from {oldest_year}\n{:<36} {:>8}  {:<10}  NEWEST\n",
            "BRANCH", "COMMITS", "OLDEST"
        ));
        if age.oldest_branches.is_empty() {
            output.push_str("(none)\n");
        }
        for branch in &age.oldest_branches {
            output.push_str(&format!(
                "{:<36} {:>8}  {:<10}  {}\n",
                crate::terminal_text::escape(&branch.branch),
                branch.commits,
                branch.oldest,
                branch.newest
            ));
        }
    }
    output
}

fn format_csv(report: &PromotionReport) -> Result<String, CliError> {
    let mut file = Vec::new();
    csv_row(
        &mut file,
        &[
            "branch",
            "started",
            "last",
            "ahead",
            "lastAuthor",
            "mergedEnvironments",
            "jiraIssue.key",
            "jiraIssue.fields.status.name",
            "jiraIssue.fields.summary",
            "jiraIssue.fields.assignee.displayName",
            "jiraIssue.fields.fixVersions",
            "jiraIssue.self",
            "jiraIssue.browseUrl",
            "jiraError",
        ],
    )?;
    for row in &report.branches {
        let ahead = row.ahead.to_string();
        let merged_environments = row.merged_environments.join(", ");
        let (key, status, summary, assignee, versions, api_url, browse_url, jira_error) =
            match &row.jira {
                JiraIssueState::Loaded(issue) => (
                    issue.key.clone(),
                    issue.status.clone(),
                    issue.summary.clone(),
                    issue.assignee.clone().unwrap_or_default(),
                    issue.fix_versions.join(", "),
                    issue.api_url.clone(),
                    issue.url.clone(),
                    String::new(),
                ),
                JiraIssueState::Failed { key, message } => (
                    key.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    message.clone(),
                ),
                JiraIssueState::NotConfigured { key } => (
                    key.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                JiraIssueState::Loading { key } => (
                    key.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                JiraIssueState::NotFound { key } => (
                    key.clone(),
                    "not found".to_owned(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                JiraIssueState::NoTicket => (
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
            };
        csv_row(
            &mut file,
            &[
                &row.branch,
                &row.started,
                &row.last,
                &ahead,
                &row.last_author,
                &merged_environments,
                &key,
                &status,
                &summary,
                &assignee,
                &versions,
                &api_url,
                &browse_url,
                &jira_error,
            ],
        )?;
    }
    String::from_utf8(file)
        .map_err(|error| CliError::InvalidInput(format!("CSV was not valid UTF-8: {error}")))
}

fn format_age_csv(report: &PromotionReport, age: &PromotionAgeReport) -> Result<String, CliError> {
    let mut file = Vec::new();
    csv_row(
        &mut file,
        &[
            "rowType",
            "environment",
            "main",
            "asOf",
            "counting",
            "period",
            "year",
            "since",
            "before",
            "branch",
            "commits",
            "totalCommits",
            "sharePercent",
            "oldestCommit",
            "newestCommit",
            "assessment",
        ],
    )?;
    let total = age.total_commits.to_string();
    let as_of = age.as_of.to_string();
    for bucket in &age.buckets {
        let year = bucket.year.to_string();
        let commits = bucket.commits.to_string();
        let share = format!("{:.1}", share_percent(bucket.commits, age.total_commits));
        csv_row(
            &mut file,
            &[
                "bucket",
                &report.environment,
                &report.main,
                &as_of,
                "uniqueCommitsAcrossBranches",
                "year",
                &year,
                "",
                "",
                "",
                &commits,
                &total,
                &share,
                "",
                "",
                &age_bucket_reading(age, bucket),
            ],
        )?;
    }
    let recent_commits = age.last_90_days.commits.to_string();
    let recent_share = format!(
        "{:.1}",
        share_percent(age.last_90_days.commits, age.total_commits)
    );
    csv_row(
        &mut file,
        &[
            "threshold",
            &report.environment,
            &report.main,
            &as_of,
            "uniqueCommitsAcrossBranches",
            "last90Days",
            "",
            &age.last_90_days.since.to_string(),
            "",
            "",
            &recent_commits,
            &total,
            &recent_share,
            "",
            "",
            "Genuinely in flight",
        ],
    )?;
    let older_commits = age.older_than_one_year.commits.to_string();
    let older_share = format!(
        "{:.1}",
        share_percent(age.older_than_one_year.commits, age.total_commits)
    );
    csv_row(
        &mut file,
        &[
            "threshold",
            &report.environment,
            &report.main,
            &as_of,
            "uniqueCommitsAcrossBranches",
            "olderThanOneYear",
            "",
            "",
            &age.older_than_one_year.before.to_string(),
            "",
            &older_commits,
            &total,
            &older_share,
            "",
            "",
            "Will not ship without a decision",
        ],
    )?;
    for branch in &age.oldest_branches {
        let commits = branch.commits.to_string();
        csv_row(
            &mut file,
            &[
                "oldestBranch",
                &report.environment,
                &report.main,
                &as_of,
                "uniqueCommitsAcrossBranches",
                "",
                "",
                "",
                "",
                &branch.branch,
                &commits,
                &total,
                "",
                &branch.oldest.to_string(),
                &branch.newest.to_string(),
                "",
            ],
        )?;
    }
    String::from_utf8(file)
        .map_err(|error| CliError::InvalidInput(format!("CSV was not valid UTF-8: {error}")))
}

struct ValidatedOutput {
    destination: PathBuf,
    parent: PathBuf,
    repository: PathBuf,
}

fn validate_output_path(path: &Path) -> Result<ValidatedOutput, CliError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
        || path.to_string_lossy().chars().any(char::is_control)
    {
        return Err(CliError::InvalidInput(
            "--output must be a relative file path within the current directory".to_owned(),
        ));
    }
    let current = std::env::current_dir()?.canonicalize()?;
    let lexical_destination = current.join(path);
    let parent = lexical_destination
        .parent()
        .ok_or_else(|| CliError::InvalidInput("--output must name a file".to_owned()))?;
    let parent = parent.canonicalize().map_err(|error| {
        CliError::InvalidInput(format!(
            "--output parent directory must already exist: {error}"
        ))
    })?;
    if !parent.starts_with(&current) {
        return Err(CliError::InvalidInput(
            "--output must stay within the current directory".to_owned(),
        ));
    }
    let leaf = lexical_destination
        .file_name()
        .ok_or_else(|| CliError::InvalidInput("--output must name a file".to_owned()))?;
    let destination = parent.join(leaf);
    if std::fs::symlink_metadata(&destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(CliError::InvalidInput(
            "--output must not replace a symbolic link".to_owned(),
        ));
    }
    Ok(ValidatedOutput {
        destination,
        parent,
        repository: current,
    })
}

fn revalidate_output_parent(output: &ValidatedOutput) -> Result<(), CliError> {
    let parent = output.parent.canonicalize()?;
    if parent != output.parent || !parent.starts_with(&output.repository) {
        return Err(CliError::InvalidInput(
            "--output parent directory changed while the report was being written".to_owned(),
        ));
    }
    if std::fs::symlink_metadata(&output.destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(CliError::InvalidInput(
            "--output must not replace a symbolic link".to_owned(),
        ));
    }
    Ok(())
}

fn csv_row(writer: &mut impl Write, fields: &[&str]) -> Result<(), CliError> {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        writer.write_all(b"\"")?;
        writer.write_all(field.replace('"', "\"\"").as_bytes())?;
        writer.write_all(b"\"")?;
    }
    writer.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn interactive_fetch_does_not_print_a_status_message() {
        assert_eq!(fetch_status_message("origin", false, true), None);
        assert_eq!(fetch_status_message("origin", true, true), None);
    }

    #[test]
    fn jira_404_becomes_a_not_found_issue_state() {
        let state = jira_issue_state("PROJ-404".to_owned(), Err(CliError::JiraStatus(404)));

        assert_eq!(
            state,
            JiraIssueState::NotFound {
                key: "PROJ-404".to_owned()
            }
        );
    }

    #[test]
    fn dates_are_formatted_without_local_timezone_drift() {
        assert_eq!(unix_date(0), "1970-01-01");
        assert_eq!(unix_date(1_704_067_200), "2024-01-01");
    }

    #[test]
    fn csv_quotes_every_field_and_doubles_quotes() -> Result<(), CliError> {
        let mut output = Vec::new();
        csv_row(&mut output, &["branch", "O\"Brien, Pat"])?;
        assert_eq!(output, b"\"branch\",\"O\"\"Brien, Pat\"\n");
        Ok(())
    }

    #[test]
    fn csv_keeps_jira_errors_out_of_issue_fields() -> Result<(), CliError> {
        let report = PromotionReport {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                commits: Vec::new(),
                merged_environments: vec!["qa".to_owned()],
                jira: JiraIssueState::Failed {
                    key: "PROJ-123".to_owned(),
                    message: "request timed out".to_owned(),
                },
            }],
        };

        let csv = format_csv(&report)?;

        assert!(csv
            .lines()
            .next()
            .is_some_and(|line| line.ends_with("\"jiraError\"")));
        assert!(csv.lines().nth(1).is_some_and(|line| {
            line.contains("\"qa\",\"PROJ-123\",\"\",\"\"")
                && line.ends_with("\"request timed out\"")
        }));
        Ok(())
    }

    #[tokio::test]
    async fn plain_reports_require_an_explicit_finished_update() {
        let (sender, receiver) = mpsc::unbounded_channel();
        assert!(sender
            .send(DiffUpdate::Skeleton {
                environment: "qa".to_owned(),
                main: "main".to_owned(),
                branches: Vec::new(),
            })
            .is_ok());
        drop(sender);

        let result = collect_plain(receiver).await;

        assert!(
            matches!(result, Err(CliError::Git(message)) if message.contains("before the scan completed"))
        );
    }

    #[test]
    fn output_rejects_paths_outside_the_current_directory() {
        assert!(validate_output_path(Path::new("../report.json")).is_err());
        assert!(validate_output_path(Path::new("/tmp/report.json")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn output_uses_the_canonical_parent_for_internal_symlinks(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let current = std::env::current_dir()?;
        let directory = tempfile::tempdir_in(&current)?;
        let reports = directory.path().join("reports");
        std::fs::create_dir(&reports)?;
        symlink(&reports, directory.path().join("report-link"))?;
        let relative = directory
            .path()
            .strip_prefix(&current)?
            .join("report-link/qa.json");

        let output = validate_output_path(&relative)?;

        assert_eq!(output.destination, reports.canonicalize()?.join("qa.json"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn output_rejects_a_parent_symlink_that_escapes_the_repository(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let current = std::env::current_dir()?;
        let directory = tempfile::tempdir_in(&current)?;
        let outside = tempfile::tempdir()?;
        symlink(outside.path(), directory.path().join("outside"))?;
        let relative = directory
            .path()
            .strip_prefix(&current)?
            .join("outside/report.json");

        assert!(validate_output_path(&relative).is_err());
        Ok(())
    }

    #[test]
    fn json_report_uses_camel_case_and_jira_api_field_shapes() {
        let report = PromotionReport {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![PromotionBranch {
                branch: "feature/PROJ-123-login".to_owned(),
                started: "2024-01-01".to_owned(),
                last: "2024-01-02".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                commits: Vec::new(),
                merged_environments: vec!["qa".to_owned()],
                jira: JiraIssueState::Loaded(graduate::promotion::JiraIssueSummary {
                    key: "PROJ-123".to_owned(),
                    api_url: "https://example.atlassian.net/rest/api/3/issue/10001".to_owned(),
                    summary: "Add login".to_owned(),
                    status: "Ready for QA".to_owned(),
                    assignee: Some("Pat".to_owned()),
                    fix_versions: vec!["1.2".to_owned()],
                    url: "https://example.atlassian.net/browse/PROJ-123".to_owned(),
                }),
            }],
        };

        let value = report_value(&report);

        assert_eq!(value["branches"][0]["lastAuthor"], "Pat");
        assert_eq!(value["branches"][0]["mergedEnvironments"][0], "qa");
        assert_eq!(
            value["branches"][0]["jiraIssue"]["fields"]["assignee"]["displayName"],
            "Pat"
        );
        assert_eq!(
            value["branches"][0]["jiraIssue"]["fields"]["fixVersions"][0]["name"],
            "1.2"
        );
    }

    #[test]
    fn age_json_is_self_describing_and_uses_explicit_thresholds(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let report = PromotionReport {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![PromotionBranch {
                branch: "feature/legacy".to_owned(),
                started: "2019-12-31".to_owned(),
                last: "2026-08-01".to_owned(),
                ahead: 2,
                last_author: "Pat".to_owned(),
                commits: vec![
                    PromotionCommit {
                        id: "111111111111".to_owned(),
                        short_id: "1111111".to_owned(),
                        subject: "Current".to_owned(),
                        author: "Pat".to_owned(),
                        date: "2026-08-01".to_owned(),
                    },
                    PromotionCommit {
                        id: "222222222222".to_owned(),
                        short_id: "2222222".to_owned(),
                        subject: "Legacy".to_owned(),
                        author: "Pat".to_owned(),
                        date: "2019-12-31".to_owned(),
                    },
                ],
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }],
        };
        let age = PromotionAgeReport::new(&report.branches, ReportDate::parse("2026-08-04")?)?;

        let value = age_report_value(&report, &age);

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["report"], "age");
        assert_eq!(value["counting"], "uniqueCommitsAcrossBranches");
        assert_eq!(value["asOf"], "2026-08-04");
        assert_eq!(value["totalCommits"], 2);
        assert_eq!(value["oldestYear"], 2019);
        assert_eq!(value["buckets"].as_array().map(Vec::len), Some(2));
        assert_eq!(value["buckets"][0]["period"]["kind"], "year");
        assert_eq!(value["buckets"][0]["period"]["year"], 2026);
        assert_eq!(value["buckets"][1]["period"]["year"], 2019);
        assert_eq!(value["buckets"][0]["sharePercent"], 50.0);
        assert_eq!(value["thresholds"]["last90Days"]["since"], "2026-05-07");
        assert_eq!(
            value["thresholds"]["olderThanOneYear"]["before"],
            "2025-08-04"
        );
        assert_eq!(value["oldestBranches"][0]["branch"], "feature/legacy");
        Ok(())
    }

    #[test]
    fn age_table_calls_out_old_work_and_the_branches_that_carry_it(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let report = PromotionReport {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![PromotionBranch {
                branch: "feature/legacy".to_owned(),
                started: "2019-12-31".to_owned(),
                last: "2019-12-31".to_owned(),
                ahead: 1,
                last_author: "Pat".to_owned(),
                commits: vec![PromotionCommit {
                    id: "222222222222".to_owned(),
                    short_id: "2222222".to_owned(),
                    subject: "Legacy".to_owned(),
                    author: "Pat".to_owned(),
                    date: "2019-12-31".to_owned(),
                }],
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }],
        };
        let age = PromotionAgeReport::new(&report.branches, ReportDate::parse("2026-08-04")?)?;

        let table = format_age_table(&report, &age);

        assert!(table.contains("Age of unshipped work in qa but not main"));
        assert!(table.contains("2019"));
        assert!(!table.contains("Before 2020"));
        assert!(table.contains("Older than one year"));
        assert!(table.contains("Will not ship without a decision"));
        assert!(table.contains("Branches carrying commits from 2019"));
        assert!(table.contains("feature/legacy"));
        Ok(())
    }

    #[test]
    fn age_csv_identifies_bucket_and_threshold_rows() -> Result<(), Box<dyn std::error::Error>> {
        let report = PromotionReport {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![PromotionBranch {
                branch: "feature/current".to_owned(),
                started: "2026-08-01".to_owned(),
                last: "2026-08-01".to_owned(),
                ahead: 1,
                last_author: "Pat".to_owned(),
                commits: vec![PromotionCommit {
                    id: "111111111111".to_owned(),
                    short_id: "1111111".to_owned(),
                    subject: "Current".to_owned(),
                    author: "Pat".to_owned(),
                    date: "2026-08-01".to_owned(),
                }],
                merged_environments: Vec::new(),
                jira: JiraIssueState::NoTicket,
            }],
        };
        let age = PromotionAgeReport::new(&report.branches, ReportDate::parse("2026-08-04")?)?;

        let csv = format_age_csv(&report, &age)?;

        assert!(csv.lines().next().is_some_and(|header| {
            header.contains("\"rowType\"") && header.contains("\"assessment\"")
        }));
        assert!(csv.contains(
            "\"bucket\",\"qa\",\"main\",\"2026-08-04\",\"uniqueCommitsAcrossBranches\",\"year\",\"2026\""
        ));
        assert!(csv.contains("\"threshold\",\"qa\",\"main\",\"2026-08-04\",\"uniqueCommitsAcrossBranches\",\"last90Days\""));
        assert!(csv.contains("\"threshold\",\"qa\",\"main\",\"2026-08-04\",\"uniqueCommitsAcrossBranches\",\"olderThanOneYear\""));
        Ok(())
    }

    #[test]
    fn known_environment_and_backup_refs_are_excluded() {
        assert!(excluded_branch("qa", "qa", "main"));
        assert!(excluded_branch("backup/old", "qa", "main"));
        assert!(!excluded_branch("feature/PROJ-1", "qa", "main"));
    }

    #[test]
    fn json_params_select_sort_and_deduplicate_branches() -> Result<(), Box<dyn std::error::Error>>
    {
        let branches = parse_selected_branches(Some(
            r#"{"branches":["feature/PROJ-2","feature/PROJ-1","feature/PROJ-2"]}"#,
        ))?
        .ok_or("JSON params did not select branches")?;

        assert_eq!(branches, ["feature/PROJ-1", "feature/PROJ-2"]);
        assert!(parse_selected_branches(None)?.is_none());
        Ok(())
    }

    #[test]
    fn json_params_reject_empty_or_unknown_selection_fields() {
        let empty = parse_selected_branches(Some(r#"{"branches":[]}"#))
            .err()
            .map(|error| error.to_string());
        let unknown = parse_selected_branches(Some(
            r#"{"branches":["feature/PROJ-1"],"branch":"feature/PROJ-2"}"#,
        ))
        .err()
        .map(|error| error.to_string());
        let invalid_ref = parse_selected_branches(Some(r#"{"branches":["feature/bad branch"]}"#))
            .err()
            .map(|error| error.to_string());

        assert!(empty.is_some_and(|message| message.contains("at least one feature branch")));
        assert!(unknown.is_some_and(|message| message.contains("unknown field `branch`")));
        assert!(invalid_ref.is_some_and(|message| message.contains("Git branch or remote name")));
    }

    #[test]
    fn stale_remote_head_falls_back_to_an_existing_main_name(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("file"), "base\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["update-ref", "refs/remotes/origin/main", "refs/heads/main"],
        )?;
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/old-main",
            ],
        )?;
        let repository = gix::discover(directory.path())?;

        let main = resolve_main_branch(&repository, "refs/remotes/origin/", None)?;

        assert_eq!(main, "main");
        Ok(())
    }

    #[test]
    fn scan_finds_a_feature_in_environment_but_not_main() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("file"), "base\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-123-login"],
        )?;
        std::fs::write(directory.path().join("file"), "base\nfeature\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "feature", "2024-02-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-123-login",
                "-m",
                "promote",
            ],
        )?;
        for branch in ["main", "qa", "feature/PROJ-123-login"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();

        assert!(matches!(
            updates.first(),
            Some(DiffUpdate::Skeleton { branches, main, .. })
                if branches == &["feature/PROJ-123-login"] && main == "main"
        ));
        assert!(matches!(
            updates.get(1),
            Some(DiffUpdate::Measured(PromotionBranch {
                branch,
                ahead: 1,
                started,
                commits,
                jira: JiraIssueState::NotConfigured { key },
                ..
            })) if branch == "feature/PROJ-123-login"
                && started == "2024-02-01"
                && commits.len() == 1
                && commits[0].subject == "feature"
                && commits[0].author == "Test Author"
                && commits[0].date == "2024-02-01"
                && commits[0].short_id.len() == 7
                && key == "PROJ-123"
        ));
        Ok(())
    }

    #[test]
    fn scan_can_scope_a_report_to_multiple_json_selected_branches(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("base"), "base\n")?;
        run_git(directory.path(), &["add", "base"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(directory.path(), &["branch", "qa", "main"])?;

        for (branch, file, date) in [
            ("feature/PROJ-1-first", "first", "2024-02-01T00:00:00Z"),
            ("feature/PROJ-2-second", "second", "2024-02-02T00:00:00Z"),
            ("feature/PROJ-3-third", "third", "2024-02-03T00:00:00Z"),
        ] {
            run_git(directory.path(), &["checkout", "-q", "main"])?;
            run_git(directory.path(), &["checkout", "-q", "-b", branch])?;
            std::fs::write(directory.path().join(file), format!("{file}\n"))?;
            run_git(directory.path(), &["add", file])?;
            commit(directory.path(), file, date)?;
            run_git(directory.path(), &["checkout", "-q", "qa"])?;
            run_git(
                directory.path(),
                &["merge", "-q", "--no-ff", branch, "-m", "promote"],
            )?;
        }
        for branch in [
            "main",
            "qa",
            "feature/PROJ-1-first",
            "feature/PROJ-2-second",
            "feature/PROJ-3-third",
        ] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: Some(vec![
                    "feature/PROJ-3-third".to_owned(),
                    "feature/PROJ-1-first".to_owned(),
                ]),
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
        let measured = updates
            .iter()
            .filter_map(|update| match update {
                DiffUpdate::Measured(row) => Some(row.branch.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(matches!(
            updates.first(),
            Some(DiffUpdate::Skeleton { branches, .. })
                if branches == &["feature/PROJ-1-first", "feature/PROJ-3-third"]
        ));
        assert_eq!(measured, ["feature/PROJ-1-first", "feature/PROJ-3-third"]);

        let (missing_sender, _missing_receiver) = mpsc::unbounded_channel();
        let missing = scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: Some(vec!["feature/PROJ-404-missing".to_owned()]),
            },
            &missing_sender,
        )
        .err();
        assert!(matches!(
            missing,
            Some(CliError::InvalidInput(message)) if message.contains("does not exist")
        ));
        Ok(())
    }

    #[test]
    fn merge_commits_are_excluded_from_the_ahead_count_and_history(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("file"), "base\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-123-login"],
        )?;
        std::fs::write(directory.path().join("feature-file"), "feature\n")?;
        run_git(directory.path(), &["add", "feature-file"])?;
        commit(directory.path(), "feature", "2024-02-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "main"])?;
        std::fs::write(directory.path().join("main-file"), "main\n")?;
        run_git(directory.path(), &["add", "main-file"])?;
        commit(directory.path(), "main work", "2024-02-02T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "feature/PROJ-123-login"],
        )?;
        run_git(
            directory.path(),
            &["merge", "-q", "--no-ff", "main", "-m", "sync main"],
        )?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-123-login",
                "-m",
                "promote",
            ],
        )?;
        for branch in ["main", "qa", "feature/PROJ-123-login"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();

        assert!(matches!(
            updates.get(1),
            Some(DiffUpdate::Measured(PromotionBranch {
                branch,
                ahead: 1,
                commits,
                ..
            })) if branch == "feature/PROJ-123-login"
                && commits.len() == 1
                && commits[0].subject == "feature"
        ));
        Ok(())
    }

    #[test]
    fn environment_merges_into_a_feature_branch_are_flagged(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("base"), "base\n")?;
        run_git(directory.path(), &["add", "base"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-200-first"],
        )?;
        std::fs::write(directory.path().join("one"), "one\n")?;
        run_git(directory.path(), &["add", "one"])?;
        commit(directory.path(), "first feature", "2024-02-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-200-first",
                "-m",
                "promote first",
            ],
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-201-second", "main"],
        )?;
        std::fs::write(directory.path().join("two"), "two\n")?;
        run_git(directory.path(), &["add", "two"])?;
        commit(directory.path(), "second feature", "2024-02-02T00:00:00Z")?;
        run_git(
            directory.path(),
            &["merge", "-q", "--no-ff", "qa", "-m", "sync qa"],
        )?;
        run_git(directory.path(), &["checkout", "-q", "qa"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-201-second",
                "-m",
                "promote second",
            ],
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-202-third", "main"],
        )?;
        std::fs::write(directory.path().join("three"), "three\n")?;
        run_git(directory.path(), &["add", "three"])?;
        commit(directory.path(), "third feature", "2024-02-03T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "main"])?;
        std::fs::write(directory.path().join("main-file"), "main\n")?;
        run_git(directory.path(), &["add", "main-file"])?;
        commit(directory.path(), "main work", "2024-02-04T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "feature/PROJ-202-third"],
        )?;
        run_git(
            directory.path(),
            &["merge", "-q", "--no-ff", "main", "-m", "sync main"],
        )?;
        run_git(directory.path(), &["checkout", "-q", "qa"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-202-third",
                "-m",
                "promote third",
            ],
        )?;
        for branch in [
            "main",
            "qa",
            "feature/PROJ-200-first",
            "feature/PROJ-201-second",
            "feature/PROJ-202-third",
        ] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
        let merged_environments = |branch: &str| {
            updates.iter().find_map(|update| match update {
                DiffUpdate::Measured(row) if row.branch == branch => {
                    Some(row.merged_environments.clone())
                }
                _ => None,
            })
        };

        assert_eq!(
            merged_environments("feature/PROJ-200-first").as_deref(),
            Some(&[][..])
        );
        assert_eq!(
            merged_environments("feature/PROJ-201-second").as_deref(),
            Some(&["qa".to_owned()][..])
        );
        assert_eq!(
            merged_environments("feature/PROJ-202-third").as_deref(),
            Some(&[][..])
        );
        Ok(())
    }

    #[test]
    fn environment_merges_hidden_by_an_environment_rebuild_are_flagged(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("base"), "base\n")?;
        run_git(directory.path(), &["add", "base"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-300-noise"],
        )?;
        std::fs::write(directory.path().join("noise"), "noise\n")?;
        run_git(directory.path(), &["add", "noise"])?;
        commit(directory.path(), "noise feature", "2024-02-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-300-noise",
                "-m",
                "Merge branch 'feature/PROJ-300-noise' into qa",
            ],
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-301-stale", "qa"],
        )?;
        std::fs::write(directory.path().join("stale"), "stale\n")?;
        run_git(directory.path(), &["add", "stale"])?;
        commit(directory.path(), "stale feature", "2024-02-02T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "qa-rebuild", "main"],
        )?;
        std::fs::write(directory.path().join("rebuild"), "rebuild\n")?;
        run_git(directory.path(), &["add", "rebuild"])?;
        commit(directory.path(), "rebuild base", "2024-02-03T00:00:00Z")?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "qa",
                "-m",
                "Merge branch 'qa' of https://example.com/repo into qa",
            ],
        )?;
        run_git(
            directory.path(),
            &["branch", "-q", "-f", "qa", "qa-rebuild"],
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-302-clean", "main"],
        )?;
        std::fs::write(directory.path().join("clean"), "clean\n")?;
        run_git(directory.path(), &["add", "clean"])?;
        commit(directory.path(), "clean feature", "2024-02-04T00:00:00Z")?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-300-noise",
                "-m",
                "Merge branch 'feature/PROJ-300-noise' into feature/PROJ-302-clean",
            ],
        )?;
        run_git(directory.path(), &["checkout", "-q", "qa"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-301-stale",
                "-m",
                "promote stale",
            ],
        )?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-302-clean",
                "-m",
                "promote clean",
            ],
        )?;
        for branch in [
            "main",
            "qa",
            "feature/PROJ-301-stale",
            "feature/PROJ-302-clean",
        ] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
        let merged_environments = |branch: &str| {
            updates.iter().find_map(|update| match update {
                DiffUpdate::Measured(row) if row.branch == branch => {
                    Some(row.merged_environments.clone())
                }
                _ => None,
            })
        };

        assert_eq!(
            merged_environments("feature/PROJ-301-stale").as_deref(),
            Some(&["qa".to_owned()][..])
        );
        assert_eq!(
            merged_environments("feature/PROJ-302-clean").as_deref(),
            Some(&[][..])
        );
        Ok(())
    }

    #[test]
    fn no_ff_environment_merges_survive_an_environment_reset(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("base"), "base\n")?;
        run_git(directory.path(), &["add", "base"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-400-noise"],
        )?;
        std::fs::write(directory.path().join("noise"), "noise\n")?;
        run_git(directory.path(), &["add", "noise"])?;
        commit(directory.path(), "noise feature", "2024-02-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-400-noise",
                "-m",
                "promote noise",
            ],
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-401-dependent", "main"],
        )?;
        std::fs::write(directory.path().join("dependent"), "dependent\n")?;
        run_git(directory.path(), &["add", "dependent"])?;
        commit(
            directory.path(),
            "dependent feature",
            "2024-02-02T00:00:00Z",
        )?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "qa",
                "-m",
                "Merge branch 'qa' into feature/PROJ-401-dependent",
            ],
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-402-sibling", "main"],
        )?;
        std::fs::write(directory.path().join("sibling"), "sibling\n")?;
        run_git(directory.path(), &["add", "sibling"])?;
        commit(directory.path(), "sibling feature", "2024-02-03T00:00:00Z")?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-400-noise",
                "-m",
                "Merge branch 'feature/PROJ-400-noise' into feature/PROJ-402-sibling",
            ],
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-403-remote", "main"],
        )?;
        std::fs::write(directory.path().join("remote"), "remote\n")?;
        run_git(directory.path(), &["add", "remote"])?;
        commit(directory.path(), "remote feature", "2024-02-04T00:00:00Z")?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "qa",
                "-m",
                "Merge remote-tracking branch 'origin/qa'",
            ],
        )?;
        // Reset the environment so the old promote-noise merge leaves its
        // first-parent line, then promote the features onto the new line.
        run_git(directory.path(), &["branch", "-q", "-f", "qa", "main"])?;
        run_git(directory.path(), &["checkout", "-q", "qa"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-401-dependent",
                "-m",
                "promote dependent",
            ],
        )?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-402-sibling",
                "-m",
                "promote sibling",
            ],
        )?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-403-remote",
                "-m",
                "promote remote",
            ],
        )?;
        for branch in [
            "main",
            "qa",
            "feature/PROJ-401-dependent",
            "feature/PROJ-402-sibling",
            "feature/PROJ-403-remote",
        ] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
        let merged_environments = |branch: &str| {
            updates.iter().find_map(|update| match update {
                DiffUpdate::Measured(row) if row.branch == branch => {
                    Some(row.merged_environments.clone())
                }
                _ => None,
            })
        };

        assert_eq!(
            merged_environments("feature/PROJ-401-dependent").as_deref(),
            Some(&["qa".to_owned()][..])
        );
        assert_eq!(
            merged_environments("feature/PROJ-402-sibling").as_deref(),
            Some(&[][..])
        );
        assert_eq!(
            merged_environments("feature/PROJ-403-remote").as_deref(),
            Some(&["qa".to_owned()][..])
        );
        Ok(())
    }

    #[test]
    fn main_merges_and_environment_like_branch_names_are_never_flagged(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("base"), "base\n")?;
        run_git(directory.path(), &["add", "base"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/qa", "main"],
        )?;
        std::fs::write(directory.path().join("lookalike"), "lookalike\n")?;
        run_git(directory.path(), &["add", "lookalike"])?;
        commit(
            directory.path(),
            "lookalike feature",
            "2024-02-01T00:00:00Z",
        )?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-500-clean", "main"],
        )?;
        std::fs::write(directory.path().join("clean"), "clean\n")?;
        run_git(directory.path(), &["add", "clean"])?;
        commit(directory.path(), "clean feature", "2024-02-02T00:00:00Z")?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/qa",
                "-m",
                "Merge remote-tracking branch 'origin/feature/qa'",
            ],
        )?;
        run_git(directory.path(), &["checkout", "-q", "main"])?;
        std::fs::write(directory.path().join("mainline"), "mainline\n")?;
        run_git(directory.path(), &["add", "mainline"])?;
        commit(directory.path(), "main work", "2024-02-03T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "feature/PROJ-500-clean"],
        )?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "main",
                "-m",
                "Merge branch 'main' into feature/PROJ-500-clean",
            ],
        )?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-500-clean",
                "-m",
                "promote clean",
            ],
        )?;
        for branch in ["main", "qa", "feature/PROJ-500-clean"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
        let merged_environments = |branch: &str| {
            updates.iter().find_map(|update| match update {
                DiffUpdate::Measured(row) if row.branch == branch => {
                    Some(row.merged_environments.clone())
                }
                _ => None,
            })
        };

        assert_eq!(
            merged_environments("feature/PROJ-500-clean").as_deref(),
            Some(&[][..])
        );
        Ok(())
    }

    #[test]
    fn multiple_environment_merges_are_deduplicated_and_sorted(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("base"), "base\n")?;
        run_git(directory.path(), &["add", "base"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        for helper in ["helper-one", "helper-two", "helper-three"] {
            run_git(directory.path(), &["checkout", "-q", "-b", helper, "main"])?;
            std::fs::write(directory.path().join(helper), format!("{helper}\n"))?;
            run_git(directory.path(), &["add", helper])?;
            commit(
                directory.path(),
                &format!("{helper} work"),
                "2024-02-01T00:00:00Z",
            )?;
        }
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-600-mixed", "main"],
        )?;
        std::fs::write(directory.path().join("mixed"), "mixed\n")?;
        run_git(directory.path(), &["add", "mixed"])?;
        commit(directory.path(), "mixed feature", "2024-02-02T00:00:00Z")?;
        for (helper, subject) in [
            (
                "helper-one",
                "Merge branch 'staging' into feature/PROJ-600-mixed",
            ),
            (
                "helper-two",
                "Merge branch 'qa' into feature/PROJ-600-mixed",
            ),
            (
                "helper-three",
                "Merge branch 'qa' into feature/PROJ-600-mixed",
            ),
        ] {
            run_git(
                directory.path(),
                &["merge", "-q", "--no-ff", helper, "-m", subject],
            )?;
        }
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-600-mixed",
                "-m",
                "promote mixed",
            ],
        )?;
        for branch in ["main", "qa", "feature/PROJ-600-mixed"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
        let merged = updates.iter().find_map(|update| match update {
            DiffUpdate::Measured(row) if row.branch == "feature/PROJ-600-mixed" => {
                Some(row.merged_environments.clone())
            }
            _ => None,
        });

        assert_eq!(
            merged.as_deref(),
            Some(&["qa".to_owned(), "staging".to_owned()][..])
        );
        Ok(())
    }

    #[test]
    fn work_from_a_deleted_branch_is_recovered_by_commit_subject_jira_key(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("file"), "base\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "-b", "PROJ-500"])?;
        std::fs::write(directory.path().join("file"), "base\nwidget\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(
            directory.path(),
            "PROJ-500: add widget",
            "2024-02-01T00:00:00Z",
        )?;
        std::fs::write(directory.path().join("file"), "base\nwidget\npolish\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(
            directory.path(),
            "PROJ-500: polish widget",
            "2024-02-02T00:00:00Z",
        )?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "PROJ-500",
                "-m",
                "Merged PR 1: PROJ-500",
            ],
        )?;
        // Only main and qa exist on the remote: the feature branch was
        // deleted when its pull request completed.
        for branch in ["main", "qa"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();

        assert!(matches!(
            updates.first(),
            Some(DiffUpdate::Skeleton { branches, .. }) if branches == &["PROJ-500"]
        ));
        assert!(matches!(
            updates.get(1),
            Some(DiffUpdate::Measured(PromotionBranch {
                branch,
                started,
                last,
                ahead: 2,
                commits,
                jira: JiraIssueState::NotConfigured { key },
                ..
            })) if branch == "PROJ-500"
                && started == "2024-02-01"
                && last == "2024-02-02"
                && commits.len() == 2
                && commits[0].subject == "PROJ-500: polish widget"
                && commits[1].subject == "PROJ-500: add widget"
                && key == "PROJ-500"
        ));
        Ok(())
    }

    #[test]
    fn merge_commit_subjects_never_create_recovered_ticket_rows(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("file"), "base\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        // A branch whose commits carry no ticket key, merged with a subject
        // that does: the key must not surface because merge commits are
        // skipped and no non-merge commit names it.
        run_git(directory.path(), &["checkout", "-q", "-b", "throwaway"])?;
        std::fs::write(directory.path().join("file"), "base\nwork\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "no ticket here", "2024-02-01T00:00:00Z")?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "throwaway",
                "-m",
                "Merged PR 3: PROJ-700",
            ],
        )?;
        for branch in ["main", "qa"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();

        assert!(matches!(
            updates.first(),
            Some(DiffUpdate::Skeleton { branches, .. }) if branches.is_empty()
        ));
        assert_eq!(updates.len(), 1);
        Ok(())
    }

    #[test]
    fn a_surviving_branch_suppresses_the_recovered_row_for_its_key(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        run_git(directory.path(), &["init", "-q", "-b", "main"])?;
        run_git(directory.path(), &["config", "user.name", "Test Author"])?;
        run_git(
            directory.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        std::fs::write(directory.path().join("file"), "base\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
        run_git(
            directory.path(),
            &["checkout", "-q", "-b", "feature/PROJ-123-login"],
        )?;
        std::fs::write(directory.path().join("file"), "base\nfeature\n")?;
        run_git(directory.path(), &["add", "file"])?;
        commit(
            directory.path(),
            "PROJ-123: add login",
            "2024-02-01T00:00:00Z",
        )?;
        run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "feature/PROJ-123-login",
                "-m",
                "promote",
            ],
        )?;
        for branch in ["main", "qa", "feature/PROJ-123-login"] {
            run_git(
                directory.path(),
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    &format!("refs/heads/{branch}"),
                ],
            )?;
        }
        run_git(
            directory.path(),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        scan_repository(
            &ScanOptions {
                repository: directory.path().to_path_buf(),
                environment: "qa".to_owned(),
                main: None,
                remote: "origin".to_owned(),
                jira_configured: false,
                fetch_before_scan: false,
                selected_branches: None,
            },
            &sender,
        )?;
        drop(sender);
        let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();

        assert!(matches!(
            updates.first(),
            Some(DiffUpdate::Skeleton { branches, .. })
                if branches == &["feature/PROJ-123-login"]
        ));
        assert_eq!(updates.len(), 2);
        Ok(())
    }

    fn run_git(path: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(path)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("git {} failed with {status}", arguments.join(" ")).into())
        }
    }

    fn commit(path: &Path, message: &str, date: &str) -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("git")
            .args(["commit", "-q", "-m", message])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .current_dir(path)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("git commit failed with {status}").into())
        }
    }
}
