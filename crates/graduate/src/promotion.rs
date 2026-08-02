//! Deterministic promotion-report contracts and branch-to-ticket mapping.

/// Jira information associated with a feature branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JiraIssueSummary {
    pub key: String,
    pub api_url: String,
    pub summary: String,
    pub status: String,
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

/// One commit that belongs to a promotion branch but not the main branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionCommit {
    pub short_id: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

/// One remote feature branch that has reached an environment but not main.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionBranch {
    pub branch: String,
    pub started: String,
    pub last: String,
    pub ahead: usize,
    pub last_author: String,
    pub commits: Vec<PromotionCommit>,
    /// Environment branches whose merge history is reachable from this
    /// branch, meaning the environment was merged into the feature branch.
    pub merged_environments: Vec<String>,
    pub jira: JiraIssueState,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_normalizes_a_ticket_key() {
        assert_eq!(
            jira_key_from_branch("feature/proj-123-add-login").as_deref(),
            Some("PROJ-123")
        );
    }

    #[test]
    fn ignores_text_without_a_complete_ticket_key() {
        assert_eq!(jira_key_from_branch("feature/no-ticket"), None);
        assert_eq!(jira_key_from_branch("feature/PROJ-next"), None);
        assert_eq!(jira_key_from_branch("feature/PROJ-123abc"), None);
        assert_eq!(jira_key_from_branch("feature/PROJ-123é"), None);
    }
}
