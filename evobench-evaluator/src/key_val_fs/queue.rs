use std::{
    borrow::Cow,
    fmt::Display,
    fs::File,
    path::Path,
    sync::atomic::AtomicU64,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use genawaiter::rc::Gen;
use serde::{de::DeserializeOwned, Serialize};

use crate::lockable_file::{ExclusiveFileLock, SharedFileLock};

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

pub struct Queue<V: DeserializeOwned + Serialize>(KeyVal<TimeKey, V>);

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

    pub fn push_front(&mut self, val: &V) -> Result<(), KeyValError> {
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
    ) -> impl Iterator<Item = Result<Entry<'s, TimeKey, V>, KeyValError>> + use<'s, V> {
        Gen::new(|co| async move {
            match self.0.sorted_keys(wait_for_entries) {
                Ok(keys) => {
                    for key in keys {
                        if let Some(res) = self.0.entry(&key).transpose() {
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
}
