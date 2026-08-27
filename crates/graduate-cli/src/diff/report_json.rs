//! JSON and YAML report projections.

use graduate::promotion::{AgeBucket, JiraIssueState, PromotionAgeReport, PromotionCommit};

use super::PromotionReport;

pub(super) fn report_value(report: &PromotionReport) -> serde_json::Value {
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
        "schemaVersion": 1,
        "environment": report.environment,
        "main": report.main,
        "commitInventory": {
            "aheadOfMain": commit_inventory_value(&report.inventory.ahead),
            "behindMain": commit_inventory_value(&report.inventory.behind_main),
        },
        "branches": branches,
    })
}

fn commit_inventory_value(commits: &[PromotionCommit]) -> serde_json::Value {
    let commits = commits
        .iter()
        .map(|commit| {
            serde_json::json!({
                "id": commit.id,
                "shortId": commit.short_id,
                "subject": commit.subject,
                "author": commit.author,
                "authoredDate": commit.date,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "count": commits.len(),
        "commits": commits,
    })
}

pub(super) fn age_report_value(
    report: &PromotionReport,
    age: &PromotionAgeReport,
) -> serde_json::Value {
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
        "schemaVersion": 2,
        "report": "age",
        "environment": report.environment,
        "main": report.main,
        "asOf": age.as_of.to_string(),
        "counting": "uniqueEnvironmentCommits",
        "commitInventory": {
            "aheadOfMain": commit_inventory_value(&report.inventory.ahead),
            "behindMain": commit_inventory_value(&report.inventory.behind_main),
        },
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
