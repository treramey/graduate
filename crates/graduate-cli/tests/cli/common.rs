//! Shared command construction and session helpers.

use std::error::Error;
use std::path::Path;

use assert_cmd::Command;

pub(crate) fn gd_command() -> Result<Command, Box<dyn Error>> {
    let mut command = Command::cargo_bin("gd")?;
    for variable in [
        "GRADUATE_CONFIG",
        "ATLASSIAN_HOST",
        "ATLASSIAN_EMAIL",
        "ATLASSIAN_TOKEN",
        "GIT_PAT",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(variable);
    }
    Ok(command)
}

pub(crate) fn isolate_gd_storage(command: &mut Command, root: &Path) {
    // `dirs::cache_dir` uses HOME on macOS rather than XDG_CACHE_HOME.
    command.env("HOME", root).env("XDG_CACHE_HOME", root);
}

pub(crate) fn structured_restack_error(
    output: std::process::Output,
) -> Result<serde_json::Value, Box<dyn Error>> {
    if output.status.success() {
        return Err("restack invocation unexpectedly succeeded".into());
    }
    let stderr = String::from_utf8(output.stderr)?;
    Ok(serde_json::from_str(
        stderr.lines().last().ok_or("structured restack error")?,
    )?)
}

pub(crate) fn expire_session(work_area: &Path) -> Result<(), Box<dyn Error>> {
    let session_directory = work_area.parent().ok_or("session directory")?;
    let metadata_path = session_directory.join("session.json");
    let mut envelope: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)?;
    envelope["metadata"]["expiresAt"] = serde_json::json!(0);
    let sessions_root = session_directory.parent().ok_or("session store")?;
    let key = std::fs::read(sessions_root.join("integrity.key"))?;
    sign_session_envelope(&mut envelope, &key)?;
    let mut contents = serde_json::to_vec_pretty(&envelope)?;
    contents.push(b'\n');
    std::fs::write(metadata_path, contents)?;
    Ok(())
}

pub(crate) fn session_token_secret(token: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let secret = token.split('.').nth(2).ok_or("session token secret")?;
    let mut key = Vec::with_capacity(secret.len() / 2);
    for index in (0..secret.len()).step_by(2) {
        key.push(u8::from_str_radix(&secret[index..index + 2], 16)?);
    }
    Ok(key)
}

pub(crate) fn sign_session_envelope(
    envelope: &mut serde_json::Value,
    key: &[u8],
) -> Result<(), Box<dyn Error>> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let canonical = serde_json::to_vec(&envelope["metadata"])?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
    mac.update(&canonical);
    mac.update(&[0]);
    mac.update(
        envelope["capability"]
            .as_str()
            .ok_or("session capability")?
            .as_bytes(),
    );
    let integrity = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    envelope["integrity"] = serde_json::json!(integrity);
    Ok(())
}
