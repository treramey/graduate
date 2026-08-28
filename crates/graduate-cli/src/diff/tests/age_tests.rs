use graduate::promotion::{PromotionAgeReport, PromotionCommit, ReportDate};

use super::super::age_csv::format_age_csv;
use super::super::report_json::age_report_value;
use super::super::report_table::format_age_table;
use super::*;

#[test]
fn age_json_is_self_describing_and_uses_explicit_thresholds(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut report = PromotionReport {
        environment: "qa".to_owned(),
        main: "main".to_owned(),
        inventory: EnvironmentInventory::default(),
        branches: vec![PromotionBranch {
            branch: "feature/legacy".to_owned(),
            started: "2019-12-31".to_owned(),
            last: "2026-08-01".to_owned(),
            ahead: 2,
            last_author: "Pat".to_owned(),
            commits: vec![
                PromotionCommit {
                    id: "111111111111".to_owned(),
                    short_id: "1111111".to_owned(),
                    subject: "Current".to_owned(),
                    author: "Pat".to_owned(),
                    date: "2026-08-01".to_owned(),
                },
                PromotionCommit {
                    id: "222222222222".to_owned(),
                    short_id: "2222222".to_owned(),
                    subject: "Legacy".to_owned(),
                    author: "Pat".to_owned(),
                    date: "2019-12-31".to_owned(),
                },
            ],
            merged_environments: Vec::new(),
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NoTicket,
        }],
    };
    report.inventory.ahead = report.branches[0].commits.clone();
    report.inventory.behind_main = vec![PromotionCommit {
        id: "333333333333".to_owned(),
        short_id: "3333333".to_owned(),
        subject: "Main-only work".to_owned(),
        author: "Alex".to_owned(),
        date: "2026-08-02".to_owned(),
    }];
    let age = PromotionAgeReport::new(
        &report.inventory.ahead,
        &report.branches,
        ReportDate::parse("2026-08-04")?,
    )?;

    let value = age_report_value(&report, &age);

    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["report"], "age");
    assert_eq!(value["counting"], "uniqueEnvironmentCommits");
    assert_eq!(value["commitInventory"]["behindMain"]["count"], 1);
    assert_eq!(
        value["commitInventory"]["behindMain"]["commits"][0]["subject"],
        "Main-only work"
    );
    assert_eq!(value["asOf"], "2026-08-04");
    assert_eq!(value["totalCommits"], 2);
    assert_eq!(value["oldestYear"], 2019);
    assert_eq!(value["buckets"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["buckets"][0]["period"]["kind"], "year");
    assert_eq!(value["buckets"][0]["period"]["year"], 2026);
    assert_eq!(value["buckets"][1]["period"]["year"], 2019);
    assert_eq!(value["buckets"][0]["sharePercent"], 50.0);
    assert_eq!(value["thresholds"]["last90Days"]["since"], "2026-05-07");
    assert_eq!(
        value["thresholds"]["olderThanOneYear"]["before"],
        "2025-08-04"
    );
    assert_eq!(value["oldestBranches"][0]["branch"], "feature/legacy");
    Ok(())
}

#[test]
fn age_table_calls_out_old_work_and_the_branches_that_carry_it(
) -> Result<(), Box<dyn std::error::Error>> {
    let report = PromotionReport {
        environment: "qa".to_owned(),
        main: "main".to_owned(),
        inventory: EnvironmentInventory {
            ahead: Vec::new(),
            behind_main: vec![PromotionCommit {
                id: "333333333333".to_owned(),
                short_id: "3333333".to_owned(),
                subject: "Main-only work".to_owned(),
                author: "Alex".to_owned(),
                date: "2026-08-02".to_owned(),
            }],
        },
        branches: vec![PromotionBranch {
            branch: "feature/legacy".to_owned(),
            started: "2019-12-31".to_owned(),
            last: "2019-12-31".to_owned(),
            ahead: 1,
            last_author: "Pat".to_owned(),
            commits: vec![PromotionCommit {
                id: "222222222222".to_owned(),
                short_id: "2222222".to_owned(),
                subject: "Legacy".to_owned(),
                author: "Pat".to_owned(),
                date: "2019-12-31".to_owned(),
            }],
            merged_environments: Vec::new(),
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NoTicket,
        }],
    };
    let age = PromotionAgeReport::new(
        &report.branches[0].commits,
        &report.branches,
        ReportDate::parse("2026-08-04")?,
    )?;

    let table = format_age_table(&report, &age);

    assert!(table.contains("Age of unshipped work in qa but not main"));
    assert!(table.contains("2019"));
    assert!(!table.contains("Before 2020"));
    assert!(table.contains("Older than one year"));
    assert!(table.contains("Will not ship without a decision"));
    assert!(table.contains("Branches carrying commits from 2019"));
    assert!(table.contains("feature/legacy"));
    assert!(table.contains("1 commit behind main"));
    assert!(table.contains("Main-only work"));
    Ok(())
}

#[test]
fn age_csv_identifies_bucket_and_threshold_rows() -> Result<(), Box<dyn std::error::Error>> {
    let report = PromotionReport {
        environment: "qa".to_owned(),
        main: "main".to_owned(),
        inventory: EnvironmentInventory {
            ahead: Vec::new(),
            behind_main: vec![PromotionCommit {
                id: "333333333333".to_owned(),
                short_id: "3333333".to_owned(),
                subject: "Main-only work".to_owned(),
                author: "Alex".to_owned(),
                date: "2026-08-02".to_owned(),
            }],
        },
        branches: vec![PromotionBranch {
            branch: "feature/current".to_owned(),
            started: "2026-08-01".to_owned(),
            last: "2026-08-01".to_owned(),
            ahead: 1,
            last_author: "Pat".to_owned(),
            commits: vec![PromotionCommit {
                id: "111111111111".to_owned(),
                short_id: "1111111".to_owned(),
                subject: "Current".to_owned(),
                author: "Pat".to_owned(),
                date: "2026-08-01".to_owned(),
            }],
            merged_environments: Vec::new(),
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NoTicket,
        }],
    };
    let age = PromotionAgeReport::new(
        &report.branches[0].commits,
        &report.branches,
        ReportDate::parse("2026-08-04")?,
    )?;

    let csv = format_age_csv(&report, &age)?;

    assert!(csv.lines().next().is_some_and(|header| {
        header.contains("\"rowType\"") && header.contains("\"assessment\"")
    }));
    assert!(csv.contains(
        "\"bucket\",\"qa\",\"main\",\"2026-08-04\",\"uniqueEnvironmentCommits\",\"year\",\"2026\""
    ));
    assert!(csv.contains(
        "\"threshold\",\"qa\",\"main\",\"2026-08-04\",\"uniqueEnvironmentCommits\",\"last90Days\""
    ));
    assert!(csv.contains("\"threshold\",\"qa\",\"main\",\"2026-08-04\",\"uniqueEnvironmentCommits\",\"olderThanOneYear\""));
    assert!(csv.lines().any(|line| {
        line.starts_with("\"inventory\",\"qa\",\"main\"") && line.contains("\"behindMain\",\"1\"")
    }));
    assert!(csv.lines().any(|line| {
        line.starts_with("\"commit\",\"qa\",\"main\"")
            && line.contains("\"behindMain\"")
            && line.contains("\"333333333333\"")
            && line.contains("\"Main-only work\"")
    }));
    Ok(())
}
