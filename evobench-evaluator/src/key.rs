//! The representation of the parameters (given or recorded) for a
//! benchmarking run. This does not necessarily include *all* recorded
//! metainformation. The aim is to allow to automatically group runs
//! for which a sensible `summary` can be calculated, and to allow to
//! specify sets of runs for which the `change` should be
//! calculated. Maybe be fine-grained enough to e.g. allow to include
//! runs from different hosts if their CPU and memory configuration is
//! identical. And maybe provide for the means to allow for manual
//! overrides, to include all runs in a summary with keys "close
//! enough".

//! Some parameters, e.g. hostname, may be irrelevant when the
//! hardware and software versions are given; or it may turn out
//! controlling for those is not enough; thus, some key pieces are
//! redundant, or not?

//! Time-of-day may be relevant (rather: were other processes shut
//! down or not), strongly or weakly, but can't be part of the key or
//! grouping would not work; this is a piece of information to track
//! separately for verification.

//! Custom parameters can be given and be relevant, e.g. whether
//! providing input data to an application sorted or not.

use std::{
    collections::BTreeMap, fmt::Display, num::NonZeroU32, path::PathBuf, str::FromStr, sync::Arc,
};

use anyhow::{bail, Result};
use itertools::Itertools;
use kstring::KString;
use serde::{Deserialize, Serialize};

use crate::{
    crypto_hash::crypto_hash,
    ctx,
    git::GitHash,
    key_val_fs::as_key::AsKey,
    run::{
        config::BenchmarkingCommand,
        custom_parameter::{AllowedCustomParameter, CustomParameterValue},
        run_job::AllowableCustomEnvVar,
    },
    serde::allowed_env_var::AllowedEnvVar,
};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OsInfo {
    /// e.g. taken from `UName.sysname`
    pub os: String, // XX use the enum from other lib, move
    /// e.g. "6.1.0-37-amd64"
    pub release: String,
    /// e.g. "#1 SMP PREEMPT_DYNAMIC Debian 6.1.140-1 (2025-05-22)"
    pub version: String,
}

/// Information that together should allow a host to be determined to
/// be equivalent.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInfo {
    pub cpu_model: String,
    pub num_cores: NonZeroU32,
    // ram_kb: NonZeroU32,
    // and swap?
    // ^ Both irrelevant as long as there's *enough* RAM.
    // XX Thus, log these things (together with free mem
    // before/during?/after? time of evaluation), then allow to
    // correlate, but don't make it part of the key, OK?
    pub os_info: OsInfo,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    /// e.g. taken from `UName.nodename`
    pub hostname: String,
    pub host_info: HostInfo,
}

/// As determined by evobench-run (but should compare to duplicates
/// of some of the fields in the bench log file resulting from a run)
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyContext {
    pub host: Host,
    pub username: String,
}

/// Custom key/value pairings, passed on as environment variables when
/// executing the benchmarking runner of the target project. These
/// are checked for allowed and required values.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomParameters(BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, CustomParameterValue>);

impl CustomParameters {
    pub fn btree_map(
        &self,
    ) -> &BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, CustomParameterValue> {
        &self.0
    }
    pub fn checked_from(
        keyvals: &BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, KString>,
        custom_parameters_required: &BTreeMap<
            AllowedEnvVar<AllowableCustomEnvVar>,
            AllowedCustomParameter,
        >,
    ) -> Result<Self> {
        // XX: keyvals.iter() is never containing duplicates, now that
        // this is a BTreeMap they are removed silently by serde
        // (bummer!)
        let mut res = BTreeMap::new();
        for kv in keyvals {
            let (key, value) = kv;
            if res.contains_key(key) {
                bail!("duplicated custom parameter with name {:?}", key.as_str())
            }
            let allowable_key: AllowedEnvVar<AllowableCustomEnvVar> = AllowedEnvVar::from_str(key)?;
            if let Some(allowed_custom_parameter) = custom_parameters_required.get(&allowable_key) {
                let val =
                    CustomParameterValue::checked_from(allowed_custom_parameter.r#type, value)
                        .map_err(ctx!("for variable {:?}", key.as_str()))?;

                res.insert(key.clone(), val);
            } else {
                let valid_params = custom_parameters_required
                    .keys()
                    .map(|key| format!("{:?}", key.as_str()))
                    .join(", ");
                bail!(
                    "invalid custom parameter name {:?} (valid are: {valid_params})",
                    key.as_str()
                )
            }
        }
        for (key, allowed_custom_parameter) in custom_parameters_required.iter() {
            if allowed_custom_parameter.required {
                if !res.contains_key(key) {
                    bail!("missing custom parameter with name {:?}", key.as_str())
                }
            }
        }

        Ok(CustomParameters(res))
    }
}

impl Display for CustomParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut is_first = true;
        for (k, custom_parameter_value) in self.btree_map().iter() {
            let v = custom_parameter_value.as_str();
            write!(f, "{}{k}={v}", if is_first { "" } else { "," })?;
            is_first = false;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Clone, clap::Parser)]
