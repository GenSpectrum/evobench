use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use crate::{
    fallback_to_default, fallback_to_option,
    git::GitHash,
    key::{BenchmarkingJobParameters, CustomParameters, RunParameters},
    run::{
        config::RunConfig,
        sub_command::insert::{ForceInvalidOpt, InsertBenchmarkingJobOpts},
    },
    serde_types::priority::{NonComparableNumber, Priority},
    utillib::{arc::CloneArc, fallback::FallingBackTo},
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
    /// queues). Default (if not defined elsewhere): 5
    #[clap(short, long)]
    count: Option<u8>,

    /// How many times a job is allowed to fail before it is removed
    /// from the pipeline. Default (if not defined elsewhere): 3
    #[clap(short, long)]
    error_budget: Option<u8>,
}

pub struct BenchmarkingJobSettings {
    count: u8,
    error_budget: u8,
}

impl Default for BenchmarkingJobSettings {
    fn default() -> Self {
        Self {
            count: 5,
            error_budget: 3,
        }
    }
}

impl FallingBackTo for BenchmarkingJobSettingsOpts {
    fn falling_back_to(
        self,
        fallback: &BenchmarkingJobSettingsOpts,
    ) -> BenchmarkingJobSettingsOpts {
        let Self {
            count,
            error_budget,
        } = self;
        fallback_to_option!(fallback.count);
        fallback_to_option!(fallback.error_budget);
        BenchmarkingJobSettingsOpts {
            count,
            error_budget,
        }
    }
}

