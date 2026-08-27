//! Source repository identity, configuration, and Git environment isolation.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use graduate::restack::RestackAuthor;
use serde_json::json;

use super::errors::{reconstruction_error, validation_error};
use super::machine_output::machine_failure;
use crate::error::CliError;

pub(super) fn configured_author(source: &Path) -> Result<RestackAuthor, CliError> {
    let name = source_config(source, "user.name")?;
    let email = source_config(source, "user.email")?;
    if !valid_identity_value(&name) || !valid_identity_value(&email) {
        return Err(machine_failure(
            "missing_identity",
            "Git user.name and user.email must be configured",
            json!({}),
        ));
    }
    Ok(RestackAuthor { name, email })
}

fn source_config(source: &Path, key: &str) -> Result<String, CliError> {
    let output = source_git(source)
        .args(["config", "--get", key])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "git_unavailable",
                "could not run Git for restack preflight",
                json!({"stage": "identity"}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "missing_identity",
            "Git user.name and user.email must be configured",
            json!({}),
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(|_| {
        machine_failure(
            "invalid_identity",
            "the configured Git identity is not valid UTF-8",
            json!({}),
        )
    })?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn valid_identity_value(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

pub(super) fn source_object_directory(source: &Path) -> Result<Vec<u8>, CliError> {
    let output = source_git(source)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "git_unavailable",
                "could not run Git for restack preflight",
                json!({"stage": "objectStore"}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "object_store_unavailable",
            "could not locate the source repository object store",
            json!({}),
        ));
    }
    let mut path = output.stdout;
    while matches!(path.last(), Some(b'\r' | b'\n')) {
        path.pop();
    }
    if path.is_empty() || path.contains(&b'\n') || path.contains(&b'\r') {
        return Err(machine_failure(
            "object_store_unavailable",
            "could not locate the source repository object store",
            json!({}),
        ));
    }
    Ok(path)
}

pub(super) fn source_repository_identity(source: &Path) -> Result<String, CliError> {
    let output = source_git(source)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "repository_unavailable",
                "could not identify the source repository",
                json!({}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "repository_not_found",
            "the current directory is not inside a Git repository",
            json!({}),
        ));
    }
    let path = String::from_utf8(output.stdout).map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "the source repository path is not valid UTF-8",
            json!({}),
        )
    })?;
    let canonical = fs::canonicalize(path.trim_end_matches(['\r', '\n'])).map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not identify the source repository",
            json!({}),
        )
    })?;
    canonical.to_str().map(str::to_owned).ok_or_else(|| {
        machine_failure(
            "repository_unavailable",
            "the source repository path is not valid UTF-8",
            json!({}),
        )
    })
}

fn source_git(source: &Path) -> Command {
    let mut command = Command::new("git");
    clear_repository_location_environment(&mut command);
    command.current_dir(source);
    command
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

pub(super) fn clear_isolated_environment(command: &mut Command) {
    clear_repository_location_environment(command);
    for variable in [
        "GIT_AUTHOR_DATE",
        "GIT_AUTHOR_EMAIL",
        "GIT_AUTHOR_NAME",
        "GIT_COMMITTER_DATE",
        "GIT_COMMITTER_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_EXEC_PATH",
    ] {
        command.env_remove(variable);
    }
}

pub(super) fn read_success_text(
    result: std::io::Result<Output>,
    stage: &'static str,
) -> Result<String, CliError> {
    let output = result.map_err(|_| reconstruction_error(stage))?;
    if !output.status.success() {
        return Err(reconstruction_error(stage));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| validation_error(stage))?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}
