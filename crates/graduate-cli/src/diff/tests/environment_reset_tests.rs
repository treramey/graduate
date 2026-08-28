//! Environment merges that survive an environment reset.

use super::*;

#[test]
fn no_ff_environment_merges_survive_an_environment_reset() -> Result<(), Box<dyn std::error::Error>>
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
        &["checkout", "-q", "-b", "feature/PROJ-400-noise"],
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
            "feature/PROJ-400-noise",
            "-m",
            "promote noise",
        ],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-401-dependent", "main"],
    )?;
    std::fs::write(directory.path().join("dependent"), "dependent\n")?;
    run_git(directory.path(), &["add", "dependent"])?;
    commit(
        directory.path(),
        "dependent feature",
        "2024-02-02T00:00:00Z",
    )?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "qa",
            "-m",
            "Merge branch 'qa' into feature/PROJ-401-dependent",
        ],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-402-sibling", "main"],
    )?;
    std::fs::write(directory.path().join("sibling"), "sibling\n")?;
    run_git(directory.path(), &["add", "sibling"])?;
    commit(directory.path(), "sibling feature", "2024-02-03T00:00:00Z")?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-400-noise",
            "-m",
            "Merge branch 'feature/PROJ-400-noise' into feature/PROJ-402-sibling",
        ],
    )?;
    run_git(
        directory.path(),
        &["checkout", "-q", "-b", "feature/PROJ-403-remote", "main"],
    )?;
    std::fs::write(directory.path().join("remote"), "remote\n")?;
    run_git(directory.path(), &["add", "remote"])?;
    commit(directory.path(), "remote feature", "2024-02-04T00:00:00Z")?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "qa",
            "-m",
            "Merge remote-tracking branch 'origin/qa'",
        ],
    )?;
    // Reset the environment so the old promote-noise merge leaves its
    // first-parent line, then promote the features onto the new line.
    run_git(directory.path(), &["branch", "-q", "-f", "qa", "main"])?;
    run_git(directory.path(), &["checkout", "-q", "qa"])?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-401-dependent",
            "-m",
            "promote dependent",
        ],
    )?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-402-sibling",
            "-m",
            "promote sibling",
        ],
    )?;
    run_git(
        directory.path(),
        &[
            "merge",
            "-q",
            "--no-ff",
            "feature/PROJ-403-remote",
            "-m",
            "promote remote",
        ],
    )?;
    for branch in [
        "main",
        "qa",
        "feature/PROJ-401-dependent",
        "feature/PROJ-402-sibling",
        "feature/PROJ-403-remote",
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
            check_merge_onto_main: false,
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
        merged_environments("feature/PROJ-401-dependent").as_deref(),
        Some(&["qa".to_owned()][..])
    );
    assert_eq!(
        merged_environments("feature/PROJ-402-sibling").as_deref(),
        Some(&[][..])
    );
    assert_eq!(
        merged_environments("feature/PROJ-403-remote").as_deref(),
        Some(&["qa".to_owned()][..])
    );
    Ok(())
}
