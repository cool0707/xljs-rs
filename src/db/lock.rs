//! The lock file of a shared database: a reader-writer lock between the
//! people who use the database from a file server, kept apart from
//! SQLite's own byte-range locks (which network file systems do not always
//! honor).
//!
//! On Windows the lock is the sharing mode of the open lock file, which the
//! file server enforces itself: readers open it for reading and share
//! reading, a writer opens it for writing and shares nothing. The server
//! drops the handle when the client goes away, so a crashed client never
//! leaves a stale lock behind. Elsewhere (tests) flock does the same.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A held lock; dropping it releases the lock.
pub struct Lock {
    _file: File,
    pub write: bool,
}

/// Why a lock could not be taken.
pub enum LockError {
    /// Someone else held it until the timeout.
    Busy,
    Io(io::Error),
}

#[cfg(windows)]
fn try_open(path: &Path, write: bool) -> io::Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    let mut o = OpenOptions::new();
    o.read(true);
    if write {
        o.write(true).access_mode(GENERIC_READ | GENERIC_WRITE).share_mode(0);
    } else {
        o.access_mode(GENERIC_READ).share_mode(FILE_SHARE_READ);
    }
    match o.open(path) {
        Ok(f) => Ok(Some(f)),
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        Err(e) if matches!(e.raw_os_error(), Some(32) | Some(33)) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // The lock file went missing: create it (sharing everything,
            // so that this cannot get in anybody's way) and try again.
            match OpenOptions::new().write(true).create(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).open(path) {
                Ok(_) => Ok(None),
                Err(e) if matches!(e.raw_os_error(), Some(32) | Some(33)) => Ok(None),
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}

#[cfg(not(windows))]
fn try_open(path: &Path, write: bool) -> io::Result<Option<File>> {
    let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
    let r = if write { f.try_lock() } else { f.try_lock_shared() };
    match r {
        Ok(()) => Ok(Some(f)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Takes the lock at path (shared for reading, exclusive for writing),
/// waiting up to timeout for others to let go.
pub fn acquire(path: &Path, write: bool, timeout: Duration) -> Result<Lock, LockError> {
    let start = Instant::now();
    let mut pause = 20u64;
    loop {
        match try_open(path, write) {
            Ok(Some(f)) => return Ok(Lock { _file: f, write }),
            Ok(None) => {}
            Err(e) => return Err(LockError::Io(e)),
        }
        if start.elapsed() >= timeout {
            return Err(LockError::Busy);
        }
        // Waiting for others is not the script being stuck.
        crate::engine::watchdog::waited();
        // A little jitter so that clients waiting together do not retry in
        // step.
        let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64 % 20).unwrap_or(0);
        std::thread::sleep(Duration::from_millis(pause + jitter));
        pause = (pause * 3 / 2).min(250);
    }
}

/// Creates the lock file if it is missing.
pub fn ensure(path: &PathBuf) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        match OpenOptions::new().write(true).create(true).truncate(false).share_mode(3).open(path) {
            Ok(_) => Ok(()),
            Err(e) if matches!(e.raw_os_error(), Some(32) | Some(33)) => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(windows))]
    {
        OpenOptions::new().write(true).create(true).truncate(false).open(path).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_share_writers_exclude() {
        let dir = crate::testutil::tempdir();
        let p = dir.join("lock");
        ensure(&p).unwrap();
        let short = Duration::from_millis(60);
        let r1 = acquire(&p, false, short).ok().unwrap();
        let r2 = acquire(&p, false, short).ok().unwrap();
        assert!(matches!(acquire(&p, true, short), Err(LockError::Busy)));
        drop(r1);
        drop(r2);
        let w = acquire(&p, true, short).ok().unwrap();
        assert!(matches!(acquire(&p, false, short), Err(LockError::Busy)));
        assert!(matches!(acquire(&p, true, short), Err(LockError::Busy)));
        drop(w);
        assert!(acquire(&p, false, short).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
