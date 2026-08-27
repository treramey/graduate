//! Permission-restricted directories, files, and locks.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::{fs, io};

use super::{SessionError, LOCK_FILE};

pub(super) fn create_restricted_directory(path: &Path) -> Result<(), SessionError> {
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
pub(super) fn create_new_restricted_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_new_restricted_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

pub(super) fn require_directory(path: &Path) -> Result<(), SessionError> {
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

pub(super) fn create_lock(directory: &Path) -> Result<File, SessionError> {
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(directory.join(LOCK_FILE))
        .map_err(|_| SessionError::Unavailable)?;
    restrict_file(&file)?;
    Ok(file)
}

pub(super) fn open_lock(directory: &Path) -> Result<File, SessionError> {
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
pub(super) fn restricted_file_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o777 == 0o600
}

#[cfg(not(unix))]
fn restricted_file_metadata(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
pub(super) fn restrict_directory(path: &Path) -> Result<(), SessionError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| SessionError::Unavailable)
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<(), SessionError> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn restrict_file(file: &File) -> Result<(), SessionError> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| SessionError::Unavailable)
}

#[cfg(not(unix))]
fn restrict_file(_file: &File) -> Result<(), SessionError> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}
