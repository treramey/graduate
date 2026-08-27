use super::*;

#[test]
fn scan_finds_a_feature_in_environment_but_not_main() -> Result<(), Box<dyn std::error::Error>> {
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
    commit(directory.path(), "feature", "2024-02-01T00:00:00Z")?;
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
        Some(DiffUpdate::Skeleton { branches, main, .. })
            if branches == &["feature/PROJ-123-login"] && main == "main"
    ));
    assert!(matches!(
        updates.get(1),
        Some(DiffUpdate::Measured(PromotionBranch {
            branch,
            ahead: 1,
            started,
            commits,
            jira: JiraIssueState::NotConfigured { key },
            ..
        })) if branch == "feature/PROJ-123-login"
            && started == "2024-02-01"
            && commits.len() == 1
            && commits[0].subject == "feature"
            && commits[0].author == "Test Author"
            && commits[0].date == "2024-02-01"
            && commits[0].short_id.len() == 7
            && key == "PROJ-123"
    ));
    Ok(())
}

#[test]
fn scan_reports_non_merge_commits_that_the_environment_is_behind_main(
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

    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "shared-helper", "main"],
    )?;
    std::fs::write(directory.path().join("shared"), "shared\n")?;
    run_git(directory.path(), &["add", "shared"])?;
    commit(directory.path(), "shared work", "2024-01-15T00:00:00Z")?;
    for branch in ["qa", "main"] {
        run_git(directory.path(), &["checkout", "-q", branch])?;
        run_git(
            directory.path(),
            &[
                "merge",
                "-q",
                "--no-ff",
                "shared-helper",
                "-m",
                &format!("merge shared work into {branch}"),
            ],
        )?;
    }

    std::fs::write(directory.path().join("main-work"), "main work\n")?;
    run_git(directory.path(), &["add", "main-work"])?;
    commit(
        directory.path(),
        "main work absent from QA",
        "2024-02-01T00:00:00Z",
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
        updates.iter().find(|update| matches!(update, DiffUpdate::Inventory(_))),
        Some(DiffUpdate::Inventory(inventory))
            if inventory.ahead.is_empty()
                && inventory.behind_main.len() == 1
                && inventory.behind_main[0].subject == "main work absent from QA"
                && inventory.behind_main[0].date == "2024-02-01"
    ));
    Ok(())
}

#[test]
fn merge_commits_are_excluded_from_the_ahead_count_and_history(
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
    std::fs::write(directory.path().join("feature-file"), "feature\n")?;
    run_git(directory.path(), &["add", "feature-file"])?;
    commit(directory.path(), "feature", "2024-02-01T00:00:00Z")?;
    run_git(directory.path(), &["checkout", "-q", "main"])?;
    std::fs::write(directory.path().join("main-file"), "main\n")?;
    run_git(directory.path(), &["add", "main-file"])?;
    commit(directory.path(), "main work", "2024-02-02T00:00:00Z")?;
    run_git(
        directory.path(),
        &["checkout", "-q", "feature/PROJ-123-login"],
    )?;
    run_git(
        directory.path(),
        &["merge", "-q", "--no-ff", "main", "-m", "sync main"],
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
        updates.get(1),
        Some(DiffUpdate::Measured(PromotionBranch {
            branch,
            ahead: 1,
            commits,
            ..
        })) if branch == "feature/PROJ-123-login"
            && commits.len() == 1
            && commits[0].subject == "feature"
    ));
    Ok(())
}
