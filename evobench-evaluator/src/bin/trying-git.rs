use std::io::stdout;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use cj_path_util::unix::fixup_path::CURRENT_DIRECTORY;
use clap::Parser;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::git::GitGraph;
use evobench_evaluator::serde::git_branch_name::GitBranchName;
use evobench_evaluator::utillib::logging::set_log_level;
use evobench_evaluator::utillib::logging::LogLevelOpt;

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpt,

    /// The git branch name to get the history from
    reference: GitBranchName,

    /// The directory to the Git repository to get the history from
    directory: Option<PathBuf>,
}

fn main() -> Result<()> {
    let Opts {
        log_level,
        reference,
        directory,
    } = Opts::parse();

    set_log_level(log_level.try_into()?);

    let directory: &Path = directory.as_deref().unwrap_or(*CURRENT_DIRECTORY);

    let graph = GitGraph::new();
    let history = graph
        .lock()
        .add_history_from_dir_ref(directory, reference.as_str())?;
    dbg!(graph.lock().commits().len());
    dbg!(history.commit_id);
    let id = history.commit_id;
    {
        let commit = { &graph.lock()[id] };
        eprintln!("entry commit: {commit:?}");
    }

    let ids = graph.lock().history_as_btreeset_from(history.commit_id);
    let sorted_ids = graph
        .lock()
        .sorted_by(&ids, |ecommit| ecommit.commit.committer_time);
    {
        let graph_lock = graph.lock();
        let ecommits = graph_lock.ids_as_commits(&sorted_ids);
        let mut out = BufWriter::new(stdout().lock());
        for ecommit in ecommits {
            let commit = ecommit.with_ids_as_hashes(&graph_lock);
            writeln!(&mut out, "{commit}")?;
        }
        out.flush()?;
    }

    Ok(())
}
