//! CSV readiness report formatting.

use graduate::promotion::{PromotionReadinessReport, ReadinessBucket};

use super::report_csv::{csv_row, yes_no};
use super::PromotionReport;
use crate::shared::error::CliError;

pub(super) fn format_readiness_csv(
    report: &PromotionReport,
    readiness: &PromotionReadinessReport,
) -> Result<String, CliError> {
    let mut file = Vec::new();
    csv_row(
        &mut file,
        &[
            "rowType",
            "environment",
            "main",
            "owner",
            "bucket",
            "count",
            "branch",
            "tip",
            "last",
            "ahead",
            "unmergedAhead",
            "absorbedEnvironmentMerges",
            "mergesCleanlyOntoMain",
            "conflictingPaths",
            "jiraIssueKey",
            "jiraStatus",
            "remediation",
        ],
    )?;
    for bucket in ReadinessBucket::ALL {
        let count = readiness.total(bucket).to_string();
        csv_row(
            &mut file,
            &[
                "summary",
                &report.environment,
                &report.main,
                "",
                bucket.label(),
                &count,
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                bucket.remediation(),
            ],
        )?;
    }
    for group in &readiness.groups {
        for row in &group.rows {
            let ahead = row.ahead.to_string();
            let unmerged = row.unmerged_ahead.to_string();
            let absorbed = row.absorbed_environment_merges.to_string();
            let merges_clean = row.merges_cleanly_onto_main.map_or("", yes_no);
            let conflicting = row
                .conflicting_paths
                .map_or(String::new(), |paths| paths.to_string());
            csv_row(
                &mut file,
                &[
                    "branch",
                    &report.environment,
                    &report.main,
                    &group.owner,
                    row.bucket.label(),
                    "",
                    &row.branch,
                    row.tip.as_deref().unwrap_or_default(),
                    &row.last,
                    &ahead,
                    &unmerged,
                    &absorbed,
                    merges_clean,
                    &conflicting,
                    row.jira_key.as_deref().unwrap_or_default(),
                    row.jira_status.as_deref().unwrap_or_default(),
                    row.bucket.remediation(),
                ],
            )?;
        }
    }
    String::from_utf8(file)
        .map_err(|error| CliError::InvalidInput(format!("CSV was not valid UTF-8: {error}")))
}
