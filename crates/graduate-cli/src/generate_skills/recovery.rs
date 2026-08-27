//! Interrupted publication recovery and rollback.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::paths::{
    create_parents_without_symlinks, require_directory, require_regular_file, revalidate_parent,
    safe_remove_file, safe_restore_backup, sync_directory, validate_path_without_symlinks,
    validate_recovery_relative_path,
};
use super::staging::{AppliedArtifact, PublicationManifest};
use super::{GeneratedFile, COMMITTED_NAME, MANIFEST_NAME, STAGING_PREFIX};
use crate::shared::error::CliError;

pub(super) fn recover_pending_publications(
    current: &Path,
    expected: &[GeneratedFile<'_>],
) -> Result<(), CliError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(STAGING_PREFIX) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CliError::InvalidInput(format!(
                "invalid skill publication recovery path: {}",
                entry.path().display()
            )));
        }
        recover_publication(current, &entry.path(), expected)?;
        fs::remove_dir_all(entry.path())?;
        sync_directory(current)?;
    }
    Ok(())
}

fn recover_publication(
    current: &Path,
    staging_root: &Path,
    expected: &[GeneratedFile<'_>],
) -> Result<(), CliError> {
    let manifest_path = staging_root.join(MANIFEST_NAME);
    let contents = match fs::read(&manifest_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(CliError::Config(format!(
                "preserving unrecognized skill publication directory without a manifest: {}",
                staging_root.display()
            )))
        }
        Err(error) => return Err(error.into()),
    };
    let manifest: PublicationManifest = serde_json::from_slice(&contents).map_err(|error| {
        CliError::Config(format!(
            "could not recover skill publication from {}: {error}",
            manifest_path.display()
        ))
    })?;
    if manifest.version != 1 {
        return Err(CliError::Config(format!(
            "unsupported skill publication recovery version {} in {}",
            manifest.version,
            manifest_path.display()
        )));
    }
    validate_recovery_manifest(current, staging_root, &manifest, expected)?;
    if staging_root.join(COMMITTED_NAME).is_file() {
        return Ok(());
    }
    for artifact in manifest.artifacts.iter().rev() {
        validate_recovery_relative_path(&artifact.destination)?;
        validate_recovery_relative_path(&artifact.staged)?;
        validate_recovery_relative_path(&artifact.backup)?;
        let destination = current.join(&artifact.destination);
        let staged = staging_root.join(&artifact.staged);
        let backup = staging_root.join(&artifact.backup);
        let parent = destination.parent().ok_or_else(|| {
            CliError::InvalidInput("recovery destination must have a parent".to_owned())
        })?;
        create_parents_without_symlinks(current, parent)?;
        validate_path_without_symlinks(current, &destination, false)?;
        if backup.exists() {
            require_regular_file(&backup, true)?;
            remove_replaced_artifact(current, &destination, &artifact.new_content)?;
            revalidate_parent(current, parent)?;
            safe_restore_backup(current, &backup, &destination)?;
            sync_directory(parent)?;
            if let Some(backup_parent) = backup.parent() {
                sync_directory(backup_parent)?;
            }
        } else if !artifact.had_destination {
            recover_new_artifact(current, &destination, &staged, &artifact.new_content)?;
            sync_directory(parent)?;
        }
    }
    Ok(())
}

fn validate_recovery_manifest(
    current: &Path,
    staging_root: &Path,
    manifest: &PublicationManifest,
    expected: &[GeneratedFile<'_>],
) -> Result<(), CliError> {
    if manifest.artifacts.len() != expected.len() {
        return Err(invalid_recovery_layout(staging_root));
    }
    require_regular_file(&staging_root.join(MANIFEST_NAME), true)?;
    require_directory(&staging_root.join("backups"), true)?;
    require_regular_file(&staging_root.join(COMMITTED_NAME), false)?;
    for (index, (artifact, expected)) in manifest.artifacts.iter().zip(expected).enumerate() {
        let expected_staged = index.to_string();
        let destination = expected.path.strip_prefix(current).map_err(|_| {
            CliError::InvalidInput("generated destination escaped the repository".to_owned())
        })?;
        if artifact.destination != destination
            || artifact.staged != Path::new(&expected_staged)
            || artifact.backup != PathBuf::from("backups").join(index.to_string())
            || artifact.had_destination != artifact.old_content.is_some()
            || artifact.new_content != expected.content.as_bytes()
        {
            return Err(invalid_recovery_layout(staging_root));
        }
        require_regular_file(&staging_root.join(&artifact.staged), false)?;
        let backup = staging_root.join(&artifact.backup);
        require_regular_file(&backup, false)?;
        if backup.is_file()
            && fs::read(&backup)?.as_slice() != artifact.old_content.as_deref().unwrap_or_default()
        {
            return Err(invalid_recovery_layout(staging_root));
        }
    }
    Ok(())
}

fn invalid_recovery_layout(staging_root: &Path) -> CliError {
    CliError::Config(format!(
        "preserving invalid skill publication transaction at {}",
        staging_root.display()
    ))
}

fn remove_replaced_artifact(
    current: &Path,
    destination: &Path,
    expected: &[u8],
) -> Result<(), CliError> {
    match fs::read(destination) {
        Ok(contents) if contents == expected => {
            safe_remove_file(current, destination).map_err(CliError::from)
        }
        Ok(_) => Err(CliError::Config(format!(
            "refusing to overwrite a file changed after an interrupted skill publication: {}",
            destination.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn recover_new_artifact(
    current: &Path,
    destination: &Path,
    staged: &Path,
    expected: &[u8],
) -> Result<(), CliError> {
    match fs::read(destination) {
        Ok(contents) if contents == expected => {
            safe_remove_file(current, destination).map_err(CliError::from)
        }
        Ok(_) if staged.exists() => Ok(()),
        Ok(_) => Err(CliError::Config(format!(
            "refusing to overwrite a file changed after an interrupted skill publication: {}",
            destination.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn rollback_after_error(
    error: io::Error,
    current: &Path,
    applied: &[AppliedArtifact],
) -> CliError {
    match rollback_artifacts(current, applied) {
        Ok(()) => error.into(),
        Err(rollback_error) => io::Error::other(format!(
            "{error}; restoring the prior skill catalog also failed: {rollback_error}"
        ))
        .into(),
    }
}

pub(super) fn rollback_artifacts(current: &Path, applied: &[AppliedArtifact]) -> io::Result<()> {
    for artifact in applied.iter().rev() {
        let parent = artifact
            .destination
            .parent()
            .ok_or_else(|| io::Error::other("generated destination had no parent"))?;
        validate_path_without_symlinks(current, &artifact.destination, false)
            .map_err(io::Error::other)?;
        revalidate_parent(current, parent).map_err(io::Error::other)?;
        let contents = fs::read(&artifact.destination)?;
        if contents != artifact.new_content {
            return Err(io::Error::other(format!(
                "refusing to overwrite a file changed during skill publication: {}",
                artifact.destination.display()
            )));
        }
        safe_remove_file(current, &artifact.destination)?;
        if let Some(backup) = &artifact.backup {
            revalidate_parent(current, parent).map_err(io::Error::other)?;
            safe_restore_backup(current, backup, &artifact.destination)?;
        } else {
            sync_directory(parent)?;
        }
    }
    Ok(())
}
