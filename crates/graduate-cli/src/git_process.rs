//! Shared Git subprocess boundaries.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use graduate::restack::RemoteEndpointIdentity;
use sha2::{Digest, Sha256};

use crate::error::CliError;

pub(crate) struct RestackRemote {
    fetch_url: String,
    push_url: String,
    identity: RemoteEndpointIdentity,
}

impl RestackRemote {
    pub(crate) fn identity(&self) -> RemoteEndpointIdentity {
        self.identity.clone()
    }

    pub(crate) fn has_distinct_push_endpoint(&self) -> bool {
        self.fetch_url != self.push_url
    }
}

pub(crate) fn resolve_restack_remote(
    remote: &str,
    source: &Path,
) -> Result<RestackRemote, CliError> {
    let fetch_url = one_remote_url(source, remote, false)?;
    let push_url = one_remote_url(source, remote, true)?;
    let identity = RemoteEndpointIdentity {
        fetch_sha256: endpoint_digest(&fetch_url),
        push_sha256: endpoint_digest(&push_url),
    };
    Ok(RestackRemote {
        fetch_url,
        push_url,
        identity,
    })
}

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

pub(crate) fn fetch_restack_remote(
    remote: &RestackRemote,
    remote_name: &str,
    source: &Path,
) -> Result<(), CliError> {
    let pat = std::env::var("GIT_PAT")
        .ok()
        .filter(|value| !value.is_empty());
    let used_pat = pat.is_some();
    if let Some(message) = fetch_status_message(remote_name, used_pat, false) {
        eprintln!("{message}");
    }
    let refspec = format!("+refs/heads/*:refs/remotes/{remote_name}/*");
    let (mut command, endpoint_name) = endpoint_command(source, &remote.fetch_url, pat.as_deref())?;
    command
        .args([
            "fetch",
            "--prune",
            "--no-tags",
            "--refmap=",
            &endpoint_name,
            &refspec,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    check_fetch(command.output(), used_pat, true)
}

pub(crate) fn read_restack_remote_refs(
    remote: &RestackRemote,
    source: &Path,
    refs: &[String],
    push_endpoint: bool,
) -> Result<BTreeMap<String, String>, CliError> {
    let endpoint = if push_endpoint {
        &remote.push_url
    } else {
        &remote.fetch_url
    };
    let pat = std::env::var("GIT_PAT")
        .ok()
        .filter(|value| !value.is_empty());
    let (mut command, endpoint_name) = endpoint_command(source, endpoint, pat.as_deref())?;
    command
        .args(["ls-remote", "--refs", &endpoint_name])
        .args(refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = command.output()?;
    if !output.status.success() {
        return Err(CliError::Git(
            "could not read the remote refs before restack publication".to_owned(),
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| CliError::Git("the remote returned invalid ref data".to_owned()))?;
    let mut resolved = BTreeMap::new();
    for line in output.lines() {
        let Some((oid, name)) = line.split_once('\t') else {
            return Err(CliError::Git(
                "the remote returned invalid ref data".to_owned(),
            ));
        };
        if !refs.iter().any(|expected| expected == name)
            || !valid_object_id(oid)
            || resolved.insert(name.to_owned(), oid.to_owned()).is_some()
        {
            return Err(CliError::Git(
                "the remote returned invalid ref data".to_owned(),
            ));
        }
    }
    Ok(resolved)
}

pub(crate) fn push_restack_commit(
    remote: &RestackRemote,
    repository: &Path,
    hooks: &Path,
    empty_global_config: &Path,
    commit: &str,
    environment_ref: &str,
    expected_environment: &str,
) -> Result<(), CliError> {
    let pat = std::env::var("GIT_PAT")
        .ok()
        .filter(|value| !value.is_empty());
    let endpoint_name = random_endpoint_name()?;
    let mut settings = vec![
        (
            "core.hooksPath".to_owned(),
            hooks.to_string_lossy().into_owned(),
        ),
        ("core.fsmonitor".to_owned(), "false".to_owned()),
        ("commit.gpgSign".to_owned(), "false".to_owned()),
        ("tag.gpgSign".to_owned(), "false".to_owned()),
        ("rerere.enabled".to_owned(), "true".to_owned()),
        ("rerere.autoupdate".to_owned(), "false".to_owned()),
        ("core.autocrlf".to_owned(), "false".to_owned()),
        (
            format!("remote.{endpoint_name}.url"),
            remote.push_url.clone(),
        ),
    ];
    add_credentials(&mut settings, pat.as_deref());
    let mut command = Command::new("git");
    clear_isolated_environment(&mut command);
    apply_config_environment(&mut command, &settings);
    let lease = format!("--force-with-lease={environment_ref}:{expected_environment}");
    let update = format!("{commit}:{environment_ref}");
    command
        .current_dir(repository)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", empty_global_config)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "push",
            "--porcelain",
            "--no-verify",
            &lease,
            &endpoint_name,
            &update,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(pat) = pat {
        command.env("GIT_PAT", pat);
    }
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CliError::Git(
            "the remote rejected the leased restack publication".to_owned(),
        ))
    }
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

fn one_remote_url(source: &Path, remote: &str, push: bool) -> Result<String, CliError> {
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

fn endpoint_digest(endpoint: &str) -> String {
    format!("{:x}", Sha256::digest(endpoint.as_bytes()))
}

fn endpoint_command(
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

fn add_credentials(settings: &mut Vec<(String, String)>, pat: Option<&str>) {
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

fn apply_config_environment(command: &mut Command, settings: &[(String, String)]) {
    command.env("GIT_CONFIG_COUNT", settings.len().to_string());
    for (index, (key, value)) in settings.iter().enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
}

fn random_endpoint_name() -> Result<String, CliError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| CliError::Git("could not prepare the remote operation".to_owned()))?;
    let suffix = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("graduate-restack-{suffix}"))
}

fn valid_object_id(value: &str) -> bool {
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

fn clear_isolated_environment(command: &mut Command) {
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
