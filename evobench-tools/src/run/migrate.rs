//! Database migration. Structs with incompatible changes should be
//! copied here and an upgrade function provided.

use std::{
    collections::btree_map,
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    path::PathBuf,
    sync::Arc,
    time::SystemTime,
};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    ctx,
    key::{BenchmarkingJobParameters, BenchmarkingJobParametersHash, RunParameters},
    key_val_fs::{
        as_key::AsKey,
        key_val::{KeyVal, KeyValError},
    },
    run::{
        benchmarking_job::{BenchmarkingJob, BenchmarkingJobPublic, BenchmarkingJobState},
        config::{BenchmarkingCommand, PreExecLevel2},
        run_queue::RunQueue,
    },
    serde::{priority::Priority, proper_dirname::ProperDirname},
    util::grep_diff::LogExtract,
    utillib::type_name_short::type_name_short,
    warn,
};

trait FromStrMigrating: Sized + DeserializeOwned + Serialize {
    /// Parse from whatever serialisation format that's appropriate
    /// for your type. Returns whether migration was needed as the
    /// second result.
    fn from_str_migrating(s: &str) -> Result<(Self, bool)>;
}

/// Returns how many items were migrated. `handle_conflict` receives
/// two values resulting from migration or pre-existing entry in the
/// table, that both yield the same key; its return value is stored
/// for the key. It can return an error to stop migration.
fn migrate_key_val<K: AsKey + Debug + Clone + PartialEq + Ord, T: FromStrMigrating>(
    table: &KeyVal<K, T>,
    gen_key: impl Fn(&K, &T) -> K,
    handle_conflict: impl Fn(&K, T, T) -> Result<T>,
) -> Result<usize> {
    let mut num_migrated = 0;
    // Take a lock on the whole table since we need to ensure files
    // still exist when we are ready to overwrite them (the
    // `_entry_lock` is not enough for this). Still lock the items,
    // too, OK? XX What about deadlocks? (Table users are not required
    // to take the dir lock first! Todo: at least document that
    // ordering is required!)
    let _table_lock = table.lock_exclusive()?;

    // Collect changes until after iteration has finished, to avoid
    // the "iterator invalidation" problem.
    let mut saves: BTreeMap<K, Vec<(bool, T)>> = BTreeMap::new();
    let mut deletions: BTreeSet<K> = BTreeSet::new();

    for old_key in table.keys(false, None)? {
        let old_key = old_key?;
        // `Entry` works for us as it does not transparently decode.
        if let Some(mut entry) = table.entry_opt(&old_key)? {
            let _entry_lock = entry
                .take_lockable_file()
                .expect("succeeds since calling it the first time");
            let path = entry.target_path();
            let s = std::fs::read_to_string(path).map_err(ctx!("reading file {path:?}"))?;
            let (value, needs_saving) = T::from_str_migrating(&s)?;
            let new_key = gen_key(&old_key, &value);
            let key_changed = new_key != old_key;
            if needs_saving || key_changed {
                if key_changed {
                    deletions.insert(old_key);
                }
                match saves.entry(new_key) {
                    btree_map::Entry::Vacant(vacant_entry) => {
                        vacant_entry.insert(vec![(key_changed, value)]);
                    }
                    btree_map::Entry::Occupied(mut occupied_entry) => {
                        occupied_entry.get_mut().push((key_changed, value));
                    }
                }
                num_migrated += 1;
            }
        }
    }

    for old_key in &deletions {
        table.delete(old_key)?;
    }
    for (new_key, mut values) in saves {
        let (key_changed, value) = {
            // I don't really know what I'm doing here: if we have
            // multiple values that were migrated *and* are hashing to
            // the same key, still try to apply `handle_conflict`,
            // does that make sense? In what order, how do the changed
            // flags matter?
            let (mut key_changed, mut value) =
                values.pop().expect("at least one must have been inserted");
            for (_other_key_changed, other_value) in values {
                value = handle_conflict(&new_key, value, other_value)?;
                key_changed = false; // at that point, overwrite will have to happen.
            }
            (key_changed, value)
        };
        // If key_changed is false, then it's OK to
        // overwrite. If the key changed, and there is a
        // conflict, then that probably means that migrated
        // data clashes with pre-existing data that wasn't
        // modified, "or something like that".
        match table.insert(&new_key, &value, key_changed) {
            Ok(()) => (),
            Err(e) => match e {
                KeyValError::KeyExists {
                    base_dir: _,
                    key_debug_string: _,
                } => {
                    let old_value = table.get(&new_key)?.ok_or_else(|| {
                        anyhow!("entry {new_key:?} has vanished while we held the lock")
                    })?;
                    let value = handle_conflict(&new_key, old_value, value)?;
                    // Now overwrite it.
                    table.insert(&new_key, &value, false)?;
                }
                _ => Err(e)?,
            },
        }
    }

    Ok(num_migrated)
}

