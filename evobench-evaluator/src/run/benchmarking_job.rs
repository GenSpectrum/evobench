use crate::key::{CustomParametersSet, RunParameters, RunParametersOpts};

#[derive(Debug, PartialEq, Clone, clap::Args, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename = "BenchmarkingJobKnobs")]
pub struct BenchmarkingJobKnobsOpts {
    /// The number of times the job should be run in total (across all
    /// queues). Default: 5
    #[clap(short, long)]
    count: Option<u8>,

    /// How many times a job is allowed to fail before it is removed
    /// from the pipeline. Default: 3
    #[clap(short, long)]
    error_budget: Option<u8>,
}

pub struct BenchmarkingJobKnobs {
    count: u8,
    error_budget: u8,
}

impl BenchmarkingJobKnobsOpts {
    pub fn complete(&self, fallback: Option<&BenchmarkingJobKnobsOpts>) -> BenchmarkingJobKnobs {
        let Self {
            count,
            error_budget,
        } = self;
        let count = count
            .or_else(|| {
                let fallback = fallback?;
                fallback.count
            })
            .unwrap_or(5);
        let error_budget = error_budget
            .or_else(|| {
                let fallback = fallback?;
                fallback.error_budget
            })
            .unwrap_or(3);
        BenchmarkingJobKnobs {
            count,
            error_budget,
        }
    }
}

#[derive(Debug, PartialEq, Clone, clap::Args)]
pub struct BenchmarkingJobOpts {
    #[clap(flatten)]
    pub benchmarking_job_knobs: BenchmarkingJobKnobsOpts,

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
        benchmarking_job_knobs_fallback: Option<&BenchmarkingJobKnobsOpts>,
        custom_parameters_set: &CustomParametersSet,
    ) -> Vec<BenchmarkingJob> {
        let Self {
            benchmarking_job_knobs,
            run_parameters,
        } = self;
        let BenchmarkingJobKnobs {
            count,
            error_budget,
        } = benchmarking_job_knobs.complete(benchmarking_job_knobs_fallback);

        custom_parameters_set
            .iter()
            .map(|custom_parameters| BenchmarkingJob {
                run_parameters: run_parameters.complete(custom_parameters),
                remaining_count: count,
                remaining_error_budget: error_budget,
            })
            .collect()
    }
}
