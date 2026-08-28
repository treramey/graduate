//! Deterministic promotion-report contracts and branch-to-ticket mapping.

mod age;
mod readiness;
#[cfg(test)]
mod tests;

pub use age::{
    AgeBucket, OlderCommitSummary, OldestPromotionBranch, PromotionAgeReport, RecentCommitSummary,
    ReportDate, ReportDateError,
};
pub use readiness::{
    jira_issue_is_closed, PromotionReadinessReport, ReadinessBucket, ReadinessOwnerGroup,
    ReadinessRow, NO_TICKET_ROW,
};

/// Jira information associated with a feature branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JiraIssueSummary {
    pub key: String,
    pub api_url: String,
    pub summary: String,
    pub status: String,
    /// Jira's `statusCategory.key` (`new`, `indeterminate`, `done`) when the
    /// site returned one.
    pub status_category: Option<String>,
    pub assignee: Option<String>,
    pub fix_versions: Vec<String>,
    pub url: String,
}

/// Jira enrichment state for a branch report row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JiraIssueState {
    NoTicket,
    NotConfigured { key: String },
    Loading { key: String },
    NotFound { key: String },
    Loaded(JiraIssueSummary),
    Failed { key: String, message: String },
}

impl JiraIssueState {
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        match self {
            Self::NoTicket => None,
            Self::NotConfigured { key }
            | Self::Loading { key }
            | Self::NotFound { key }
            | Self::Failed { key, .. } => Some(key),
            Self::Loaded(issue) => Some(&issue.key),
        }
    }
}

/// Jira lookups that resolved to `NotFound`, out of every resolved lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotFoundSummary {
    pub not_found: usize,
    pub resolved: usize,
}

/// Detect a likely Jira site or permission problem across a report.
///
/// Jira Cloud answers 404 for tickets the account cannot browse, so a report
/// where nearly every lookup misses usually means one systemic problem, not
/// many bad branch names. Returns a summary when at least three lookups
/// resolved and at least four in five of them returned not found. Branches
/// without a ticket key and lookups still in flight are ignored.
#[must_use]
pub fn systemic_not_found<'a, I>(states: I) -> Option<NotFoundSummary>
where
    I: IntoIterator<Item = &'a JiraIssueState>,
{
    let mut not_found = 0usize;
    let mut resolved = 0usize;
    for state in states {
        match state {
            JiraIssueState::NotFound { .. } => {
                not_found += 1;
                resolved += 1;
            }
            JiraIssueState::Loaded(_) | JiraIssueState::Failed { .. } => resolved += 1,
            JiraIssueState::NoTicket
            | JiraIssueState::NotConfigured { .. }
            | JiraIssueState::Loading { .. } => {}
        }
    }
    (resolved >= 3 && not_found * 5 >= resolved * 4).then_some(NotFoundSummary {
        not_found,
        resolved,
    })
}

/// One commit that belongs to a promotion branch but not the main branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionCommit {
    /// Full object ID used to recognize the same commit across branches.
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

/// Authoritative non-merge commit sets for one environment compared with main.
///
/// Branch and Jira rows may attribute these commits, but they are not the
/// source of truth for either side of the comparison.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentInventory {
    /// Commits reachable from the environment and absent from main.
    pub ahead: Vec<PromotionCommit>,
    /// Commits reachable from main and absent from the environment.
    pub behind_main: Vec<PromotionCommit>,
}

/// One remote feature branch that has reached an environment but not main.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionBranch {
    pub branch: String,
    /// Full object ID of the branch tip. Empty for synthetic rows that
    /// recover work from a deleted branch.
    pub tip: String,
    pub started: String,
    pub last: String,
    pub ahead: usize,
    pub last_author: String,
    pub commits: Vec<PromotionCommit>,
    /// Environment branches whose merge history is reachable from this
    /// branch, meaning the environment was merged into the feature branch.
    pub merged_environments: Vec<String>,
    /// Whether the branch tip is reachable from the environment. `false`
    /// means the branch was merged once and then extended.
    pub tip_in_environment: bool,
    /// Non-merge commits reachable from the tip but not from the
    /// environment; always zero when the tip is in the environment.
    pub unmerged_ahead: usize,
    /// First-parent merges of the requested environment that this branch
    /// reaches, meaning the environment itself was merged into the branch.
    pub absorbed_environment_merges: usize,
    /// Result of merging the tip onto main, populated only by the readiness
    /// report.
    pub merge_onto_main: Option<MergeOntoMain>,
    pub jira: JiraIssueState,
}

/// Outcome of an in-memory three-way merge of a branch tip onto main.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeOntoMain {
    pub clean: bool,
    /// Paths with unresolved conflicts; zero when `clean`.
    pub conflicting_paths: usize,
}

/// Extract the first Jira-style issue key from a branch name.
///
/// Keys begin with an ASCII letter, may contain ASCII letters, digits, or
/// underscores, and end in a hyphen followed by one or more digits.
#[must_use]
pub fn jira_key_from_branch(branch: &str) -> Option<String> {
    let bytes = branch.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'-' {
            continue;
        }
        let separator = index;
        index += 1;
        let digits = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let has_boundary = index == bytes.len()
            || branch[index..]
                .chars()
                .next()
                .is_some_and(|character| !character.is_alphanumeric());
        if index > digits && has_boundary {
            return Some(branch[start..index].to_ascii_uppercase());
        }
        index = separator + 1;
    }
    None
}
