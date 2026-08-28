use graduate::promotion::PromotionCommit;

use super::super::report_csv::{csv_row, format_csv};
use super::super::report_json::report_value;
use super::super::report_table::format_table;
use super::super::scan_channel::{collect_plain, jira_issue_state};
use super::*;
use crate::shared::environment_git::unix_date;

#[test]
fn interactive_fetch_does_not_print_a_status_message() {
    assert_eq!(fetch_status_message("origin", false, true), None);
    assert_eq!(fetch_status_message("origin", true, true), None);
}

#[test]
fn jira_404_becomes_a_not_found_issue_state() {
    let state = jira_issue_state("PROJ-404".to_owned(), Err(CliError::JiraStatus(404)));

    assert_eq!(
        state,
        JiraIssueState::NotFound {
            key: "PROJ-404".to_owned()
        }
    );
}

#[test]
fn dates_are_formatted_without_local_timezone_drift() {
    assert_eq!(unix_date(0), "1970-01-01");
    assert_eq!(unix_date(1_704_067_200), "2024-01-01");
}

#[test]
fn csv_quotes_every_field_and_doubles_quotes() -> Result<(), CliError> {
    let mut output = Vec::new();
    csv_row(&mut output, &["branch", "O\"Brien, Pat"])?;
    assert_eq!(output, b"\"branch\",\"O\"\"Brien, Pat\"\n");
    Ok(())
}

#[test]
fn csv_keeps_jira_errors_out_of_issue_fields() -> Result<(), CliError> {
    let report = PromotionReport {
        environment: "qa".to_owned(),
        main: "main".to_owned(),
        inventory: EnvironmentInventory {
            ahead: Vec::new(),
            behind_main: vec![PromotionCommit {
                id: "abcdef123456".to_owned(),
                short_id: "abcdef1".to_owned(),
                subject: "Main-only work".to_owned(),
                author: "Alex".to_owned(),
                date: "2026-08-03".to_owned(),
            }],
        },
        branches: vec![PromotionBranch {
            branch: "feature/PROJ-123-login".to_owned(),
            started: "2024-01-01".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 2,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: vec!["qa".to_owned()],
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::Failed {
                key: "PROJ-123".to_owned(),
                message: "request timed out".to_owned(),
            },
        }],
    };

    let csv = format_csv(&report)?;

    assert!(csv
        .lines()
        .next()
        .is_some_and(|line| line.ends_with("\"jiraError\"")));
    assert!(csv.lines().next().is_some_and(|line| line.contains(
        "\"ahead\",\"tip\",\"tipInEnvironment\",\"unmergedAhead\",\"absorbedEnvironmentMerges\",\"mergesCleanlyOntoMain\",\"conflictingPaths\",\"lastAuthor\""
    )));
    assert!(csv
        .lines()
        .any(|line| line.contains("\"2\",\"\",\"yes\",\"0\",\"0\",\"\",\"\",\"Pat\"")));
    assert!(csv.contains("\"inventory\",\"qa\",\"main\",\"behindMain\",\"1\""));
    assert!(csv.contains("\"commit\",\"qa\",\"main\",\"behindMain\",\"\",\"abcdef123456\""));
    assert!(csv.lines().any(|line| {
        line.starts_with("\"branch\",\"qa\",\"main\"") && line.ends_with("\"request timed out\"")
    }));
    Ok(())
}

#[tokio::test]
async fn plain_reports_require_an_explicit_finished_update() {
    let (sender, receiver) = mpsc::unbounded_channel();
    assert!(sender
        .send(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: Vec::new(),
        })
        .is_ok());
    drop(sender);

    let result = collect_plain(receiver).await;

    assert!(
        matches!(result, Err(CliError::Git(message)) if message.contains("before the scan completed"))
    );
}

#[test]
fn json_report_uses_camel_case_and_jira_api_field_shapes() {
    let report = PromotionReport {
        environment: "qa".to_owned(),
        main: "main".to_owned(),
        inventory: EnvironmentInventory {
            ahead: Vec::new(),
            behind_main: vec![PromotionCommit {
                id: "abcdef123456".to_owned(),
                short_id: "abcdef1".to_owned(),
                subject: "Main work absent from QA".to_owned(),
                author: "Alex".to_owned(),
                date: "2026-08-03".to_owned(),
            }],
        },
        branches: vec![PromotionBranch {
            branch: "feature/PROJ-123-login".to_owned(),
            started: "2024-01-01".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 2,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: vec!["qa".to_owned()],
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::Loaded(graduate::promotion::JiraIssueSummary {
                key: "PROJ-123".to_owned(),
                api_url: "https://example.atlassian.net/rest/api/3/issue/10001".to_owned(),
                summary: "Add login".to_owned(),
                status: "Ready for QA".to_owned(),
                assignee: Some("Pat".to_owned()),
                fix_versions: vec!["1.2".to_owned()],
                url: "https://example.atlassian.net/browse/PROJ-123".to_owned(),
            }),
        }],
    };

    let value = report_value(&report);

    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["branches"][0]["lastAuthor"], "Pat");
    assert_eq!(value["branches"][0]["tip"], serde_json::Value::Null);
    assert_eq!(value["branches"][0]["tipInEnvironment"], true);
    assert_eq!(value["branches"][0]["unmergedAhead"], 0);
    assert_eq!(value["branches"][0]["absorbedEnvironmentMerges"], 0);
    assert_eq!(value["commitInventory"]["behindMain"]["count"], 1);
    assert_eq!(
        value["commitInventory"]["behindMain"]["commits"][0]["subject"],
        "Main work absent from QA"
    );
    assert_eq!(
        value["commitInventory"]["behindMain"]["commits"][0]["authoredDate"],
        "2026-08-03"
    );
    assert_eq!(value["branches"][0]["mergedEnvironments"][0], "qa");
    assert_eq!(
        value["branches"][0]["jiraIssue"]["fields"]["assignee"]["displayName"],
        "Pat"
    );
    assert_eq!(
        value["branches"][0]["jiraIssue"]["fields"]["fixVersions"][0]["name"],
        "1.2"
    );
    let table = format_table(&report);
    assert!(table.contains("TIP IN ENV"));
    assert!(table.contains("UNMERGED"));
    assert!(table.contains("ABSORBED"));
    assert!(table.contains("1 commit behind main"));
    assert!(table.contains("Main work absent from QA"));
}
