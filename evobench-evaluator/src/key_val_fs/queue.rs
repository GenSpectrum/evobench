use std::{
    borrow::Cow,
    fmt::Display,
    fs::File,
    path::{Path, PathBuf},
    sync::atomic::AtomicU64,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use genawaiter::rc::Gen;
use ouroboros::self_referencing;
use serde::{de::DeserializeOwned, Serialize};

use crate::lockable_file::{ExclusiveFileLock, LockableFile, SharedFileLock};

use super::{
    as_key::AsKey,
    key_val::{Entry, KeyVal, KeyValConfig, KeyValError},
};

#[macro_export]
macro_rules! info_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprintln!($($arg)*);
        }
    }
}

#[macro_export]
macro_rules! info_noln_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprint!($($arg)*);
        }
    }
}

fn next_id() -> u64 {
    static IDS: AtomicU64 = AtomicU64::new(0);
    // Relaxed means each thread might get ids out of order with
    // reagards to other actions of the threads, but each still gets a
    // unique id, which is enough for us.
    IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeKey {
    /// Nanoseconds since UNIX_EPOCH
    nanos: u128,
    pid: u32,
    id: u64,
}

fn datetime_from_nanoseconds(nanos: u128) -> DateTime<Utc> {
    let secs = (nanos / 1_000_000_000) as u64;
    let nanos = (nanos % 1_000_000_000) as u32;
    let system_time = UNIX_EPOCH + Duration::new(secs, nanos);
    system_time.into()
}

impl Display for TimeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { nanos, pid, id } = self;
        let t = datetime_from_nanoseconds(*nanos);
        write!(f, "{t} ({pid}-{id})")
    }
}

impl TimeKey {
    /// Possibly panics if the system clock is outside the range
    /// representable as duration by `std::time`.
    pub fn now() -> Self {
        let time = SystemTime::now();
        let t = time
            .duration_since(UNIX_EPOCH)
            .expect("now is never out of range");
        let nanos: u128 = t.as_nanos();
        let pid = std::process::id();
        let id = next_id();
        Self { nanos, pid, id }
    }
}

impl AsKey for TimeKey {
    fn as_filename_str(&self) -> Cow<str> {
        let Self { nanos, pid, id } = self;
        format!("{nanos}-{pid}-{id}").into()
    }

    fn try_from_filename_str(file_name: &str) -> Option<Self> {
        let (nanos, pid_id) = file_name.split_once('-')?;
        let (pid, id) = pid_id.split_once('-')?;
        let nanos: u128 = nanos.parse().ok()?;
        let pid: u32 = pid.parse().ok()?;
        let id: u64 = id.parse().ok()?;
        Some(Self { nanos, pid, id })
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct QueueIterationOpts {
    /// Used for debugging in some places
    pub verbose: bool,
    /// Wait for entries if the queue is empty (i.e. go on forever)
    pub wait: bool,
    /// Stop at this time if given. Unblocks "wait" (waiting for new
    /// messages), but not currently blocking on locks of entries!
    pub stop_at: Option<SystemTime>,
    /// Do not attempt to lock entries (default: false)
    pub no_lock: bool,
    /// Instead of blocking to get a lock on an entry, return with an
    /// error if an entry is locked.
    pub error_when_locked: bool,
    /// Delete entry before returning the item (alternatively, call
    /// `delete`).
    pub delete_first: bool,
}

enum PerhapsLock<T> {
    /// User asked not to lock (`no_lock`)
    NoLock,
    /// Entry vanished thus let go lock again
    EntryGone,
    /// Lock
    Lock(T),
}

impl<T> PerhapsLock<T> {
    fn is_gone(&self) -> bool {
        match self {
            PerhapsLock::NoLock => false,
            PerhapsLock::EntryGone => true,
            PerhapsLock::Lock(_) => false,
        }
    }
}

#[self_referencing]
pub struct QueueItem<'basedir, V: DeserializeOwned + Serialize + 'static> {
    verbose: bool,
    lockable: LockableFile<File>,
    entry: Entry<'basedir, TimeKey, V>,
    #[borrows(mut entry, mut lockable)]
    #[covariant]
    // The lock unless `no_lock` was given.
    perhaps_lock: (
        &'this mut Entry<'basedir, TimeKey, V>,
        PerhapsLock<ExclusiveFileLock<'this, File>>,
    ),
}

impl<'basedir, V: DeserializeOwned + Serialize> QueueItem<'basedir, V> {
    /// Delete this item now (alternatively, give `delete_first` in
    /// `QueueIterationOpts`)
    pub fn delete(&mut self) -> Result<(), KeyValError> {
        let deleted = self.with_perhaps_lock_mut(|(entry, _lock)| entry.delete())?;
        info_if!(*self.borrow_verbose(), "deleted entry: {:?}", deleted);
        Ok(())
    }
}

#[derive(Debug)]
pub struct Queue<V: DeserializeOwned + Serialize>(KeyVal<TimeKey, V>);

fn keyvalerror_from_lock_error<V>(
    res: Result<V, std::io::Error>,
    base_dir: &PathBuf,
    path: &Path,
) -> Result<V, KeyValError> {
    res.map_err(|error| KeyValError::IO {
        ctx: "getting lock on file",
        base_dir: base_dir.clone(),
        path: path.to_owned(),
        error,
    })
}

impl<V: DeserializeOwned + Serialize + 'static> Queue<V> {
    pub fn open(base_dir: impl AsRef<Path>, config: KeyValConfig) -> Result<Self, KeyValError> {
        Ok(Queue(KeyVal::open(base_dir, config)?))
    }

    pub fn lock_exclusive(&mut self) -> Result<ExclusiveFileLock<File>, KeyValError> {
        self.0.lock_exclusive()
    }
    pub fn lock_shared(&mut self) -> Result<SharedFileLock<File>, KeyValError> {
        self.0.lock_shared()
    }

    pub fn push_front(&self, val: &V) -> Result<(), KeyValError> {
        let key = TimeKey::now();
        self.0.insert(&key, val, true)
    }

    /// Get all entries in order of insertion according to hires
    /// system time (assumes correct clocks!). The entries are
    /// collected at the time of this method call; entries
    /// disappearing later are skipped, but no entries inserted after
    /// this method call are returned from the iterator. Because this
    /// has O(n) cost with the number of entries, and there's no more
    /// efficient possibility for a `pop_back`, this should be used
    /// and amortized by handling all entries if possible. If that's
    /// not possible, just taking the first entry is still currently
    /// the best the underlying storage can do.
    pub fn sorted_entries<'s>(
        &'s self,
        wait_for_entries: bool,
        stop_at: Option<SystemTime>,
    ) -> impl Iterator<Item = Result<Entry<'s, TimeKey, V>, KeyValError>> + use<'s, V> {
        Gen::new(|co| async move {
            match self.0.sorted_keys(wait_for_entries, stop_at) {
                Ok(keys) => {
                    for key in keys {
                        if let Some(res) = self.0.entry_opt(&key).transpose() {
                            co.yield_(res).await;
                        }
                    }
                }
                Err(error) => {
                    co.yield_(Err(error)).await;
                }
            }
        })
        .into_iter()
    }

