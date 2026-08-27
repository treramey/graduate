//! Permission-restricted, expiring state for resumable restack conflicts.

use std::fs::{self};
use std::io::{self};
use std::path::PathBuf;

use graduate::restack::{
    MergeOutcome, OrphanedCommit, RemoteEndpointIdentity, RestackAuthor, RestackSelection,
    RestackSnapshot, RESTACK_SCHEMA_VERSION,
};
use hmac::Hmac;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use handles::{canonical_session_root, load_or_create_integrity_key, SessionToken};
use metadata::{
    now, random_bytes, random_hex, read_metadata, session_expired_without_token, valid_hex,
};
use restricted_fs::{
    create_lock, create_new_restricted_directory, create_restricted_directory, open_lock,
    require_directory, restrict_directory,
};

mod handles;
mod metadata;
mod restricted_fs;

pub(crate) use handles::{SessionDraft, SessionHandle};

const SESSION_TTL_SECONDS: u64 = 24 * 60 * 60;

const SESSION_FILE: &str = "session.json";

const LOCK_FILE: &str = "session.lock";

const INTEGRITY_KEY_FILE: &str = "integrity.key";

const REPOSITORY_DIRECTORY: &str = "repository";

const MAX_METADATA_BYTES: u64 = 1024 * 1024;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SessionMetadata {
    pub(crate) schema_version: u8,
    pub(crate) repository_id: String,
    pub(crate) snapshot: RestackSnapshot,
    pub(crate) remote_endpoints: RemoteEndpointIdentity,
    pub(crate) author: RestackAuthor,
    pub(crate) selection: RestackSelection,
    pub(crate) orphaned_commits: Vec<OrphanedCommit>,
    pub(crate) merges: Vec<MergeOutcome>,
    pub(crate) next_feature: usize,
    pub(crate) expected_head: String,
    pub(crate) expected_head_reflog: String,
    pub(crate) expected_feature_tip: Option<String>,
    pub(crate) status: SessionStatus,
    pub(crate) final_tree: Option<String>,
    pub(crate) preview_commit: Option<String>,
    pub(crate) plan_digest: Option<String>,
    pub(crate) created_at: u64,
    pub(crate) last_activity: u64,
    pub(crate) expires_at: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SessionStatus {
    Conflicted,
    Sealed,
    Publishing,
    Consumed,
}

pub(crate) struct SessionConflict {
    pub(crate) merges: Vec<MergeOutcome>,
    pub(crate) next_feature: usize,
    pub(crate) expected_head: String,
    pub(crate) expected_head_reflog: String,
    pub(crate) expected_feature_tip: String,
}

impl SessionMetadata {
    pub(crate) fn conflicted(
        repository_id: String,
        snapshot: RestackSnapshot,
        remote_endpoints: RemoteEndpointIdentity,
        author: RestackAuthor,
        selection: RestackSelection,
        orphaned_commits: Vec<OrphanedCommit>,
        conflict: SessionConflict,
    ) -> Result<Self, SessionError> {
        let now = now()?;
        Ok(Self {
            schema_version: RESTACK_SCHEMA_VERSION,
            repository_id,
            snapshot,
            remote_endpoints,
            author,
            selection,
            orphaned_commits,
            merges: conflict.merges,
            next_feature: conflict.next_feature,
            expected_head: conflict.expected_head,
            expected_head_reflog: conflict.expected_head_reflog,
            expected_feature_tip: Some(conflict.expected_feature_tip),
            status: SessionStatus::Conflicted,
            final_tree: None,
            preview_commit: None,
            plan_digest: None,
            created_at: now,
            last_activity: now,
            expires_at: now.saturating_add(SESSION_TTL_SECONDS),
        })
    }

    pub(crate) fn refresh(&mut self) -> Result<(), SessionError> {
        let now = now()?;
        self.last_activity = now;
        self.expires_at = now.saturating_add(SESSION_TTL_SECONDS);
        Ok(())
    }

    pub(crate) fn is_expired(&self) -> Result<bool, SessionError> {
        Ok(self.expires_at <= now()?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SessionError {
    Unavailable,
    InvalidToken,
    Missing,
    Locked,
    Tampered,
    Expired,
    /// The session was written by a Graduate release with a different restack schema.
    SchemaMismatch {
        found: u64,
        expected: u8,
    },
}

pub(crate) struct SessionStore {
    root: PathBuf,
    integrity_key: [u8; 32],
}

impl SessionStore {
    pub(crate) fn open() -> Result<Self, SessionError> {
        let cache = dirs::cache_dir().ok_or(SessionError::Unavailable)?;
        Self::open_root(cache.join("graduate").join("restack").join("sessions"))
    }

    /// Open or create a session store rooted at `root`.
    pub(crate) fn open_root(root: PathBuf) -> Result<Self, SessionError> {
        create_restricted_directory(&root)?;
        let root = canonical_session_root(root)?;
        if root.to_str().is_none() {
            return Err(SessionError::Unavailable);
        }
        let integrity_key = load_or_create_integrity_key(&root)?;
        Ok(Self {
            root,
            integrity_key,
        })
    }

    pub(crate) fn purge_expired(&self) -> Result<(), SessionError> {
        self.purge_expired_except(None)
    }

    pub(crate) fn prepare_resume(&self, token: &str) -> Result<(), SessionError> {
        match SessionToken::parse(token) {
            Ok(token) => self.purge_expired_except(Some(&token.id)),
            Err(error) => {
                self.purge_expired_except(None)?;
                Err(error)
            }
        }
    }

    fn purge_expired_except(&self, excluded: Option<&str>) -> Result<(), SessionError> {
        let entries = fs::read_dir(&self.root).map_err(|_| SessionError::Unavailable)?;
        for entry in entries {
            let entry = entry.map_err(|_| SessionError::Unavailable)?;
            let name = entry.file_name();
            let Some(id) = name.to_str() else {
                continue;
            };
            if !valid_hex(id, 32) {
                continue;
            }
            if excluded == Some(id) {
                continue;
            }
            let directory = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&directory) else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let Ok(lock) = open_lock(&directory) else {
                continue;
            };
            if fs2::FileExt::try_lock_exclusive(&lock).is_err() {
                continue;
            }
            if session_expired_without_token(&directory).unwrap_or(false) {
                drop(lock);
                let _ = fs::remove_dir_all(directory);
            }
        }
        Ok(())
    }

    pub(crate) fn begin(&self) -> Result<SessionDraft, SessionError> {
        for _ in 0..8 {
            let id = random_hex::<16>()?;
            let secret = random_bytes::<32>()?;
            let directory = self.root.join(&id);
            match create_new_restricted_directory(&directory) {
                Ok(()) => {
                    restrict_directory(&directory)?;
                    let lock = create_lock(&directory)?;
                    fs2::FileExt::lock_exclusive(&lock).map_err(|_| SessionError::Unavailable)?;
                    return Ok(SessionDraft {
                        id,
                        secret,
                        integrity_key: self.integrity_key,
                        directory,
                        lock: Some(lock),
                        preserved: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(SessionError::Unavailable),
            }
        }
        Err(SessionError::Unavailable)
    }

    pub(crate) fn resume(&self, token: &str) -> Result<SessionHandle, SessionError> {
        let token = SessionToken::parse(token)?;
        let directory = self.root.join(&token.id);
        require_directory(&directory)?;
        let lock = open_lock(&directory)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| SessionError::Locked)?;
        let metadata = read_metadata(&directory, &self.integrity_key, &token.secret)?;
        if metadata.is_expired()? {
            drop(lock);
            let _ = fs::remove_dir_all(&directory);
            return Err(SessionError::Expired);
        }
        Ok(SessionHandle {
            secret: token.secret,
            integrity_key: self.integrity_key,
            directory,
            lock: Some(lock),
            metadata,
        })
    }
}
