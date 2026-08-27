use super::*;

#[test]
fn environment_merges_into_a_feature_branch_are_flagged() -> Result<(), Box<dyn std::error::Error>>
{
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
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-200-first"],
    )?;
    std::fs::write(directory.path().join("one"), "one\n")?;
    run_git(directory.path(), &["add", "one"])?;
    commit(directory.path(), "first feature", "2024-02-01T00:00:00Z")?;
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-200-first",
            "-m",
            "promote first",
        ],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-201-second", "main"],
    )?;
    std::fs::write(directory.path().join("two"), "two\n")?;
    run_git(directory.path(), &["add", "two"])?;
    commit(directory.path(), "second feature", "2024-02-02T00:00:00Z")?;
    run_git(
        directory.path(),
        &["merge", "-q", "--no-ff", "qa", "-m", "sync qa"],
    )?;
    run_git(directory.path(), &["checkout", "-q", "qa"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-201-second",
            "-m",
            "promote second",
        ],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-202-third", "main"],
    )?;
    std::fs::write(directory.path().join("three"), "three\n")?;
    run_git(directory.path(), &["add", "three"])?;
    commit(directory.path(), "third feature", "2024-02-03T00:00:00Z")?;
    run_git(directory.path(), &["checkout", "-q", "main"])?;
    std::fs::write(directory.path().join("main-file"), "main\n")?;
    run_git(directory.path(), &["add", "main-file"])?;
    commit(directory.path(), "main work", "2024-02-04T00:00:00Z")?;
    run_git(
        directory.path(),
        &["checkout", "-q", "feature/PROJ-202-third"],
    )?;
    run_git(
        directory.path(),
        &["merge", "-q", "--no-ff", "main", "-m", "sync main"],
    )?;
    run_git(directory.path(), &["checkout", "-q", "qa"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-202-third",
            "-m",
            "promote third",
        ],
    )?;
    for branch in [
        "main",
        "qa",
        "feature/PROJ-200-first",
        "feature/PROJ-201-second",
        "feature/PROJ-202-third",
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
            selected_branches: None,
        },
        &sender,
    )?;
    drop(sender);
    let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
    let merged_environments = |branch: &str| {
        updates.iter().find_map(|update| match update {
            DiffUpdate::Measured(row) if row.branch == branch => {
                Some(row.merged_environments.clone())
            }
            _ => None,
        })
    };

    assert_eq!(
        merged_environments("feature/PROJ-200-first").as_deref(),
        Some(&[][..])
    );
    assert_eq!(
        merged_environments("feature/PROJ-201-second").as_deref(),
        Some(&["qa".to_owned()][..])
    );
    assert_eq!(
        merged_environments("feature/PROJ-202-third").as_deref(),
        Some(&[][..])
    );
    Ok(())
}

#[test]
fn environment_merges_hidden_by_an_environment_rebuild_are_flagged(
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
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-300-noise"],
    )?;
    std::fs::write(directory.path().join("noise"), "noise\n")?;
    run_git(directory.path(), &["add", "noise"])?;
    commit(directory.path(), "noise feature", "2024-02-01T00:00:00Z")?;
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-300-noise",
            "-m",
            "Merge branch 'feature/PROJ-300-noise' into qa",
        ],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-301-stale", "qa"],
    )?;
    std::fs::write(directory.path().join("stale"), "stale\n")?;
    run_git(directory.path(), &["add", "stale"])?;
    commit(directory.path(), "stale feature", "2024-02-02T00:00:00Z")?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "qa-rebuild", "main"],
    )?;
    std::fs::write(directory.path().join("rebuild"), "rebuild\n")?;
    run_git(directory.path(), &["add", "rebuild"])?;
    commit(directory.path(), "rebuild base", "2024-02-03T00:00:00Z")?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "qa",
            "-m",
            "Merge branch 'qa' of https://example.com/repo into qa",
        ],
    )?;
    run_git(
        directory.path(),
        &["branch", "-q", "-f", "qa", "qa-rebuild"],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-302-clean", "main"],
    )?;
    std::fs::write(directory.path().join("clean"), "clean\n")?;
    run_git(directory.path(), &["add", "clean"])?;
    commit(directory.path(), "clean feature", "2024-02-04T00:00:00Z")?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-300-noise",
            "-m",
            "Merge branch 'feature/PROJ-300-noise' into feature/PROJ-302-clean",
        ],
    )?;
    run_git(directory.path(), &["checkout", "-q", "qa"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-301-stale",
            "-m",
            "promote stale",
        ],
    )?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-302-clean",
            "-m",
            "promote clean",
        ],
    )?;
    for branch in [
        "main",
        "qa",
        "feature/PROJ-301-stale",
        "feature/PROJ-302-clean",
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
            selected_branches: None,
        },
        &sender,
    )?;
    drop(sender);
    let updates = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
    let merged_environments = |branch: &str| {
        updates.iter().find_map(|update| match update {
            DiffUpdate::Measured(row) if row.branch == branch => {
                Some(row.merged_environments.clone())
            }
            _ => None,
        })
    };

    assert_eq!(
        merged_environments("feature/PROJ-301-stale").as_deref(),
        Some(&["qa".to_owned()][..])
    );
    assert_eq!(
        merged_environments("feature/PROJ-302-clean").as_deref(),
        Some(&[][..])
    );
    Ok(())
}
