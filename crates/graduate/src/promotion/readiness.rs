//! Owner-grouped rebuild readiness projection.

use std::collections::{BTreeMap, HashSet};

use super::{JiraIssueState, PromotionBranch, PromotionCommit};

/// What the owner of a branch must do before an environment rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReadinessBucket {
    /// The branch re-merges as-is.
    Ready,
    /// The tip no longer merges cleanly onto main.
    Stale,
    /// The tip was extended after it was merged into the environment.
    Partial,
    /// The branch merged the environment into itself.
    Tainted,
    /// The Jira issue is closed.
    Closed,
    /// Work in the environment with no live branch.
    Orphan,
}

impl ReadinessBucket {
    pub const ALL: [Self; 6] = [
        Self::Ready,
        Self::Stale,
        Self::Partial,
        Self::Tainted,
        Self::Closed,
        Self::Orphan,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Stale => "stale",
            Self::Partial => "partial",
            Self::Tainted => "tainted",
            Self::Closed => "closed",
            Self::Orphan => "orphan",
        }
    }

    #[must_use]
    pub fn remediation(self) -> &'static str {
        match self {
            Self::Ready => "Nothing to do; the rebuild re-merges the branch as-is.",
            Self::Stale => {
                "Merge or rebase onto main and resolve the conflicts before the rebuild."
            }
            Self::Partial => {
                "Promote the branch again so its tip reaches the environment, or drop the unmerged commits."
            }
            Self::Tainted => {
                "Recreate the branch from main and cherry-pick its commits; it merged the environment into itself."
            }
            Self::Closed => "The Jira issue is closed; delete the branch or reopen the issue.",
            Self::Orphan => {
                "No live branch carries this work; recreate a branch from these commits or accept that the rebuild drops them."
            }
        }
    }
}

/// Whether a Jira status means the work is finished or abandoned.
///
/// Jira's `statusCategory.key` is authoritative when present; otherwise the
/// status name is matched against the common done, closed, resolved, and
/// canceled spellings.
#[must_use]
pub fn jira_issue_is_closed(status_name: &str, status_category: Option<&str>) -> bool {
    if let Some(category) = status_category {
        return category.eq_ignore_ascii_case("done");
    }
    let lowered = status_name.to_ascii_lowercase();
    matches!(lowered.as_str(), "done" | "closed" | "resolved") || lowered.contains("cancel")
}

/// One branch or orphan group with its readiness bucket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadinessRow {
    pub branch: String,
    /// Full tip object ID; `None` for orphan rows.
    pub tip: Option<String>,
    pub last_author: String,
    pub last: String,
    pub ahead: usize,
    pub unmerged_ahead: usize,
    pub absorbed_environment_merges: usize,
    pub merges_cleanly_onto_main: Option<bool>,
    pub conflicting_paths: Option<usize>,
    pub jira_key: Option<String>,
    pub jira_status: Option<String>,
    pub bucket: ReadinessBucket,
}

/// Every row owned by one author, with per-bucket counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadinessOwnerGroup {
    pub owner: String,
    pub rows: Vec<ReadinessRow>,
    pub counts: BTreeMap<ReadinessBucket, usize>,
}

/// Rows grouped by owner with deterministic bucket totals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionReadinessReport {
    pub groups: Vec<ReadinessOwnerGroup>,
    pub totals: BTreeMap<ReadinessBucket, usize>,
}

/// Owner label for environment commits without a branch or ticket.
pub const NO_TICKET_ROW: &str = "(no ticket)";

