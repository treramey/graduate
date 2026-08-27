//! Atomic artifact replacement.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::paths::{
    create_parents_without_symlinks, require_regular_file, revalidate_parent, safe_hard_link,
    safe_remove_file, safe_rename, safe_restore_backup, sync_directory,
    validate_path_without_symlinks,
};
use super::recovery::{rollback_after_error, rollback_artifacts};
use super::staging::{
    mark_committed, write_manifest, AppliedArtifact, PublicationManifest, RecoveryArtifact,
    StagedArtifact,
};
use crate::shared::error::CliError;

pub(super) fn replace_artifacts(
    current: &Path,
    staging_root: &Path,
    artifacts: &[StagedArtifact],
    force: bool,
) -> Result<(), CliError> {
    let backup_root = staging_root.join("backups");
    create_parents_without_symlinks(current, &backup_root)?;
    let manifest = PublicationManifest {
        version: 1,
        artifacts: artifacts
            .iter()
            .enumerate()
            .map(|(index, artifact)| {
                Ok(RecoveryArtifact {
                    destination: artifact
                        .destination
                        .strip_prefix(current)
                        .map_err(|_| {
                            CliError::InvalidInput(
                                "generated destination escaped the repository".to_owned(),
                            )
                        })?
                        .to_path_buf(),
                    staged: artifact
                        .staged
                        .strip_prefix(staging_root)
                        .map_err(|_| {
                            CliError::InvalidInput(
                                "staged artifact escaped its transaction".to_owned(),
                            )
                        })?
                        .to_path_buf(),
                    backup: PathBuf::from("backups").join(index.to_string()),
                    had_destination: fs::symlink_metadata(&artifact.destination).is_ok(),
                    old_content: match fs::read(&artifact.destination) {
                        Ok(content) => Some(content),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                        Err(error) => return Err(error.into()),
                    },
                    new_content: fs::read(&artifact.staged)?,
                })
            })
            .collect::<Result<Vec<_>, CliError>>()?,
    };
    write_manifest(staging_root, &manifest)?;

    let mut applied = Vec::with_capacity(artifacts.len());
    for (index, artifact) in artifacts.iter().enumerate() {
        require_regular_file(&artifact.staged, true)?;
        let parent = artifact.destination.parent().ok_or_else(|| {
            CliError::InvalidInput("generated destination must have a parent".to_owned())
        })?;
        validate_path_without_symlinks(current, &artifact.destination, false)?;
        revalidate_parent(current, parent)?;
        let exists = fs::symlink_metadata(&artifact.destination).is_ok();
        let expected_old = manifest.artifacts[index].old_content.as_deref();
        if force && expected_old.is_some() && !exists {
            return Err(rollback_cli_error(
                CliError::Config(format!(
                    "generated destination changed during publication: {}",
                    artifact.destination.display()
                )),
                current,
                &applied,
            ));
        }
        if exists && !force {
            return Err(rollback_cli_error(
                CliError::GeneratedFileExists(artifact.destination.clone()),
                current,
                &applied,
            ));
        }
        if !force || expected_old.is_none() {
            revalidate_parent(current, parent)?;
            match safe_hard_link(current, &artifact.staged, &artifact.destination) {
                Ok(()) => {
                    applied.push(AppliedArtifact {
                        destination: artifact.destination.clone(),
                        backup: None,
                        new_content: manifest.artifacts[index].new_content.clone(),
                    });
                    validate_published_content(
                        &artifact.destination,
                        &manifest.artifacts[index].new_content,
                    )
                    .map_err(|error| rollback_cli_error(error, current, &applied))?;
                    if let Err(error) = safe_remove_file(current, &artifact.staged) {
                        return Err(rollback_after_error(error, current, &applied));
                    }
                    sync_directory(parent)?;
                    sync_directory(staging_root)?;
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let error = if force {
                        CliError::Config(format!(
                            "generated destination changed during publication: {}",
                            artifact.destination.display()
                        ))
                    } else {
                        CliError::GeneratedFileExists(artifact.destination.clone())
                    };
                    return Err(rollback_cli_error(error, current, &applied));
                }
                Err(error) => return Err(rollback_after_error(error, current, &applied)),
            }
        }
        let backup = if exists {
            let backup = backup_root.join(index.to_string());
            revalidate_parent(current, parent)?;
            if let Err(error) = backup_validated_destination(
                current,
                &artifact.destination,
                &backup,
                manifest.artifacts[index]
                    .old_content
                    .as_deref()
                    .unwrap_or_default(),
            ) {
                return Err(rollback_cli_error(error, current, &applied));
            }
            Some(backup)
        } else {
            None
        };
        revalidate_parent(current, parent)?;
        if let Err(error) = safe_hard_link(current, &artifact.staged, &artifact.destination) {
            let restore_error = backup.as_ref().and_then(|backup| {
                safe_restore_backup(current, backup, &artifact.destination).err()
            });
            let rollback_error = rollback_artifacts(current, &applied).err();
            if restore_error.is_some() || rollback_error.is_some() {
                let recovery_errors = restore_error
                    .into_iter()
                    .chain(rollback_error)
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(io::Error::other(format!(
                    "{error}; restoring the prior skill catalog also failed: {recovery_errors}"
                ))
                .into());
            }
            return Err(error.into());
        }
        applied.push(AppliedArtifact {
            destination: artifact.destination.clone(),
            backup: backup.clone(),
            new_content: manifest.artifacts[index].new_content.clone(),
        });
        validate_published_content(
            &artifact.destination,
            &manifest.artifacts[index].new_content,
        )
        .map_err(|error| rollback_cli_error(error, current, &applied))?;
        if let Err(error) = safe_remove_file(current, &artifact.staged) {
            return Err(rollback_after_error(error, current, &applied));
        }
        sync_directory(parent)?;
        sync_directory(staging_root)?;
    }
    if let Err(error) = mark_committed(staging_root) {
        return Err(rollback_cli_error(error, current, &applied));
    }
    Ok(())
}

pub(super) fn backup_validated_destination(
    current: &Path,
    destination: &Path,
    backup: &Path,
    expected: &[u8],
) -> Result<(), CliError> {
    safe_rename(current, destination, backup)?;
    if let Some(parent) = destination.parent() {
        sync_directory(parent)?;
    }
    if let Some(parent) = backup.parent() {
        sync_directory(parent)?;
    }
    let verification = require_regular_file(backup, true).and_then(|()| {
        if fs::read(backup)? == expected {
            Ok(())
        } else {
            Err(CliError::Config(format!(
                "generated destination changed during publication: {}",
                destination.display()
            )))
        }
    });
    let Err(verification_error) = verification else {
        return Ok(());
    };
    safe_restore_backup(current, backup, destination).map_err(|error| {
        CliError::Io(io::Error::other(format!(
            "{verification_error}; restoration failed: {error}"
        )))
    })?;
    Err(verification_error)
}

fn validate_published_content(destination: &Path, expected: &[u8]) -> Result<(), CliError> {
    require_regular_file(destination, true)?;
    if fs::read(destination)? != expected {
        return Err(CliError::Config(format!(
            "generated destination changed during publication: {}",
            destination.display()
        )));
    }
    Ok(())
}

fn rollback_cli_error(error: CliError, current: &Path, applied: &[AppliedArtifact]) -> CliError {
    match rollback_artifacts(current, applied) {
        Ok(()) => error,
        Err(rollback_error) => io::Error::other(format!(
            "{error}; restoring the prior skill catalog also failed: {rollback_error}"
        ))
        .into(),
    }
}
