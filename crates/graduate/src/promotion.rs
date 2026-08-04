//! Deterministic promotion-report contracts and branch-to-ticket mapping.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

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

/// A validated UTC calendar date used for deterministic report thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReportDate {
    year: i32,
    month: u8,
    day: u8,
    unix_days: i64,
}

impl ReportDate {
    /// Parse an ISO 8601 calendar date in `YYYY-MM-DD` form.
    pub fn parse(input: &str) -> Result<Self, ReportDateError> {
        let bytes = input.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(ReportDateError::Invalid(input.to_owned()));
        }
        let year = parse_date_part::<i32>(&input[0..4], input)?;
        let month = parse_date_part::<u8>(&input[5..7], input)?;
        let day = parse_date_part::<u8>(&input[8..10], input)?;
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return Err(ReportDateError::Invalid(input.to_owned()));
        }
        Ok(Self {
            year,
            month,
            day,
            unix_days: days_from_civil(year, month, day),
        })
    }

    #[must_use]
    pub fn year(self) -> i32 {
        self.year
    }

    fn days_before(self, days: i64) -> Result<Self, ReportDateError> {
        date_from_unix_days(self.unix_days - days)
    }

    fn previous_year_anniversary(self) -> Result<Self, ReportDateError> {
        let year = self
            .year
            .checked_sub(1)
            .ok_or(ReportDateError::OutsideSupportedRange)?;
        let day = self.day.min(days_in_month(year, self.month));
        Self::parse(&format!("{year:04}-{:02}-{day:02}", self.month))
    }
}

impl fmt::Display for ReportDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

/// Failure to construct a report calendar date.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReportDateError {
    #[error("invalid report date: {0}")]
    Invalid(String),
    #[error("report date is outside the supported range")]
    OutsideSupportedRange,
}

/// Count of unique unshipped commits in one calendar period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgeBucket {
    pub year: i32,
    pub commits: usize,
}

/// Commits authored during a recent window, inclusive of `since`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecentCommitSummary {
    pub since: ReportDate,
    pub commits: usize,
}

/// Commits authored strictly before a decision threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OlderCommitSummary {
    pub before: ReportDate,
    pub commits: usize,
}

/// One branch carrying commits from the oldest authored year in the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OldestPromotionBranch {
    pub branch: String,
    pub commits: usize,
    pub oldest: ReportDate,
    pub newest: ReportDate,
}

/// Age distribution for unique commits in an environment but not main.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionAgeReport {
    pub as_of: ReportDate,
    pub total_commits: usize,
    pub buckets: Vec<AgeBucket>,
    pub last_90_days: RecentCommitSummary,
    pub older_than_one_year: OlderCommitSummary,
    pub oldest_branches: Vec<OldestPromotionBranch>,
}

impl PromotionAgeReport {
    /// Build an age report from the authoritative environment inventory while
    /// retaining attribution rows only for the oldest-work detail.
    pub fn new(
        commits: &[PromotionCommit],
        branches: &[PromotionBranch],
        as_of: ReportDate,
    ) -> Result<Self, ReportDateError> {
        let since = as_of.days_before(89)?;
        let anniversary = as_of.previous_year_anniversary()?;
        let mut unique_commits = HashMap::<String, ReportDate>::new();

        for commit in commits {
            let date = ReportDate::parse(&commit.date)?;
            unique_commits.entry(commit.id.clone()).or_insert(date);
        }

        let oldest_year = unique_commits.values().map(|date| date.year).min();
        let mut oldest_branches = Vec::new();
        if let Some(oldest_year) = oldest_year {
            for branch in branches {
                let mut unique_on_branch = HashSet::new();
                let mut oldest_dates = Vec::new();
                for commit in &branch.commits {
                    let date = ReportDate::parse(&commit.date)?;
                    if date.year == oldest_year && unique_on_branch.insert(commit.id.as_str()) {
                        oldest_dates.push(date);
                    }
                }
                oldest_dates.sort_unstable();
                if let (Some(oldest), Some(newest)) = (oldest_dates.first(), oldest_dates.last()) {
                    oldest_branches.push(OldestPromotionBranch {
                        branch: branch.branch.clone(),
                        commits: oldest_dates.len(),
                        oldest: *oldest,
                        newest: *newest,
                    });
                }
            }
        }

        let mut years = BTreeMap::new();
        let mut recent = 0usize;
        let mut older = 0usize;
        for date in unique_commits.values() {
            *years.entry(date.year).or_default() += 1;
            if *date >= since && *date <= as_of {
                recent += 1;
            }
            if *date < anniversary {
                older += 1;
            }
        }
        let buckets = years
            .into_iter()
            .rev()
            .map(|(year, commits)| AgeBucket { year, commits })
            .collect::<Vec<_>>();
        oldest_branches.sort_by(|left, right| {
            right
                .commits
                .cmp(&left.commits)
                .then_with(|| left.branch.cmp(&right.branch))
        });

        Ok(Self {
            as_of,
            total_commits: unique_commits.len(),
            buckets,
            last_90_days: RecentCommitSummary {
                since,
                commits: recent,
            },
            older_than_one_year: OlderCommitSummary {
                before: anniversary,
                commits: older,
            },
            oldest_branches,
        })
    }

    /// The oldest authored year represented by the report.
    #[must_use]
    pub fn oldest_year(&self) -> Option<i32> {
        self.buckets.iter().map(|bucket| bucket.year).min()
    }
}

