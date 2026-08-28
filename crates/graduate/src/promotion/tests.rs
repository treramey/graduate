//! Tests.

use super::age::*;
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
        tip: String::new(),
        tip_in_environment: true,
        unmerged_ahead: 0,
        absorbed_environment_merges: 0,
        merge_onto_main: None,
        jira: JiraIssueState::NoTicket,
    }
}

#[test]
fn age_report_buckets_unique_commits_and_exposes_decision_thresholds() -> Result<(), ReportDateError>
{
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
    let report = PromotionAgeReport::new(&inventory, &branches, ReportDate::parse("2026-08-04")?)?;

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
fn age_report_counts_inventory_commits_without_attribution_rows() -> Result<(), ReportDateError> {
    let inventory = vec![commit("111111111111", "2026-08-01")];

    let report = PromotionAgeReport::new(&inventory, &[], ReportDate::parse("2026-08-04")?)?;

    assert_eq!(report.total_commits, 1);
    assert_eq!(report.last_90_days.commits, 1);
    assert!(report.oldest_branches.is_empty());
    Ok(())
}

#[test]
fn report_dates_reject_impossible_days_and_handle_leap_anniversaries() -> Result<(), ReportDateError>
{
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
        status_category: None,
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
