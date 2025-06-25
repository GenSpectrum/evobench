use std::collections::BTreeMap;

use anyhow::Result;

use crate::key::{RunParameters, RunParametersOpts};

#[derive(Debug, PartialEq, Clone, clap::Args)]
pub struct BenchmarkingJobOpts {
    /// The number of times the job should be run in total (across all
    /// queues)
    #[clap(short, long, default_value = "5")]
    count: u8,

    /// How many times a job is allowed to fail before it is removed
    /// from the pipeline
    #[clap(short, long, default_value = "3")]
    error_budget: u8,

    #[clap(flatten)]
    pub run_parameters: RunParametersOpts,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingJob {
    pub run_parameters: RunParameters,
    pub remaining_count: u8,
    pub remaining_error_budget: u8,
}

impl BenchmarkingJobOpts {
    pub fn checked(
        self,
        custom_parameters_required: &BTreeMap<String, bool>,
    ) -> Result<BenchmarkingJob> {
        let Self {
            count,
            error_budget,
            run_parameters,
        } = self;

        Ok(BenchmarkingJob {
            run_parameters: run_parameters.checked(custom_parameters_required)?,
            remaining_count: count,
            remaining_error_budget: error_budget,
        })
    }
}
