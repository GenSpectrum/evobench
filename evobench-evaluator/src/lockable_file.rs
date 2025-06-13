//! Wrapper guards around `fs2` crate.

//! First move your file handle into a `LockableFile` via
//! From/Into. Then call locking methods on that to get a guard with
//! access to the file handle.

use std::{
    fmt::Display,
    ops::{Deref, DerefMut},
};

use fs2::{lock_contended_error, FileExt};

// -----------------------------------------------------------------------------

pub struct SharedFileLock<'s, F: FileExt> {
    // XX joke: need DerefMut anyway, even reading requires mut
    // access. So the two locks are identical now. TODO: eliminate or
    // ? Parameterize instead?
    file: &'s mut F,
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

// XX joke: need DerefMut anyway, even reading requires mut access.
impl<'s, F: FileExt> DerefMut for SharedFileLock<'s, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.file
    }
}

// -----------------------------------------------------------------------------

pub struct ExclusiveFileLock<'s, F: FileExt> {
    file: &'s mut F,
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

impl<'s, F: FileExt> DerefMut for ExclusiveFileLock<'s, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.file
    }
}

// -----------------------------------------------------------------------------

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

    pub fn lock_shared<'s>(&'s mut self) -> std::io::Result<SharedFileLock<'s, F>> {
        FileExt::lock_shared(&self.file)?;
        Ok(SharedFileLock {
            file: &mut self.file,
        })
    }

    pub fn lock_exclusive<'s>(&'s mut self) -> std::io::Result<ExclusiveFileLock<'s, F>> {
        FileExt::lock_exclusive(&self.file)?;
        Ok(ExclusiveFileLock {
            file: &mut self.file,
        })
    }

    pub fn try_lock_shared<'s>(&'s mut self) -> std::io::Result<Option<SharedFileLock<'s, F>>> {
        match FileExt::try_lock_shared(&self.file) {
            Ok(()) => Ok(Some(SharedFileLock {
                file: &mut self.file,
            })),
            Err(e) => {
                if e.kind() == lock_contended_error().kind() {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    pub fn try_lock_exclusive<'s>(
        &'s mut self,
    ) -> std::io::Result<Option<ExclusiveFileLock<'s, F>>> {
        match FileExt::try_lock_exclusive(&self.file) {
            Ok(()) => Ok(Some(ExclusiveFileLock {
                file: &mut self.file,
            })),
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
