//! Permission-restricted, expiring state for resumable restack conflicts.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use graduate::restack::{
    MergeOutcome, OrphanedCommit, RemoteEndpointIdentity, RestackAuthor, RestackSelection,
    RestackSnapshot, RESTACK_SCHEMA_VERSION,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

#[derive(Debug)]
pub(crate) enum SessionError {
    Unavailable,
    InvalidToken,
    Missing,
    Locked,
    Tampered,
    Expired,
}

pub(crate) struct SessionStore {
    root: PathBuf,
    integrity_key: [u8; 32],
}

impl SessionStore {
    pub(crate) fn open() -> Result<Self, SessionError> {
        let cache = dirs::cache_dir().ok_or(SessionError::Unavailable)?;
        let root = cache.join("graduate").join("restack").join("sessions");
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

fn load_or_create_integrity_key(root: &Path) -> Result<[u8; 32], SessionError> {
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
fn canonical_session_root(root: PathBuf) -> Result<PathBuf, SessionError> {
    fs::canonicalize(root).map_err(|_| SessionError::Unavailable)
}

#[cfg(windows)]
fn canonical_session_root(root: PathBuf) -> Result<PathBuf, SessionError> {
    Ok(root)
}

pub(crate) struct SessionDraft {
    id: String,
    secret: [u8; 32],
    integrity_key: [u8; 32],
    directory: PathBuf,
    lock: Option<File>,
    preserved: bool,
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
        if self.preserved {
            return;
        }
        if let Some(lock) = self.lock.take() {
            let _ = fs2::FileExt::unlock(&lock);
            drop(lock);
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

pub(crate) struct SessionHandle {
    secret: [u8; 32],
    integrity_key: [u8; 32],
    directory: PathBuf,
    lock: Option<File>,
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
struct SessionEnvelope {
    metadata: SessionMetadata,
    capability: String,
    integrity: String,
}

struct SessionToken {
    id: String,
    secret: [u8; 32],
}

impl SessionToken {
    fn parse(value: &str) -> Result<Self, SessionError> {
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

fn write_metadata(
    directory: &Path,
    integrity_key: &[u8; 32],
    secret: &[u8; 32],
    metadata: &SessionMetadata,
) -> Result<(), SessionError> {
    let capability = capability_digest(secret);
    let integrity = metadata_integrity(integrity_key, metadata, &capability)?;
    let envelope = SessionEnvelope {
        metadata: metadata.clone(),
        capability,
        integrity,
    };
    let mut contents =
        serde_json::to_vec_pretty(&envelope).map_err(|_| SessionError::Unavailable)?;
    contents.push(b'\n');
    let destination = directory.join(SESSION_FILE);

    #[cfg(windows)]
    {
        let file = atomicwrites::AtomicFile::new(&destination, atomicwrites::AllowOverwrite);
        file.write(|temporary| temporary.write_all(&contents))
            .map_err(|_| SessionError::Unavailable)?;
    }

    #[cfg(not(windows))]
    {
        let mut temporary = tempfile::Builder::new()
            .prefix(".session-")
            .suffix(".tmp")
            .tempfile_in(directory)
            .map_err(|_| SessionError::Unavailable)?;
        restrict_file(temporary.as_file())?;
        temporary
            .write_all(&contents)
            .map_err(|_| SessionError::Unavailable)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| SessionError::Unavailable)?;
        temporary
            .persist(destination)
            .map_err(|_| SessionError::Unavailable)?;
    }
    sync_directory(directory).map_err(|_| SessionError::Unavailable)
}

fn read_metadata(
    directory: &Path,
    integrity_key: &[u8; 32],
    secret: &[u8; 32],
) -> Result<SessionMetadata, SessionError> {
    let path = directory.join(SESSION_FILE);
    let file_metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SessionError::Missing
        } else {
            SessionError::Unavailable
        }
    })?;
    if file_metadata.file_type().is_symlink()
        || !file_metadata.is_file()
        || file_metadata.len() > MAX_METADATA_BYTES
        || !restricted_file_metadata(&file_metadata)
    {
        return Err(SessionError::Tampered);
    }
    let contents = fs::read(path).map_err(|_| SessionError::Unavailable)?;
    let envelope: SessionEnvelope =
        serde_json::from_slice(&contents).map_err(|_| SessionError::Tampered)?;
    if envelope.metadata.schema_version != RESTACK_SCHEMA_VERSION {
        return Err(SessionError::Tampered);
    }
    if envelope.capability != capability_digest(secret) {
        return Err(SessionError::InvalidToken);
    }
    let expected = metadata_integrity(integrity_key, &envelope.metadata, &envelope.capability)?;
    if expected != envelope.integrity {
        return Err(SessionError::Tampered);
    }
    Ok(envelope.metadata)
}

fn metadata_integrity(
    integrity_key: &[u8; 32],
    metadata: &SessionMetadata,
    capability: &str,
) -> Result<String, SessionError> {
    let canonical = serde_json::to_value(metadata).map_err(|_| SessionError::Unavailable)?;
    let encoded = serde_json::to_vec(&canonical).map_err(|_| SessionError::Unavailable)?;
    let mut mac =
        HmacSha256::new_from_slice(integrity_key).map_err(|_| SessionError::Unavailable)?;
    mac.update(&encoded);
    mac.update(&[0]);
    mac.update(capability.as_bytes());
    Ok(encode_hex(&mac.finalize().into_bytes()))
}

fn capability_digest(secret: &[u8; 32]) -> String {
    encode_hex(&Sha256::digest(secret))
}

fn session_expired_without_token(directory: &Path) -> Result<bool, SessionError> {
    let path = directory.join(SESSION_FILE);
    match fs::read(&path) {
        Ok(contents) => {
            let expired = match serde_json::from_slice::<SessionEnvelope>(&contents) {
                Ok(envelope) => envelope.metadata.expires_at <= now()?,
                Err(_) => false,
            };
            Ok(expired || directory_inactive(directory)?)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => directory_inactive(directory),
        Err(_) => Err(SessionError::Unavailable),
    }
}

fn directory_inactive(directory: &Path) -> Result<bool, SessionError> {
    let modified = fs::metadata(directory)
        .and_then(|metadata| metadata.modified())
        .map_err(|_| SessionError::Unavailable)?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .map(|age| age.as_secs() >= SESSION_TTL_SECONDS)
        .unwrap_or(false))
}

fn create_restricted_directory(path: &Path) -> Result<(), SessionError> {
    create_restricted_directory_all(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| SessionError::Unavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SessionError::Tampered);
    }
    restrict_directory(path)
}

#[cfg(unix)]
fn create_restricted_directory_all(path: &Path) -> Result<(), SessionError> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path).map_err(|_| SessionError::Unavailable)
}

#[cfg(not(unix))]
fn create_restricted_directory_all(path: &Path) -> Result<(), SessionError> {
    fs::create_dir_all(path).map_err(|_| SessionError::Unavailable)
}

#[cfg(unix)]
fn create_new_restricted_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_new_restricted_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

fn require_directory(path: &Path) -> Result<(), SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SessionError::Missing
        } else {
            SessionError::Unavailable
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SessionError::Tampered);
    }
    if !restricted_directory_metadata(&metadata) {
        return Err(SessionError::Tampered);
    }
    Ok(())
}

