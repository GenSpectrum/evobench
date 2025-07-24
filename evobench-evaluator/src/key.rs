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
//! down or not), strongly or weakly.

//! Custom parameters can be given and be relevant, e.g. whether
//! providing input data to an application sorted or not.

use std::{collections::BTreeMap, fmt::Display, num::NonZeroU32, path::PathBuf};

use anyhow::{bail, Result};
use itertools::Itertools;
use kstring::KString;

use crate::{
    crypto_hash::crypto_hash,
    git::GitHash,
    key_val_fs::as_key::AsKey,
    run::custom_parameter::{AllowedCustomParameter, CustomParameterValue},
    serde::date_and_time::DateTimeWithOffset,
};

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInfo {
    pub cpu_model: String,
    pub num_cores: NonZeroU32,
    // ram_kb: NonZeroU32,
    // and swap?
    // ^ Both irrelevant as long as there's *enough* RAM.
    pub os_info: OsInfo,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    /// e.g. taken from `UName.nodename`
    pub hostname: String,
    pub host_info: HostInfo,
}

/// As determined by evobench-run (but should compare to duplicates
/// of some of the fields in the bench log file resulting from a run)
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyContext {
    pub host: Host,
    pub username: String,
    /// The time when the benchmarking run was started
    pub start_datetime: DateTimeWithOffset,
}

/// Custom key/value pairings, passed on as environment variables when
/// executing the benchmarking runner of the target project. These
/// are checked for allowed and required values.
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomParameters(BTreeMap<KString, CustomParameterValue>);

impl CustomParameters {
    pub fn btree_map(&self) -> &BTreeMap<KString, CustomParameterValue> {
        &self.0
    }
    pub fn checked_from(
        keyvals: &BTreeMap<KString, KString>,
        custom_parameters_required: &BTreeMap<KString, AllowedCustomParameter>,
    ) -> Result<Self> {
        let mut res = BTreeMap::new();
        for kv in keyvals {
            let (key, value) = kv;
            if res.contains_key(key.as_str()) {
                bail!("duplicated custom parameter with name {key:?}")
            }
            if let Some(allowed_custom_parameter) = custom_parameters_required.get(key) {
                let val =
                    CustomParameterValue::checked_from(allowed_custom_parameter.r#type, value)?;

                res.insert(key.clone(), val);
            } else {
                let valid_params = custom_parameters_required
                    .keys()
                    .map(|key| format!("{key:?}"))
                    .join(", ");
                bail!(
                    "invalid custom parameter name {key:?} \
                     (valid are: {valid_params})"
                )
            }
        }
        for (key, allowed_custom_parameter) in custom_parameters_required.iter() {
            if allowed_custom_parameter.required {
                if !res.contains_key(key.as_str()) {
                    bail!("missing custom parameter with name {key:?}")
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

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunParameters {
    pub commit_id: GitHash,
    pub custom_parameters: CustomParameters,
}

impl RunParameters {
    /// Extend `path` with segments leading to the folder to use for
    /// files for this run. TEMPORARY solution.
    pub fn extend_path(&self, mut path: PathBuf) -> PathBuf {
        for (key, val) in self.custom_parameters.0.iter() {
            let v = val.as_str();
            path.push(format!("{key}={v}"));
        }
        path.push(self.commit_id.to_string());
        path
    }
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunParametersHash(String);

impl From<&RunParameters> for RunParametersHash {
    fn from(value: &RunParameters) -> Self {
        Self(crypto_hash(value))
    }
}

impl AsKey for RunParametersHash {
    fn as_filename_str(&self) -> std::borrow::Cow<str> {
        (&self.0).into()
    }

    fn try_from_filename_str(file_name: &str) -> Option<Self> {
        Some(Self(file_name.into()))
    }
}

impl RunParametersOpts {
    pub fn complete(&self, custom_parameters: &CustomParameters) -> RunParameters {
        let Self { commit_id } = self;
        RunParameters {
            commit_id: commit_id.clone(),
            custom_parameters: custom_parameters.clone(),
        }
    }
}

/// As output by the benchmark runner of the target project (currently
/// always the evobench-probes library)
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateContext {
    /// Taken from `log_message::Metadata`: including version, as determined
    /// by evobench-probes, e.g. "GCC 12.2.0"
    pub compiler: String,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
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
