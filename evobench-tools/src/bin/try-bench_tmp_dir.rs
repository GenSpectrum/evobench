use std::{fs::File, time::Duration};

use anyhow::Result;
use cj_path_util::path_util::AppendToPath;
use clap::Parser;
use evobench_tools::{
    run::bench_tmp_dir::bench_tmp_dir,
    utillib::{
        cleanup_daemon::{DeletionItem, FileCleanupHandler},
        get_terminal_width::get_terminal_width,
        logging::{LogLevel, LogLevelOpts, set_log_level},
    },
};
use nix::unistd::getpid;

#[derive(clap::Parser, Debug)]
#[command(
    next_line_help = true,
    term_width = get_terminal_width(4),
    allow_hyphen_values = true,
    bin_name = "evobench",
)]
/// Test the bench_tmp_dir facility against systemd
struct Opts {
    /// How long to sleep before exiting (seconds)
    #[clap(long, default_value = "30")]
    duration: u64,

    #[clap(flatten)]
    log_level_opts: LogLevelOpts,

    /// Alternative to --quiet / --verbose / --debug for setting the
    /// log-level (an error is reported if both are given and they
    /// don't agree)
    #[clap(long)]
    log_level: Option<LogLevel>,
}

fn main() -> Result<()> {
    let Opts {
        duration,
        log_level_opts,
        log_level,
    } = Opts::parse();

    let log_level = log_level_opts.xor_log_level(log_level)?;
    set_log_level(log_level);

    let file_cleanup_handler = FileCleanupHandler::start()?;

    {
        let bench_tmp_dir = bench_tmp_dir()?;
        dbg!(&bench_tmp_dir);

        let pid = getpid();
        // File for evobench library output
        let evobench_log = file_cleanup_handler.register_temporary_file(DeletionItem::File(
            (&bench_tmp_dir)
                .append(format!("evobench-{pid}.log"))
                .into(),
        ))?;
        // File for other output, for optional use by target application
        let bench_output_log = file_cleanup_handler.register_temporary_file(DeletionItem::File(
            (&bench_tmp_dir)
                .append(format!("bench-output-{pid}.log"))
                .into(),
        ))?;

        dbg!(&*evobench_log);
        dbg!(&*bench_output_log);

        File::create(&evobench_log)?;

        let _ = std::fs::remove_file(&evobench_log);
        let _ = std::fs::remove_file(&bench_output_log);

        File::create(&evobench_log)?;

        eprintln!("Sleeping {duration} seconds.");
        std::thread::sleep(Duration::from_secs(duration));
    }
    eprintln!("Done with it, sleeping 3 more seconds.");

    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
