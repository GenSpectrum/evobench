//! Variant of `lockable_file.rs` that does not have lifetimes in its
//! locks, but instead relies on Arc.

use std::{fs::File, ops::Deref, path::Path, sync::Arc};

use fs2::{lock_contended_error, FileExt};
use lazy_static::lazy_static;

use crate::lockable_file::{LockStatus, HELD_LOCKS};

// -----------------------------------------------------------------------------

pub struct OwningSharedFileLock<F: FileExt> {
    debug: Option<Arc<Path>>,
    // XX joke: need DerefMut anyway, even reading requires mut
    // access. So the two locks are identical now. TODO: eliminate or
    // ? Parameterize instead?
    file: Arc<F>,
}

impl<F: FileExt> Drop for OwningSharedFileLock<F> {
    fn drop(&mut self) {
        self.file
            .unlock()
            .expect("no way another path to unlock exists");
        if let Some(path) = &self.debug {
            eprintln!("dropped SharedFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| {
                set.remove(&**path);
            });
        }
    }
}

impl<F: FileExt> Deref for OwningSharedFileLock<F> {
    type Target = Arc<F>;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

// -----------------------------------------------------------------------------

#[derive(Debug)]
pub struct OwningExclusiveFileLock<F: FileExt> {
    debug: Option<Arc<Path>>,
    file: Arc<F>,
}

impl<F: FileExt> PartialEq for OwningExclusiveFileLock<F> {
    fn eq(&self, other: &Self) -> bool {
        self.debug == other.debug
    }
}

impl<F: FileExt> Drop for OwningExclusiveFileLock<F> {
    fn drop(&mut self) {
        self.file
            .unlock()
            .expect("no way another path to unlock exists");
        if let Some(path) = &self.debug {
            eprintln!("dropped ExclusiveFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| {
                set.remove(&**path);
            });
        }
    }
}

impl<F: FileExt> Deref for OwningExclusiveFileLock<F> {
    type Target = Arc<F>;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

// -----------------------------------------------------------------------------

#[derive(Debug)]
pub struct OwningLockableFile<F: FileExt> {
    /// Path for, and only if, debugging is set via
    /// `DEBUG_LOCKABLE_FILE` env var
    debug: Option<Arc<Path>>,
    file: Arc<F>,
}

impl<F: FileExt> OwningLockableFile<F> {
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

    pub fn lock_shared(&self) -> std::io::Result<OwningSharedFileLock<F>> {
        if let Some(path) = self.debug.as_ref() {
            eprintln!("getting SharedFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| -> std::io::Result<()> {
                // XXX: ah, todo: allow multiple shared
                if set.contains(&**path) {
                    panic!("{path:?} is already locked by this thread")
                }
                FileExt::lock_shared(&*self.file)?;
                eprintln!("got SharedFileLock on {path:?}");
                set.insert((&**path).to_owned());
                Ok(())
            })?;
        } else {
            FileExt::lock_shared(&*self.file)?;
        }
        Ok(OwningSharedFileLock {
            debug: self.debug.as_ref().map(Arc::clone),
            file: self.file.clone(),
        })
    }

    pub fn lock_exclusive(&self) -> std::io::Result<OwningExclusiveFileLock<F>> {
        if let Some(path) = self.debug.as_ref() {
            eprintln!("getting ExclusiveFileLock on {path:?}");
            HELD_LOCKS.with_borrow_mut(|set| -> std::io::Result<()> {
                if set.contains(&**path) {
                    panic!("{path:?} is already locked by this thread")
                }
                FileExt::lock_exclusive(&*self.file)?;
                eprintln!("got ExclusiveFileLock on {path:?}");
                set.insert((&**path).to_owned());
                Ok(())
            })?;
        } else {
            FileExt::lock_exclusive(&*self.file)?;
        }
        Ok(OwningExclusiveFileLock {
            debug: self.debug.as_ref().map(Arc::clone),
            file: self.file.clone(),
        })
    }

    pub fn try_lock_shared(&self) -> std::io::Result<Option<OwningSharedFileLock<F>>> {
        match FileExt::try_lock_shared(&*self.file) {
            Ok(()) => {
                if let Some(path) = self.debug.as_ref() {
                    eprintln!("got SharedFileLock on {path:?}");
                    HELD_LOCKS.with_borrow_mut(|set| {
                        set.insert((&**path).to_owned());
                    });
                }
                Ok(Some(OwningSharedFileLock {
                    debug: self.debug.as_ref().map(Arc::clone),
                    file: self.file.clone(),
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

    pub fn try_lock_exclusive(&self) -> std::io::Result<Option<OwningExclusiveFileLock<F>>> {
        match FileExt::try_lock_exclusive(&*self.file) {
            Ok(()) => {
                if let Some(path) = self.debug.as_ref() {
                    eprintln!("got ExclusiveFileLock on {path:?}");
                    HELD_LOCKS.with_borrow_mut(|set| {
                        set.insert((&**path).to_owned());
                    });
                }
                Ok(Some(OwningExclusiveFileLock {
                    debug: self.debug.as_ref().map(Arc::clone),
                    file: self.file.clone(),
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

impl OwningLockableFile<File> {
    pub fn open(path: Arc<Path>) -> std::io::Result<Self> {
        File::open(path.as_ref()).and_then(|file| {
            let path = if *DEBUGGING { Some(path) } else { None };

            Ok(OwningLockableFile {
                debug: path,
                file: Arc::new(file),
            })
        })
    }
}
