use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    fs::File,
    path::{Path, PathBuf},
    sync::atomic::AtomicU64,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Local};
use genawaiter::rc::Gen;
use ouroboros::self_referencing;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    info_if,
    lockable_file::{ExclusiveFileLock, LockableFile, SharedFileLock},
    utillib::slice_or_box::SliceOrBox,
};

use super::{
    as_key::AsKey,
    key_val::{Entry, KeyVal, KeyValConfig, KeyValError},
};

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

impl Display for TimeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { nanos: _, pid, id } = self;

        let t = self.datetime();
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

    pub fn unixtime_nanoseconds(&self) -> u128 {
        self.nanos
    }

    pub fn unixtime_seconds_and_nanoseconds(&self) -> (u64, u32) {
        let nanos = self.nanos;
        let secs = (nanos / 1_000_000_000) as u64;
        let nanos = (nanos % 1_000_000_000) as u32;
        (secs, nanos)
    }

    pub fn system_time(&self) -> SystemTime {
        let (secs, nanos) = self.unixtime_seconds_and_nanoseconds();
        UNIX_EPOCH + Duration::new(secs, nanos)
    }

    pub fn datetime(&self) -> DateTime<Local> {
        self.system_time().into()
    }
}

impl AsKey for TimeKey {
    fn as_filename_str(&self) -> Cow<'_, str> {
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

#[derive(Debug, PartialEq, Clone, Copy)]
pub struct QueueGetItemOpts {
    /// Used for debugging in some places
    pub verbose: bool,
    /// Do not attempt to lock entries (default: false)
    pub no_lock: bool,
    /// Instead of blocking to get a lock on an entry, return with an
    /// error if an entry is locked.
    pub error_when_locked: bool,
    /// Delete entry before returning the item (alternatively, call
    /// `delete`).
    pub delete_first: bool,
}

#[derive(Debug, PartialEq, Clone)]
pub struct QueueIterationOpts {
    /// Wait for entries if the queue is empty (i.e. go on forever)
    pub wait: bool,
    /// Stop at this time if given. Unblocks "wait" (waiting for new
    /// messages), but not currently blocking on locks of entries!
    pub stop_at: Option<SystemTime>,
    /// Sort in reverse
    pub reverse: bool,

    pub get_item_opts: QueueGetItemOpts,
}

#[derive(Debug)]
enum PerhapsLock<'l, T> {
    /// User asked not to lock (`no_lock`)
    NoLock(&'l mut LockableFile<File>),
    /// Entry vanished thus let go lock again
    EntryGone,
    /// Lock
    Lock(T),
}

impl<'l, T> PartialEq for PerhapsLock<'l, T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NoLock(_), Self::NoLock(_)) => true,
            (Self::EntryGone, Self::EntryGone) => true,
            (Self::Lock(_), Self::Lock(_)) => true,
            _ => false,
        }
    }
}

impl<'l, T> PerhapsLock<'l, T> {
    fn is_gone(&self) -> bool {
        match self {
            PerhapsLock::NoLock(_) => false,
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
        PerhapsLock<'this, ExclusiveFileLock<'this, File>>,
    ),
}

impl<'basedir, V: DeserializeOwned + Serialize + 'static + Debug> PartialEq
    for QueueItem<'basedir, V>
{
    fn eq(&self, other: &Self) -> bool {
        self.borrow_verbose() == other.borrow_verbose()
            && self.borrow_perhaps_lock() == other.borrow_perhaps_lock()
    }
}

impl<'basedir, V: DeserializeOwned + Serialize + 'static + Debug> Debug for QueueItem<'basedir, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let perhaps_lock = self.with_perhaps_lock(|v| v);
        write!(
            f,
            "QueueItem {{ verbose: {}, perhaps_lock: {perhaps_lock:?} }}",
            self.borrow_verbose()
        )
    }
}

