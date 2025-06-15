use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{exit, Command},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

use evobench_evaluator::{
    get_terminal_width::get_terminal_width,
    key_val_fs::{
        key_val::{Entry, KeyValConfig},
        queue::Queue,
    },
};

macro_rules! info_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprintln!($($arg)*);
        }
    }
}

macro_rules! info_noln_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprint!($($arg)*);
        }
    }
}

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
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
        arguments: Vec<OsString>,
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
        error_on_lock: bool,

        /// Only process at most those many entries (default: all of
        /// them).
        #[clap(long)]
        limit: Option<u64>,

        /// Sleep for that many seconds between steps (for debugging
        /// purposes).
        #[clap(long)]
        sleep: Option<u64>,

        /// Program name or path to run
        program: PathBuf,
        /// Program arguments to be prepended to the ones from the queue
        first_arguments: Vec<OsString>,
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

    let mut queue: Queue<Vec<OsString>> = Queue::open(&path, KeyValConfig::default())?;
    match subcommand {
        SubCommand::WithExclusiveLock {
            verbose,
            command,
            arguments,
        } => {
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
            let lock = queue.lock_shared()?;
            info_if!(verbose, "got lock");
            let exit_code = run_command(command, arguments)?;
            drop(lock);
            exit(exit_code)
        }
        SubCommand::List => {
            for entry in queue.sorted_entries(false) {
                let mut entry = entry?;
                let file_name = get_filename(&entry)?;
                let key = entry.key()?;
                let val = entry.get()?;
                let locking = entry
                    .take_lockable_file()
                    .expect("not taken before")
                    .lock_status()?;
                println!("{file_name} ({key})\t{locking}\t{val:?}");
            }
        }
        SubCommand::Insert { arguments } => queue.push_front(&arguments)?,
        SubCommand::Run {
            no_lock,
            limit,
            sleep,
            error_on_lock,
            wait,
            verbose,
            delete_first,
            program,
            first_arguments,
        } => {
            let num_processed = AtomicU64::new(0);
            let mut entries = None;
            loop {
                if let Some(limit) = &limit {
                    if num_processed.load(Ordering::SeqCst) >= *limit {
                        info_if!(verbose, "reached given limit, stop processing the queue");
                        break;
                    }
                }
                if entries.is_none() {
                    entries = Some(queue.sorted_entries(wait))
                }
                if let Some(entry) = entries.as_mut().expect("set 2 lines above").next() {
                    let mut entry = entry?;
                    let mut arguments = first_arguments.clone();
                    {
                        let mut queue_arguments = entry.get()?;
                        arguments.append(&mut queue_arguments);
                    }

                    let perhaps_sleep = || {
                        if let Some(secs) = &sleep {
                            info_noln_if!(verbose, "sleeping {secs} seconds...");
                            thread::sleep(Duration::from_secs(*secs));
                            info_if!(verbose, "done.");
                        }
                    };
                    let run = |entry: &mut Entry<_, _>| -> Result<()> {
                        let mut delete = || -> Result<()> {
                            let deleted = entry.delete()?;
                            info_if!(verbose, "deleted entry: {:?}", deleted);
                            Ok(())
                        };

                        if delete_first {
                            delete()?;
                        }
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
                            delete()?;
                        }
                        num_processed.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    };
                    if no_lock {
                        run(&mut entry)?;
                        perhaps_sleep();
                    } else {
                        let mut lockable = entry
                            .take_lockable_file()
                            .expect("we have not taken it yet");
                        let lock = if error_on_lock {
                            lockable.try_lock_exclusive()?.ok_or_else(|| {
                                let file_name_str = match get_filename(&entry) {
                                    Ok(v) => format!("{v:?}"),
                                    Err(e) => format!("-- error retrieving file name: {e:?}"),
                                };
                                anyhow!("lock is already taken on {file_name_str}")
                            })?
                        } else {
                            lockable.lock_exclusive()?
                        };
                        info_if!(verbose, "got lock");
                        let exists = entry.exists();
                        if exists {
                            run(&mut entry)?;
                            perhaps_sleep();
                        } else {
                            info_if!(verbose, "but entry now deleted by another process");
                        }
                        drop(lock);
                        info_if!(verbose, "released lock");
                        perhaps_sleep();
                    }
                } else {
                    entries = None
                }
            }
        }
    }
    Ok(())
}
