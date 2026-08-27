//! Tests.

use std::path::{Path, PathBuf};

use super::paths::validate_output_dir;
use super::publication::{backup_validated_destination, replace_artifacts};
use super::recovery::recover_pending_publications;
use super::staging::{write_manifest, PublicationManifest, RecoveryArtifact, StagedArtifact};
use super::*;

#[test]
fn existing_destination_is_rejected_before_any_file_is_written(
) -> Result<(), Box<dyn std::error::Error>> {
    let current = std::env::current_dir()?.canonicalize()?;
    let directory = tempfile::tempdir_in(current)?;
    let skill = directory.path().join("skills/graduate/SKILL.md");
    let index = directory.path().join("docs/skills.md");
    let parent = index.parent().ok_or("index has no parent")?;
    fs::create_dir_all(parent)?;
    fs::write(&index, "existing index")?;
    let files = [
        GeneratedFile {
            path: skill.clone(),
            content: "skill",
        },
        GeneratedFile {
            path: index.clone(),
            content: "index",
        },
    ];

    let result = write_generated(&files, false);

    assert!(matches!(result, Err(CliError::GeneratedFileExists(_))));
    assert!(!skill.exists());
    assert_eq!(fs::read_to_string(index)?, "existing index");
    Ok(())
}

#[test]
fn output_directory_rejects_absolute_and_parent_paths() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let current = directory.path().canonicalize()?;
    assert!(validate_output_dir(&current, Path::new("/tmp/skills")).is_err());
    assert!(validate_output_dir(&current, Path::new("../skills")).is_err());
    assert!(validate_output_dir(&current, Path::new(".")).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn output_directory_rejects_symlink_traversal() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let current = directory.path().canonicalize()?;
    let outside = tempfile::tempdir()?;
    symlink(outside.path(), current.join("skills"))?;

    let result = validate_output_dir(&current, Path::new("skills"));

    assert!(matches!(result, Err(CliError::InvalidInput(message)) if message.contains("symlink")));
    Ok(())
}

#[test]
fn artifact_replacement_rolls_back_prior_outputs_when_commit_fails(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let current = directory.path().canonicalize()?;
    let staging = current.join("staging");
    fs::create_dir(&staging)?;
    let first_destination = current.join("first");
    let second_destination = current.join("second");
    let first_staged = staging.join("first");
    fs::write(&first_destination, "old first")?;
    fs::write(&second_destination, "old second")?;
    fs::write(&first_staged, "new first")?;
    let artifacts = [
        StagedArtifact {
            staged: first_staged,
            destination: first_destination.clone(),
        },
        StagedArtifact {
            staged: staging.join("missing"),
            destination: second_destination.clone(),
        },
    ];

    assert!(replace_artifacts(&current, &staging, &artifacts, true).is_err());
    assert_eq!(fs::read_to_string(first_destination)?, "old first");
    assert_eq!(fs::read_to_string(second_destination)?, "old second");
    Ok(())
}

#[test]
fn no_force_policy_is_rechecked_during_replacement() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let current = directory.path().canonicalize()?;
    let staging = current.join("staging");
    fs::create_dir(&staging)?;
    let first_destination = current.join("first");
    let second_destination = current.join("second");
    let first_staged = staging.join("first");
    let second_staged = staging.join("second");
    fs::write(&first_staged, "new first")?;
    fs::write(&second_staged, "new second")?;
    fs::write(&second_destination, "concurrent edit")?;
    let artifacts = [
        StagedArtifact {
            staged: first_staged,
            destination: first_destination.clone(),
        },
        StagedArtifact {
            staged: second_staged,
            destination: second_destination.clone(),
        },
    ];

    let result = replace_artifacts(&current, &staging, &artifacts, false);

    assert!(
        matches!(result, Err(CliError::GeneratedFileExists(path)) if path == second_destination)
    );
    assert!(!first_destination.exists());
    assert_eq!(fs::read_to_string(second_destination)?, "concurrent edit");
    Ok(())
}

#[test]
fn forced_replacement_restores_a_destination_that_changed_after_validation(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let current = directory.path().canonicalize()?;
    let destination = current.join("generated");
    let backup = current.join("backup");
    fs::write(&destination, "concurrent edit")?;

    let result =
        backup_validated_destination(&current, &destination, &backup, b"validated content");

    assert!(
        matches!(result, Err(CliError::Config(message)) if message.contains("changed during publication"))
    );
    assert_eq!(fs::read_to_string(destination)?, "concurrent edit");
    assert!(!backup.exists());
    Ok(())
}

