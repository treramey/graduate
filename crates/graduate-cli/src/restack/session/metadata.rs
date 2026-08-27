//! Authenticated metadata persistence, encoding, and expiry.

use std::fs::{self};
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use graduate::restack::RESTACK_SCHEMA_VERSION;
use hmac::Mac;
use sha2::{Digest, Sha256};

use super::handles::SessionEnvelope;
use super::restricted_fs::{restrict_file, restricted_file_metadata, sync_directory};
use super::{
    HmacSha256, SessionError, SessionMetadata, MAX_METADATA_BYTES, SESSION_FILE,
    SESSION_TTL_SECONDS,
};

pub(super) fn write_metadata(
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

pub(super) fn read_metadata(
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
    // Older schemas lack fields the typed envelope requires, so check the
    // version before typed deserialization or a mismatch reads as tampering.
    let untyped: serde_json::Value =
        serde_json::from_slice(&contents).map_err(|_| SessionError::Tampered)?;
    let found = untyped["metadata"]["schemaVersion"]
        .as_u64()
        .ok_or(SessionError::Tampered)?;
    if found != u64::from(RESTACK_SCHEMA_VERSION) {
        return Err(SessionError::SchemaMismatch {
            found,
            expected: RESTACK_SCHEMA_VERSION,
        });
    }
    let envelope: SessionEnvelope =
        serde_json::from_value(untyped).map_err(|_| SessionError::Tampered)?;
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

pub(super) fn session_expired_without_token(directory: &Path) -> Result<bool, SessionError> {
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

pub(super) fn random_hex<const N: usize>() -> Result<String, SessionError> {
    Ok(encode_hex(&random_bytes::<N>()?))
}

pub(super) fn random_bytes<const N: usize>() -> Result<[u8; N], SessionError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| SessionError::Unavailable)?;
    Ok(bytes)
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub(super) fn decode_secret(value: &str) -> Result<[u8; 32], SessionError> {
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

pub(super) fn valid_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn now() -> Result<u64, SessionError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| SessionError::Unavailable)
}
