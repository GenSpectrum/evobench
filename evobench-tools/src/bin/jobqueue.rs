use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{exit, Command},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

use evobench_tools::{
    get_terminal_width::get_terminal_width,
    info_if,
    key_val_fs::{
        key_val::{Entry, KeyValConfig},
        queue::{Queue, QueueGetItemOptions, QueueIterationOptions},
    },
    safe_string::SafeString,
};

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
/// Schedule jobs to run, which is external commands with
/// arguments. This is more of an example of the use of the queue
/// library, but could be useful (and tweaked to be more useful) for
/// scheduling jobs from shell scripts. The Rust library allows to
/// make queues of any serializable type, here the type is just a list
/// of command name or path and argument strings.
struct Opts {
    /// The path to the directory holding the queue
    path: PathBuf,

    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// List the current jobs
    List,
    /// Get an exclusive lock on the queue then run a command while holding the lock
    WithExclusiveLock {
        /// Write to stderr what is being done
        #[clap(long)]
        verbose: bool,

        command: PathBuf,
        arguments: Vec<OsString>,
    },
    /// Get a shared lock on the queue then run a command while holding the lock
    WithSharedLock {
        /// Write to stderr what is being done
        #[clap(long)]
        verbose: bool,

        command: PathBuf,
        arguments: Vec<OsString>,
    },
    /// Insert a job
    Insert {
        /// The arguments to be passed to the program when using the
        /// `run` subcommand
        arguments: Vec<SafeString>,
    },
    /// Process the entries in the queue
    Run {
        /// Write to stderr what is being done
        #[clap(long)]
        verbose: bool,

        /// Whether to delete the entry *before* acting on it. By
        /// default, the entry is only deleted after the action was
        /// run successfully.
        #[clap(long)]
        delete_first: bool,

        /// Do not lock the queue entry while running the
        /// program. Careful, this allows for multiple executions of
        /// the same entry, in parallel!
        #[clap(long)]
        no_lock: bool,

        /// Instead of emptying the current contents of the queue,
        /// process it forever, waiting for new entries to arrive.
        #[clap(long)]
        wait: bool,

        /// If an entry is locked, stop with an "Error: lock taken on"
        /// message (by default, blocks until it gets the lock).
        #[clap(long)]
        error_when_locked: bool,

        /// Only process at most those many entries (default: all of
        /// them).
        #[clap(long)]
        limit: Option<usize>,

        /// Program name or path to run
        program: PathBuf,
        /// Program arguments to be prepended to the ones from the queue
        first_arguments: Vec<SafeString>,
    },
}

fn run_command<I, S>(command: impl AsRef<Path>, arguments: I) -> Result<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    (|| -> Result<_> {
        let mut child = Command::new(command.as_ref()).args(arguments).spawn()?;
        let res = child.wait()?;
        let exit_code = if res.success() {
            0
        } else if let Some(code) = res.code() {
            code
        } else {
            // ?
            113
        };
        Ok(exit_code)
    })()
    .with_context(|| anyhow!("running {:?}", command.as_ref()))
}

fn main() -> Result<()> {
    let Opts { path, subcommand } = Opts::parse();

    let get_filename = |entry: &Entry<_, _>| -> Result<String> {
        let file_name = entry.file_name();
        Ok(file_name
            .to_str()
            .ok_or_else(|| anyhow!("filename that cannot be decoded as UTF-8: {file_name:?}"))?
            .to_string())
    };

    let open_queue = |create_dir_if_not_exists| -> Result<Queue<Vec<SafeString>>> {
        Ok(Queue::open(
            &path,
            KeyValConfig {
                create_dir_if_not_exists,
                ..KeyValConfig::default()
            },
        )?)
    };
    match subcommand {
        SubCommand::WithExclusiveLock {
            verbose,
            command,
            arguments,
        } => {
            let queue = open_queue(true)?;
            let lock = queue.lock_exclusive()?;
            info_if!(verbose, "got lock");
            let exit_code = run_command(command, arguments)?;
            drop(lock);
            exit(exit_code)
        }
        SubCommand::WithSharedLock {
            verbose,
            command,
            arguments,
        } => {
            let queue = open_queue(true)?;
            let lock = queue.lock_shared()?;
            info_if!(verbose, "got lock");
            let exit_code = run_command(command, arguments)?;
            drop(lock);
            exit(exit_code)
        }
        SubCommand::List => {
            let queue = open_queue(false)?;
            for entry in queue.sorted_entries(false, None, false)? {
                let mut entry = entry?;
                let file_name = get_filename(&entry)?;
                let key = entry.key()?;
                let val = entry.get()?;
                let locking = entry
                    .take_lockable_file()
                    .expect("not taken before")
                    .get_lock_status()?;
                println!("{file_name} ({key})\t{locking}\t{val:?}");
            }
        }
        SubCommand::Insert { arguments } => {
            let queue = open_queue(true)?;
            queue.push_front(&arguments)?
        }
        SubCommand::Run {
            no_lock,
            limit,
            error_when_locked,
            wait,
            verbose,
            delete_first,
            program,
            first_arguments,
        } => {
            let queue = open_queue(true)?;
            let opts = QueueIterationOptions {
                wait,
                stop_at: None,
                reverse: false,
                get_item_opts: QueueGetItemOptions {
                    no_lock,
                    error_when_locked,
                    verbose,
                    delete_first,
                },
            };
            let items = queue.items(opts);
            let items: Box<dyn Iterator<Item = _>> = if let Some(limit) = limit {
                Box::new(items.take(limit))
            } else {
                Box::new(items)
            };

            for item_value in items {
                let (item, mut queue_arguments) = item_value?;

                let mut arguments = first_arguments.clone();
                arguments.append(&mut queue_arguments);

                info_if!(verbose, "running {program:?} with arguments {arguments:?}");
                let mut command = Command::new(&program)
                    .args(arguments)
                    .spawn()
                    .with_context(|| anyhow!("running program {program:?}"))?;
                let status = command.wait();
                if status.is_err() {
                    bail!("command {program:?} exited with code {status:?}")
                }
                if !delete_first {
                    item.delete()?;
                }
            }
        }
    }
    Ok(())
}