#[test]
fn interrupted_publication_is_rolled_back_from_its_manifest(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let current = directory.path().canonicalize()?;
    let staging = current.join(format!("{STAGING_PREFIX}interrupted"));
    let backup = staging.join("backups/0");
    fs::create_dir_all(backup.parent().ok_or("backup has no parent")?)?;
    let destination_relative = PathBuf::from("skills/graduate/SKILL.md");
    let destination = current.join(&destination_relative);
    fs::create_dir_all(destination.parent().ok_or("destination has no parent")?)?;
    fs::write(&destination, "new skill")?;
    fs::write(&backup, "old skill")?;
    write_manifest(
        &staging,
        &PublicationManifest {
            version: 1,
            artifacts: vec![RecoveryArtifact {
                destination: destination_relative,
                staged: PathBuf::from("0"),
                backup: PathBuf::from("backups/0"),
                had_destination: true,
                old_content: Some(b"old skill".to_vec()),
                new_content: b"new skill".to_vec(),
            }],
        },
    )?;
    let files = [GeneratedFile {
        path: destination.clone(),
        content: "new skill",
    }];

    recover_pending_publications(&current, &files)?;

    assert_eq!(fs::read_to_string(destination)?, "old skill");
    assert!(!staging.exists());
    Ok(())
}

#[test]
fn recovery_preserves_prefixed_directories_without_a_manifest(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let staging = directory
        .path()
        .join(format!("{STAGING_PREFIX}unrecognized"));
    fs::create_dir(&staging)?;
    fs::write(staging.join("user-file"), "keep me")?;
    let files = [GeneratedFile {
        path: directory.path().join("skills/graduate/SKILL.md"),
        content: "new skill",
    }];

    let result = recover_pending_publications(directory.path(), &files);

    assert!(
        matches!(result, Err(CliError::Config(message)) if message.contains("without a manifest"))
    );
    assert_eq!(fs::read_to_string(staging.join("user-file"))?, "keep me");
    Ok(())
}

#[test]
fn recovery_rejects_destinations_outside_the_expected_artifacts(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let staging = directory.path().join(format!("{STAGING_PREFIX}crafted"));
    fs::create_dir_all(staging.join("backups"))?;
    fs::write(staging.join("0"), "new skill")?;
    write_manifest(
        &staging,
        &PublicationManifest {
            version: 1,
            artifacts: vec![RecoveryArtifact {
                destination: PathBuf::from("important.txt"),
                staged: PathBuf::from("0"),
                backup: PathBuf::from("backups/0"),
                had_destination: false,
                old_content: None,
                new_content: b"new skill".to_vec(),
            }],
        },
    )?;
    let expected_path = directory.path().join("skills/graduate/SKILL.md");
    let files = [GeneratedFile {
        path: expected_path,
        content: "new skill",
    }];

    let result = recover_pending_publications(directory.path(), &files);

    assert!(
        matches!(result, Err(CliError::Config(message)) if message.contains("preserving invalid"))
    );
    assert!(staging.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn recovery_rejects_symlinked_backups() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let staging = directory.path().join(format!("{STAGING_PREFIX}symlink"));
    fs::create_dir_all(staging.join("backups"))?;
    let outside = directory.path().join("outside");
    fs::write(&outside, "old skill")?;
    symlink(&outside, staging.join("backups/0"))?;
    write_manifest(
        &staging,
        &PublicationManifest {
            version: 1,
            artifacts: vec![RecoveryArtifact {
                destination: PathBuf::from("skills/graduate/SKILL.md"),
                staged: PathBuf::from("0"),
                backup: PathBuf::from("backups/0"),
                had_destination: true,
                old_content: Some(b"old skill".to_vec()),
                new_content: b"new skill".to_vec(),
            }],
        },
    )?;
    let files = [GeneratedFile {
        path: directory.path().join("skills/graduate/SKILL.md"),
        content: "new skill",
    }];

    let result = recover_pending_publications(directory.path(), &files);

    assert!(
        matches!(result, Err(CliError::Config(message)) if message.contains("not a regular file"))
    );
    assert!(staging.exists());
    Ok(())
}
