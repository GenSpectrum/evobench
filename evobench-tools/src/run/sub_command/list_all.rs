use std::{borrow::Cow, fmt::Display, io::stdout, time::SystemTime};

use anyhow::Result;

use crate::{
    key::{BenchmarkingJobParameters, RunParameters},
    run::{
        config::{BenchmarkingCommand, RunConfigBundle},
        insert_jobs::open_already_inserted,
        sub_command::list::TARGET_NAME_WIDTH,
    },
    serde::date_and_time::system_time_to_rfc3339,
    terminal_table::{TerminalTable, TerminalTableOpts, TerminalTableTitle},
};

#[derive(Debug, Clone, clap::Args)]
pub struct ListAllOpts {
    #[clap(flatten)]
    terminal_table_opts: TerminalTableOpts,
}

impl ListAllOpts {
    pub fn run(self, run_config_bundle: &RunConfigBundle) -> Result<()> {
        let Self {
            terminal_table_opts,
        } = self;

        let already_inserted = open_already_inserted(&run_config_bundle.global_app_state_dir)?;

        let mut flat_jobs: Vec<(BenchmarkingJobParameters, SystemTime)> = Vec::new();
        for job in already_inserted
            .keys(false, None)?
            .map(|hash| -> Result<_> {
                let hash = hash?;
                Ok(already_inserted.get(&hash)?)
            })
            .filter_map(|r| r.transpose())
        {
            let (params, insertion_times) = job?;
            for t in insertion_times {
                flat_jobs.push((params.clone(), t));
            }
        }
        flat_jobs.sort_by_key(|v| v.1);
        let mut table = TerminalTable::start(
            &[38, 43, TARGET_NAME_WIDTH],
            &[
                TerminalTableTitle {
                    text: Cow::Borrowed("Insertion time"),
                    span: 1,
                },
                TerminalTableTitle {
                    text: Cow::Borrowed("Commit id"),
                    span: 1,
                },
                TerminalTableTitle {
                    text: Cow::Borrowed("Target name"),
                    span: 1,
                },
                TerminalTableTitle {
                    text: Cow::Borrowed("Custom parameters"),
                    span: 1,
                },
            ],
            None,
            terminal_table_opts,
            stdout().lock(),
        )?;
        for (params, insertion_time) in flat_jobs {
            let t = system_time_to_rfc3339(insertion_time, true);
            let BenchmarkingJobParameters {
                run_parameters,
                command,
            } = params;
            let RunParameters {
                commit_id,
                custom_parameters,
            } = &*run_parameters;
            let BenchmarkingCommand {
                target_name,
                subdir: _,
                command: _,
                arguments: _,
                pre_exec_bash_code: _,
            } = &*command;

            let values: &[&dyn Display] =
                &[&t, &commit_id, &target_name.as_str(), &custom_parameters];
            table.write_data_row(values, None)?;
        }
        drop(table.finish()?);
        Ok(())
    }
}
