use std::collections::BTreeMap;

use anyhow::Result;

use crate::key::{CheckedRunParameters, RunParameters};

#[derive(Debug, PartialEq, Clone, clap::Args)]
pub struct BenchmarkingJobOpts {
    /// The number of times the job should be run in total (across all
    /// queues)
    #[clap(short, long, default_value = "5")]
    count: u8,

    #[clap(flatten)]
    pub run_parameters: RunParameters,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingJob {
    pub remaining_count: u8,
    pub run_parameters: CheckedRunParameters,
}

impl BenchmarkingJobOpts {
    pub fn checked(
        self,
        custom_parameters_required: &BTreeMap<String, bool>,
    ) -> Result<BenchmarkingJob> {
        let Self {
            count,
            run_parameters,
        } = self;

        Ok(BenchmarkingJob {
            remaining_count: count,
            run_parameters: run_parameters.checked(custom_parameters_required)?,
        })
    }
}
