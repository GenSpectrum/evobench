use std::io::stdout;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::git::git_log_commits;
use evobench_evaluator::git::GitGraph;
use evobench_evaluator::git::GitHistory;
use evobench_evaluator::serde::git_branch_name::GitBranchName;
use evobench_evaluator::utillib::logging::set_log_level;
use evobench_evaluator::utillib::logging::LogLevelOpt;
use kstring::KString;

fn graph(directory: &Path, branch: &GitBranchName) -> Result<()> {
    let commits = git_log_commits(directory, branch.as_str())?;
    let graph = GitGraph::new();
    let history = GitHistory::from_commits(
        KString::from_ref(branch.as_str()),
        commits.iter().rev(),
        &mut graph.lock(),
    );
    dbg!(graph.lock().commits().len());
    dbg!(history.entry_commit_id);
    let id = history.entry_commit_id;
    {
        let commit = { &graph.lock()[id] };
        eprintln!("entry commit: {commit:?}");
    }

    let ids = graph.lock().history_from(history.entry_commit_id);
    let sorted_ids = graph.lock().sorted_by(&ids, |commit| commit.committer_time);
    {
        let graph_lock = graph.lock();
        let commits = graph_lock.ids_as_commits(&sorted_ids);
        let mut out = stdout().lock();
        for commit in commits {
            let commit = commit.to_hashes(&graph_lock);
            writeln!(&mut out, "{commit}")?;
        }
    }

    Ok(())
}

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

    graph(
        directory.unwrap_or_else(|| PathBuf::from(".")).as_ref(),
        &reference,
    )?;

    Ok(())
}
