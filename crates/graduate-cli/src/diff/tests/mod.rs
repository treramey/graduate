//! Shared fixtures and helpers.

use super::*;
use crate::environment_git::{excluded_branch, resolve_main_branch};
use crate::git_process::fetch_status_message;

mod age_tests;
mod environment_merge_edge_cases_tests;
mod environment_merges_tests;
mod environment_reset_tests;
mod formats_tests;
mod output_paths_tests;
mod recovered_tickets_tests;
mod scan_tests;
mod selection_tests;

fn run_git(path: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = crate::environment_git::isolated_git_command()
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
    let status = crate::environment_git::isolated_git_command()
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
