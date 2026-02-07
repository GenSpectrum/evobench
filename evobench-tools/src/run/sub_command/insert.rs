//! -- see insert.md -- todo: copy here automatically via script?

use std::{collections::BTreeSet, fmt::Display, path::PathBuf, str::FromStr};

use anyhow::{Result, anyhow, bail};
use cj_path_util::unix::fixup_path::CURRENT_DIRECTORY;
use itertools::Itertools;
use run_git::git::GitWorkingDir;

use crate::{
    config_file::backend_from_path,
    git::GitHash,
    git_ext::MoreGitWorkingDir,
    info,
    run::{
        benchmarking_job::{
            BenchmarkingJob, BenchmarkingJobOpts, BenchmarkingJobReasonOpt,
            BenchmarkingJobSettingsOpts,
        },
        config::{JobTemplate, RunConfigBundle},
        insert_jobs::{DryRunOpt, ForceOpt, QuietOpt, insert_jobs},
        polling_pool::PollingPool,
        run_queues::RunQueues,
        sub_command::open_polling_pool,
    },
    serde::{
        date_and_time::DateTimeWithOffset, git_branch_name::GitBranchName,
        git_reference::GitReference, priority::Priority,
    },
    serde_util::serde_read_json,
    utillib::fallback::FallingBackTo,
};

#[derive(Debug, Clone, clap::Args)]
pub struct ForceInvalidOpt {
    /// Normally, values from a job file are checked for validity
    /// against the configuration. This disables that check.
    #[clap(long)]
    pub force_invalid: bool,
}

// (Note: clap::ArgEnum is only for the CLI help texts--FromStr is
// still necessary!)
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
/// Whether to look up Git references in the remote repository
/// or in a local clone (in which case the current working dir
/// must be inside it)
pub enum LocalOrRemote {
    /// Resolve branch names and references in the local repository
    /// (fails if the current working directory is not inside a clone
    /// of the target project repository)
    Local,
    /// Resolve branch names and references in the remote repository
    Remote,
}

impl LocalOrRemote {
    pub fn as_str(self) -> &'static str {
        match self {
            LocalOrRemote::Local => "local",
            LocalOrRemote::Remote => "remote",
        }
    }

    pub fn as_char(self) -> char {
        match self {
            LocalOrRemote::Local => 'L',
            LocalOrRemote::Remote => 'R',
        }
    }

    pub fn load(self, run_config_bundle: &RunConfigBundle) -> Result<LocalOrRemoteGitWorkingDir> {
        match self {
            LocalOrRemote::Local => {
                let git_working_dir = GitWorkingDir {
                    working_dir_path: CURRENT_DIRECTORY.to_owned().into(),
                };
                Ok(LocalOrRemoteGitWorkingDir::Local { git_working_dir })
            }
            LocalOrRemote::Remote => {
                let polling_pool = open_polling_pool(run_config_bundle)?;
                Ok(LocalOrRemoteGitWorkingDir::Remote { polling_pool })
            }
        }
    }
}

impl Display for LocalOrRemote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub enum LocalOrRemoteGitWorkingDir {
    Local { git_working_dir: GitWorkingDir },
    Remote { polling_pool: PollingPool },
}

impl LocalOrRemoteGitWorkingDir {
    pub fn resolve_references<R: AsRef<GitReference>>(
        &mut self,
        references: impl IntoIterator<Item = R>,
    ) -> Result<Vec<Option<GitHash>>> {
        match self {
            LocalOrRemoteGitWorkingDir::Local { git_working_dir } => references
                .into_iter()
                .map(|reference| -> Result<Option<GitHash>> {
                    let reference = reference.as_ref();
                    Ok(git_working_dir
                        .git_rev_parse(reference.as_str(), true)?
                        .map(|s| {
                            GitHash::from_str(&s).expect("git rev-parse always returns hashes")
                        }))
                })
                .try_collect(),
            LocalOrRemoteGitWorkingDir::Remote { polling_pool } => {
                let working_dir_id = polling_pool.updated_working_dir()?;
                polling_pool.resolve_references(working_dir_id, references)
            }
        }
    }

