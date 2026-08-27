//! Session drafts, handles, envelopes, and capability tokens.

use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::metadata::{decode_secret, encode_hex, random_bytes, valid_hex, write_metadata};
use super::restricted_fs::{restrict_file, restricted_file_metadata, sync_directory};
use super::{
    SessionError, SessionMetadata, SessionStatus, INTEGRITY_KEY_FILE, REPOSITORY_DIRECTORY,
};

pub(super) fn load_or_create_integrity_key(root: &Path) -> Result<[u8; 32], SessionError> {
    let path = root.join(INTEGRITY_KEY_FILE);
    match OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
    {
        Ok(mut file) => {
            restrict_file(&file)?;
            fs2::FileExt::lock_exclusive(&file).map_err(|_| SessionError::Unavailable)?;
            let key = random_bytes::<32>()?;
            file.write_all(&key)
                .and_then(|()| file.sync_all())
                .map_err(|_| SessionError::Unavailable)?;
            sync_directory(root).map_err(|_| SessionError::Unavailable)?;
            Ok(key)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&path).map_err(|_| SessionError::Unavailable)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !restricted_file_metadata(&metadata)
            {
                return Err(SessionError::Tampered);
            }
            let mut file = OpenOptions::new()
                .read(true)
                .open(path)
                .map_err(|_| SessionError::Unavailable)?;
            fs2::FileExt::lock_shared(&file).map_err(|_| SessionError::Unavailable)?;
            let mut key = Vec::new();
            file.read_to_end(&mut key)
                .map_err(|_| SessionError::Unavailable)?;
            key.try_into().map_err(|_| SessionError::Tampered)
        }
        Err(_) => Err(SessionError::Unavailable),
    }
}

#[cfg(not(windows))]
pub(super) fn canonical_session_root(root: PathBuf) -> Result<PathBuf, SessionError> {
    fs::canonicalize(root).map_err(|_| SessionError::Unavailable)
}

#[cfg(windows)]
pub(super) fn canonical_session_root(root: PathBuf) -> Result<PathBuf, SessionError> {
    Ok(root)
}

pub(crate) struct SessionDraft {
    pub(super) id: String,
    pub(super) secret: [u8; 32],
    pub(super) integrity_key: [u8; 32],
    pub(super) directory: PathBuf,
    pub(super) lock: Option<File>,
    pub(super) preserved: bool,
}

impl SessionDraft {
    pub(crate) fn repository(&self) -> PathBuf {
        self.directory.join(REPOSITORY_DIRECTORY)
    }

    pub(crate) fn token(&self) -> String {
        format!("v1.{}.{}", self.id, encode_hex(&self.secret))
    }

    pub(crate) fn save(&mut self, metadata: &SessionMetadata) -> Result<(), SessionError> {
        write_metadata(&self.directory, &self.integrity_key, &self.secret, metadata)?;
        self.preserved = true;
        Ok(())
    }

    pub(crate) fn discard(mut self) -> Result<(), SessionError> {
        let directory = self.directory.clone();
        if let Some(lock) = self.lock.take() {
            let _ = fs2::FileExt::unlock(&lock);
            drop(lock);
        }
        self.preserved = true;
        fs::remove_dir_all(directory).map_err(|_| SessionError::Unavailable)
    }
}

impl Drop for SessionDraft {
    fn drop(&mut self) {
        // Release the lock explicitly so a preserved session can be resumed
        // by this process without waiting on the file descriptor's close.
        if let Some(lock) = self.lock.take() {
            let _ = fs2::FileExt::unlock(&lock);
            drop(lock);
        }
        if !self.preserved {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

pub(crate) struct SessionHandle {
    pub(super) secret: [u8; 32],
    pub(super) integrity_key: [u8; 32],
    pub(super) directory: PathBuf,
    pub(super) lock: Option<File>,
    pub(crate) metadata: SessionMetadata,
}

impl SessionHandle {
    pub(crate) fn repository(&self) -> PathBuf {
        self.directory.join(REPOSITORY_DIRECTORY)
    }

    pub(crate) fn save(&self) -> Result<(), SessionError> {
        write_metadata(
            &self.directory,
            &self.integrity_key,
            &self.secret,
            &self.metadata,
        )
    }

    pub(crate) fn begin_publication(&mut self) -> Result<(), SessionError> {
        self.metadata.status = SessionStatus::Publishing;
        self.save()
    }

    pub(crate) fn restore_sealed(&mut self) -> Result<(), SessionError> {
        self.metadata.status = SessionStatus::Sealed;
        self.save()
    }

    pub(crate) fn consume(mut self) -> Result<(), SessionError> {
        self.metadata.status = SessionStatus::Consumed;
        self.save()?;
        let directory = self.directory.clone();
        if let Some(lock) = self.lock.take() {
            let _ = fs2::FileExt::unlock(&lock);
            drop(lock);
        }
        fs::remove_dir_all(directory).map_err(|_| SessionError::Unavailable)
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        if let Some(lock) = &self.lock {
            let _ = fs2::FileExt::unlock(lock);
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SessionEnvelope {
    pub(super) metadata: SessionMetadata,
    pub(super) capability: String,
    pub(super) integrity: String,
}

pub(super) struct SessionToken {
    pub(super) id: String,
    pub(super) secret: [u8; 32],
}

impl SessionToken {
    pub(super) fn parse(value: &str) -> Result<Self, SessionError> {
        let mut fields = value.split('.');
        let version = fields.next();
        let id = fields.next();
        let secret = fields.next();
        if version != Some("v1") || fields.next().is_some() {
            return Err(SessionError::InvalidToken);
        }
        let id = id
            .filter(|id| valid_hex(id, 32))
            .ok_or(SessionError::InvalidToken)?;
        let secret = secret
            .filter(|secret| valid_hex(secret, 64))
            .ok_or(SessionError::InvalidToken)?;
        Ok(Self {
            id: id.to_owned(),
            secret: decode_secret(secret)?,
        })
    }
}