/// Migrate a queue. Returns how many items were migrated.
pub fn migrate_queue(run_queue: &RunQueue) -> Result<usize> {
    migrate_key_val(
        run_queue.key_val(),
        // The key (a `TimeKey`) remains the same
        |k, _v| k.clone(),
        // Conflicts can't happen since we never change the key
        |_, _, _| bail!("can't happen"),
    )
}

/// Migrate the already_inserted table. Returns how many items
/// (buckets of time stamps) were migrated.
pub fn migrate_already_inserted(
    table: &KeyVal<BenchmarkingJobParametersHash, (BenchmarkingJobParameters, Vec<SystemTime>)>,
) -> Result<usize> {
    migrate_key_val(
        table,
        // Recalculate the key from the `BenchmarkingJobParameters`
        |_k, v| v.0.slow_hash(),
        |key, (params1, times1), (_params2, times2)| {
            warn!(
                "note: after migration, two buckets for key {key:?} exist; \
                 taking the older one, assuming that the newer entry was erroneous"
            );
            Ok((params1, times1.min(times2)))
        },
    )
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename = "BenchmarkingCommand")]
pub struct BenchmarkingCommand1 {
    pub target_name: ProperDirname,
    pub subdir: PathBuf,
    pub command: PathBuf,
    pub arguments: Vec<String>,
    pub log_extracts: Option<Vec<LogExtract>>,
}

impl From<Arc<BenchmarkingCommand1>> for BenchmarkingCommand {
    fn from(value: Arc<BenchmarkingCommand1>) -> Self {
        let BenchmarkingCommand1 {
            target_name,
            subdir,
            command,
            arguments,
            log_extracts: _ignore,
        } = Arc::into_inner(value).expect("guaranteed 1 reference");
        let command = command.to_string_lossy().to_string();
        // ^ XX lossy. But have not been using any such paths.
        BenchmarkingCommand {
            target_name,
            subdir,
            command,
            arguments,
            pre_exec_bash_code: PreExecLevel2::new(None),
        }
    }
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
// #[serde(rename = "BenchmarkingJobPublic")]
pub struct BenchmarkingJobPublic1 {
    pub reason: Option<String>,
    pub run_parameters: Arc<RunParameters>,
    pub command: Arc<BenchmarkingCommand1>,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename = "BenchmarkingJob")]
pub struct BenchmarkingJob1 {
    #[serde(flatten)]
    pub benchmarking_job_public: BenchmarkingJobPublic1,
    #[serde(flatten)]
    pub benchmarking_job_state: BenchmarkingJobState,
    priority: Priority,
    current_boost: Priority,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(rename = "BenchmarkingJobParameters")]
pub struct BenchmarkingJobParameters1 {
    pub run_parameters: Arc<RunParameters>,
    pub command: Arc<BenchmarkingCommand1>,
}

impl From<BenchmarkingJobParameters1> for BenchmarkingJobParameters {
    fn from(value: BenchmarkingJobParameters1) -> Self {
        let BenchmarkingJobParameters1 {
            run_parameters,
            command,
        } = value;
        BenchmarkingJobParameters {
            run_parameters,
            command: Arc::new(command.into()),
        }
    }
}

impl FromStrMigrating for (BenchmarkingJobParameters, Vec<SystemTime>) {
    fn from_str_migrating(s: &str) -> Result<(Self, bool)> {
        if let Ok(v) = serde_json::from_str::<(BenchmarkingJobParameters, Vec<SystemTime>)>(s) {
            return Ok((v, false));
        }
        if let Ok((params, times)) =
            serde_json::from_str::<(BenchmarkingJobParameters1, Vec<SystemTime>)>(s)
        {
            return Ok(((params.into(), times), true));
        }
        bail!(
            "can't parse/migrate as {}: {s:?}",
            type_name_short::<Self>()
        )
    }
}

impl FromStrMigrating for BenchmarkingJob {
    fn from_str_migrating(s: &str) -> Result<(Self, bool)> {
        if let Ok(v) = serde_json::from_str::<BenchmarkingJob>(s) {
            return Ok((v, false));
        }
        if let Ok(v) = serde_json::from_str::<BenchmarkingJob1>(s) {
            let BenchmarkingJob1 {
                benchmarking_job_public,
                benchmarking_job_state,
                priority,
                current_boost,
            } = v;
            let BenchmarkingJobPublic1 {
                reason,
                run_parameters,
                command,
            } = benchmarking_job_public;

            let v = BenchmarkingJob::new(
                BenchmarkingJobPublic {
                    reason,
                    run_parameters,
                    command: Arc::new(command.into()),
                },
                benchmarking_job_state,
                priority,
                current_boost,
            );
            return Ok((v, true));
        }
        bail!(
            "can't parse/migrate as {}: {s:?}",
            type_name_short::<Self>()
        )
    }
}
