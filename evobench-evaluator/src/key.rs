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

use std::num::NonZeroU32;

use crate::{
    git::GitHash,
    serde::{datetime::DateTimeWithOffset, key_val::KeyVal},
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

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize, clap::Parser)]
#[serde(deny_unknown_fields)]
pub struct RunParameters {
    /// The commit of the source code of the target (benchmarked)
    /// project
    pub commit_id: GitHash,

    /// Custom "key=value" pairs. They are passed on as environment
    /// variables when executing the benchmarking runner of the target
    /// project
    pub custom_parameters: Vec<KeyVal>,
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