impl<'basedir, V: DeserializeOwned + Serialize> QueueItem<'basedir, V> {
    pub fn from_entry<'s>(
        mut entry: Entry<'s, TimeKey, V>,
        base_dir: &PathBuf,
        opts: QueueGetItemOpts,
    ) -> Result<QueueItem<'s, V>, KeyValError> {
        let QueueGetItemOpts {
            no_lock,
            error_when_locked,
            delete_first,
            verbose,
        } = opts;
        let lockable = entry
            .take_lockable_file()
            .expect("we have not taken it yet");

        QueueItem::try_new(
            verbose,
            lockable,
            entry,
            |entry, lockable: &mut LockableFile<File>| -> Result<_, _> {
                if no_lock {
                    if delete_first {
                        entry.delete()?;
                    }
                    Ok((entry, PerhapsLock::NoLock(lockable)))
                } else {
                    let lock = if error_when_locked {
                        keyvalerror_from_lock_error(
                            lockable.try_lock_exclusive(),
                            base_dir,
                            entry.target_path(),
                        )?
                        .ok_or_else(|| KeyValError::LockTaken {
                            base_dir: base_dir.clone(),
                            path: entry.target_path().to_owned(),
                        })?
                    } else {
                        keyvalerror_from_lock_error(
                            lockable.lock_exclusive(),
                            base_dir,
                            entry.target_path(),
                        )?
                    };
                    info_if!(verbose, "got lock");
                    let exists = entry.exists();
                    if !exists {
                        info_if!(verbose, "but entry now deleted by another process");
                        return Ok((entry, PerhapsLock::EntryGone));
                    }
                    if delete_first {
                        entry.delete()?;
                    }
                    Ok((entry, PerhapsLock::Lock(lock)))
                }
            },
        )
    }

    /// Get the key inside this queue, usable with `get_entry`
    /// (careful, there is no check against using it in the wrong
    /// queue!)
    pub fn key(&self) -> Result<TimeKey, KeyValError> {
        // (Does not take a lock, in spite of the name of this method
        // making it sound like.)
        self.with_perhaps_lock(|(entry, _lock)| entry.key())
    }

    /// Delete this item now (alternatively, give `delete_first` in
    /// `QueueIterationOpts`)
    pub fn delete(&self) -> Result<(), KeyValError> {
        let deleted = self.with_perhaps_lock(|(entry, _lock)| entry.delete())?;
        info_if!(*self.borrow_verbose(), "deleted entry: {:?}", deleted);
        Ok(())
    }

    /// Lock this item lazily (when `no_lock` == true given, but now a
    /// lock is needed). If `no_lock` was false, then this gives a
    /// `KeyValError::AlreadyLocked` error rather than dead-locking.
    pub fn lock_exclusive<'s>(&'s self) -> Result<ExclusiveFileLock<'s, File>, KeyValError> {
        let (entry, perhaps_lock) = self.borrow_perhaps_lock();
        match perhaps_lock {
            PerhapsLock::NoLock(lockable_file) => {
                return lockable_file.lock_exclusive().map_err(|error| {
                    let path = entry.target_path().to_owned();
                    let base_dir = path.parent().expect("always has a parent dir").to_owned();
                    KeyValError::IO {
                        base_dir,
                        path,
                        ctx: "QueueItem.lock_exclusive",
                        error,
                    }
                })
            }
            PerhapsLock::EntryGone => (),
            PerhapsLock::Lock(_) => (),
        }
        let path = entry.target_path().to_owned();
        let base_dir = path.parent().expect("always has a parent dir").to_owned();
        Err(KeyValError::AlreadyLocked { base_dir, path })
    }
}

#[derive(Debug, PartialEq)]
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

    pub fn base_dir(&self) -> &PathBuf {
        &self.0.base_dir
    }

    pub fn get_entry<'s>(
        &'s self,
        key: &TimeKey,
    ) -> Result<Option<Entry<'s, TimeKey, V>>, KeyValError> {
        self.0.entry_opt(key)
    }

    pub fn get_item<'s>(
        &'s self,
        key: &TimeKey,
        opts: QueueGetItemOpts,
    ) -> Result<Option<QueueItem<'s, V>>, KeyValError> {
        if let Some(entry) = self.0.entry_opt(key)? {
            Ok(Some(QueueItem::from_entry(entry, self.base_dir(), opts)?))
        } else {
            Ok(None)
        }
    }

    pub fn lock_exclusive(&self) -> Result<ExclusiveFileLock<'_, File>, KeyValError> {
        self.0.lock_exclusive()
    }
    pub fn lock_shared(&self) -> Result<SharedFileLock<'_, File>, KeyValError> {
        self.0.lock_shared()
    }

    pub fn push_front(&self, val: &V) -> Result<(), KeyValError> {
        let key = TimeKey::now();
        self.0.insert(&key, val, true)
    }

    pub fn resolve_entries<'s>(
        &'s self,
        keys: SliceOrBox<'s, TimeKey>,
    ) -> impl Iterator<Item = Result<Entry<'s, TimeKey, V>, KeyValError>> + use<'s, V> {
        keys.into_iter()
            .filter_map(|key| self.0.entry_opt(key.as_ref()).transpose())
    }

    pub fn sorted_keys(
        &self,
        wait_for_entries: bool,
        stop_at: Option<SystemTime>,
        reverse: bool,
    ) -> Result<Vec<TimeKey>, KeyValError> {
        self.0.sorted_keys(wait_for_entries, stop_at, reverse)
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
        reverse: bool,
    ) -> Result<
        impl Iterator<Item = Result<Entry<'s, TimeKey, V>, KeyValError>> + use<'s, V>,
        KeyValError,
    > {
        Ok(self.resolve_entries(self.sorted_keys(wait_for_entries, stop_at, reverse)?.into()))
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
                wait,
                stop_at,
                get_item_opts,
                reverse,
            } = opts;

            let mut entries = None;
            loop {
                if entries.is_none() {
                    match self.sorted_entries(wait, stop_at, reverse) {
                        Ok(v) => entries = Some(v),
                        Err(e) => {
                            co.yield_(Err(e)).await;
                            return;
                        }
                    }
                }
                if let Some(entry) = entries.as_mut().expect("set 2 lines above").next() {
                    match entry {
                        Ok(mut entry) => match entry.get() {
                            Ok(value) => {
                                match QueueItem::from_entry(entry, &base_dir, get_item_opts) {
                                    Ok(item) => {
                                        if !item.borrow_perhaps_lock().1.is_gone() {
                                            co.yield_(Ok((item, value))).await;
                                        }
                                    }
                                    Err(e) => {
                                        co.yield_(Err(e)).await;
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                co.yield_(Err(e)).await;
                                return;
                            }
                        },
                        Err(e) => {
                            co.yield_(Err(e)).await;
                            return;
                        }
                    }
                } else {
                    entries = None;
                    if !wait {
                        break;
                    }
                }
            }
        })
        .into_iter()
    }
}
