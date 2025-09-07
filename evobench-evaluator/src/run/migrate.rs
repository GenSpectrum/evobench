//! Database migration. Structs with incompatible changes should be
//! copied here and an upgrade function provided.

use std::{fmt::Debug, path::PathBuf, sync::Arc, time::SystemTime};

use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    ctx,
    key::{BenchmarkingJobParameters, BenchmarkingJobParametersHash, RunParameters},
    key_val_fs::{as_key::AsKey, key_val::KeyVal},
    run::{
        benchmarking_job::{BenchmarkingJob, BenchmarkingJobPublic, BenchmarkingJobState},
        config::BenchmarkingCommand,
        run_queue::RunQueue,
    },
    serde::{priority::Priority, proper_dirname::ProperDirname},
    util::grep_diff::LogExtract,
    utillib::type_name_short::type_name_short,
};

pub trait FromStrMigrating: Sized + DeserializeOwned + Serialize {
    /// Parse from whatever serialisation format that's appropriate
    /// for your type. Returns whether migration was needed as the
    /// second result.
    fn from_str_migrating(s: &str) -> Result<(Self, bool)>;
}

/// Returns how many items were migrated
fn migrate_key_val<K: AsKey + Debug + Clone + PartialEq, T: FromStrMigrating>(
    table: &KeyVal<K, T>,
    gen_key: impl Fn(&K, &T) -> K,
) -> Result<usize> {
    let mut num_migrated = 0;
    // Take a lock on the whole table since we need to ensure files
    // still exist when we are ready to overwrite them (the
    // `_entry_lock` is not enough for this). Still lock the items,
    // too, OK? XX What about deadlocks? (Table users are not required
    // to take the dir lock first! Todo: at least document that
    // ordering is required!)
    let _table_lock = table.lock_exclusive()?;

    for old_key in table.keys(false, None)? {
        let old_key = old_key?;
        // Entry works for us as it does not transparently decode.
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
                    table.delete(&old_key)?;
                }
                table.insert(&new_key, &value, false)?;
                num_migrated += 1;
            }
        }
    }
    Ok(num_migrated)
}

/// Migrate a queue. Returns how many items were migrated.
pub fn migrate_queue(run_queue: &RunQueue) -> Result<usize> {
    migrate_key_val(
        run_queue.queue.key_val(),
        // The key (a `TimeKey`) remains the same
        |k, _v| k.clone(),
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
        BenchmarkingCommand {
            target_name,
            subdir,
            command,
            arguments,
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