fn parse_date_part<T>(part: &str, input: &str) -> Result<T, ReportDateError>
where
    T: std::str::FromStr,
{
    part.parse()
        .map_err(|_| ReportDateError::Invalid(input.to_owned()))
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let mut year = i64::from(year);
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn date_from_unix_days(unix_days: i64) -> Result<ReportDate, ReportDateError> {
    let z = unix_days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let year = i32::try_from(year).map_err(|_| ReportDateError::OutsideSupportedRange)?;
    let month = u8::try_from(month).map_err(|_| ReportDateError::OutsideSupportedRange)?;
    let day = u8::try_from(day).map_err(|_| ReportDateError::OutsideSupportedRange)?;
    Ok(ReportDate {
        year,
        month,
        day,
        unix_days,
    })
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

    fn commit(id: &str, date: &str) -> PromotionCommit {
        PromotionCommit {
            id: id.to_owned(),
            short_id: id.chars().take(7).collect(),
            subject: "Work".to_owned(),
            author: "Pat".to_owned(),
            date: date.to_owned(),
        }
    }

    fn branch(name: &str, commits: Vec<PromotionCommit>) -> PromotionBranch {
        PromotionBranch {
            branch: name.to_owned(),
            started: commits
                .last()
                .map_or_else(String::new, |commit| commit.date.clone()),
            last: commits
                .first()
                .map_or_else(String::new, |commit| commit.date.clone()),
            ahead: commits.len(),
            last_author: "Pat".to_owned(),
            commits,
            merged_environments: Vec::new(),
            jira: JiraIssueState::NoTicket,
        }
    }

    #[test]
    fn age_report_buckets_unique_commits_and_exposes_decision_thresholds(
    ) -> Result<(), ReportDateError> {
        let shared = commit("111111111111", "2026-08-01");
        let branches = vec![
            branch(
                "feature/current",
                vec![shared.clone(), commit("222222222222", "2025-08-04")],
            ),
            branch(
                "feature/legacy",
                vec![
                    shared,
                    commit("333333333333", "2025-08-03"),
                    commit("444444444444", "2019-12-31"),
                ],
            ),
        ];

        let inventory = branches
            .iter()
            .flat_map(|branch| branch.commits.iter().cloned())
            .collect::<Vec<_>>();
        let report =
            PromotionAgeReport::new(&inventory, &branches, ReportDate::parse("2026-08-04")?)?;

        assert_eq!(report.total_commits, 4);
        assert_eq!(report.last_90_days.commits, 1);
        assert_eq!(report.last_90_days.since.to_string(), "2026-05-07");
        assert_eq!(report.older_than_one_year.commits, 2);
        assert_eq!(report.older_than_one_year.before.to_string(), "2025-08-04");
        assert_eq!(
            report
                .buckets
                .iter()
                .map(|bucket| (bucket.year, bucket.commits))
                .collect::<Vec<_>>(),
            vec![(2026, 1), (2025, 2), (2019, 1)]
        );
        assert_eq!(report.oldest_branches.len(), 1);
        assert_eq!(report.oldest_year(), Some(2019));
        assert_eq!(report.oldest_branches[0].branch, "feature/legacy");
        assert_eq!(report.oldest_branches[0].commits, 1);
        Ok(())
    }

    #[test]
    fn age_report_counts_inventory_commits_without_attribution_rows() -> Result<(), ReportDateError>
    {
        let inventory = vec![commit("111111111111", "2026-08-01")];

        let report = PromotionAgeReport::new(&inventory, &[], ReportDate::parse("2026-08-04")?)?;

        assert_eq!(report.total_commits, 1);
        assert_eq!(report.last_90_days.commits, 1);
        assert!(report.oldest_branches.is_empty());
        Ok(())
    }

    #[test]
    fn report_dates_reject_impossible_days_and_handle_leap_anniversaries(
    ) -> Result<(), ReportDateError> {
        assert!(ReportDate::parse("2025-02-29").is_err());
        let report = PromotionAgeReport::new(&[], &[], ReportDate::parse("2024-02-29")?)?;

        assert_eq!(report.older_than_one_year.before.to_string(), "2023-02-28");
        assert_eq!(report.last_90_days.since.to_string(), "2023-12-02");
        Ok(())
    }

    fn not_found(key: &str) -> JiraIssueState {
        JiraIssueState::NotFound {
            key: key.to_owned(),
        }
    }

    fn loaded(key: &str) -> JiraIssueState {
        JiraIssueState::Loaded(JiraIssueSummary {
            key: key.to_owned(),
            api_url: String::new(),
            summary: String::new(),
            status: String::new(),
            assignee: None,
            fix_versions: Vec::new(),
            url: String::new(),
        })
    }

    #[test]
    fn flags_a_report_where_most_lookups_return_not_found() {
        let states = vec![
            not_found("A-1"),
            not_found("A-2"),
            not_found("A-3"),
            not_found("A-4"),
            loaded("A-5"),
        ];
        assert_eq!(
            systemic_not_found(&states),
            Some(NotFoundSummary {
                not_found: 4,
                resolved: 5,
            })
        );
    }

    #[test]
    fn ignores_ticketless_branches_and_in_flight_lookups() {
        let states = vec![
            not_found("A-1"),
            not_found("A-2"),
            not_found("A-3"),
            JiraIssueState::NoTicket,
            JiraIssueState::Loading {
                key: "A-4".to_owned(),
            },
        ];
        assert_eq!(
            systemic_not_found(&states),
            Some(NotFoundSummary {
                not_found: 3,
                resolved: 3,
            })
        );
    }

    #[test]
    fn stays_quiet_when_misses_are_a_minority_or_the_sample_is_small() {
        let mixed = vec![
            not_found("A-1"),
            not_found("A-2"),
            not_found("A-3"),
            loaded("A-4"),
            loaded("A-5"),
        ];
        assert_eq!(systemic_not_found(&mixed), None);
        let small = vec![not_found("A-1"), not_found("A-2")];
        assert_eq!(systemic_not_found(&small), None);
    }

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
