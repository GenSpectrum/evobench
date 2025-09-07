use std::{
    ffi::{OsStr, OsString},
    fmt::Debug,
    fs::{create_dir, File},
    io::{Read, Write},
    marker::PhantomData,
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, SystemTime},
};

use serde::{de::DeserializeOwned, Serialize};

use crate::{
    lockable_file::{ExclusiveFileLock, LockableFile, SharedFileLock},
    path_util::AppendToPath,
};

use super::as_key::AsKey;

/// Returns `None` if `file_name` is for a tmp file
fn key_from_file_name<K: AsKey>(
    file_name: &OsStr,
    base_dir: &PathBuf,
) -> Result<Option<K>, KeyValError> {
    let file_name = file_name
        .to_str()
        .ok_or_else(|| KeyValError::InvalidFileNameInStorage {
            base_dir: base_dir.clone(),
            ctx: "does not decode as string",
            invalid_file_name: file_name.to_owned(),
        })?;
    if file_name.starts_with('.') {
        // tmp file
        Ok(None)
    } else {
        if let Some(key) = K::try_from_filename_str(&file_name) {
            Ok(Some(key))
        } else {
            Err(KeyValError::InvalidFileNameInStorage {
                base_dir: base_dir.clone(),
                ctx: "can't be parsed back into key",
                invalid_file_name: file_name.into(),
            })
        }
    }
}

// Locking the dir (whole `KeyVal` data structure)
macro_rules! define_lock_helper {
    { $name:ident, $lock_type:tt, $method:tt } =>  {
        fn $name<'l>(
            lock_file: &'l LockableFile<File>,
            base_dir: &PathBuf,
        ) -> Result<$lock_type<'l, File>, KeyValError> {
            lock_file.$method().map_err(|error| KeyValError::IO {
                ctx: "getting lock on dir",
                base_dir: base_dir.clone(),
                path: base_dir.clone(),
                error,
            })
        }
    }
}
define_lock_helper! {lock_exclusive, ExclusiveFileLock, lock_exclusive}
define_lock_helper! {lock_shared, SharedFileLock, lock_shared}

/// Accessor for an entry, embeds an open file handle, allows to load
/// or lock-and-delete etc.
#[derive(Debug)]
pub struct Entry<'p, K: AsKey, V: DeserializeOwned + Serialize> {
    key_type: PhantomData<fn() -> K>,
    val_type: PhantomData<fn() -> V>,
    base_dir: &'p PathBuf,
    target_path: PathBuf,
    // Becoming None when calling `take_lockable_file`
    value_file: Option<File>,
}

impl<'p, K: AsKey, V: DeserializeOwned + Serialize> PartialEq for Entry<'p, K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.base_dir == other.base_dir && self.target_path == other.target_path
    }
}

impl<'p, K: AsKey, V: DeserializeOwned + Serialize> Entry<'p, K, V> {
    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    pub fn file_name(&self) -> &OsStr {
        self.target_path
            .file_name()
            .expect("entries always have a file name")
    }

    /// Can return file name decoding errors for files that were
    /// inserted into the directory via other means than this library.
    pub fn key(&self) -> Result<K, KeyValError> {
        let maybe_key = key_from_file_name(self.file_name(), &self.base_dir)?;
        Ok(maybe_key.expect("an `Entry` is never created from a tmp file"))
    }

    pub fn get(&mut self) -> Result<V, KeyValError> {
        let mut s = String::new();
        if let Some(file) = &mut self.value_file {
            file.read_to_string(&mut s)
                .map_err(|error| KeyValError::IO {
                    ctx: "reading/UTF-decoding value file",
                    base_dir: self.base_dir.clone(),
                    path: self.target_path.clone(),
                    error,
                })?;
            let val: V =
                serde_json::from_str(&s).map_err(|error| KeyValError::Deserialization {
                    base_dir: self.base_dir.clone(),
                    path: self.target_path.clone(),
                    error,
                })?;
            Ok(val)
        } else {
            Err(KeyValError::FileTaken {
                base_dir: self.base_dir.clone(),
                path: self.target_path.clone(),
            })
        }
    }

    /// Giving up the Entry accessor, means you have to call `get`
    /// first. (Yes, locking wouldn't protect that anyway! This lock
    /// is not for that!) Returns None if already taken.
    pub fn take_lockable_file(&mut self) -> Option<LockableFile<File>> {
        Some(LockableFile::from(self.value_file.take()?))
    }

    /// Whether the entry currently (still) exists -- XX But there is
    /// no guarantee it is still the old one!
    pub fn exists(&mut self) -> bool {
        self.target_path.exists()
    }

