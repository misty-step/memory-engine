use std::{fs, path::Path};

#[cfg(unix)]
use std::os::fd::AsRawFd;

use crate::ApiFailure;

pub(crate) struct FileDescriptorLock {
    _file: fs::File,
}

pub(crate) fn acquire(path: &Path) -> Result<FileDescriptorLock, ApiFailure> {
    try_acquire(path)?.ok_or_else(|| {
        ApiFailure::conflict("The shared file state is busy; try the operation again.")
    })
}

#[cfg(unix)]
pub(crate) fn try_acquire(path: &Path) -> Result<Option<FileDescriptorLock>, ApiFailure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| ApiFailure::internal(error.to_string()))?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| ApiFailure::internal(error.to_string()))?;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(FileDescriptorLock { _file: file }));
    }
    let error = std::io::Error::last_os_error();
    if error
        .raw_os_error()
        .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
    {
        return Ok(None);
    }
    Err(ApiFailure::internal(error.to_string()))
}

#[cfg(not(unix))]
pub(crate) fn try_acquire(_path: &Path) -> Result<Option<FileDescriptorLock>, ApiFailure> {
    Err(ApiFailure::internal(
        "Shared file locking is unsupported on this platform.".to_owned(),
    ))
}