    pub fn get_branch_default(&mut self) -> Result<Option<GitBranchName>> {
        match self {
            LocalOrRemoteGitWorkingDir::Local { git_working_dir } => {
                git_working_dir.get_current_branch()
            }
            LocalOrRemoteGitWorkingDir::Remote { polling_pool } => {
                let id = polling_pool.updated_working_dir()?;
                polling_pool.process_in_working_directory(
                    id,
                    &DateTimeWithOffset::now(None),
                    |wdwp| {
                        let wd = wdwp.into_inner().expect("still there?");
                        wd.git_working_dir.get_current_branch()
                    },
                    "LocalOrRemoteGitWorkingDir.get_branch_default",
                )
            }
        }
    }
}

// (Note: strum is useless as its FromStr has no helpful error
// message, thus derive our own.)
impl FromStr for LocalOrRemote {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "local" => Ok(LocalOrRemote::Local),
            "remote" => Ok(LocalOrRemote::Remote),
            _ => bail!("invalid argument {s:?}, expecting 'local' or 'remote'"),
        }
    }
}

/// Options to change insertion behaviour
#[derive(clap::Args, Debug)]
pub struct InsertBehaviourOpts {
    #[clap(flatten)]
    force_opt: ForceOpt,
    #[clap(flatten)]
    quiet_opt: QuietOpt,
    #[clap(flatten)]
    dry_run_opt: DryRunOpt,
}

/// Options to set or override job settings from elsewhere
#[derive(Debug, Clone, clap::Args)]
#[command(allow_hyphen_values = true)]
pub struct InsertBenchmarkingJobOpts {
    #[clap(flatten)]
    pub reason: BenchmarkingJobReasonOpt,

    #[clap(flatten)]
    pub benchmarking_job_settings: BenchmarkingJobSettingsOpts,

    /// The priority (overrides the priority given elsewhere).
    #[clap(long)]
    pub priority: Option<Priority>,

    /// The initial priority boost (overrides the boost given
    /// elsewhere).
    #[clap(long)]
    pub initial_boost: Option<Priority>,
}

impl InsertBenchmarkingJobOpts {
    /// Fill in fallback values (from the RunConfig) for the only part
    /// that has those
    pub fn complete_with(self, fallback: &BenchmarkingJobSettingsOpts) -> Self {
        let Self {
            reason,
            benchmarking_job_settings,
            priority,
            initial_boost,
        } = self;
        let benchmarking_job_settings = benchmarking_job_settings.falling_back_to(fallback);
        Self {
            reason,
            benchmarking_job_settings,
            priority,
            initial_boost,
        }
    }
}

//Unused
// impl FallingBackTo for InsertBenchmarkingJobOpts {
//     fn falling_back_to(self, fallback: &Self) -> Self {
//         let Self {
//             reason,
//             benchmarking_job_settings,
//             priority,
//             initial_boost,
//         } = self;
//         fallback_to_trait!(fallback.reason);
//         fallback_to_trait!(fallback.benchmarking_job_settings);
//         fallback_to_option!(fallback.priority);
//         fallback_to_option!(fallback.initial_boost);
//         Self {
//             reason,
//             benchmarking_job_settings,
//             priority,
//             initial_boost,
//         }
//     }
// }

#[derive(clap::Args, Debug)]
pub struct InsertOpts {
    #[clap(flatten)]
    insert_behaviour_opts: InsertBehaviourOpts,

    #[clap(flatten)]
    insert_benchmarking_job_opts: InsertBenchmarkingJobOpts,
}

#[derive(clap::Subcommand, Debug)]
pub enum Insert {
    /// Take template definitions of a given named entry from the
    /// configuration file, and commits from explicitly specified
    /// references.
    #[command(after_help = "  Note: more job‑setting options are available in the parent command!")]
    Templates {
        /// The name of the entry in the `job_template_lists_name`
        /// field in the configuration file (RunConfig).
        job_template_lists_name: String,
        /// Whether to look up Git references in the remote repository
        /// or in a local clone (in which case the current working dir
        /// must be inside it)
        local_or_remote: LocalOrRemote,
        /// Git references to the commits that should be benchmarked
        /// (commit ids, branch oder tag names, and other syntax like
        /// `HEAD^`).
        reference_names: Vec<GitReference>,
    },

