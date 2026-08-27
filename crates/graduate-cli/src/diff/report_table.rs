//! Human-readable table formatting.

use graduate::promotion::{JiraIssueState, PromotionAgeReport};

use super::report_json::{age_bucket_label, age_bucket_reading, share_percent};
use super::PromotionReport;

pub(super) fn format_table(report: &PromotionReport) -> String {
    let mut output = format!(
        "Branches in {} but not {}\n{} ahead of main; {} behind main.\n\
         {:<36} {:<10} {:<10} {:>5}  {:<12} {:<14} LAST AUTHOR\n",
        crate::shared::terminal_text::escape(&report.environment),
        crate::shared::terminal_text::escape(&report.main),
        commit_count(report.inventory.ahead.len()),
        commit_count(report.inventory.behind_main.len()),
        "BRANCH",
        "STARTED",
        "LAST",
        "AHEAD",
        "JIRA",
        "STATUS"
    );
    if report.branches.is_empty() {
        output.push_str("(nothing to promote)\n");
    }
    for row in &report.branches {
        // Only a Jira-validated ticket key may appear in the JIRA column.
        let key = match &row.jira {
            JiraIssueState::Loaded(issue) => crate::shared::terminal_text::escape(&issue.key),
            _ => String::new(),
        };
        let status = match &row.jira {
            JiraIssueState::Loaded(issue) => issue.status.as_str(),
            JiraIssueState::Failed { .. } => "error",
            JiraIssueState::NotFound { .. } | JiraIssueState::NoTicket => "not found",
            JiraIssueState::NotConfigured { .. } => "not configured",
            JiraIssueState::Loading { .. } => "loading",
        };
        output.push_str(&format!(
            "{:<36} {:<10} {:<10} {:>5}  {:<12} {:<14} {}\n",
            crate::shared::terminal_text::escape(&row.branch),
            row.started,
            row.last,
            row.ahead,
            key,
            crate::shared::terminal_text::escape(status),
            crate::shared::terminal_text::escape(&row.last_author)
        ));
    }
    append_behind_commits_table(&mut output, report);
    output
}

fn commit_count(count: usize) -> String {
    if count == 1 {
        "1 commit".to_owned()
    } else {
        format!("{count} commits")
    }
}

fn append_behind_commits_table(output: &mut String, report: &PromotionReport) {
    if report.inventory.behind_main.is_empty() {
        return;
    }
    output.push_str(&format!(
        "\nCommits on {} missing from {}\n\
         {:<9} {:<10} {:<24} SUBJECT\n",
        crate::shared::terminal_text::escape(&report.main),
        crate::shared::terminal_text::escape(&report.environment),
        "SHA",
        "DATE",
        "AUTHOR"
    ));
    for commit in &report.inventory.behind_main {
        output.push_str(&format!(
            "{:<9} {:<10} {:<24} {}\n",
            crate::shared::terminal_text::escape(&commit.short_id),
            commit.date,
            crate::shared::terminal_text::escape(&commit.author),
            crate::shared::terminal_text::escape(&commit.subject)
        ));
    }
}

pub(super) fn format_age_table(report: &PromotionReport, age: &PromotionAgeReport) -> String {
    let mut output = format!(
        "Age of unshipped work in {} but not {} (as of {})\n\
         All {} unique environment commits; {} attribution rows.\n\
         {:<24} {:>10} {:>8}  READING\n",
        crate::shared::terminal_text::escape(&report.environment),
        crate::shared::terminal_text::escape(&report.main),
        age.as_of,
        age.total_commits,
        report.branches.len(),
        "WRITTEN IN",
        "COMMITS",
        "SHARE"
    );
    for bucket in &age.buckets {
        output.push_str(&format!(
            "{:<24} {:>10} {:>7.1}%  {}\n",
            age_bucket_label(bucket.year),
            bucket.commits,
            share_percent(bucket.commits, age.total_commits),
            age_bucket_reading(age, bucket)
        ));
    }
    output.push_str(&format!(
        "{:<24} {:>10} {:>7.1}%  Genuinely in flight\n",
        "Written in last 90 days",
        age.last_90_days.commits,
        share_percent(age.last_90_days.commits, age.total_commits)
    ));
    output.push_str(&format!(
        "{:<24} {:>10} {:>7.1}%  Will not ship without a decision\n",
        "Older than one year",
        age.older_than_one_year.commits,
        share_percent(age.older_than_one_year.commits, age.total_commits)
    ));

    if let Some(oldest_year) = age.oldest_year() {
        output.push_str(&format!(
            "\nBranches carrying commits from {oldest_year}\n{:<36} {:>8}  {:<10}  NEWEST\n",
            "BRANCH", "COMMITS", "OLDEST"
        ));
        if age.oldest_branches.is_empty() {
            output.push_str("(none)\n");
        }
        for branch in &age.oldest_branches {
            output.push_str(&format!(
                "{:<36} {:>8}  {:<10}  {}\n",
                crate::shared::terminal_text::escape(&branch.branch),
                branch.commits,
                branch.oldest,
                branch.newest
            ));
        }
    }
    if !report.inventory.behind_main.is_empty() {
        output.push_str(&format!(
            "\n{} behind main.\n",
            commit_count(report.inventory.behind_main.len())
        ));
    }
    append_behind_commits_table(&mut output, report);
    output
}
