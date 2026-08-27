use super::*;

#[test]
fn main_merges_and_environment_like_branch_names_are_never_flagged(
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
        &["checkout", "-q", "-b", "feature/qa", "main"],
    )?;
    std::fs::write(directory.path().join("lookalike"), "lookalike\n")?;
    run_git(directory.path(), &["add", "lookalike"])?;
    commit(
        directory.path(),
        "lookalike feature",
        "2024-02-01T00:00:00Z",
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-500-clean", "main"],
    )?;
    std::fs::write(directory.path().join("clean"), "clean\n")?;
    run_git(directory.path(), &["add", "clean"])?;
    commit(directory.path(), "clean feature", "2024-02-02T00:00:00Z")?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/qa",
            "-m",
            "Merge remote-tracking branch 'origin/feature/qa'",
        ],
    )?;
    run_git(directory.path(), &["checkout", "-q", "main"])?;
    std::fs::write(directory.path().join("mainline"), "mainline\n")?;
    run_git(directory.path(), &["add", "mainline"])?;
    commit(directory.path(), "main work", "2024-02-03T00:00:00Z")?;
    run_git(
        directory.path(),
        &["checkout", "-q", "feature/PROJ-500-clean"],
    )?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "main",
            "-m",
            "Merge branch 'main' into feature/PROJ-500-clean",
        ],
    )?;
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-500-clean",
            "-m",
            "promote clean",
        ],
    )?;
    for branch in ["main", "qa", "feature/PROJ-500-clean"] {
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
        merged_environments("feature/PROJ-500-clean").as_deref(),
        Some(&[][..])
    );
    Ok(())
}

#[test]
fn multiple_environment_merges_are_deduplicated_and_sorted(
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
    for helper in ["helper-one", "helper-two", "helper-three"] {
        run_git(directory.path(), &["checkout", "-q", "-b", helper, "main"])?;
        std::fs::write(directory.path().join(helper), format!("{helper}\n"))?;
        run_git(directory.path(), &["add", helper])?;
        commit(
            directory.path(),
            &format!("{helper} work"),
            "2024-02-01T00:00:00Z",
        )?;
    }
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-600-mixed", "main"],
    )?;
    std::fs::write(directory.path().join("mixed"), "mixed\n")?;
    run_git(directory.path(), &["add", "mixed"])?;
    commit(directory.path(), "mixed feature", "2024-02-02T00:00:00Z")?;
    for (helper, subject) in [
        (
            "helper-one",
            "Merge branch 'staging' into feature/PROJ-600-mixed",
        ),
        (
            "helper-two",
            "Merge branch 'qa' into feature/PROJ-600-mixed",
        ),
        (
            "helper-three",
            "Merge branch 'qa' into feature/PROJ-600-mixed",
        ),
    ] {
        run_git(
            directory.path(),
            &["merge", "-q", "--no-ff", helper, "-m", subject],
        )?;
    }
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-600-mixed",
            "-m",
            "promote mixed",
        ],
    )?;
    for branch in ["main", "qa", "feature/PROJ-600-mixed"] {
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
    let merged = updates.iter().find_map(|update| match update {
        DiffUpdate::Measured(row) if row.branch == "feature/PROJ-600-mixed" => {
            Some(row.merged_environments.clone())
        }
        _ => None,
    });

    assert_eq!(
        merged.as_deref(),
        Some(&["qa".to_owned(), "staging".to_owned()][..])
    );
    Ok(())
}