    /// Take template definitions of a branch name from the
    /// configuration file, and commits specified separately (if you
    /// want to take the commit from the same branch that you specify,
    /// you can use the `branch` subcommand instead).
    #[command(after_help = "  Note: more job‑setting options are available in the parent command!")]
    TemplatesOfBranch {
        branch_name: GitBranchName,
        #[clap(value_enum)]
        /// Whether to look up Git references in the remote repository
        /// or in a local clone (in which case the current working dir
        /// must be inside it)
        local_or_remote: LocalOrRemote,
        /// Git references to the commits that should be benchmarked
        /// (commit ids, branch oder tag names, and other syntax like
        /// `HEAD^`).
        reference_names: Vec<GitReference>,
    },

    /// Take template definitions of a branch name from the
    /// configuration file. If no branch name is given, takes the
    /// currently checked-out branch for 'local' or the default branch
    /// for 'remote'.  If you just want to take the configuration from
    /// a branch, but specify the commit independently, use the
    /// `template-of-branch` subcommand instead.
    #[command(after_help = "  Note: more job‑setting options are available in the parent command!")]
    Branch {
        /// Whether to look up Git references in the remote repository
        /// or in a local clone (in which case the current working dir
        /// must be inside it)
        #[clap(value_enum)]
        local_or_remote: LocalOrRemote,
        /// Branch name to use for template lookup and commit id. If
        /// not given, the currently checked-out branch name is tried
        /// (fails if that branch has no template configuration). Be
        /// careful when using `local` mode if your local branch
        /// naming conventions differ from the remote ones.
        branch_name: Option<GitBranchName>,
        /// Further commits to insert, specified via Git references
        /// (commit ids, branch oder tag names, and other syntax like
        /// `HEAD^`).
        more_reference_names: Vec<GitReference>,
    },

    /// Take template definitions and commit from job specification
    /// files (e.g. to re-use files of failed jobs from queues, or
    /// edit manually)
    #[command(after_help = "  Note: more job‑setting options are available in the parent command!")]
    JobFiles {
        #[clap(flatten)]
        force_invalid_opt: ForceInvalidOpt,

        /// Override the commit id found in the file
        #[clap(long)]
        commit: Option<GitHash>,

        /// Path(s) to the JSON file(s) to insert. The format is the
        /// one used in the `~/.evobench/queues/` directories,
        /// except you can alternatively choose JSON5, RON, or one of
        /// the other formats shown in `config-formats` if the file
        /// has a corresponding file extension.
        paths: Vec<PathBuf>,
    },
}

fn insert_templates_with_references(
    run_config_bundle: &RunConfigBundle,
    insert_opts: InsertOpts,
    queues: &RunQueues,
    mut gwd: LocalOrRemoteGitWorkingDir,
    job_templates: &[JobTemplate],
    reference_names: BTreeSet<GitReference>,
) -> Result<usize> {
    let InsertOpts {
        insert_behaviour_opts:
            InsertBehaviourOpts {
                force_opt,
                quiet_opt,
                dry_run_opt,
            },
        insert_benchmarking_job_opts,
    } = insert_opts;

    // Do not forget to use the config entries! (XX how to improve the
    // code to enforce this?)
    let insert_benchmarking_job_opts = insert_benchmarking_job_opts
        .complete_with(&run_config_bundle.run_config.benchmarking_job_settings);

    let commits: Vec<Option<GitHash>> = gwd.resolve_references(reference_names)?;
    let commits: BTreeSet<GitHash> = commits.into_iter().filter_map(|v| v).collect();

    let benchmarking_jobs: Vec<BenchmarkingJob> = commits
        .into_iter()
        .map(|commit_id| {
            let benchmarking_job_opts = BenchmarkingJobOpts {
                insert_benchmarking_job_opts: insert_benchmarking_job_opts.clone(),
                commit_id,
            };
            benchmarking_job_opts.complete_jobs(job_templates)
        })
        .flatten()
        .collect();

    insert_jobs(
        benchmarking_jobs,
        run_config_bundle,
        dry_run_opt,
        force_opt,
        quiet_opt,
        queues,
    )
}

