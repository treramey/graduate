//! Staging directory and publication manifest records.

use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};

use super::paths::sync_directory;
use super::{COMMITTED_NAME, MANIFEST_NAME, STAGING_SEQUENCE};
use crate::error::CliError;

pub(super) struct StagingDirectory {
    pub(super) path: PathBuf,
    parent: PathBuf,
    cleanup: bool,
}

impl StagingDirectory {
    pub(super) fn create(parent: &Path) -> io::Result<Self> {
        for _ in 0..100 {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".graduate-generate-skills-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    sync_directory(parent)?;
                    return Ok(Self {
                        path,
                        parent: parent.to_path_buf(),
                        cleanup: true,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique skill staging directory",
        ))
    }

    pub(super) fn preserve(&mut self) {
        self.cleanup = false;
    }

    pub(super) fn remove(&mut self) -> io::Result<()> {
        fs::remove_dir_all(&self.path)?;
        sync_directory(&self.parent)?;
        self.cleanup = false;
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) struct StagedArtifact {
    pub(super) staged: PathBuf,
    pub(super) destination: PathBuf,
}

pub(super) struct AppliedArtifact {
    pub(super) destination: PathBuf,
    pub(super) backup: Option<PathBuf>,
    pub(super) new_content: Vec<u8>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PublicationManifest {
    pub(super) version: u32,
    pub(super) artifacts: Vec<RecoveryArtifact>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RecoveryArtifact {
    pub(super) destination: PathBuf,
    pub(super) staged: PathBuf,
    pub(super) backup: PathBuf,
    pub(super) had_destination: bool,
    pub(super) old_content: Option<Vec<u8>>,
    pub(super) new_content: Vec<u8>,
}

pub(super) fn write_manifest(
    staging_root: &Path,
    manifest: &PublicationManifest,
) -> Result<(), CliError> {
    let mut contents = serde_json::to_vec_pretty(manifest)?;
    contents.push(b'\n');
    let mut temporary = tempfile::Builder::new()
        .prefix(".transaction-")
        .suffix(".tmp")
        .tempfile_in(staging_root)?;
    temporary.write_all(&contents)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(staging_root.join(MANIFEST_NAME))
        .map_err(|error| CliError::Io(error.error))?;
    sync_directory(staging_root)?;
    Ok(())
}

pub(super) fn mark_committed(staging_root: &Path) -> Result<(), CliError> {
    let mut marker = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(staging_root.join(COMMITTED_NAME))?;
    marker.write_all(b"committed\n")?;
    marker.sync_all()?;
    sync_directory(staging_root)?;
    Ok(())
}