pub struct RunParametersOpts {
    /// The commit of the source code of the target (benchmarked)
    /// project
    pub commit_id: GitHash,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunParameters {
    pub commit_id: GitHash,
    pub custom_parameters: Arc<CustomParameters>,
}

impl RunParameters {
    /// Extend `path` with segments leading to the folder to use for
    /// files for this run. TEMPORARY solution.
    pub fn extend_path(&self, mut path: PathBuf) -> PathBuf {
        for (key, val) in self.custom_parameters.0.iter() {
            let val = val.as_str();
            // key.len() + 1 + val.len() is statically guaranteed to
            // fit in the 255 bytes of max file name length on
            // Linux. \0 is disallowed on construction time. Since we
            // interpolate a =, there are no possible remaining
            // invalid cases.
            path.push(format!("{key}={val}"));
        }
        path.push(self.commit_id.to_string());
        path
    }
}

/// Only the parts of a BenchmarkingJob that determine results--but
/// also excluding schedule_condition, which *does* have a conscious
/// influence on results, but comes from the configured pipeline, not
/// the insertion. This here is used for insertion uniqueness
/// checking, but maybe also for key determination lager (XX todo).
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BenchmarkingJobParameters {
    pub run_parameters: Arc<RunParameters>,
    /// NOTE that BenchmarkingCommand has both `target_name` and the
    /// actual values; we currently make our key be based on *both* at
    /// the same time! I.e. a job with the same actual values but
    /// different `target_name` will be treated as another key!
    /// (FUTURE: is this really right?)
    pub command: Arc<BenchmarkingCommand>,
}

impl BenchmarkingJobParameters {
    pub fn slow_hash(&self) -> BenchmarkingJobParametersHash {
        self.into()
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BenchmarkingJobParametersHash(String);

impl From<&BenchmarkingJobParameters> for BenchmarkingJobParametersHash {
    fn from(value: &BenchmarkingJobParameters) -> Self {
        Self(crypto_hash(value))
    }
}

impl AsKey for BenchmarkingJobParametersHash {
    fn as_filename_str(&self) -> std::borrow::Cow<'_, str> {
        (&self.0).into()
    }

    fn try_from_filename_str(file_name: &str) -> Option<Self> {
        Some(Self(file_name.into()))
    }
}

impl RunParametersOpts {
    pub fn complete(&self, custom_parameters: Arc<CustomParameters>) -> RunParameters {
        let Self { commit_id } = self;
        RunParameters {
            commit_id: commit_id.clone(),
            custom_parameters,
        }
    }
}

/// As output by the benchmark runner of the target project (currently
/// always the evobench-probes library)
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateContext {
    /// Taken from `log_message::Metadata`: including version, as determined
    /// by evobench-probes, e.g. "GCC 12.2.0"
    pub compiler: String,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    /// Info gleaned by evobench-run before executing a run.
    pub early_context: EarlyContext,
    /// Parameters requested by the user and passed to the benchmark
    /// runner of the target project.
    pub run_parameters: RunParameters,
    /// Info gleaned by evobench-run from the output file of the
    /// evobench-probes library after executing a run.
    pub late_context: LateContext,
}
