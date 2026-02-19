use std::{
    io::{BufWriter, Write, stderr, stdout},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};

use crate::run::working_directory_pool::{
    WdAllowBareOpt, WorkingDirectoryIdOpt, WorkingDirectoryPool,
};

#[derive(Debug, clap::Args)]
pub struct LogOrLogf {
    /// Instead of opening the last log file, show a list of all
    /// log files for the given working directory, sorted by run
    /// start time (newest at the bottom)
    #[clap(short, long)]
    list: bool,

    #[clap(flatten)]
    allow_bare: WdAllowBareOpt,

    /// The ID of the working direcory for which to show the (last)
    /// log file(s)
    id: WorkingDirectoryIdOpt,
}

impl LogOrLogf {
    pub fn run(self, logf: bool, working_directory_pool: &WorkingDirectoryPool) -> Result<()> {
        let Self {
            list,
            allow_bare,
            id,
        } = self;
        let id = id.to_working_directory_id(allow_bare)?;

        let working_directory_path =
            if let Some(wd) = working_directory_pool.get_working_directory(id) {
                wd.working_directory_path()
            } else {
                let mut out = BufWriter::new(stderr().lock());
                writeln!(
                    &mut out,
                    "NOTE: working directory with id {id} does not exist. \
                     Looking for log files anyway."
                )?;
                out.flush()?;
                if !list {
                    std::thread::sleep(Duration::from_millis(1400));
                }
                working_directory_pool.get_working_directory_path(id)
            };

        if list {
            let mut out = BufWriter::new(stdout().lock());
            for (standard_log_path, _run_id) in working_directory_path.standard_log_paths()? {
                out.write_all(standard_log_path.as_os_str().as_bytes())?;
                out.write_all(b"\n")?;
            }
            out.flush()?;
        } else {
            let (standard_log_path, _run_id) = working_directory_path
                .last_standard_log_path()?
                .ok_or_else(|| anyhow!("could not find a log file for working directory {id}"))?;

            if logf {
                let mut cmd = Command::new("tail");
                cmd.arg("-F");
                cmd.arg("--");
                cmd.arg(standard_log_path);
                return Err(cmd.exec()).with_context(|| anyhow!("executing {cmd:?}"));
            } else {
                let pager = match std::env::var("PAGER") {
                    Ok(s) => s,
                    Err(e) => match e {
                        std::env::VarError::NotPresent => "less".into(),
                        _ => bail!("can't decode PAGER env var: {e:#}"),
                    },
                };

                let mut cmd = Command::new(&pager);
                cmd.arg(standard_log_path);
                return Err(cmd.exec()).with_context(|| anyhow!("executing pager {pager:?}"));
            }
        }
        Ok(())
    }
}
