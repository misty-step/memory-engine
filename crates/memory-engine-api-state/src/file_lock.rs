use std::{fs, path::Path};

#[cfg(unix)]
use std::os::fd::AsRawFd;

use crate::ApiFailure;

pub(crate) struct FileDescriptorLock {
    _file: fs::File,
}

pub(crate) fn acquire(path: &Path) -> Result<FileDescriptorLock, ApiFailure> {
    acquire_transient_tolerant(path)?.ok_or_else(|| {
        ApiFailure::conflict("The shared file state is busy; try the operation again.")
    })
}

/// Acquire, waiting out transient holders for a bounded window.
///
/// A logically free lock can appear held: spawning any subprocess forks this
/// process, and until the child execs (closing its CLOEXEC copy of the fd
/// table) it co-owns every open flock — including locks it has nothing to do
/// with. Those windows are tiny, but on a slow machine they are wide enough
/// to make a nonblocking acquire refuse a lock nobody logically holds, which
/// silently dropped notification claims and completions (memory-engine-101).
/// Genuine holders do microsecond-scale read-modify-write work, so a short
/// bounded retry preserves refusal semantics for real contention: `None` is
/// returned only when the lock stays held past the deadline. Correctness
/// never depends on winning the lock — the persisted claim/fence state does —
/// so waiting is always safe.
pub(crate) fn acquire_transient_tolerant(
    path: &Path,
) -> Result<Option<FileDescriptorLock>, ApiFailure> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        if let Some(lock) = try_acquire(path)? {
            return Ok(Some(lock));
        }
        if std::time::Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[cfg(unix)]
pub(crate) fn acquire_blocking(path: &Path) -> Result<FileDescriptorLock, ApiFailure> {
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
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if result == 0 {
        Ok(FileDescriptorLock { _file: file })
    } else {
        Err(ApiFailure::internal(
            std::io::Error::last_os_error().to_string(),
        ))
    }
}

#[cfg(not(unix))]
pub(crate) fn acquire_blocking(_path: &Path) -> Result<FileDescriptorLock, ApiFailure> {
    Err(ApiFailure::internal(
        "Shared file locking is unsupported on this platform.".to_owned(),
    ))
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
