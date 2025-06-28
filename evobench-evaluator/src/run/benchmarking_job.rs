use crate::key::{CustomParametersSet, RunParameters, RunParametersOpts};

#[derive(Debug, PartialEq, Clone, clap::Args, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingJobKnobs {
    /// The number of times the job should be run in total (across all
    /// queues)
    #[clap(short, long, default_value = "5")]
    count: u8,

    /// How many times a job is allowed to fail before it is removed
    /// from the pipeline
    #[clap(short, long, default_value = "3")]
    error_budget: u8,
}

#[derive(Debug, PartialEq, Clone, clap::Args)]
pub struct BenchmarkingJobOpts {
    #[clap(flatten)]
    pub benchmarking_job_knobs: BenchmarkingJobKnobs,

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
    pub fn complete_jobs(
        &self,
        custom_parameters_set: &CustomParametersSet,
    ) -> Vec<BenchmarkingJob> {
        let Self {
            benchmarking_job_knobs:
                BenchmarkingJobKnobs {
                    count,
                    error_budget,
                },
            run_parameters,
        } = self;

        custom_parameters_set
            .iter()
            .map(|custom_parameters| BenchmarkingJob {
                run_parameters: run_parameters.complete(custom_parameters),
                remaining_count: *count,
                remaining_error_budget: *error_budget,
            })
            .collect()
    }
}
