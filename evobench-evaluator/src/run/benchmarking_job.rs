use crate::{
    key::{CustomParametersSet, RunParameters, RunParametersOpts},
    serde::priority::Priority,
};

#[derive(Debug, PartialEq, Clone, clap::Args, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename = "BenchmarkingJobSettings")]
pub struct BenchmarkingJobSettingsOpts {
    /// The number of times the job should be run in total (across all
    /// queues). Default taken from config file or: 5
    #[clap(short, long)]
    count: Option<u8>,

    /// How many times a job is allowed to fail before it is removed
    /// from the pipeline. Default taken from config file or: 3
    #[clap(short, long)]
    error_budget: Option<u8>,

    /// The priority of this job (a floating point number, or the
    /// names `normal` (alias for 0.), `high` (alias for 1.), and
    /// `low` (alias for -1.)). Jobs with a higher priority value (in
    /// the positive direction) are scheduled before other
    /// jobs. Default taken from config file or: 0
    #[clap(short, long)]
    priority: Option<Priority>,
}

pub struct BenchmarkingJobSettings {
    count: u8,
    error_budget: u8,
    priority: Priority,
}

impl BenchmarkingJobSettingsOpts {
    pub fn complete(
        &self,
        fallback: Option<&BenchmarkingJobSettingsOpts>,
    ) -> BenchmarkingJobSettings {
        let Self {
            count,
            error_budget,
            priority,
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
        let priority = priority
            .or_else(|| {
                let fallback = fallback?;
                fallback.priority
            })
            .unwrap_or(Priority::new(0.).expect("0 works"));
        BenchmarkingJobSettings {
            count,
            error_budget,
            priority,
        }
    }
}

#[derive(Debug, PartialEq, Clone, clap::Args)]
pub struct BenchmarkingJobOpts {
    #[clap(flatten)]
    pub benchmarking_job_settings: BenchmarkingJobSettingsOpts,

    #[clap(flatten)]
    pub run_parameters: RunParametersOpts,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingJob {
    pub run_parameters: RunParameters,
    pub priority: Priority,
    pub remaining_count: u8,
    pub remaining_error_budget: u8,
}

impl BenchmarkingJobOpts {
    pub fn complete_jobs(
        &self,
        benchmarking_job_settings_fallback: Option<&BenchmarkingJobSettingsOpts>,
        custom_parameters_set: &CustomParametersSet,
    ) -> Vec<BenchmarkingJob> {
        let Self {
            benchmarking_job_settings,
            run_parameters,
        } = self;
        let BenchmarkingJobSettings {
            count,
            error_budget,
            priority,
        } = benchmarking_job_settings.complete(benchmarking_job_settings_fallback);

        custom_parameters_set
            .iter()
            .map(|custom_parameters| BenchmarkingJob {
                run_parameters: run_parameters.complete(custom_parameters),
                remaining_count: count,
                remaining_error_budget: error_budget,
                priority,
            })
            .collect()
    }
}
