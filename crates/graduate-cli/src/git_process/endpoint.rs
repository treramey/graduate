//! Remote endpoint identity, one-shot credentials, and Git process environment.

use std::path::Path;
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::error::CliError;

pub(super) fn one_remote_url(source: &Path, remote: &str, push: bool) -> Result<String, CliError> {
    let mut command = Command::new("git");
    clear_repository_location_environment(&mut command);
    command.current_dir(source).args(["remote", "get-url"]);
    if push {
        command.arg("--push");
    }
    command
        .args(["--all", remote])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = command.output()?;
    if !output.status.success() {
        return Err(CliError::Git(
            "could not resolve the selected remote endpoint".to_owned(),
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| CliError::Git("the selected remote endpoint is not valid UTF-8".to_owned()))?;
    let urls = output
        .lines()
        .map(str::to_owned)
        .filter(|url| !url.is_empty())
        .collect::<Vec<_>>();
    if urls.len() != 1 || urls[0].chars().any(char::is_control) {
        return Err(CliError::Git(
            "the selected remote must resolve to exactly one endpoint".to_owned(),
        ));
    }
    let endpoint = urls.into_iter().next().ok_or_else(|| {
        CliError::Git("could not resolve the selected remote endpoint".to_owned())
    })?;
    normalize_local_endpoint(source, endpoint)
}

fn normalize_local_endpoint(source: &Path, endpoint: String) -> Result<String, CliError> {
    let path = Path::new(&endpoint);
    if path.is_absolute() {
        return canonical_local_path(path);
    }
    if let Ok(url) = url::Url::parse(&endpoint) {
        if url.scheme() != "file" {
            return Ok(endpoint);
        }
        let path = url.to_file_path().map_err(|_| {
            CliError::Git("could not resolve the selected remote endpoint".to_owned())
        })?;
        let canonical = path.canonicalize().map_err(|_| {
            CliError::Git("could not resolve the selected remote endpoint".to_owned())
        })?;
        return url::Url::from_file_path(canonical)
            .map(String::from)
            .map_err(|()| {
                CliError::Git("could not resolve the selected remote endpoint".to_owned())
            });
    }
    if endpoint.starts_with("ext::") || endpoint.contains(':') {
        return Ok(endpoint);
    }
    canonical_local_path(&source.join(path))
}

fn canonical_local_path(path: &Path) -> Result<String, CliError> {
    let endpoint = path
        .canonicalize()
        .map_err(|_| CliError::Git("could not resolve the selected remote endpoint".to_owned()))?;
    url::Url::from_file_path(endpoint)
        .map(String::from)
        .map_err(|()| CliError::Git("the selected remote endpoint is not valid UTF-8".to_owned()))
}

pub(super) fn endpoint_digest(endpoint: &str) -> String {
    format!("{:x}", Sha256::digest(endpoint.as_bytes()))
}

pub(super) fn endpoint_command(
    source: &Path,
    endpoint: &str,
    pat: Option<&str>,
) -> Result<(Command, String), CliError> {
    let endpoint_name = random_endpoint_name()?;
    let mut settings = vec![(format!("remote.{endpoint_name}.url"), endpoint.to_owned())];
    add_credentials(&mut settings, pat);
    let mut command = Command::new("git");
    clear_repository_location_environment(&mut command);
    apply_config_environment(&mut command, &settings);
    command.current_dir(source).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(pat) = pat {
        command.env("GIT_PAT", pat);
    }
    Ok((command, endpoint_name))
}

pub(super) fn add_credentials(settings: &mut Vec<(String, String)>, pat: Option<&str>) {
    if pat.is_some() {
        settings.push(("credential.helper".to_owned(), String::new()));
        settings.push((
            "credential.helper".to_owned(),
            "!f() { echo username=x-access-token; echo \"password=$GIT_PAT\"; }; f".to_owned(),
        ));
    } else {
        settings.push(("credential.interactive".to_owned(), "false".to_owned()));
    }
}

pub(super) fn apply_config_environment(command: &mut Command, settings: &[(String, String)]) {
    command.env("GIT_CONFIG_COUNT", settings.len().to_string());
    for (index, (key, value)) in settings.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
}

pub(super) fn random_endpoint_name() -> Result<String, CliError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| CliError::Git("could not prepare the remote operation".to_owned()))?;
    let suffix = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("graduate-restack-{suffix}"))
}

pub(super) fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
