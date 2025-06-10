//! Representation of benchmark log messages

use kstring::KString;
use serde::{Deserialize, Serialize};

use crate::times::{MicroTime, NanoTime};

/// Only increment this for incompatible changes, not for additional
/// fields that can be handled as `Option`. Also, you might want to
/// create a new module for the new version and keep this module for
/// reading old logs.
pub const EVOBENCH_LOG_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadId(pub u64);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UName {
    pub sysname: String,
    pub nodename: String,
    pub release: String,
    pub version: String,
    pub machine: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub hostname: String,
    pub username: String,
    pub uname: UName,
    pub compiler: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timing {
    // Probe name ("module|local" if using the macros)
    pub pn: KString,
    pub pid: ProcessId,
    pub tid: ThreadId,
    pub r: NanoTime,
    pub u: MicroTime,
    pub s: MicroTime,
    // These are `long int`, which could be i64, hence always use that
    // since we don't know the width of the machine in question. Also,
    // Option since I don't know how to get those numbers on macOS.
    pub maxrss: Option<i64>,
    pub minflt: Option<i64>,
    pub majflt: Option<i64>,
    pub inblock: Option<i64>,
    pub oublock: Option<i64>,
    pub nvcsw: Option<i64>,
    pub nivcsw: Option<i64>,
}

impl Timing {
    #[inline]
    pub fn nvcsw(&self) -> Option<u64> {
        Some(
            self.nvcsw?
                .try_into()
                .expect("ctx switches (nvcsw) should be non-negative"),
        )
    }

    #[inline]
    pub fn nivcsw(&self) -> Option<u64> {
        Some(
            self.nivcsw?
                .try_into()
                .expect("ctx switches (nivcsw) should be non-negative"),
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyValue {
    pub tid: ThreadId,
    pub k: KString,
    pub v: KString,
}

include! {"../include/evobench_point_kind.rs"}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum LogMessage {
    /// `Start` describes the file format. Do not change this ennum
    /// item! This is separate from `Metadata` so that `Metadata` can
    /// be changed; `Start` must not change so that the parser can
    /// detect the version then switch to the right parser for the
    /// rest.
    Start {
        evobench_log_version: u32,
        evobench_version: KString,
    },
    /// Random information from the program.
    Metadata(Metadata),
    /// Random information from the program.
    KeyValue(KeyValue),
    /// For the other items, see the doc comments in evobench_evaluator.hpp
    TStart(Timing),
    T(Timing),
    TS(Timing),
    TE(Timing),
    TThreadStart(Timing),
    TThreadEnd(Timing),
    TEnd(Timing),
    TIO(Timing),
}

pub enum DataMessage<'t> {
    KeyValue(&'t KeyValue),
    Timing(PointKind, &'t Timing),
}

impl LogMessage {
    pub fn opt_data_message(&self) -> Option<DataMessage> {
        match self {
            LogMessage::Start {
                evobench_log_version: _,
                evobench_version: _,
            } => None,
            LogMessage::Metadata(_) => None,
            LogMessage::KeyValue(keyvalue) => Some(DataMessage::KeyValue(keyvalue)),
            LogMessage::TStart(timing) => Some(DataMessage::Timing(PointKind::TStart, timing)),
            LogMessage::T(timing) => Some(DataMessage::Timing(PointKind::T, timing)),
            LogMessage::TS(timing) => Some(DataMessage::Timing(PointKind::TS, timing)),
            LogMessage::TE(timing) => Some(DataMessage::Timing(PointKind::TE, timing)),
            LogMessage::TThreadStart(timing) => {
                Some(DataMessage::Timing(PointKind::TThreadStart, timing))
            }
            LogMessage::TThreadEnd(timing) => {
                Some(DataMessage::Timing(PointKind::TThreadEnd, timing))
            }
            LogMessage::TEnd(timing) => Some(DataMessage::Timing(PointKind::TEnd, timing)),
            LogMessage::TIO(timing) => Some(DataMessage::Timing(PointKind::TIO, timing)),
        }
    }
    /// Note: panics for Start and Metadata messages, because those
    /// are not in `LogData::messages` any more (XX type safe?). Use
    /// `opt_data_message()` instead if not accessing those.
    pub fn data_message(&self) -> DataMessage {
        self.opt_data_message()
            .expect("non-DataMessage not contained in LogData::messages")
    }
}
