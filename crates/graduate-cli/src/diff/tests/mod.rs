//! Shared fixtures and helpers.

use graduate::promotion::MergeOntoMain;

use super::*;
use crate::shared::environment_git::{excluded_branch, resolve_main_branch};
use crate::shared::git_process::fetch_status_message;

mod age_tests;
mod environment_merge_edge_cases_tests;
mod environment_merges_tests;
mod environment_reset_tests;
mod formats_tests;
mod membership_tests;
mod merge_check_tests;
mod output_paths_tests;
mod readiness_tests;
mod recovered_tickets_tests;
mod scan_tests;
mod selection_tests;

fn run_git(path: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = crate::shared::environment_git::isolated_git_command()
        .args(["-c", "core.fsmonitor=false"])
        .args(arguments)
        .current_dir(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {} failed with {status}", arguments.join(" ")).into())
    }
}

fn commit(path: &Path, message: &str, date: &str) -> Result<(), Box<dyn std::error::Error>> {
    let status = crate::shared::environment_git::isolated_git_command()
        .args(["-c", "core.fsmonitor=false"])
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git commit failed with {status}").into())
    }
}

/// Create an empty repository on `main` with a single `base` commit.
fn init_repository(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    run_git(path, &["init", "-q", "-b", "main"])?;
    run_git(path, &["config", "user.name", "Test Author"])?;
    run_git(path, &["config", "user.email", "test@example.com"])?;
    std::fs::write(path.join("base"), "base\n")?;
    run_git(path, &["add", "base"])?;
    commit(path, "base", "2024-01-01T00:00:00Z")
}

/// Write `content` to `file` and commit it with `message`.
fn commit_file(
    path: &Path,
    file: &str,
    content: &str,
    message: &str,
    date: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(path.join(file), content)?;
    run_git(path, &["add", file])?;
    commit(path, message, date)
}

/// Mirror local branches onto `origin` and point `origin/HEAD` at `main`.
fn publish(path: &Path, branches: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    for branch in branches {
        run_git(
            path,
            &[
                "update-ref",
                &format!("refs/remotes/origin/{branch}"),
                &format!("refs/heads/{branch}"),
            ],
        )?;
    }
    run_git(
        path,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    )
}

fn scan_options(path: &Path, environment: &str) -> ScanOptions {
    ScanOptions {
        repository: path.to_path_buf(),
        environment: environment.to_owned(),
        main: None,
        remote: "origin".to_owned(),
        jira_configured: false,
        fetch_before_scan: false,
        selected_branches: None,
        check_merge_onto_main: false,
    }
}

fn measured_rows(options: &ScanOptions) -> Result<Vec<PromotionBranch>, CliError> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    scan_repository(options, &sender)?;
    drop(sender);
    Ok(std::iter::from_fn(|| receiver.try_recv().ok())
        .filter_map(|update| match update {
            DiffUpdate::Measured(row) => Some(row),
            _ => None,
        })
        .collect())
}
