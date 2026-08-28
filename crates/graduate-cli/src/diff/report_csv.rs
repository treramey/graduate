//! CSV promotion report formatting.

use std::io::Write;

use graduate::promotion::JiraIssueState;

use super::PromotionReport;
use crate::shared::error::CliError;

pub(super) fn format_csv(report: &PromotionReport) -> Result<String, CliError> {
    let mut file = Vec::new();
    csv_row(
        &mut file,
        &[
            "rowType",
            "environment",
            "main",
            "direction",
            "commitCount",
            "commitId",
            "shortId",
            "subject",
            "author",
            "authoredDate",
            "branch",
            "started",
            "last",
            "ahead",
            "tip",
            "tipInEnvironment",
            "unmergedAhead",
            "absorbedEnvironmentMerges",
            "mergesCleanlyOntoMain",
            "conflictingPaths",
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
                direction,
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
                    direction,
                    "",
                    &commit.id,
                    &commit.short_id,
                    &commit.subject,
                    &commit.author,
                    &commit.date,
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
                ],
            )?;
        }
    }
    for row in &report.branches {
        let ahead = row.ahead.to_string();
        let tip_in_environment = yes_no(row.tip_in_environment);
        let unmerged_ahead = row.unmerged_ahead.to_string();
        let absorbed = row.absorbed_environment_merges.to_string();
        let merges_cleanly = row.merge_onto_main.map_or("", |merge| yes_no(merge.clean));
        let conflicting_paths = row
            .merge_onto_main
            .map_or(String::new(), |merge| merge.conflicting_paths.to_string());
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
                "branch",
                &report.environment,
                &report.main,
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                &row.branch,
                &row.started,
                &row.last,
                &ahead,
                &row.tip,
                tip_in_environment,
                &unmerged_ahead,
                &absorbed,
                merges_cleanly,
                &conflicting_paths,
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

pub(super) fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

pub(super) fn csv_row(writer: &mut impl Write, fields: &[&str]) -> Result<(), CliError> {
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
