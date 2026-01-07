use std::{path::PathBuf, time::Duration};

use anyhow::{anyhow, Context, Result};
use clap::Parser;

use evobench_evaluator::{get_terminal_width::get_terminal_width, polling_signals::PollingSignals};

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
/// Schedule and query benchmarking jobs.
struct Opts {
    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    Poll {
        file: PathBuf,
    },
    Send {
        /// Number of signals
        #[clap(short, long, default_value = "1")]
        n: usize,

        file: PathBuf,
    },
}

fn main() -> Result<()> {
    let Opts { subcommand } = Opts::parse();
    match subcommand {
        SubCommand::Poll { file } => {
            let mut signal =
                PollingSignals::open(&file).with_context(|| anyhow!("opening {file:?}"))?;
            loop {
                let n = signal.get_number_of_signals();
                if n > 0 {
                    println!("got {n} signal(s)");
                }

                std::thread::sleep(Duration::from_secs(1));
            }
        }
        SubCommand::Send { n, file } => {
            let mut signal =
                PollingSignals::open(&file).with_context(|| anyhow!("opening {file:?}"))?;
            for _ in 0..n {
                signal.send_signal();
            }
            Ok(())
        }
    }
}