    /// Like `sorted_entries`, but (1) allows to lock entries and in
    /// this case skips over entries that have been deleted by the
    /// time we have the lock, (2) allows to go on forever, (3) always
    /// retrieves the values, and offers an easy method to delete the
    /// entry as well as delete it automatically immediately.
    pub fn items<'s>(
        &'s self,
        opts: QueueIterationOpts,
    ) -> impl Iterator<Item = Result<(QueueItem<'s, V>, V), KeyValError>> + use<'s, V> {
        let base_dir = self.0.base_dir.clone();
        Gen::new(|co| async move {
            let QueueIterationOpts {
                verbose,
                wait,
                stop_at,
                no_lock,
                error_when_locked,
                delete_first,
            } = opts;

            let mut entries = None;
            // Whether we have tried to get an entry from entries (for "EOF" checking)
            let mut got_entry = false;
            loop {
                if entries.is_none() {
                    entries = Some(self.sorted_entries(wait, stop_at));
                    got_entry = false;
                }
                if let Some(entry) = entries.as_mut().expect("set 2 lines above").next() {
                    got_entry = true;
                    match entry {
                        Ok(mut entry) => {
                            let value = entry
                                .get()
                                .expect("version of serialized ds has not changed");
                            let lockable = entry
                                .take_lockable_file()
                                .expect("we have not taken it yet");

                            match QueueItem::try_new(
                                verbose,
                                lockable,
                                entry,
                                |entry, lockable: &mut LockableFile<File>| -> Result<_, _> {
                                    if no_lock {
                                        if delete_first {
                                            entry.delete()?;
                                        }
                                        Ok((entry, PerhapsLock::NoLock))
                                    } else {
                                        let lock = if error_when_locked {
                                            keyvalerror_from_lock_error(
                                                lockable.try_lock_exclusive(),
                                                &base_dir,
                                                entry.target_path(),
                                            )?
                                            .ok_or_else(|| KeyValError::LockTaken {
                                                base_dir: base_dir.clone(),
                                                path: entry.target_path().to_owned(),
                                            })?
                                        } else {
                                            keyvalerror_from_lock_error(
                                                lockable.lock_exclusive(),
                                                &base_dir,
                                                entry.target_path(),
                                            )?
                                        };
                                        info_if!(verbose, "got lock");
                                        let exists = entry.exists();
                                        if !exists {
                                            info_if!(
                                                verbose,
                                                "but entry now deleted by another process"
                                            );
                                            return Ok((entry, PerhapsLock::EntryGone));
                                        }
                                        if delete_first {
                                            entry.delete()?;
                                        }
                                        Ok((entry, PerhapsLock::Lock(lock)))
                                    }
                                },
                            ) {
                                Ok(item) => {
                                    if !item.borrow_perhaps_lock().1.is_gone() {
                                        co.yield_(Ok((item, value))).await;
                                    }
                                }
                                Err(e) => co.yield_(Err(e)).await,
                            }
                        }
                        Err(e) => co.yield_(Err(e)).await,
                    }
                } else {
                    entries = None;
                    if !(got_entry || wait) {
                        break;
                    }
                }
            }
        })
        .into_iter()
    }
}