fn create_lock(directory: &Path) -> Result<File, SessionError> {
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.join(LOCK_FILE))
        .map_err(|_| SessionError::Unavailable)?;
    restrict_file(&file)?;
    Ok(file)
}

fn open_lock(directory: &Path) -> Result<File, SessionError> {
    let path = directory.join(LOCK_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SessionError::Missing
        } else {
            SessionError::Unavailable
        }
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || !restricted_file_metadata(&metadata)
    {
        return Err(SessionError::Tampered);
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| SessionError::Unavailable)
}

fn random_hex<const N: usize>() -> Result<String, SessionError> {
    Ok(encode_hex(&random_bytes::<N>()?))
}

fn random_bytes<const N: usize>() -> Result<[u8; N], SessionError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| SessionError::Unavailable)?;
    Ok(bytes)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_secret(value: &str) -> Result<[u8; 32], SessionError> {
    let mut bytes = [0_u8; 32];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(SessionError::InvalidToken);
    }
    for (index, pair) in pairs.iter().enumerate() {
        let high = decode_nibble(pair[0]).ok_or(SessionError::InvalidToken)?;
        let low = decode_nibble(pair[1]).ok_or(SessionError::InvalidToken)?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn now() -> Result<u64, SessionError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| SessionError::Unavailable)
}

#[cfg(unix)]
fn restricted_directory_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o777 == 0o700
}

#[cfg(not(unix))]
fn restricted_directory_metadata(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn restricted_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o777 == 0o600
}

#[cfg(not(unix))]
fn restricted_file_metadata(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<(), SessionError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| SessionError::Unavailable)
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<(), SessionError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(file: &File) -> Result<(), SessionError> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| SessionError::Unavailable)
}

#[cfg(not(unix))]
fn restrict_file(_file: &File) -> Result<(), SessionError> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}
