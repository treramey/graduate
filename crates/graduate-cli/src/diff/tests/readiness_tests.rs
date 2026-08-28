use graduate::promotion::{
    JiraIssueSummary, PromotionCommit, PromotionReadinessReport, ReadinessBucket, NO_TICKET_ROW,
};

use super::super::readiness_csv::format_readiness_csv;
use super::super::report_readiness::{format_readiness_table, readiness_report_value};
use super::*;

fn row(branch: &str, author: &str) -> PromotionBranch {
    PromotionBranch {
        branch: branch.to_owned(),
        tip: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        started: "2024-01-01".to_owned(),
        last: "2024-01-02".to_owned(),
        ahead: 1,
        last_author: author.to_owned(),
        commits: vec![PromotionCommit {
            id: format!("{branch}-commit"),
            short_id: "0123456".to_owned(),
            subject: branch.to_owned(),
            author: author.to_owned(),
            date: "2024-01-02".to_owned(),
        }],
        merged_environments: Vec::new(),
        tip_in_environment: true,
        unmerged_ahead: 0,
        absorbed_environment_merges: 0,
        merge_onto_main: Some(MergeOntoMain {
            clean: true,
            conflicting_paths: 0,
        }),
        jira: JiraIssueState::NoTicket,
    }
}

fn jira(key: &str, status: &str, category: Option<&str>) -> JiraIssueState {
    JiraIssueState::Loaded(JiraIssueSummary {
        key: key.to_owned(),
        api_url: format!("https://example.atlassian.net/rest/api/3/issue/{key}"),
        summary: "Work".to_owned(),
        status: status.to_owned(),
        status_category: category.map(str::to_owned),
        assignee: None,
        fix_versions: Vec::new(),
        url: format!("https://example.atlassian.net/browse/{key}"),
    })
}

fn fixture_branches() -> Vec<PromotionBranch> {
    let ready = row("feature/ready", "Pat");
    let mut stale = row("feature/stale", "Pat");
    stale.merge_onto_main = Some(MergeOntoMain {
        clean: false,
        conflicting_paths: 2,
    });
    let mut partial = row("feature/partial", "Alex");
    partial.tip_in_environment = false;
    partial.unmerged_ahead = 3;
    partial.merge_onto_main = Some(MergeOntoMain {
        clean: false,
        conflicting_paths: 1,
    });
    let mut tainted = row("feature/PROJ-7-tainted", "Alex");
    tainted.absorbed_environment_merges = 2;
    tainted.jira = jira("PROJ-7", "Done", Some("done"));
    let mut closed = row("feature/PROJ-8-closed", "Alex");
    closed.jira = jira("PROJ-8", "Won't Do", Some("done"));
    let mut closed_by_name = row("feature/PROJ-9-resolved", "Sam");
    closed_by_name.jira = jira("PROJ-9", "Resolved", None);
    let mut orphan = row("PROJ-10", "Sam");
    orphan.tip = String::new();
    orphan.jira = JiraIssueState::NotFound {
        key: "PROJ-10".to_owned(),
    };
    vec![
        ready,
        stale,
        partial,
        tainted,
        closed,
        closed_by_name,
        orphan,
    ]
}

fn bucket_of(report: &PromotionReadinessReport, branch: &str) -> Option<ReadinessBucket> {
    report
        .groups
        .iter()
        .flat_map(|group| group.rows.iter())
        .find(|row| row.branch == branch)
        .map(|row| row.bucket)
}

#[test]
fn buckets_follow_precedence_and_group_by_owner() {
    let untracked = vec![
        PromotionCommit {
            id: "loose-1".to_owned(),
            short_id: "loose1".to_owned(),
            subject: "loose work".to_owned(),
            author: "Sam".to_owned(),
            date: "2024-03-01".to_owned(),
        },
        PromotionCommit {
            id: "feature/ready-commit".to_owned(),
            short_id: "0123456".to_owned(),
            subject: "attributed".to_owned(),
            author: "Pat".to_owned(),
            date: "2024-01-02".to_owned(),
        },
    ];
    let report = PromotionReadinessReport::new(&fixture_branches(), &untracked);

    assert_eq!(
        bucket_of(&report, "feature/ready"),
        Some(ReadinessBucket::Ready)
    );
    assert_eq!(
        bucket_of(&report, "feature/stale"),
        Some(ReadinessBucket::Stale)
    );
    // Partial outranks a conflicting merge.
    assert_eq!(
        bucket_of(&report, "feature/partial"),
        Some(ReadinessBucket::Partial)
    );
    // Closed outranks tainted.
    assert_eq!(
        bucket_of(&report, "feature/PROJ-7-tainted"),
        Some(ReadinessBucket::Closed)
    );
    assert_eq!(
        bucket_of(&report, "feature/PROJ-8-closed"),
        Some(ReadinessBucket::Closed)
    );
    assert_eq!(
        bucket_of(&report, "feature/PROJ-9-resolved"),
        Some(ReadinessBucket::Closed)
    );
    assert_eq!(bucket_of(&report, "PROJ-10"), Some(ReadinessBucket::Orphan));
    assert_eq!(
        bucket_of(&report, NO_TICKET_ROW),
        Some(ReadinessBucket::Orphan)
    );

    let owners = report
        .groups
        .iter()
        .map(|group| group.owner.as_str())
        .collect::<Vec<_>>();
    assert_eq!(owners, ["Alex", "Pat", "Sam"]);
    let sam = &report.groups[2];
    assert_eq!(sam.rows.len(), 3);
    let loose = sam
        .rows
        .iter()
        .find(|row| row.branch == NO_TICKET_ROW)
        .map(|row| (row.ahead, row.last.as_str(), row.tip.clone()));
    assert_eq!(loose, Some((1, "2024-03-01", None)));
    assert_eq!(report.total(ReadinessBucket::Closed), 3);
    assert_eq!(report.total(ReadinessBucket::Orphan), 2);
    assert_eq!(report.total(ReadinessBucket::Ready), 1);
}

