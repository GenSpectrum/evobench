use std::fmt::Display;
use std::io::{stdout, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::log_file::LogData;
use evobench_evaluator::log_message::Timing;
use evobench_evaluator::pn_summary::ByScope;
use evobench_evaluator::scope::Scope;
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

fn scopestats<T: Into<u64> + From<u64>>(
    scopes: &[Scope],
    extract: impl Fn(&Timing) -> T,
) -> Stats<T> {
    let vals: Vec<_> = scopes
        .into_iter()
        .map(|scope| -> u64 { extract(&scope.end).into() - extract(&scope.start).into() })
        .collect();
    Stats::from_values(vals, 11)
}

fn stats<T: Into<u64> + From<u64> + Display>(
    byscope: &ByScope,
    extract_name: &str,
    pn: &str,
    extract: impl Fn(&Timing) -> T,
    mut out: impl Write,
) -> Result<()> {
    let s: Stats<T> = scopestats(byscope.scopes_by_pn(pn).unwrap(), extract);
    // println!("{pn:?} => {s}");
    s.print_tsv_line(&mut out, &[extract_name, pn])?;
    Ok(())
}

fn stats_all_probes<T: Into<u64> + From<u64> + Display>(
    mut out: impl Write,
    byscope: &ByScope,
    extract_name: &str,
    extract: impl Fn(&Timing) -> T,
) -> Result<()> {
    // println!("----{extract_name}-----------------------------------------------------------------------------------");
    for pn in byscope.probe_names() {
        stats(byscope, extract_name, pn, &extract, &mut out)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut out = stdout().lock();
    let opts: Opts = Opts::parse();
    match &opts.command {
        Command::Version => println!("{PROGRAM_NAME} version {EVOBENCH_VERSION}"),
        Command::Read { path } => {
            let data = LogData::read_file(path)?;
            let byscope = ByScope::from_logdata(&data)?;
            // dbg!(byscope);
            Stats::<bool>::print_tsv_header(&mut out, &["field", "probe name"])?;
            stats_all_probes(&mut out, &byscope, "real time", |timing: &Timing| timing.r)?;
            stats_all_probes(&mut out, &byscope, "cpu time", |timing: &Timing| timing.u)?;
            stats_all_probes(&mut out, &byscope, "sys time", |timing: &Timing| timing.s)?;
        }
    }

    Ok(())
}
