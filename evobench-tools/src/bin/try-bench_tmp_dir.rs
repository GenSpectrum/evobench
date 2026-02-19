use std::{fs::File, time::Duration};

use anyhow::Result;
use cj_path_util::path_util::AppendToPath;
use clap::Parser;
use evobench_tools::{
    io_utils::temporary_file::TemporaryFile, run::bench_tmp_dir::bench_tmp_dir,
    utillib::get_terminal_width::get_terminal_width,
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
    #[clap(short, long, default_value = "30")]
    duration: u64,
}

fn main() -> Result<()> {
    let Opts { duration } = Opts::parse();
    {
        let bench_tmp_dir = bench_tmp_dir()?;
        dbg!(&bench_tmp_dir);

        let pid = getpid();
        // File for evobench library output
        let evobench_log =
            TemporaryFile::from((&bench_tmp_dir).append(format!("evobench-{pid}.log")));
        // File for other output, for optional use by target application
        let bench_output_log =
            TemporaryFile::from((&bench_tmp_dir).append(format!("bench-output-{pid}.log")));

        dbg!(evobench_log.path());
        dbg!(bench_output_log.path());

        let p = evobench_log.path();
        File::create(p)?;

        let _ = std::fs::remove_file(evobench_log.path());
        let _ = std::fs::remove_file(bench_output_log.path());

        File::create(p)?;

        eprintln!("Sleeping {duration} seconds.");
        std::thread::sleep(Duration::from_secs(duration));
    }
    eprintln!("Done with it, sleeping 3 more seconds.");

    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
