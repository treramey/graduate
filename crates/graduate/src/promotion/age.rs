//! Report dates and commit-age projections.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use super::{PromotionBranch, PromotionCommit};

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
