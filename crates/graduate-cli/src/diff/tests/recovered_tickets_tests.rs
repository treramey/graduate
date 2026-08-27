use super::*;

#[test]
fn work_from_a_deleted_branch_is_recovered_by_commit_subject_jira_key(
) -> Result<(), Box<dyn std::error::Error>> {
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
    run_git(directory.path(), &["checkout", "-q", "-b", "PROJ-500"])?;
    std::fs::write(directory.path().join("file"), "base\nwidget\n")?;
    run_git(directory.path(), &["add", "file"])?;
    commit(
        directory.path(),
        "PROJ-500: add widget",
        "2024-02-01T00:00:00Z",
    )?;
    std::fs::write(directory.path().join("file"), "base\nwidget\npolish\n")?;
    run_git(directory.path(), &["add", "file"])?;
    commit(
        directory.path(),
        "PROJ-500: polish widget",
        "2024-02-02T00:00:00Z",
    )?;
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "PROJ-500",
            "-m",
            "Merged PR 1: PROJ-500",
        ],
    )?;
    // Only main and qa exist on the remote: the feature branch was
    // deleted when its pull request completed.
    for branch in ["main", "qa"] {
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

    assert!(matches!(
        updates.first(),
        Some(DiffUpdate::Skeleton { branches, .. }) if branches == &["PROJ-500"]
    ));
    assert!(matches!(
        updates.get(1),
        Some(DiffUpdate::Measured(PromotionBranch {
            branch,
            started,
            last,
            ahead: 2,
            commits,
            jira: JiraIssueState::NotConfigured { key },
            ..
        })) if branch == "PROJ-500"
            && started == "2024-02-01"
            && last == "2024-02-02"
            && commits.len() == 2
            && commits[0].subject == "PROJ-500: polish widget"
            && commits[1].subject == "PROJ-500: add widget"
            && key == "PROJ-500"
    ));
    Ok(())
}

#[test]
fn merge_commit_subjects_never_create_recovered_ticket_rows(
) -> Result<(), Box<dyn std::error::Error>> {
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
    // A branch whose commits carry no ticket key, merged with a subject
    // that does: the key must not surface because merge commits are
    // skipped and no non-merge commit names it.
    run_git(directory.path(), &["checkout", "-q", "-b", "throwaway"])?;
    std::fs::write(directory.path().join("file"), "base\nwork\n")?;
    run_git(directory.path(), &["add", "file"])?;
    commit(directory.path(), "no ticket here", "2024-02-01T00:00:00Z")?;
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "throwaway",
            "-m",
            "Merged PR 3: PROJ-700",
        ],
    )?;
    for branch in ["main", "qa"] {
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

    assert!(matches!(
        updates.first(),
        Some(DiffUpdate::Skeleton { branches, .. }) if branches.is_empty()
    ));
    assert_eq!(updates.len(), 2);
    Ok(())
}

#[test]
fn a_surviving_branch_suppresses_the_recovered_row_for_its_key(
) -> Result<(), Box<dyn std::error::Error>> {
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
        &["checkout", "-q", "-b", "feature/PROJ-123-login"],
    )?;
    std::fs::write(directory.path().join("file"), "base\nfeature\n")?;
    run_git(directory.path(), &["add", "file"])?;
    commit(
        directory.path(),
        "PROJ-123: add login",
        "2024-02-01T00:00:00Z",
    )?;
    run_git(directory.path(), &["checkout", "-q", "-b", "qa", "main"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-123-login",
            "-m",
            "promote",
        ],
    )?;
    for branch in ["main", "qa", "feature/PROJ-123-login"] {
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

    assert!(matches!(
        updates.first(),
        Some(DiffUpdate::Skeleton { branches, .. })
            if branches == &["feature/PROJ-123-login"]
    ));
    assert_eq!(updates.len(), 3);
    Ok(())
}
