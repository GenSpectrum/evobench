use std::sync::Arc;

use anyhow::{anyhow, bail, Result};

use crate::{
    ctx,
    key::{BenchmarkingJobParameters, CustomParameters, RunParameters, RunParametersOpts},
    run::config::RunConfig,
    serde::priority::{NonComparableNumber, Priority},
    utillib::arc::CloneArc,
};

use super::{
    config::{BenchmarkingCommand, JobTemplate},
    working_directory_pool::WorkingDirectoryId,
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

    /// The default priority for jobs (a floating point number, or the
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
pub struct BenchmarkingJobReasonOpt {
    /// An optional short context string (should be <= 15 characters)
    /// describing the reason for or context of the job (e.g. used to
    /// report which git branch the commit was found on).
    #[clap(long)]
    pub reason: Option<String>,
}

#[derive(Debug, PartialEq, Clone, clap::Args)]
pub struct BenchmarkingJobOpts {
    #[clap(flatten)]
    pub reason: BenchmarkingJobReasonOpt,

    #[clap(flatten)]
    pub benchmarking_job_settings: BenchmarkingJobSettingsOpts,

    #[clap(flatten)]
    pub run_parameters: RunParametersOpts,
}

/// Just the public constant parts of a BenchmarkingJob
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingJobPublic {
    pub reason: Option<String>,
    pub run_parameters: Arc<RunParameters>,
    pub command: Arc<BenchmarkingCommand>,
}

/// Just the public changing parts of a BenchmarkingJob
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingJobState {
    pub remaining_count: u8,
    pub remaining_error_budget: u8,
    pub last_working_directory: Option<WorkingDirectoryId>,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingJob {
    #[serde(flatten)]
    pub benchmarking_job_public: BenchmarkingJobPublic,
    #[serde(flatten)]
    pub benchmarking_job_state: BenchmarkingJobState,
    priority: Priority,
    current_boost: Priority,
}

impl BenchmarkingJob {
    /// Constructor, since some fields are private (needed for
    /// migration code).
    pub fn new(
        benchmarking_job_public: BenchmarkingJobPublic,
        benchmarking_job_state: BenchmarkingJobState,
        priority: Priority,
        current_boost: Priority,
    ) -> Self {
        Self {
            benchmarking_job_public,
            benchmarking_job_state,
            priority,
            current_boost,
        }
    }

    /// Check a manually created BenchmarkingJob for allowable values
    /// and optionally overwrite parts with settings from the config.
    // (A little wasteful as it initializes a new CustomParameters
    // that is being dropped again.)
    pub fn check_and_init(
        &mut self,
        config: &RunConfig,
        init: bool,
        benchmarking_job_settings_opts: Option<&BenchmarkingJobSettingsOpts>,
        initial_boost: Option<Priority>,
    ) -> Result<()> {
        let Self {
            benchmarking_job_public,
            benchmarking_job_state,
            priority,
            current_boost,
        } = self;

        let BenchmarkingJobPublic {
            reason: _,
            run_parameters,
            command,
        } = benchmarking_job_public;

        let target_name = &command.target_name;
        let target = config
            .targets
            .get(target_name)
            .ok_or_else(|| anyhow!("target {:?} not found in the config", target_name.as_str()))?;
        let _ = CustomParameters::checked_from(
            &run_parameters.custom_parameters.keyvals(),
            &target.allowed_custom_parameters,
        )?;

        if *command != target.benchmarking_command {
            // (XX could do multi error collection and report them all
            // in one go)
            bail!(
                "command for target {:?} is expected to be {:?}, but is: {:?}",
                target_name.as_str(),
                target.benchmarking_command,
                command
            );
        }

        if init {
            let benchmarking_job_settings = config
                .benchmarking_job_settings
                .complete(benchmarking_job_settings_opts);

            let BenchmarkingJobState {
                remaining_count,
                remaining_error_budget,
                last_working_directory,
            } = benchmarking_job_state;

            *remaining_count = benchmarking_job_settings.count;
            *remaining_error_budget = benchmarking_job_settings.error_budget;
            *priority = benchmarking_job_settings.priority;
            if let Some(initial_boost) = initial_boost {
                *current_boost = initial_boost;
            }

            *last_working_directory = None;
        }

        Ok(())
    }

    pub fn priority(&self) -> Result<Priority, NonComparableNumber> {
        self.priority + self.current_boost
    }

    /// Clones everything except `current_boost` is set to 0. You can
    /// change the public fields afterwards.
    pub fn clone_for_queue_reinsertion(
        &self,
        benchmarking_job_state: BenchmarkingJobState,
    ) -> Self {
        let Self {
            benchmarking_job_public,
            priority,
            current_boost: _,
            benchmarking_job_state: _,
        } = self;
        Self {
            benchmarking_job_public: benchmarking_job_public.clone(),
            benchmarking_job_state,
            priority: *priority,
            current_boost: Priority::NORMAL,
        }
    }

    pub fn benchmarking_job_parameters(&self) -> BenchmarkingJobParameters {
        // Ignore all fields that are not "key" parts (inputs
        // determining/influencing the output)
        let BenchmarkingJob {
            benchmarking_job_public:
                BenchmarkingJobPublic {
                    reason: _,
                    run_parameters,
                    command,
                },
            benchmarking_job_state: _,
            priority: _,
            current_boost: _,
        } = self;
        BenchmarkingJobParameters {
            run_parameters: run_parameters.clone_arc(),
            command: command.clone_arc(),
        }
    }
}

impl BenchmarkingJobOpts {
    /// Adds priorities from config/defaults and those from job
    /// templates. Returns failure when priorities can't be added
    /// (+inf + -inf).
    pub fn complete_jobs(
        &self,
        benchmarking_job_settings_fallback: Option<&BenchmarkingJobSettingsOpts>,
        job_template_list: &[JobTemplate],
    ) -> Result<Vec<BenchmarkingJob>> {
        let Self {
            reason,
            benchmarking_job_settings,
            run_parameters,
        } = self;
        let BenchmarkingJobSettings {
            count,
            error_budget,
            priority: priority_from_config_or_defaults,
        } = benchmarking_job_settings.complete(benchmarking_job_settings_fallback);

        job_template_list
            .iter()
            .map(|job_template| -> Result<_> {
                let JobTemplate {
                    priority: priority_from_job_template,
                    initial_boost,
                    command,
                    custom_parameters,
                } = job_template;

                let priority = (priority_from_config_or_defaults + *priority_from_job_template)
                    .map_err(ctx!(
                    "can't add priority from config/defaults {priority_from_config_or_defaults} \
                     and priority from job template {priority_from_job_template}"
                ))?;

                Ok(BenchmarkingJob {
                    benchmarking_job_public: BenchmarkingJobPublic {
                        reason: reason.reason.clone(),
                        run_parameters: run_parameters
                            .complete(custom_parameters.clone_arc())
                            .into(),
                        command: command.clone_arc(),
                    },
                    benchmarking_job_state: BenchmarkingJobState {
                        remaining_count: count,
                        remaining_error_budget: error_budget,
                        last_working_directory: None,
                    },
                    priority,
                    current_boost: *initial_boost,
                })
            })
            .collect()
    }
}
