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

use std::{collections::BTreeMap, num::NonZeroU32, ops::Deref};

use anyhow::{bail, Result};
use itertools::Itertools;

use crate::{
    crypto_hash::crypto_hash, git::GitHash, key_val_fs::as_key::AsKey,
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

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
/// Custom key/value pairings, passed on as environment variables when
/// executing the benchmarking runner of the target project. (These
/// are not checked against `custom_parameters_required` yet!)
pub struct CustomParametersOpts(BTreeMap<String, String>);

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomParameters(BTreeMap<String, String>);

impl CustomParameters {
    pub fn btree_map(&self) -> &BTreeMap<String, String> {
        &self.0
    }
}

impl CustomParametersOpts {
    pub fn checked(
        &self,
        custom_parameters_required: &BTreeMap<String, bool>,
    ) -> Result<CustomParameters> {
        let mut res = BTreeMap::new();
        for kv in &self.0 {
            let (key, val) = kv;
            if !custom_parameters_required.contains_key(key) {
                let valid_params = custom_parameters_required
                    .keys()
                    .map(|key| format!("{key:?}"))
                    .join(", ");
                bail!(
                    "invalid custom parameter name {key:?} \
                     (valid are: {valid_params})"
                )
            }
            if res.contains_key(key) {
                bail!("duplicated custom parameter with name {key:?}")
            }
            res.insert(key.to_owned(), val.to_owned());
        }
        for (key, required) in custom_parameters_required.iter() {
            if *required {
                if !res.contains_key(key) {
                    bail!("missing custom parameter with name {key:?}")
                }
            }
        }

        Ok(CustomParameters(res))
    }
}

// Via config file only possible, due to multiple lists, except if
// doing a custom cmdline parser.
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
/// Give at least one entry or no benchmarking will be done at all!
/// (Note: these CustomParameters are not yet checked against
/// allowed/required keys!)
pub struct CustomParametersSetOpts(pub Vec<CustomParametersOpts>);

/// Checked parameters
pub struct CustomParametersSet(Vec<CustomParameters>);

impl Deref for CustomParametersSet {
    type Target = [CustomParameters];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl CustomParametersSetOpts {
    pub fn checked(
        &self,
        custom_parameters_required: &BTreeMap<String, bool>,
    ) -> Result<CustomParametersSet> {
        let checked_custom_parameters_set = self
            .0
            .iter()
            .map(|custom_parameters| custom_parameters.checked(custom_parameters_required))
            .collect::<Result<Vec<_>>>()?;
        Ok(CustomParametersSet(checked_custom_parameters_set))
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