    /// Returns whether a file deletion actually happened (concurrent
    /// deletes might already have removed it). XX Same caveat as with
    /// `exists`.
    pub fn delete(&self) -> Result<bool, KeyValError> {
        match std::fs::remove_file(&self.target_path) {
            Ok(()) => Ok(true),
            Err(error) => match error.kind() {
                std::io::ErrorKind::NotFound => Ok(false),
                _ => Err(KeyValError::IO {
                    base_dir: self.base_dir.clone(),
                    path: self.target_path.clone(),
                    ctx: "deleting the value file",
                    error,
                }),
            },
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum KeyValError {
    #[error("creating directory {base_dir:?}: {error}")]
    CreateDir {
        base_dir: PathBuf,
        error: std::io::Error,
    },
    #[error("key_val_fs db at {base_dir:?}: {ctx} {path:?}: {error}")]
    IO {
        base_dir: PathBuf,
        path: PathBuf,
        ctx: &'static str,
        error: std::io::Error,
    },
    #[error("serializing value to JSON")]
    Serialization {
        #[from]
        error: serde_json::Error,
    },
    #[error("deserializing value from JSON at {base_dir:?}: file {path:?}: {error}")]
    Deserialization {
        base_dir: PathBuf,
        path: PathBuf,
        error: serde_json::Error,
    },
    #[error("mapping already exists for key {key_debug_string} in {base_dir:?}")]
    KeyExists {
        base_dir: PathBuf,
        key_debug_string: String,
    },
    #[error("invalid file name in directory in {base_dir:?}: {ctx} {invalid_file_name:?}")]
    InvalidFileNameInStorage {
        base_dir: PathBuf,
        ctx: &'static str,
        invalid_file_name: OsString,
    },
    #[error("usage error: file handle already taken out (in {base_dir:?}: {path:?})")]
    FileTaken { base_dir: PathBuf, path: PathBuf },
    #[error("lock is already taken in {base_dir:?} for {path:?}")]
    LockTaken { base_dir: PathBuf, path: PathBuf },
    #[error("bug: lock has already been taken, `no_lock` was false")]
    AlreadyLocked { base_dir: PathBuf, path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyValSync {
    /// Do not call sync; fastest, but may end up with corrupt files
    /// in the database
    No,
    /// Call fsync on files before moving them in place inside the
    /// database. This should prevent the possibility for corrupt
    /// files, but does not guarantee that entries are persisted after
    /// returning from modifying functions.
    Files,
    /// Call fsync on files and then also on the containing dir. This
    /// should guarantee that changes are persisted by the time
    /// functions return.
    All,
}

impl KeyValSync {
    fn do_sync_files(self) -> bool {
        match self {
            KeyValSync::No => false,
            KeyValSync::Files => true,
            KeyValSync::All => true,
        }
    }
    fn do_sync_dirs(self) -> bool {
        match self {
            KeyValSync::No => false,
            KeyValSync::Files => false,
            KeyValSync::All => true,
        }
    }

    fn flush_and_perhaps_sync_file(
        self,
        file: &mut File,
        flush_ctx: &'static str,
        sync_ctx: &'static str,
        base_dir: &Path,
        path: &Path,
    ) -> Result<(), KeyValError> {
        file.flush().map_err(|error| KeyValError::IO {
            ctx: flush_ctx,
            base_dir: base_dir.to_owned(),
            path: path.to_owned(),
            error,
        })?;
        if self.do_sync_files() {
            file.sync_all().map_err(|error| KeyValError::IO {
                ctx: sync_ctx,
                base_dir: base_dir.to_owned(),
                path: path.to_owned(),
                error,
            })?;
        }
        Ok(())
    }

    fn perhaps_sync_dir(self, dir_file: &File, base_dir: &Path) -> Result<(), KeyValError> {
        if self.do_sync_dirs() {
            dir_file.sync_all().map_err(|error| KeyValError::IO {
                ctx: "sync of the directory",
                base_dir: base_dir.to_owned(),
                path: base_dir.to_owned(),
                error,
            })?;
        }
        Ok(())
    }
}

/// Configuration for `KeyVal`
#[derive(Debug, Clone, PartialEq)]
pub struct KeyValConfig {
    /// Whether to call fsync on files and the containing directory
    /// when doing changes (default: All).
    pub sync: KeyValSync,
    /// Whether to create the directory holding the data, if it
    /// doesn't exist already (default: true)
    pub create_dir_if_not_exists: bool,
}

impl Default for KeyValConfig {
    fn default() -> Self {
        KeyValConfig {
            sync: KeyValSync::All,
            create_dir_if_not_exists: true,
        }
    }
}

#[derive(Debug)]
pub struct KeyVal<K: AsKey, V: DeserializeOwned + Serialize> {
    keys: PhantomData<fn() -> K>,
    vals: PhantomData<fn() -> V>,
    pub config: KeyValConfig,
    pub base_dir: PathBuf,
    // Filehandle to the directory, for flock (was a .lock file, but
    // dir itself works, too, on Linux anyway)
    lock_file: LockableFile<File>,
    // Filehandle to the directory again, for sync, since the one
    // above is wrapped
    dir_file: File,
}

// Just for testing
impl<K: AsKey, V: DeserializeOwned + Serialize> PartialEq for KeyVal<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.config.eq(&other.config) && self.base_dir.eq(&other.base_dir)
    }
}

impl<K: AsKey, V: DeserializeOwned + Serialize> KeyVal<K, V> {
    pub fn open(base_dir: impl AsRef<Path>, config: KeyValConfig) -> Result<Self, KeyValError> {
        let base_dir = base_dir.as_ref().to_owned();
        let dir_needs_sync;
        if config.create_dir_if_not_exists {
            if let Err(error) = create_dir(&base_dir) {
                match error.kind() {
                    std::io::ErrorKind::AlreadyExists => dir_needs_sync = false,
                    _ => return Err(KeyValError::CreateDir { base_dir, error }),
                }
            } else {
                dir_needs_sync = true;
            }
        } else {
            dir_needs_sync = false;
        }
        let open_base_dir = || {
            File::open(&base_dir).map_err(|error| KeyValError::IO {
                ctx: "opening directory as file",
                base_dir: base_dir.to_owned(),
                path: base_dir.to_owned(),
                error,
            })
        };
        let mut dir_file = open_base_dir()?;
        /// Can we use try_clone and then call both fsync and flock on
        /// all supported architectures? It's fine on Linux. Try for
        /// now.
        const CAN_USE_DIR_FILE_CLONE: bool = true;
        let lock_file = if CAN_USE_DIR_FILE_CLONE {
            dir_file.try_clone().map_err(|error| KeyValError::IO {
                ctx: "cloning directory filehandle",
                base_dir: base_dir.to_owned(),
                path: base_dir.to_owned(),
                error,
            })
        } else {
            open_base_dir()
        }?
        .into();

        if dir_needs_sync {
            config.sync.perhaps_sync_dir(&mut dir_file, &base_dir)?;
        }
        Ok(Self {
            keys: PhantomData,
            vals: PhantomData,
            config,
            base_dir,
            lock_file,
            dir_file,
        })
    }

    pub fn lock_exclusive(&self) -> Result<ExclusiveFileLock<'_, File>, KeyValError> {
        lock_exclusive(&self.lock_file, &self.base_dir)
    }

    pub fn lock_shared(&self) -> Result<SharedFileLock<'_, File>, KeyValError> {
        lock_shared(&self.lock_file, &self.base_dir)
    }

    /// Insert a mapping; if `exclusive` is true, give an error if
    /// `key` is already in the map (otherwise the previous value is
    /// silently overwritten).
    pub fn insert(&self, key: &K, val: &V, exclusive: bool) -> Result<(), KeyValError>
    where
        K: Debug,
    {
        let valstr = serde_json::to_string(val)?;
        let key_filename = key.verified_as_filename_str();
        let tmp_path = {
            let tmp_filename = format!(".{}", key_filename);
            (&self.base_dir).append(tmp_filename)
        };
        let target_path = (&self.base_dir).append(key_filename.as_ref());

        // The lock is required since we only use 1 tmp file path for
        // all processes! Also, for a race-free existence check.
        let _lock = lock_exclusive(&self.lock_file, &self.base_dir)?;

        if exclusive && target_path.exists() {
            return Err(KeyValError::KeyExists {
                base_dir: self.base_dir.clone(),
                key_debug_string: format!("{key:?}"),
            });
        }
        let mut out = File::create(&tmp_path).map_err(|error| KeyValError::IO {
            ctx: "creating file",
            base_dir: self.base_dir.clone(),
            path: tmp_path.clone(),
            error,
        })?;
        out.write_all(valstr.as_bytes())
            .map_err(|error| KeyValError::IO {
                ctx: "writing to file",
                base_dir: self.base_dir.clone(),
                path: tmp_path.clone(),
                error,
            })?;
        self.config.sync.flush_and_perhaps_sync_file(
            &mut out,
            "flush of the file",
            "sync of the file",
            &self.base_dir,
            &tmp_path,
        )?;
        drop(out);
        std::fs::rename(&tmp_path, &target_path).map_err(|error| KeyValError::IO {
            ctx: "renaming to file",
            base_dir: self.base_dir.clone(),
            path: target_path.clone(),
            error,
        })?;
        self.config
            .sync
            .perhaps_sync_dir(&self.dir_file, &self.base_dir)?;
        Ok(())
    }

    /// Returns whether an entry was actually deleted (false means
    /// that no entry for `key` existed)
    pub fn delete(&self, key: &K) -> Result<bool, KeyValError> {
        let key_filename = key.verified_as_filename_str();
        let target_path = (&self.base_dir).append(key_filename.as_ref());
        match std::fs::remove_file(&target_path) {
            Ok(()) => Ok(true),
            Err(error) => match error.kind() {
                std::io::ErrorKind::NotFound => Ok(false),
                _ => Err(KeyValError::IO {
                    base_dir: self.base_dir.clone(),
                    path: target_path,
                    ctx: "delete",
                    error,
                }),
            },
        }
    }

    /// Get access to an entry, if it exists. Note: does not lock,
    /// and on Windows might possibly deadlock with the rename calls
    /// of `insert`?  Only tested on Linux (and macOS?)
    pub fn entry_opt(&self, key: &K) -> Result<Option<Entry<'_, K, V>>, KeyValError> {
        let key_filename = key.verified_as_filename_str();
        let target_path = (&self.base_dir).append(key_filename.as_ref());
        match File::open(&target_path) {
            Ok(value_file) => Ok(Some(Entry {
                key_type: PhantomData,
                val_type: PhantomData,
                base_dir: &self.base_dir,
                target_path,
                value_file: Some(value_file),
            })),
            Err(error) => match error.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                _ => Err(KeyValError::IO {
                    ctx: "opening value file",
                    base_dir: self.base_dir.clone(),
                    path: target_path,
                    error,
                }),
            },
        }
    }

    /// Note: does not lock, and on Windows might possibly deadlock
    /// with the rename calls of `insert`! Only tested on Linux (and
    /// macOS?)
    pub fn get(&self, key: &K) -> Result<Option<V>, KeyValError> {
        if let Some(mut entry) = self.entry_opt(key)? {
            Some(entry.get()).transpose()
        } else {
            Ok(None)
        }
    }

    /// Stops waiting at `stop_at` if given. Returns true if it found
    /// an entry, false if it timed out.
    pub fn wait_for_entries(&self, stop_at: Option<SystemTime>) -> Result<bool, KeyValError> {
        // XX hack, use file notifications instead
        let mut sleep_time = 1000;
        loop {
            let mut dir = std::fs::read_dir(&self.base_dir).map_err(|error| KeyValError::IO {
                ctx: "opening directory",
                base_dir: self.base_dir.clone(),
                path: self.base_dir.clone(),
                error,
            })?;
            if dir.next().is_some() {
                return Ok(true);
            }

            if let Some(stop_at) = stop_at {
                let now = SystemTime::now();
                if now >= stop_at {
                    return Ok(false);
                }
            }

            // dbg!(sleep_time);
            sleep(Duration::from_nanos(sleep_time));
            if sleep_time < 2_000_000_000 {
                sleep_time = (sleep_time * 101) / 100;
            }
        }
    }

    /// Get all the keys contained in the map. Their order is not
    /// defined. Note that the returned entries may not exist any more
    /// by the time your code looks at them, since exclusivity can't
    /// be statically ensured, and taking a lock for the iterator's
    /// life time seems excessive. If `wait_for_entries` is true,
    /// blocks until entries exist (but note that with concurrent
    /// deletions, by the time the entries are read, they may be gone
    /// again--the returned sequence might still be empty).
    pub fn keys<'s>(
        &'s self,
        wait_for_entries: bool,
        stop_at: Option<SystemTime>,
    ) -> Result<impl Iterator<Item = Result<K, KeyValError>> + use<'s, K, V>, KeyValError> {
        if wait_for_entries {
            self.wait_for_entries(stop_at)?;
        }

        let dir = std::fs::read_dir(&self.base_dir).map_err(|error| KeyValError::IO {
            ctx: "opening directory",
            base_dir: self.base_dir.clone(),
            path: self.base_dir.clone(),
            error,
        })?;
        Ok(dir
            .map(|entry| -> Result<Option<K>, KeyValError> {
                let entry = entry.map_err(|error| KeyValError::IO {
                    ctx: "reading directory entry",
                    base_dir: self.base_dir.clone(),
                    path: self.base_dir.clone(),
                    error,
                })?;
                key_from_file_name(&entry.file_name(), &self.base_dir)
            })
            .filter_map(|val| val.transpose()))
    }

    /// Sorted output of `keys()`.
    pub fn sorted_keys(
        &self,
        wait_for_entries: bool,
        stop_at: Option<SystemTime>,
        reverse: bool,
    ) -> Result<Vec<K>, KeyValError>
    where
        K: Ord,
    {
        let mut keys: Vec<_> = self
            .keys(wait_for_entries, stop_at)?
            .collect::<Result<_, _>>()?;
        // No way to sort in reverse from the get go?
        keys.sort();
        if reverse {
            keys.reverse();
        }
        Ok(keys)
    }
}
