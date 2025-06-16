use anyhow::{anyhow, Result};
use clap::Parser;

use std::path::PathBuf;

use evobench_evaluator::{
    get_terminal_width::get_terminal_width,
    key_val_fs::{
        key_val::{Entry, KeyValConfig, KeyValSync},
        queue::Queue,
    },
    load_config_file::LoadConfigFile,
    run::{benchmark_job::BenchmarkJob, config::RunConfig},
};

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
/// Schedule (and query?) benchmarking jobs.
struct Opts {
    /// Path to the config file, by default `~/.evobench-run.rs`, and
    /// if that is missing, using compiled-in default config values.
    #[clap(long)]
    config: Option<PathBuf>,

    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// List the current jobs
    List,
    /// Insert a job
    Insert {
        #[clap(flatten)]
        benchmark_job: BenchmarkJob,
    },
}

fn main() -> Result<()> {
    let Opts { config, subcommand } = Opts::parse();

    // COPY-PASTE from List action in jobqueue.rs
    let get_filename = |entry: &Entry<_, _>| -> Result<String> {
        let file_name = entry.file_name();
        Ok(file_name
            .to_str()
            .ok_or_else(|| anyhow!("filename that cannot be decoded as UTF-8: {file_name:?}"))?
            .to_string())
    };

    let conf = RunConfig::load_config(config)?;
    let run_queue_path = conf.run_queue_path()?;

    let mut queue = Queue::<BenchmarkJob>::open(
        &run_queue_path,
        KeyValConfig {
            sync: KeyValSync::All,
            create_dir_if_not_exists: true,
        },
    )?;

    match subcommand {
        SubCommand::List => {
            // COPY-PASTE from List action in jobqueue.rs, except
            // printing the job in :#? view on the next line.
            for entry in queue.sorted_entries(false) {
                let mut entry = entry?;
                let file_name = get_filename(&entry)?;
                let key = entry.key()?;
                let val = entry.get()?;
                let locking = entry
                    .take_lockable_file()
                    .expect("not taken before")
                    .lock_status()?;
                println!("{file_name} ({key})\t{locking}\n{val:#?}");
            }
        }
        SubCommand::Insert { benchmark_job } => {
            queue.push_front(&benchmark_job)?;
        }
    }

    Ok(())
}
