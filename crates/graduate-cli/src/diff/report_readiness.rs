//! Readiness report projections: JSON/YAML value and human-readable table.

use graduate::promotion::{PromotionReadinessReport, ReadinessBucket, ReadinessRow};

use super::PromotionReport;
use crate::shared::terminal_text::escape;

/// Schema version of the readiness report.
pub(super) const READINESS_REPORT_SCHEMA_VERSION: u8 = 1;

pub(super) fn readiness_report_value(
    report: &PromotionReport,
    readiness: &PromotionReadinessReport,
) -> serde_json::Value {
    let buckets = ReadinessBucket::ALL
        .iter()
        .map(|bucket| {
            (
                bucket.label().to_owned(),
                serde_json::json!({
                    "remediation": bucket.remediation(),
                    "count": readiness.total(*bucket),
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let owners = readiness
        .groups
        .iter()
        .map(|group| {
            let counts = ReadinessBucket::ALL
                .iter()
                .filter_map(|bucket| {
                    group
                        .counts
                        .get(bucket)
                        .map(|count| (bucket.label().to_owned(), serde_json::json!(count)))
                })
                .collect::<serde_json::Map<_, _>>();
            serde_json::json!({
                "owner": group.owner,
                "counts": counts,
                "branches": group.rows.iter().map(row_value).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "schemaVersion": READINESS_REPORT_SCHEMA_VERSION,
        "report": "readiness",
        "environment": report.environment,
        "main": report.main,
        "buckets": buckets,
        "owners": owners,
    })
}

fn row_value(row: &ReadinessRow) -> serde_json::Value {
    serde_json::json!({
        "branch": row.branch,
        "tip": row.tip,
        "last": row.last,
        "ahead": row.ahead,
        "unmergedAhead": row.unmerged_ahead,
        "absorbedEnvironmentMerges": row.absorbed_environment_merges,
        "mergesCleanlyOntoMain": row.merges_cleanly_onto_main,
        "conflictingPaths": row.conflicting_paths,
        "jiraIssueKey": row.jira_key,
        "jiraStatus": row.jira_status,
        "bucket": row.bucket.label(),
        "remediation": row.bucket.remediation(),
    })
}

pub(super) fn format_readiness_table(
    report: &PromotionReport,
    readiness: &PromotionReadinessReport,
) -> String {
    let mut output = format!(
        "Rebuild readiness for {} against {}\n{}\n",
        escape(&report.environment),
        escape(&report.main),
        bucket_counts(
            readiness
                .totals
                .iter()
                .map(|(bucket, count)| (*bucket, *count))
        )
    );
    if readiness.groups.is_empty() {
        output.push_str("(nothing to promote)\n");
    }
    for group in &readiness.groups {
        output.push_str(&format!(
            "\n{}  ·  {}\n{:<36} {:<8} {:>5} {:>8} {:>8} {:<12} {:<12} {:<14} LAST\n",
            escape(&group.owner),
            bucket_counts(group.counts.iter().map(|(bucket, count)| (*bucket, *count))),
            "BRANCH",
            "BUCKET",
            "AHEAD",
            "UNMERGED",
            "ABSORBED",
            "MERGES CLEAN",
            "JIRA",
            "STATUS"
        ));
        for row in &group.rows {
            let merges_clean = match (row.merges_cleanly_onto_main, row.conflicting_paths) {
                (Some(true), _) => "yes".to_owned(),
                (Some(false), Some(paths)) => format!("no ({paths})"),
                (Some(false), None) => "no".to_owned(),
                (None, _) => "-".to_owned(),
            };
            output.push_str(&format!(
                "{:<36} {:<8} {:>5} {:>8} {:>8} {:<12} {:<12} {:<14} {}\n",
                escape(&row.branch),
                row.bucket.label(),
                row.ahead,
                row.unmerged_ahead,
                row.absorbed_environment_merges,
                merges_clean,
                escape(row.jira_key.as_deref().unwrap_or_default()),
                escape(row.jira_status.as_deref().unwrap_or_default()),
                row.last
            ));
        }
    }
    output.push_str("\nRemediation\n");
    for bucket in ReadinessBucket::ALL {
        output.push_str(&format!(
            "{:<8}  {}\n",
            bucket.label(),
            bucket.remediation()
        ));
    }
    output
}

fn bucket_counts(counts: impl Iterator<Item = (ReadinessBucket, usize)>) -> String {
    let parts = counts
        .filter(|(_, count)| *count > 0)
        .map(|(bucket, count)| format!("{count} {}", bucket.label()))
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "0 branches".to_owned()
    } else {
        parts.join(", ")
    }
}
