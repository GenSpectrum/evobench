//! Wrapper guards around `fs2` crate.

//! First move your file handle into a `LockableFile` via
//! From/Into. Then call locking methods on that to get a guard with
//! access to the file handle.

use std::{
    cell::RefCell,
    collections::HashSet,
    fmt::Display,
    fs::File,
    ops::Deref,
    path::{Path, PathBuf},
};

use fs2::{lock_contended_error, FileExt};
use lazy_static::lazy_static;
use ouroboros::self_referencing;

// -----------------------------------------------------------------------------

pub struct SharedFileLock<'s, F: FileExt> {
    debug: &'s Option<Box<Path>>,
    // XX joke: need DerefMut anyway, even reading requires mut
    // access. So the two locks are identical now. TODO: eliminate or
    // ? Parameterize instead?
    file: &'s F,
}

impl<'s, F: FileExt> Drop for SharedFileLock<'s, F> {
    fn drop(&mut self) {
        self.file
            .unlock()
            .expect("no way another path to unlock exists");
        if let Some(path) = self.debug {
            eprintln!("dropped SharedFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| {
                set.remove(&**path);
            });
        }
    }
}

impl<'s, F: FileExt> Deref for SharedFileLock<'s, F> {
    type Target = F;

    fn deref(&self) -> &Self::Target {
        self.file
    }
}

// -----------------------------------------------------------------------------

#[derive(Debug)]
pub struct ExclusiveFileLock<'s, F: FileExt> {
    debug: &'s Option<Box<Path>>,
    file: &'s F,
}

impl<'s, F: FileExt> PartialEq for ExclusiveFileLock<'s, F> {
    fn eq(&self, other: &Self) -> bool {
        self.debug == other.debug
    }
}

impl<'s, F: FileExt> Drop for ExclusiveFileLock<'s, F> {
    fn drop(&mut self) {
        self.file
            .unlock()
            .expect("no way another path to unlock exists");
        if let Some(path) = self.debug {
            eprintln!("dropped ExclusiveFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| {
                set.remove(&**path);
            });
        }
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
    /// Path for, and only if, debugging is set via
    /// `DEBUG_LOCKABLE_FILE` env var
    debug: Option<Box<Path>>,
    file: F,
}

impl<F: FileExt> From<F> for LockableFile<F> {
    fn from(file: F) -> Self {
        Self {
            // XX can't have path here, what to do?
            debug: None,
            file,
        }
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

thread_local! {
    static HELD_LOCKS: RefCell< HashSet<PathBuf>> = Default::default();
}

impl<F: FileExt> LockableFile<F> {
    /// Determines lock status by temporarily getting locks in
    /// nonblocking manner, thus not very performant! Also, may
    /// erroneously return `LockStatus::SharedLock` if during testing
    /// an exclusive lock is released.
    pub fn get_lock_status(&mut self) -> std::io::Result<LockStatus> {
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
        if let Some(path) = self.debug.as_ref() {
            eprintln!("getting SharedFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| -> std::io::Result<()> {
                // XXX: ah, todo: allow multiple shared
                if set.contains(&**path) {
                    panic!("{path:?} is already locked by this thread")
                }
                FileExt::lock_shared(&self.file)?;
                eprintln!("got SharedFileLock on {path:?}");
                set.insert((&**path).to_owned());
                Ok(())
            })?;
        } else {
            FileExt::lock_shared(&self.file)?;
        }
        Ok(SharedFileLock {
            debug: &self.debug,
            file: &self.file,
        })
    }

    pub fn lock_exclusive<'s>(&'s self) -> std::io::Result<ExclusiveFileLock<'s, F>> {
        if let Some(path) = self.debug.as_ref() {
            eprintln!("getting ExclusiveFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| -> std::io::Result<()> {
                if set.contains(&**path) {
                    panic!("{path:?} is already locked by this thread")
                }
                FileExt::lock_exclusive(&self.file)?;
                eprintln!("got ExclusiveFileLock on {path:?}");
                set.insert((&**path).to_owned());
                Ok(())
            })?;
        } else {
            FileExt::lock_exclusive(&self.file)?;
        }
        Ok(ExclusiveFileLock {
            debug: &self.debug,
            file: &self.file,
        })
    }

    pub fn try_lock_shared<'s>(&'s self) -> std::io::Result<Option<SharedFileLock<'s, F>>> {
        match FileExt::try_lock_shared(&self.file) {
            Ok(()) => {
                if let Some(path) = self.debug.as_ref() {
                    eprintln!("got SharedFileLock on {path:?}");
                    HELD_LOCKS.with_borrow_mut(|set| {
                        set.insert((&**path).to_owned());
                    });
                }
                Ok(Some(SharedFileLock {
                    debug: &self.debug,
                    file: &self.file,
                }))
            }
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
            Ok(()) => {
                if let Some(path) = self.debug.as_ref() {
                    eprintln!("got ExclusiveFileLock on {path:?}");
                    HELD_LOCKS.with_borrow_mut(|set| {
                        set.insert((&**path).to_owned());
                    });
                }
                Ok(Some(ExclusiveFileLock {
                    debug: &self.debug,
                    file: &self.file,
                }))
            }
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

lazy_static! {
    static ref DEBUGGING: bool = if let Some(val) = std::env::var_os("DEBUG_LOCKABLE_FILE") {
        match val
            .into_string()
            .expect("utf-8 for env var DEBUG_LOCKABLE_FILE")
            .as_str()
        {
            "0" => false,
            "1" | "" => true,
            _ => panic!("need 1|0 or empty string for DEBUG_LOCKABLE_FILE"),
        }
    } else {
        false
    };
}

impl LockableFile<File> {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        File::open(path.as_ref()).and_then(|file| {
            let path = if *DEBUGGING {
                Some(path.as_ref().to_owned().into_boxed_path())
            } else {
                None
            };

            Ok(LockableFile { debug: path, file })
        })
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