impl Insert {
    pub fn run(
        self,
        mut insert_opts: InsertOpts,
        run_config_bundle: &RunConfigBundle,
        queues: &RunQueues,
    ) -> Result<usize> {
        let conf = &run_config_bundle.run_config;

        match self {
            Insert::Templates {
                job_template_lists_name,
                local_or_remote,
                reference_names,
            } => {
                let job_templates = conf
                    .job_template_lists
                    .get(&*job_template_lists_name)
                    .ok_or_else(|| {
                        anyhow!(
                            "there is no entry under `job_template_lists_name` for name \
                             {job_template_lists_name:?} in config file at {:?}",
                            run_config_bundle.config_file.path()
                        )
                    })?;

                let reference_names: BTreeSet<GitReference> = reference_names.into_iter().collect();

                insert_opts
                    .insert_benchmarking_job_opts
                    .reason
                    .reason
                    .get_or_insert(format!("T {job_template_lists_name}"));

                let gwd = local_or_remote.load(run_config_bundle)?;
                insert_templates_with_references(
                    run_config_bundle,
                    insert_opts,
                    queues,
                    gwd,
                    job_templates,
                    reference_names,
                )
            }

            Insert::TemplatesOfBranch {
                branch_name,
                local_or_remote,
                reference_names,
            } => {
                let job_templates = conf
                    .remote_repository
                    .remote_branch_names_for_poll
                    .get(&branch_name)
                    .ok_or_else(|| {
                        anyhow!(
                            "there is no entry under \
                             `remote_repository.remote_branch_names_for_poll` \
                             for branch name {branch_name}"
                        )
                    })?;

                // Do *not* add branch_name to those!
                let reference_names: BTreeSet<GitReference> = reference_names.into_iter().collect();

                insert_opts
                    .insert_benchmarking_job_opts
                    .reason
                    .reason
                    .get_or_insert(format!("{} {branch_name}", local_or_remote.as_char()));

                let gwd = local_or_remote.load(run_config_bundle)?;
                insert_templates_with_references(
                    run_config_bundle,
                    insert_opts,
                    queues,
                    gwd,
                    job_templates,
                    reference_names,
                )
            }

            Insert::Branch {
                local_or_remote,
                branch_name,
                more_reference_names,
            } => {
                let mut gwd = local_or_remote.load(run_config_bundle)?;
                let branch_name = if let Some(branch_name) = branch_name {
                    branch_name
                } else {
                    gwd.get_branch_default()?.ok_or_else(|| {
                        anyhow!("{local_or_remote} Git repository has no default/current branch")
                    })?
                };
                info!("using {local_or_remote} branch {branch_name}");

                let job_templates = conf
                    .remote_repository
                    .remote_branch_names_for_poll
                    .get(&branch_name)
                    .ok_or_else(|| {
                        anyhow!(
                            "there is no entry under \
                             `remote_repository.remote_branch_names_for_poll` \
                             for branch name {branch_name}"
                        )
                    })?;

                let mut reference_names: BTreeSet<GitReference> =
                    more_reference_names.into_iter().collect();
                reference_names.insert(branch_name.to_reference());

                insert_opts
                    .insert_benchmarking_job_opts
                    .reason
                    .reason
                    .get_or_insert(format!("{} {branch_name}", local_or_remote.as_char()));

                insert_templates_with_references(
                    run_config_bundle,
                    insert_opts,
                    queues,
                    gwd,
                    job_templates,
                    reference_names,
                )
            }

            Insert::JobFiles {
                force_invalid_opt,
                commit,
                paths,
            } => {
                let InsertOpts {
                    insert_behaviour_opts:
                        InsertBehaviourOpts {
                            force_opt,
                            quiet_opt,
                            dry_run_opt,
                        },
                    insert_benchmarking_job_opts,
                } = insert_opts;

                let mut benchmarking_jobs = Vec::new();
                for path in &paths {
                    let mut job: BenchmarkingJob = if let Ok(backend) = backend_from_path(&path) {
                        backend.load_config_file(&path)?
                    } else {
                        serde_read_json(&path)?
                    };

                    job.check_and_init(
                        conf,
                        &insert_benchmarking_job_opts,
                        commit.as_ref(),
                        &force_invalid_opt,
                    )?;

                    benchmarking_jobs.push(job);
                }

                insert_jobs(
                    benchmarking_jobs,
                    run_config_bundle,
                    dry_run_opt,
                    force_opt,
                    quiet_opt,
                    queues,
                )
            }
        }
    }
}
