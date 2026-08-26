//! Shared Git subprocess boundaries.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::CliError;

pub(crate) fn fetch_remote(remote: &str, interactive: bool) -> Result<(), CliError> {
    let pat = std::env::var("GIT_PAT")
        .ok()
        .filter(|value| !value.is_empty());
    if let Some(message) = fetch_status_message(remote, pat.is_some(), interactive) {
        eprintln!("{message}");
    }

    if let Some(pat) = pat {
        let mut authenticated = Command::new("git");
        authenticated
            .args([
                "-c",
                "credential.helper=",
                "-c",
                "credential.helper=!f() { echo username=x-access-token; echo \"password=$GIT_PAT\"; }; f",
                "fetch",
                "--prune",
                remote,
            ])
            .env("GIT_PAT", pat)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        return check_fetch(authenticated.output(), true, !interactive);
    }
    if !interactive {
        let mut unattended = Command::new("git");
        unattended
            .args([
                "-c",
                "credential.interactive=false",
                "fetch",
                "--prune",
                remote,
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        return check_fetch(unattended.output(), false, true);
    }
    let mut command = Command::new("git");
    command
        .args(["fetch", "--prune", remote])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    check_fetch(command.output(), false, false)
}

pub(crate) fn fetch_restack_remote(remote: &str, source: &Path) -> Result<(), CliError> {
    let pat = std::env::var("GIT_PAT")
        .ok()
        .filter(|value| !value.is_empty());
    let used_pat = pat.is_some();
    if let Some(message) = fetch_status_message(remote, used_pat, false) {
        eprintln!("{message}");
    }
    let refspec = format!("+refs/heads/*:refs/remotes/{remote}/*");
    let mut command = Command::new("git");
    clear_repository_location_environment(&mut command);
    command.current_dir(source);
    if let Some(pat) = pat {
        command
            .args([
                "-c",
                "credential.helper=",
                "-c",
                "credential.helper=!f() { echo username=x-access-token; echo \"password=$GIT_PAT\"; }; f",
            ])
            .env("GIT_PAT", pat);
    } else {
        command.args(["-c", "credential.interactive=false"]);
    }
    command
        .args([
            "fetch",
            "--prune",
            "--no-tags",
            "--refmap=",
            remote,
            &refspec,
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    check_fetch(command.output(), used_pat, true)
}

pub(crate) fn fetch_status_message(
    remote: &str,
    has_pat: bool,
    interactive: bool,
) -> Option<String> {
    if interactive {
        None
    } else if has_pat {
        Some(format!(
            "Contacting {remote} with the supplied PAT (non-interactive)…"
        ))
    } else {
        Some(format!(
            "Contacting {remote} (unattended; cached credentials only)…"
        ))
    }
}

fn check_fetch(
    result: io::Result<std::process::Output>,
    used_pat: bool,
    unattended: bool,
) -> Result<(), CliError> {
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) if used_pat => Err(CliError::Git(with_git_stderr(
            "could not fetch the remote; the PAT was rejected, expired, or lacks repository read access",
            &output,
        ))),
        Ok(output) if unattended => Err(CliError::Git(with_git_stderr(
            "could not fetch the remote with cached credentials; set GIT_PAT for a headless run",
            &output,
        ))),
        Ok(output) => Err(CliError::Git(with_git_stderr(
            "could not fetch the remote; complete authentication and run the command again",
            &output,
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(CliError::Git(
            "could not run git fetch because the git executable was not found".to_owned(),
        )),
        Err(error) => Err(CliError::Io(error)),
    }
}

fn with_git_stderr(message: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        message.to_owned()
    } else {
        format!("{message}\n{stderr}")
    }
}

fn clear_repository_location_environment(command: &mut Command) {
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_DIR",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    ] {
        command.env_remove(variable);
    }
}
