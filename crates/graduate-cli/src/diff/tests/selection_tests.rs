use super::super::params::parse_selected_branches;
use super::*;

#[test]
fn known_environment_and_backup_refs_are_excluded() {
    assert!(excluded_branch("qa", "qa", "main"));
    assert!(excluded_branch("backup/old", "qa", "main"));
    assert!(!excluded_branch("feature/PROJ-1", "qa", "main"));
}

#[test]
fn json_params_select_sort_and_deduplicate_branches() -> Result<(), Box<dyn std::error::Error>> {
    let branches = parse_selected_branches(Some(
        r#"{"branches":["feature/PROJ-2","feature/PROJ-1","feature/PROJ-2"]}"#,
    ))?
    .ok_or("JSON params did not select branches")?;

    assert_eq!(branches, ["feature/PROJ-1", "feature/PROJ-2"]);
    assert!(parse_selected_branches(None)?.is_none());
    Ok(())
}

#[test]
fn json_params_reject_empty_or_unknown_selection_fields() {
    let empty = parse_selected_branches(Some(r#"{"branches":[]}"#))
        .err()
        .map(|error| error.to_string());
    let unknown = parse_selected_branches(Some(
        r#"{"branches":["feature/PROJ-1"],"branch":"feature/PROJ-2"}"#,
    ))
    .err()
    .map(|error| error.to_string());
    let invalid_ref = parse_selected_branches(Some(r#"{"branches":["feature/bad branch"]}"#))
        .err()
        .map(|error| error.to_string());

    assert!(empty.is_some_and(|message| message.contains("at least one feature branch")));
    assert!(unknown.is_some_and(|message| message.contains("unknown field `branch`")));
    assert!(invalid_ref.is_some_and(|message| message.contains("Git branch or remote name")));
}

#[test]
fn stale_remote_head_falls_back_to_an_existing_main_name() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    run_git(directory.path(), &["init", "-q", "-b", "main"])?;
    run_git(directory.path(), &["config", "user.name", "Test Author"])?;
    run_git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    )?;
    std::fs::write(directory.path().join("file"), "base\n")?;
    run_git(directory.path(), &["add", "file"])?;
    commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
    run_git(
        directory.path(),
        &["update-ref", "refs/remotes/origin/main", "refs/heads/main"],
    )?;
    run_git(
        directory.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/old-main",
        ],
    )?;
    let repository = gix::discover(directory.path())?;

    let main = resolve_main_branch(&repository, "refs/remotes/origin/", None)?;

    assert_eq!(main, "main");
    Ok(())
}

#[test]
fn scan_can_scope_a_report_to_multiple_json_selected_branches(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    run_git(directory.path(), &["init", "-q", "-b", "main"])?;
    run_git(directory.path(), &["config", "user.name", "Test Author"])?;
    run_git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    )?;
    std::fs::write(directory.path().join("base"), "base\n")?;
    run_git(directory.path(), &["add", "base"])?;
    commit(directory.path(), "base", "2024-01-01T00:00:00Z")?;
    run_git(directory.path(), &["branch", "qa", "main"])?;

    for (branch, file, date) in [
        ("feature/PROJ-1-first", "first", "2024-02-01T00:00:00Z"),
        ("feature/PROJ-2-second", "second", "2024-02-02T00:00:00Z"),
        ("feature/PROJ-3-third", "third", "2024-02-03T00:00:00Z"),
    ] {
        run_git(directory.path(), &["checkout", "-q", "main"])?;
        run_git(directory.path(), &["checkout", "-q", "-b", branch])?;
        std::fs::write(directory.path().join(file), format!("{file}\n"))?;
        run_git(directory.path(), &["add", file])?;
        commit(directory.path(), file, date)?;
        run_git(directory.path(), &["checkout", "-q", "qa"])?;
        run_git(
            directory.path(),
            &["merge", "-q", "--no-ff", branch, "-m", "promote"],
        )?;
    }
    for branch in [
        "main",
        "qa",
        "feature/PROJ-1-first",
        "feature/PROJ-2-second",
        "feature/PROJ-3-third",
    ] {
        run_git(
            directory.path(),
            &[
                "update-ref",
                &format!("refs/remotes/origin/{branch}"),
                &format!("refs/heads/{branch}"),
            ],
        )?;
    }
    run_git(
        directory.path(),
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    )?;

    let (sender, mut receiver) = mpsc::unbounded_channel();
    scan_repository(
        &ScanOptions {
            repository: directory.path().to_path_buf(),
            environment: "qa".to_owned(),
            main: None,
            remote: "origin".to_owned(),
            jira_configured: false,
            fetch_before_scan: false,
            selected_branches: Some(vec![
                "feature/PROJ-3-third".to_owned(),
                "feature/PROJ-1-first".to_owned(),
            ]),
        },
        &sender,
    )?;
    drop(sender);
    let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
    let measured = updates
        .iter()
        .filter_map(|update| match update {
            DiffUpdate::Measured(row) => Some(row.branch.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let inventory = updates.iter().find_map(|update| match update {
        DiffUpdate::Inventory(inventory) => Some(inventory),
        _ => None,
    });

    assert!(matches!(
        updates.first(),
        Some(DiffUpdate::Skeleton { branches, .. })
            if branches == &["feature/PROJ-1-first", "feature/PROJ-3-third"]
    ));
    assert_eq!(measured, ["feature/PROJ-1-first", "feature/PROJ-3-third"]);
    assert_eq!(
        inventory.map(|inventory| {
            inventory
                .ahead
                .iter()
                .map(|commit| commit.subject.as_str())
                .collect::<Vec<_>>()
        }),
        Some(vec!["third", "first"])
    );

    let (missing_sender, _missing_receiver) = mpsc::unbounded_channel();
    let missing = scan_repository(
        &ScanOptions {
            repository: directory.path().to_path_buf(),
            environment: "qa".to_owned(),
            main: None,
            remote: "origin".to_owned(),
            jira_configured: false,
            fetch_before_scan: false,
            selected_branches: Some(vec!["feature/PROJ-404-missing".to_owned()]),
        },
        &missing_sender,
    )
    .err();
    assert!(matches!(
        missing,
        Some(CliError::InvalidInput(message)) if message.contains("does not exist")
    ));
    Ok(())
}