impl PromotionReadinessReport {
    /// Bucket every branch row, then add one orphan row per author for the
    /// environment commits that no row attributes.
    #[must_use]
    pub fn new(branches: &[PromotionBranch], environment_ahead: &[PromotionCommit]) -> Self {
        let mut rows = branches.iter().map(readiness_row).collect::<Vec<_>>();
        rows.extend(untracked_rows(branches, environment_ahead));

        let mut by_owner: BTreeMap<String, Vec<ReadinessRow>> = BTreeMap::new();
        for row in rows {
            by_owner
                .entry(row.last_author.clone())
                .or_default()
                .push(row);
        }
        let mut totals = BTreeMap::new();
        let groups = by_owner
            .into_iter()
            .map(|(owner, mut rows)| {
                rows.sort_by(|left, right| left.branch.cmp(&right.branch));
                let mut counts = BTreeMap::new();
                for row in &rows {
                    *counts.entry(row.bucket).or_insert(0) += 1;
                    *totals.entry(row.bucket).or_insert(0) += 1;
                }
                ReadinessOwnerGroup {
                    owner,
                    rows,
                    counts,
                }
            })
            .collect();
        Self { groups, totals }
    }

    #[must_use]
    pub fn total(&self, bucket: ReadinessBucket) -> usize {
        self.totals.get(&bucket).copied().unwrap_or(0)
    }
}

/// Bucket precedence: orphan, closed, tainted, partial, stale, ready.
fn readiness_row(branch: &PromotionBranch) -> ReadinessRow {
    let (jira_key, jira_status, closed) = match &branch.jira {
        JiraIssueState::Loaded(issue) => (
            Some(issue.key.clone()),
            Some(issue.status.clone()),
            jira_issue_is_closed(&issue.status, issue.status_category.as_deref()),
        ),
        JiraIssueState::NotFound { key } => {
            (Some(key.clone()), Some("not found".to_owned()), false)
        }
        other => (other.key().map(str::to_owned), None, false),
    };
    let bucket = if branch.tip.is_empty() {
        ReadinessBucket::Orphan
    } else if closed {
        ReadinessBucket::Closed
    } else if branch.absorbed_environment_merges > 0 {
        ReadinessBucket::Tainted
    } else if !branch.tip_in_environment {
        ReadinessBucket::Partial
    } else if branch.merge_onto_main.is_some_and(|merge| !merge.clean) {
        ReadinessBucket::Stale
    } else {
        ReadinessBucket::Ready
    };
    ReadinessRow {
        branch: branch.branch.clone(),
        tip: (!branch.tip.is_empty()).then(|| branch.tip.clone()),
        last_author: branch.last_author.clone(),
        last: branch.last.clone(),
        ahead: branch.ahead,
        unmerged_ahead: branch.unmerged_ahead,
        absorbed_environment_merges: branch.absorbed_environment_merges,
        merges_cleanly_onto_main: branch.merge_onto_main.map(|merge| merge.clean),
        conflicting_paths: branch.merge_onto_main.map(|merge| merge.conflicting_paths),
        jira_key,
        jira_status,
        bucket,
    }
}

/// One `(no ticket)` orphan row per author for environment commits that no
/// branch or recovered ticket row attributes.
fn untracked_rows(
    branches: &[PromotionBranch],
    environment_ahead: &[PromotionCommit],
) -> Vec<ReadinessRow> {
    let attributed = branches
        .iter()
        .flat_map(|branch| branch.commits.iter().map(|commit| commit.id.as_str()))
        .collect::<HashSet<_>>();
    let mut by_author: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    for commit in environment_ahead {
        if attributed.contains(commit.id.as_str()) {
            continue;
        }
        let entry = by_author.entry(&commit.author).or_insert((0, &commit.date));
        entry.0 += 1;
        if commit.date.as_str() > entry.1 {
            entry.1 = &commit.date;
        }
    }
    by_author
        .into_iter()
        .map(|(author, (count, last))| ReadinessRow {
            branch: NO_TICKET_ROW.to_owned(),
            tip: None,
            last_author: author.to_owned(),
            last: last.to_owned(),
            ahead: count,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merges_cleanly_onto_main: None,
            conflicting_paths: None,
            jira_key: None,
            jira_status: None,
            bucket: ReadinessBucket::Orphan,
        })
        .collect()
}
