use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::log_file::LogData;
use evobench_evaluator::log_message::Timing;
use evobench_evaluator::pn_summary::{ByScope, Scope};
use evobench_evaluator::stats::Stats;

include!("../../include/evobench_version.rs");

const PROGRAM_NAME: &str = "evobench-evaluator";

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
struct Opts {
    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Print version
    Version,
    /// Read a file
    Read { path: PathBuf },
}

fn scopestats(scopes: &[Scope], extract: impl Fn(&Timing) -> u64) -> Stats {
    let vals: Vec<_> = scopes
        .into_iter()
        .map(|scope| extract(&scope.end) - extract(&scope.start))
        .collect();
    Stats::from_values(vals)
}

fn stats(byscope: &ByScope, pn: &str, extract: impl Fn(&Timing) -> u64) {
    let s = scopestats(byscope.scopes_by_pn(pn).unwrap(), extract);
    println!("{pn:?} => {s:?}");
}

fn stats_all_probes(byscope: &ByScope, extract_name: &str, extract: impl Fn(&Timing) -> u64) {
    println!("----{extract_name}-----------------------------------------------------------------------------------");
    for pn in byscope.probe_names() {
        stats(byscope, pn, &extract);
    }
}

fn main() -> Result<()> {
    let opts: Opts = Opts::parse();
    match &opts.command {
        Command::Version => println!("{PROGRAM_NAME} version {EVOBENCH_VERSION}"),
        Command::Read { path } => {
            let data = LogData::read_file(path)?;
            let byscope = ByScope::from_logdata(&data)?;
            // dbg!(byscope);
            stats_all_probes(&byscope, "real time", |timing: &Timing| timing.r.to_nsec());
            stats_all_probes(&byscope, "cpu time", |timing: &Timing| timing.u.to_nsec());
            stats_all_probes(&byscope, "sys time", |timing: &Timing| timing.s.to_nsec());
        }
    }

    Ok(())
}
