use std::fmt::Display;
use std::io::{stdout, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::log_file::LogData;
use evobench_evaluator::log_message::Timing;
use evobench_evaluator::pn_summary::{LogDataIndex, ScopeId};
use evobench_evaluator::stats::Stats;
use evobench_evaluator::times::ToStringMilliseconds;

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

const TILE_COUNT: usize = 11;

fn scopestats<T: Into<u64> + From<u64>>(
    log_data_index: &LogDataIndex,
    scopes: &[ScopeId],
    extract: impl Fn(&Timing) -> T,
) -> Stats<T, TILE_COUNT> {
    let vals: Vec<_> = scopes
        .into_iter()
        .map(|scope_id| -> u64 {
            let scope = scope_id.get_from_db(log_data_index);
            extract(scope.end()).into() - extract(scope.start()).into()
        })
        .collect();
    Stats::from_values(vals)
}

fn stats<T: Into<u64> + From<u64> + ToStringMilliseconds + Display>(
    log_data_index: &LogDataIndex,
    extract_name: &str,
    pn: &str,
    extract: impl Fn(&Timing) -> T,
    mut out: impl Write,
) -> Result<()> {
    let s: Stats<T, TILE_COUNT> = scopestats(
        log_data_index,
        log_data_index.scopes_by_pn(&pn).unwrap(),
        extract,
    );
    eprintln!("{pn:?} => {s}");
    s.print_tsv_line(&mut out, &[extract_name, pn])?;
    Ok(())
}

fn stats_all_probes<T: Into<u64> + From<u64> + ToStringMilliseconds + Display>(
    mut out: impl Write,
    log_data_index: &LogDataIndex,
    extract_name: &str,
    extract: impl Fn(&Timing) -> T,
) -> Result<()> {
    eprintln!("----{extract_name}-----------------------------------------------------------------------------------");
    for pn in log_data_index.probe_names() {
        stats(log_data_index, extract_name, pn, &extract, &mut out)?;
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
            let log_data_index = LogDataIndex::from_logdata(&data)?;
            // dbg!(byscope);
            Stats::<bool, TILE_COUNT>::print_tsv_header(&mut out, &["field", "probe name"])?;
            stats_all_probes(&mut out, &log_data_index, "real time", |timing: &Timing| {
                timing.r
            })?;
            stats_all_probes(&mut out, &log_data_index, "cpu time", |timing: &Timing| {
                timing.u
            })?;
            stats_all_probes(&mut out, &log_data_index, "sys time", |timing: &Timing| {
                timing.s
            })?;
        }
    }

    Ok(())
}
