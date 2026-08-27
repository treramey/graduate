//! Isolated Git subprocess helpers.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

pub(crate) fn run_git(
    path: &Path,
    global: &Path,
    arguments: &[&str],
) -> Result<(), Box<dyn Error>> {
    let status = isolated_git(global)
        .args(arguments)
        .current_dir(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {} failed with {status}", arguments.join(" ")).into())
    }
}

pub(crate) fn git_text(
    path: &Path,
    global: &Path,
    arguments: &[&str],
) -> Result<String, Box<dyn Error>> {
    let output = isolated_git(global)
        .args(arguments)
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        return Err(format!("git {} failed with {}", arguments.join(" "), output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

pub(crate) fn isolated_git(global: &Path) -> ProcessCommand {
    let mut command = ProcessCommand::new("git");
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
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
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", global)
        .arg("-c")
        .arg("core.fsmonitor=false");
    command
}

pub(crate) fn path_text(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str()
        .ok_or_else(|| "test path is not valid UTF-8".into())
}

#[cfg(unix)]
pub(crate) fn find_git_executable() -> Result<PathBuf, Box<dyn Error>> {
    let path = std::env::var_os("PATH").ok_or("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(format!("git{}", std::env::consts::EXE_SUFFIX));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("git executable was not found on PATH".into())
}

#[cfg(unix)]
pub(crate) fn make_executable(path: &Path) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), Box<dyn Error>> {
    Ok(())
}
