//! Destination path validation and symlink-safe filesystem primitives.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use super::GeneratedFile;
use crate::error::CliError;

pub(super) fn validate_output_dir(current: &Path, output_dir: &Path) -> Result<PathBuf, CliError> {
    if output_dir.as_os_str().is_empty()
        || output_dir.is_absolute()
        || !output_dir
            .components()
            .any(|component| matches!(component, Component::Normal(_)))
        || output_dir.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(CliError::InvalidInput(
            "--output-dir must be a non-empty relative path within the current directory"
                .to_owned(),
        ));
    }
    let destination = current.join(output_dir);
    validate_path_without_symlinks(current, &destination, true)?;
    Ok(destination)
}

pub(super) fn validate_destinations(
    files: &[GeneratedFile<'_>],
    force: bool,
) -> Result<(), CliError> {
    let current = std::env::current_dir()?.canonicalize()?;
    for file in files {
        validate_path_without_symlinks(&current, &file.path, false)?;
        if fs::symlink_metadata(&file.path).is_ok() && !force {
            return Err(CliError::GeneratedFileExists(file.path.clone()));
        }
    }
    Ok(())
}

pub(super) fn create_parents_without_symlinks(
    current: &Path,
    parent: &Path,
) -> Result<(), CliError> {
    let repository = repository_directory(current)?;
    repository.create_dir_all(repository_relative(current, parent)?)?;
    revalidate_parent(current, parent)
}

fn repository_directory(current: &Path) -> io::Result<Dir> {
    Dir::open_ambient_dir(current, ambient_authority())
}

fn repository_relative<'a>(current: &Path, path: &'a Path) -> Result<&'a Path, CliError> {
    let relative = path.strip_prefix(current).map_err(|_| {
        CliError::InvalidInput(format!(
            "generated path must stay within {}",
            current.display()
        ))
    })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CliError::InvalidInput(
            "generated path must be a repository-relative normal path".to_owned(),
        ));
    }
    Ok(relative)
}

pub(super) fn safe_rename(current: &Path, source: &Path, destination: &Path) -> io::Result<()> {
    let repository = repository_directory(current)?;
    let source = repository_relative(current, source).map_err(io::Error::other)?;
    let destination = repository_relative(current, destination).map_err(io::Error::other)?;
    repository.rename(source, &repository, destination)
}

pub(super) fn safe_hard_link(current: &Path, source: &Path, destination: &Path) -> io::Result<()> {
    let repository = repository_directory(current)?;
    let source = repository_relative(current, source).map_err(io::Error::other)?;
    let destination = repository_relative(current, destination).map_err(io::Error::other)?;
    repository.hard_link(source, &repository, destination)
}

pub(super) fn safe_remove_file(current: &Path, path: &Path) -> io::Result<()> {
    let repository = repository_directory(current)?;
    let path = repository_relative(current, path).map_err(io::Error::other)?;
    repository.remove_file(path)
}

pub(super) fn safe_restore_backup(
    current: &Path,
    backup: &Path,
    destination: &Path,
) -> io::Result<()> {
    safe_hard_link(current, backup, destination)?;
    safe_remove_file(current, backup)?;
    if let Some(parent) = destination.parent() {
        sync_directory(parent)?;
    }
    if let Some(parent) = backup.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

pub(super) fn revalidate_parent(current: &Path, parent: &Path) -> Result<(), CliError> {
    let canonical = parent.canonicalize()?;
    if canonical != parent || !canonical.starts_with(current) {
        return Err(CliError::InvalidInput(format!(
            "generated path parent changed while publishing: {}",
            parent.display()
        )));
    }
    Ok(())
}

pub(super) fn validate_path_without_symlinks(
    current: &Path,
    destination: &Path,
    leaf_must_be_directory: bool,
) -> Result<(), CliError> {
    let relative = destination.strip_prefix(current).map_err(|_| {
        CliError::InvalidInput(format!(
            "generated path must stay within {}",
            current.display()
        ))
    })?;
    let mut path = current.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(CliError::InvalidInput(format!(
                "generated path must stay within {}",
                current.display()
            )));
        };
        path.push(component);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            return Err(CliError::InvalidInput(format!(
                "refusing to write through symlink {}",
                path.display()
            )));
        }
        let is_leaf = index + 1 == components.len();
        if (!is_leaf || leaf_must_be_directory) && !metadata.is_dir() {
            return Err(CliError::InvalidInput(format!(
                "generated path parent is not a directory: {}",
                path.display()
            )));
        }
        if is_leaf && !leaf_must_be_directory && !metadata.is_file() {
            return Err(CliError::InvalidInput(format!(
                "generated file destination is not a file: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn require_regular_file(path: &Path, required: bool) -> Result<(), CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(CliError::Config(format!(
            "skill publication transaction entry is not a regular file: {}",
            path.display()
        ))),
        Err(error) if !required && error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn require_directory(path: &Path, required: bool) -> Result<(), CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(CliError::Config(format!(
            "skill publication transaction entry is not a directory: {}",
            path.display()
        ))),
        Err(error) if !required && error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub(super) fn validate_recovery_relative_path(path: &Path) -> Result<(), CliError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CliError::InvalidInput(
            "skill publication recovery path was invalid".to_owned(),
        ));
    }
    Ok(())
}
