//! CSV commit-age report formatting.

use graduate::promotion::PromotionAgeReport;

use super::report_csv::csv_row;
use super::report_json::{age_bucket_reading, share_percent};
use super::PromotionReport;
use crate::shared::error::CliError;

pub(super) fn format_age_csv(
    report: &PromotionReport,
    age: &PromotionAgeReport,
) -> Result<String, CliError> {
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
            "direction",
            "inventoryCount",
            "commitId",
            "shortId",
            "commitSubject",
            "commitAuthor",
            "authoredDate",
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
                "uniqueEnvironmentCommits",
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
                "",
                "",
                "",
                "",
                "",
                "",
                "",
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
            "uniqueEnvironmentCommits",
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
            "",
            "",
            "",
            "",
            "",
            "",
            "",
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
            "uniqueEnvironmentCommits",
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
            "",
            "",
            "",
            "",
            "",
            "",
            "",
        ],
    )?;
    for (direction, commits) in [
        ("aheadOfMain", report.inventory.ahead.as_slice()),
        ("behindMain", report.inventory.behind_main.as_slice()),
    ] {
        let count = commits.len().to_string();
        csv_row(
            &mut file,
            &[
                "inventory",
                &report.environment,
                &report.main,
                &as_of,
                "uniqueEnvironmentCommits",
                "",
                "",
                "",
                "",
                "",
                &count,
                &total,
                "",
                "",
                "",
                "",
                direction,
                &count,
                "",
                "",
                "",
                "",
                "",
            ],
        )?;
        for commit in commits {
            csv_row(
                &mut file,
                &[
                    "commit",
                    &report.environment,
                    &report.main,
                    &as_of,
                    "uniqueEnvironmentCommits",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    &total,
                    "",
                    "",
                    "",
                    "",
                    direction,
                    "",
                    &commit.id,
                    &commit.short_id,
                    &commit.subject,
                    &commit.author,
                    &commit.date,
                ],
            )?;
        }
    }
    for branch in &age.oldest_branches {
        let commits = branch.commits.to_string();
        csv_row(
            &mut file,
            &[
                "oldestBranch",
                &report.environment,
                &report.main,
                &as_of,
                "uniqueEnvironmentCommits",
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
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ],
        )?;
    }
    String::from_utf8(file)
        .map_err(|error| CliError::InvalidInput(format!("CSV was not valid UTF-8: {error}")))
}
