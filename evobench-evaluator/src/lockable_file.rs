//! Wrapper guards around `fs2` crate.

//! First move your file handle into a `LockableFile` via
//! From/Into. Then call locking methods on that to get a guard with
//! access to the file handle.

use std::{
    fmt::Display,
    fs::File,
    ops::Deref,
    path::{Path, PathBuf},
};

use fs2::{lock_contended_error, FileExt};
use ouroboros::self_referencing;

// -----------------------------------------------------------------------------

pub struct SharedFileLock<'s, F: FileExt> {
    // XX joke: need DerefMut anyway, even reading requires mut
    // access. So the two locks are identical now. TODO: eliminate or
    // ? Parameterize instead?
    file: &'s F,
}

impl<'s, F: FileExt> Drop for SharedFileLock<'s, F> {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl<'s, F: FileExt> Deref for SharedFileLock<'s, F> {
    type Target = F;

    fn deref(&self) -> &Self::Target {
        self.file
    }
}

// -----------------------------------------------------------------------------

pub struct ExclusiveFileLock<'s, F: FileExt> {
    file: &'s F,
}

impl<'s, F: FileExt> Drop for ExclusiveFileLock<'s, F> {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl<'s, F: FileExt> Deref for ExclusiveFileLock<'s, F> {
    type Target = F;

    fn deref(&self) -> &Self::Target {
        self.file
    }
}

// -----------------------------------------------------------------------------

#[derive(Debug)]
pub struct LockableFile<F: FileExt> {
    file: F,
}

impl<F: FileExt> From<F> for LockableFile<F> {
    fn from(file: F) -> Self {
        Self { file }
    }
}

/// Information about what kind of lock is held
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LockStatus {
    Unlocked,
    SharedLock,
    ExclusiveLock,
}
impl LockStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LockStatus::Unlocked => "unlocked",
            LockStatus::SharedLock => "shared lock",
            LockStatus::ExclusiveLock => "exclusive lock",
        }
    }
}

impl Display for LockStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<F: FileExt> LockableFile<F> {
    /// Determines lock status by temporarily getting locks in
    /// nonblocking manner, thus not very performant! Also, may
    /// erroneously return `LockStatus::SharedLock` if during testing
    /// an exclusive lock is released.
    pub fn lock_status(&mut self) -> std::io::Result<LockStatus> {
        use LockStatus::*;
        Ok(if self.try_lock_exclusive()?.is_some() {
            Unlocked
        } else if self.try_lock_shared()?.is_some() {
            SharedLock
        } else {
            ExclusiveLock
        })
    }

    pub fn lock_shared<'s>(&'s self) -> std::io::Result<SharedFileLock<'s, F>> {
        FileExt::lock_shared(&self.file)?;
        Ok(SharedFileLock { file: &self.file })
    }

    pub fn lock_exclusive<'s>(&'s self) -> std::io::Result<ExclusiveFileLock<'s, F>> {
        FileExt::lock_exclusive(&self.file)?;
        Ok(ExclusiveFileLock { file: &self.file })
    }

    pub fn try_lock_shared<'s>(&'s self) -> std::io::Result<Option<SharedFileLock<'s, F>>> {
        match FileExt::try_lock_shared(&self.file) {
            Ok(()) => Ok(Some(SharedFileLock { file: &self.file })),
            Err(e) => {
                if e.kind() == lock_contended_error().kind() {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    pub fn try_lock_exclusive<'s>(&'s self) -> std::io::Result<Option<ExclusiveFileLock<'s, F>>> {
        match FileExt::try_lock_exclusive(&self.file) {
            Ok(()) => Ok(Some(ExclusiveFileLock { file: &self.file })),
            Err(e) => {
                if e.kind() == lock_contended_error().kind() {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }
}

impl LockableFile<File> {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        File::open(path).and_then(|file| Ok(LockableFile { file }))
    }
}

/// A simple file or dir lock based on `flock`; dropping this type
/// unlocks it and also drops the file handle at the same time, thus
/// it's less efficient than allocating a `LockableFile<File>` and
/// then doing the locking operations on it.
#[self_referencing]
pub struct StandaloneExclusiveFileLock {
    lockable: LockableFile<File>,
    #[borrows(lockable)]
    #[covariant]
    lock: Option<ExclusiveFileLock<'this, File>>,
}

#[derive(thiserror::Error, Debug)]
pub enum StandaloneFileLockError {
    #[error("error locking {path:?}: {error:#}")]
    IOError {
        path: PathBuf,
        error: std::io::Error,
    },

    #[error("{msg}: the path {path:?} is already locked")]
    AlreadyLocked { path: PathBuf, msg: String },
}

impl StandaloneExclusiveFileLock {
    pub fn lock_path<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        Self::try_new(LockableFile::open(path)?, |file| {
            Ok(Some(file.lock_exclusive()?))
        })
    }

    /// If the lock is already taken, returns a
    /// `StandaloneFileLockError::AlreadyLocked` error that includes
    /// the result of running `already_locked_msg` as the first part
    /// of the error message.
    pub fn try_lock_path<P: AsRef<Path>>(
        path: P,
        already_locked_msg: impl Fn() -> String,
    ) -> Result<Self, StandaloneFileLockError> {
        let us = (|| -> std::io::Result<_> {
            Self::try_new(LockableFile::open(path.as_ref())?, |file| {
                file.try_lock_exclusive()
            })
        })()
        .map_err(|error| StandaloneFileLockError::IOError {
            path: path.as_ref().to_owned(),
            error,
        })?;
        if us.borrow_lock().is_some() {
            Ok(us)
        } else {
            let msg = already_locked_msg();
            Err(StandaloneFileLockError::AlreadyLocked {
                path: path.as_ref().to_owned(),
                msg,
            })
        }
    }
}
