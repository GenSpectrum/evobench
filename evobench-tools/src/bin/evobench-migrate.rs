use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use evobench_tools::{
    info,
    run::{
        config::RunConfigBundle,
        global_app_state_dir::GlobalAppStateDir,
        insert_jobs::open_already_inserted,
        migrate::{migrate_already_inserted, migrate_queue},
        open_run_queues::open_run_queues,
    },
    utillib::get_terminal_width::get_terminal_width,
    utillib::logging::{LogLevelOpts, set_log_level},
};

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(term_width = get_terminal_width(4))]
/// Database migration for evobench: update storage format for jobs in
/// queues. Run this when you're getting deserialisation errors from
/// `evobench`, or when you know that the data structures have
/// changed and will cause errors.
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpts,

    /// Override the path to the config file (default: the paths
    /// `~/.evobench.*` where a single one exists where the `*` is
    /// the suffix for one of the supported config file formats (run
    /// `config-formats` to get the list), and if those are missing,
    /// use compiled-in default config values)
    #[clap(long)]
    config: Option<PathBuf>,

    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// Run the migration
    Run,
}

fn main() -> Result<()> {
    let Opts {
        log_level,
        config,
        subcommand,
    } = Opts::parse();

    set_log_level(log_level.try_into()?);

    let config = config.map(Into::into);

    match subcommand {
        SubCommand::Run => {
            let run_config_bundle = RunConfigBundle::load(
                config,
                |msg| bail!("can't load config: {msg}"),
                GlobalAppStateDir::new()?,
            )?;

            info!("migrating the queues");
            {
                let (queues, regenerate_index_files) =
                    open_run_queues(&run_config_bundle.shareable)?;
                for queue in queues.all_queues() {
                    info!("migrating queue {:?}", queue.file_name.as_str());
                    let n = migrate_queue(queue)?;
                    info!("migrated {n} items in queue {:?}", queue.file_name.as_str());
                }
                regenerate_index_files.run_one();
            }

            info!("migrating the already_inserted table");
            let already_inserted =
                open_already_inserted(&run_config_bundle.shareable.global_app_state_dir)?;
            let n = migrate_already_inserted(&already_inserted)?;
            info!("migrated {n} items in the already_inserted table");
        }
    }

    Ok(())
}
