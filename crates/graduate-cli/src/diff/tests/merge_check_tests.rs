use super::*;

/// `feature/clean` edits a different file than main; `feature/conflict`
/// edits the same line of `base` that main changed after both were promoted.
fn merge_fixture(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
    publish(path, &["main", "qa", "feature/clean", "feature/conflict"])
}

fn loose_object_count(path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let mut count = 0;
    for entry in std::fs::read_dir(path.join(".git/objects"))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            count += std::fs::read_dir(entry.path())?.count();
        }
    }
    Ok(count)
}

#[test]
fn readiness_scan_reports_clean_and_conflicting_merges_without_writing_objects(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    merge_fixture(directory.path())?;
    let objects_before = loose_object_count(directory.path())?;

    let mut options = scan_options(directory.path(), "qa");
    options.check_merge_onto_main = true;
    let rows = measured_rows(&options)?;

    let clean = rows
        .iter()
        .find(|row| row.branch == "feature/clean")
        .ok_or("feature/clean missing")?;
    assert_eq!(
        clean.merge_onto_main,
        Some(MergeOntoMain {
            clean: true,
            conflicting_paths: 0,
        })
    );
    let conflict = rows
        .iter()
        .find(|row| row.branch == "feature/conflict")
        .ok_or("feature/conflict missing")?;
    assert_eq!(
        conflict.merge_onto_main,
        Some(MergeOntoMain {
            clean: false,
            conflicting_paths: 1,
        })
    );
    assert_eq!(loose_object_count(directory.path())?, objects_before);
    Ok(())
}

#[test]
fn default_scans_do_not_run_the_merge_check() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    merge_fixture(directory.path())?;

    let rows = measured_rows(&scan_options(directory.path(), "qa"))?;

    assert!(rows.iter().all(|row| row.merge_onto_main.is_none()));
    Ok(())
}

#[test]
fn merge_fields_are_null_or_empty_until_computed() {
    let mut report = PromotionReport {
        environment: "qa".to_owned(),
        main: "main".to_owned(),
        inventory: EnvironmentInventory::default(),
        branches: vec![PromotionBranch {
            branch: "feature/x".to_owned(),
            tip: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            started: "2024-01-01".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 1,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: Vec::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NoTicket,
        }],
    };
    let value = super::super::report_json::report_value(&report);
    assert_eq!(
        value["branches"][0]["mergesCleanlyOntoMain"],
        serde_json::Value::Null
    );
    assert_eq!(
        value["branches"][0]["conflictingPaths"],
        serde_json::Value::Null
    );
    let csv = super::super::report_csv::format_csv(&report).unwrap_or_default();
    assert!(csv.contains("\"0\",\"\",\"\",\"Pat\""));

    report.branches[0].merge_onto_main = Some(MergeOntoMain {
        clean: false,
        conflicting_paths: 2,
    });
    let value = super::super::report_json::report_value(&report);
    assert_eq!(value["branches"][0]["mergesCleanlyOntoMain"], false);
    assert_eq!(value["branches"][0]["conflictingPaths"], 2);
    let csv = super::super::report_csv::format_csv(&report).unwrap_or_default();
    assert!(csv.contains("\"0\",\"no\",\"2\",\"Pat\""));
    let table = super::super::report_table::format_table(&report);
    assert!(table.contains("MERGES CLEAN"));
}
