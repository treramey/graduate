//! Shared fixtures and helpers.

use graduate::promotion::PromotionCommit;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;

mod age_report_tests;
mod history_tests;
mod inspector_tests;
mod layout_tests;
mod navigation_tests;

fn test_commit(subject: &str) -> PromotionCommit {
    PromotionCommit {
        id: format!("a1b2c3d-{subject}"),
        short_id: "a1b2c3d".to_owned(),
        subject: subject.to_owned(),
        author: "Pat".to_owned(),
        date: "2024-01-02".to_owned(),
    }
}

fn test_model() -> Result<DiffModel, graduate::promotion::ReportDateError> {
    Ok(DiffModel::new(ReportDate::parse("2026-08-04")?))
}

fn first_visible_branch(rendered: &str) -> Option<&str> {
    rendered
        .split("BRANCH")
        .nth(1)?
        .split_whitespace()
        .find(|value| value.starts_with("branch-"))
}

fn measured(
    branch: &str,
    started: &str,
    last: &str,
    ahead: usize,
    jira: JiraIssueState,
) -> DiffUpdate {
    DiffUpdate::Measured(PromotionBranch {
        branch: branch.to_owned(),
        started: started.to_owned(),
        last: last.to_owned(),
        ahead,
        last_author: "Pat".to_owned(),
        commits: Vec::new(),
        merged_environments: Vec::new(),
        tip: String::new(),
        tip_in_environment: true,
        unmerged_ahead: 0,
        absorbed_environment_merges: 0,
        merge_onto_main: None,
        jira,
    })
}

fn character_position(line: &str, needle: &str) -> Option<usize> {
    line.find(needle)
        .map(|byte_index| line[..byte_index].chars().count())
}