#[test]
fn a_tainted_open_ticket_is_tainted_and_a_clean_extended_branch_is_partial() {
    let mut tainted = row("feature/PROJ-7-tainted", "Alex");
    tainted.absorbed_environment_merges = 1;
    tainted.jira = jira("PROJ-7", "In Progress", Some("indeterminate"));
    let mut partial = row("feature/partial", "Alex");
    partial.tip_in_environment = false;
    let report = PromotionReadinessReport::new(&[tainted, partial], &[]);

    assert_eq!(
        bucket_of(&report, "feature/PROJ-7-tainted"),
        Some(ReadinessBucket::Tainted)
    );
    assert_eq!(
        bucket_of(&report, "feature/partial"),
        Some(ReadinessBucket::Partial)
    );
}

#[test]
fn readiness_formats_expose_buckets_owners_and_remediation() -> Result<(), CliError> {
    let report = PromotionReport {
        environment: "qa".to_owned(),
        main: "main".to_owned(),
        inventory: EnvironmentInventory::default(),
        branches: fixture_branches(),
    };
    let readiness = PromotionReadinessReport::new(&report.branches, &report.inventory.ahead);

    let value = readiness_report_value(&report, &readiness);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["report"], "readiness");
    assert_eq!(value["buckets"]["closed"]["count"], 3);
    assert!(value["buckets"]["stale"]["remediation"]
        .as_str()
        .is_some_and(|text| text.contains("main")));
    assert_eq!(value["owners"][0]["owner"], "Alex");
    assert_eq!(value["owners"][0]["counts"]["closed"], 2);
    assert_eq!(value["owners"][1]["branches"][1]["bucket"], "stale");
    assert_eq!(value["owners"][1]["branches"][1]["conflictingPaths"], 2);
    assert_eq!(
        value["owners"][2]["branches"][0]["tip"],
        serde_json::Value::Null
    );

    let table = format_readiness_table(&report, &readiness);
    assert!(table.contains("Rebuild readiness for qa against main"));
    assert!(table.contains("Alex  ·  1 partial, 2 closed"));
    assert!(table.contains("Pat  ·  1 ready, 1 stale"));
    assert!(table.contains("no (2)"));
    assert!(table.contains("Remediation"));

    let csv = format_readiness_csv(&report, &readiness)?;
    assert!(csv.lines().next().is_some_and(
        |line| line.starts_with("\"rowType\",\"environment\",\"main\",\"owner\",\"bucket\"")
    ));
    assert_eq!(
        csv.lines()
            .filter(|line| line.starts_with("\"summary\""))
            .count(),
        6
    );
    assert!(csv.contains("\"branch\",\"qa\",\"main\",\"Pat\",\"stale\",\"\",\"feature/stale\""));
    Ok(())
}

#[test]
fn readiness_scan_buckets_ready_and_stale_branches() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path();
    init_repository(path)?;
    run_git(path, &["checkout", "-q", "-b", "feature/clean", "main"])?;
    commit_file(
        path,
        "clean",
        "clean\n",
        "clean work",
        "2024-02-01T00:00:00Z",
    )?;
    run_git(path, &["checkout", "-q", "-b", "feature/conflict", "main"])?;
    commit_file(
        path,
        "base",
        "feature\n",
        "conflict work",
        "2024-02-01T00:00:00Z",
    )?;
    run_git(path, &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        path,
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/clean",
            "-m",
            "promote clean",
        ],
    )?;
    run_git(
        path,
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/conflict",
            "-m",
            "promote conflict",
        ],
    )?;
    run_git(path, &["checkout", "-q", "main"])?;
    commit_file(path, "base", "main\n", "main work", "2024-02-02T00:00:00Z")?;
    publish(path, &["main", "qa", "feature/clean", "feature/conflict"])?;

    let mut options = scan_options(path, "qa");
    options.check_merge_onto_main = true;
    let rows = measured_rows(&options)?;
    let report = PromotionReadinessReport::new(&rows, &[]);

    assert_eq!(
        bucket_of(&report, "feature/clean"),
        Some(ReadinessBucket::Ready)
    );
    assert_eq!(
        bucket_of(&report, "feature/conflict"),
        Some(ReadinessBucket::Stale)
    );
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].owner, "Test Author");
    Ok(())
}