impl From<BenchmarkingJobSettingsOpts> for BenchmarkingJobSettings {
    fn from(value: BenchmarkingJobSettingsOpts) -> Self {
        let BenchmarkingJobSettingsOpts {
            count,
            error_budget,
        } = value;
        let default = BenchmarkingJobSettings::default();
        fallback_to_default!(default.count);
        fallback_to_default!(default.error_budget);
        Self {
            count,
            error_budget,
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

#[derive(Debug)]
pub struct BenchmarkingJobOpts {
    /// Optional overrides for what values might come from elsewhere
    pub insert_benchmarking_job_opts: InsertBenchmarkingJobOpts,
    pub commit_id: GitHash,
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
    pub public: BenchmarkingJobPublic,
    #[serde(flatten)]
    pub state: BenchmarkingJobState,
    priority: Priority,
    current_boost: Priority,
}

impl BenchmarkingJob {
    /// Constructor, since some fields are private (needed for
    /// migration code).
    pub fn new(
        public: BenchmarkingJobPublic,
        state: BenchmarkingJobState,
        priority: Priority,
        current_boost: Priority,
    ) -> Self {
        Self {
            public,
            state,
            priority,
            current_boost,
        }
    }

    /// Overwrite values in `self` with those from the given overrides
    /// (and for values in `BenchmarkingJobSettings`, if not provided,
    /// take those from the config). Delete
    /// `last_working_directory`. Then check for allowable values
    /// (custom variables) via settings from the config, unless
    /// `force` was given.
    // (A little wasteful as it initializes a new CustomParameters
    // that is then dropped again.)
    pub fn check_and_init(
        &mut self,
        config: &RunConfig,
        override_opts: &InsertBenchmarkingJobOpts,
        override_commit: Option<&GitHash>,
        force: &ForceInvalidOpt,
    ) -> Result<()> {
        // Init: take the values from the overrides, falling back to
        // the values from `self`. But can't use FallingBackTo as Self
        // has no options anymore. Thus code up manually.
        {
            let BenchmarkingJobState {
                remaining_count,
                remaining_error_budget,
                last_working_directory,
            } = &mut self.state;

            *last_working_directory = None;

            let InsertBenchmarkingJobOpts {
                reason,
                benchmarking_job_settings,
                priority,
                initial_boost,
            } = override_opts;

            {
                let BenchmarkingJobSettings {
                    count,
                    error_budget,
                } = benchmarking_job_settings
                    .clone()
                    .falling_back_to(&config.benchmarking_job_settings)
                    .into();

                *remaining_count = count;
                *remaining_error_budget = error_budget;
            }

            if let Some(initial_boost) = initial_boost {
                self.current_boost = initial_boost.clone();
            }
            if let Some(priority) = priority {
                self.priority = priority.clone();
            }

            if let Some(reason) = &reason.reason {
                self.public.reason = Some(reason.into());
            }

            if let Some(override_commit) = override_commit {
                let mut run_parameters: RunParameters = (*self.public.run_parameters).clone();
                run_parameters.commit_id = override_commit.clone();
                self.public.run_parameters = Arc::new(run_parameters);
            }
        }

        // Checks
        if !force.force_invalid {
            let Self {
                public:
                    BenchmarkingJobPublic {
                        reason: _,
                        run_parameters,
                        command,
                    },
                state: _,
                priority: _,
                current_boost: _,
            } = self;

            let target_name = &command.target_name;
            let target = config.targets.get(target_name).ok_or_else(|| {
                anyhow!("target {:?} not found in the config", target_name.as_str())
            })?;

            let _ = CustomParameters::checked_from(
                &run_parameters.custom_parameters.keyvals(),
                &target.allowed_custom_parameters,
            )?;

            if *command != target.benchmarking_command {
                bail!(
                    "command for target {:?} is expected to be {:?}, but is: {:?}",
                    target_name.as_str(),
                    target.benchmarking_command,
                    command
                );
            }
        }

        Ok(())
    }

    pub fn priority(&self) -> Result<Priority, NonComparableNumber> {
        self.priority + self.current_boost
    }

    /// Clones everything except `current_boost` is set to 0. You can
    /// change the public fields afterwards.
    pub fn clone_for_queue_reinsertion(&self, state: BenchmarkingJobState) -> Self {
        let Self {
            public,
            priority,
            current_boost: _,
            state: _,
        } = self;
        Self {
            public: public.clone(),
            state,
            priority: *priority,
            current_boost: Priority::NORMAL,
        }
    }

    pub fn benchmarking_job_parameters(&self) -> BenchmarkingJobParameters {
        // Ignore all fields that are not "key" parts (inputs
        // determining/influencing the output)
        let BenchmarkingJob {
            public:
                BenchmarkingJobPublic {
                    reason: _,
                    run_parameters,
                    command,
                },
            state: _,
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
    /// Make one job per job template, filling the missing values
    /// (currently just `priority` and `initial_boost`) and all other
    /// values from the job template.
    pub fn complete_jobs(&self, job_template_list: &[JobTemplate]) -> Vec<BenchmarkingJob> {
        let Self {
            insert_benchmarking_job_opts:
                InsertBenchmarkingJobOpts {
                    reason,
                    benchmarking_job_settings,
                    priority: opts_priority,
                    initial_boost: opts_initial_boost,
                },
            commit_id,
        } = self;

        let BenchmarkingJobSettings {
            count,
            error_budget,
        } = benchmarking_job_settings.clone().into();

        job_template_list
            .iter()
            .map(|job_template| {
                let JobTemplate {
                    priority,
                    initial_boost,
                    command,
                    custom_parameters,
                } = job_template;

                BenchmarkingJob {
                    public: BenchmarkingJobPublic {
                        reason: reason.reason.clone(),
                        run_parameters: Arc::new(RunParameters {
                            commit_id: commit_id.clone(),
                            custom_parameters: custom_parameters.clone_arc(),
                        }),
                        command: command.clone_arc(),
                    },
                    state: BenchmarkingJobState {
                        remaining_count: count,
                        remaining_error_budget: error_budget,
                        last_working_directory: None,
                    },
                    priority: opts_priority.unwrap_or(*priority),
                    current_boost: opts_initial_boost.unwrap_or(*initial_boost),
                }
            })
            .collect()
    }
}
