use std::{
    borrow::Cow,
    env,
    io::{self, IsTerminal, stdout},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Local};
use lazy_static::lazy_static;
use yansi::{Color, Style};

use crate::{
    config_file::ron_to_string_pretty,
    get_terminal_width::get_terminal_width,
    key_val_fs::key_val::Entry,
    lockable_file::LockStatus,
    run::{
        config::RunConfig,
        output_directory_structure::{KeyDir, ToPath},
        run_queue::RunQueue,
        run_queues::RunQueues,
        working_directory::Status,
        working_directory_pool::WorkingDirectoryPoolBaseDir,
    },
    terminal_table::{TerminalTable, TerminalTableOpts, TerminalTableTitle},
    utillib::arc::CloneArc,
};

pub const TARGET_NAME_WIDTH: usize = 14;

lazy_static! {
    static ref UNICODE_IS_FINE: bool = (|| -> Option<bool> {
        let term = env::var_os("TERM")?;
        let lang = env::var_os("LANG")?;
        let lang = lang.to_str()?;
        Some(term.as_bytes().starts_with(b"xterm") && lang.contains("UTF-8"))
    })()
    .unwrap_or(false);
}

#[derive(Debug, Clone, Copy, clap::Subcommand)]
pub enum ParameterPathKind {
    /// Relative from the output base directory (default)
    Relative,
    /// Full local file system path
    Full,
    /// URL for access via the web
    Url,
}

impl Default for ParameterPathKind {
    fn default() -> Self {
        Self::Relative
    }
}

#[derive(Debug, Clone, Copy, clap::Subcommand)]
pub enum ParameterView {
    /// Show separate `Commit_id`, `Target_name`, `Custom_parameters`
    /// columns
    Separated,
    /// Show a single column that represents a path to the job in the
    /// outputs directory.
    Path {
        #[clap(subcommand)]
        kind: Option<ParameterPathKind>,
    },
}

#[derive(Debug, Clone, clap::Args)]
pub struct ListOpts {
    #[clap(flatten)]
    terminal_table_opts: TerminalTableOpts,

    /// Show details, not just one item per line
    #[clap(short, long)]
    verbose: bool,

    /// Show all jobs in the extra queues (done and failures); by
    /// default, only the last `view_jobs_max_len` jobs are shown
    /// as stated in the QueuesConfig.
    #[clap(short, long)]
    all: bool,

    /// How to show the job parameters
    #[clap(subcommand)]
    parameter_view: ParameterView,
}

fn table_with_titles<'v, 's, O: io::Write + IsTerminal>(
    titles: &'s [TerminalTableTitle],
    style: Option<Style>,
    terminal_table_opts: &TerminalTableOpts,
    out: O,
    verbose: bool,
    view: ParameterView,
) -> Result<TerminalTable<'v, 's, O>> {
    let insertion_time_width = if verbose { 82 } else { 37 };
    let widths =
    //     t                    R pr WD reason commit target
        &[insertion_time_width, 3, 6, 5, 25, 42, TARGET_NAME_WIDTH];
    let widths = match view {
        ParameterView::Separated => widths,
        ParameterView::Path { kind: _ } => &widths[0..5],
    };
    TerminalTable::start(widths, titles, style, terminal_table_opts.clone(), out)
}

impl ListOpts {
    pub fn run(
        self,
        conf: &RunConfig,
        working_directory_base_dir: &Arc<WorkingDirectoryPoolBaseDir>,
        queues: &RunQueues,
    ) -> Result<()> {
        let Self {
            terminal_table_opts,
            verbose,
            all,
            parameter_view,
        } = self;

        let path_base: Option<Arc<Path>> = {
            match parameter_view {
                ParameterView::Separated => None,
                ParameterView::Path { kind } => Some(match kind.unwrap_or_default() {
                    ParameterPathKind::Relative => PathBuf::from("").into(),
                    ParameterPathKind::Full => conf.output_dir.path.clone_arc(),
                    ParameterPathKind::Url => {
                        let url = conf.output_dir.url.as_ref().ok_or_else(|| {
                            anyhow!(
                                "the URL viewing feature requires the `output_dir.url` \
                                 field in the configuration to be set"
                            )
                        })?;
                        PathBuf::from(&**url).into()
                    }
                }),
            }
        };

        // COPY-PASTE from List action in jobqueue.rs
        let get_filename = |entry: &Entry<_, _>| -> Result<String> {
            let file_name = entry.file_name();
            Ok(file_name
                .to_str()
                .ok_or_else(|| anyhow!("filename that cannot be decoded as UTF-8: {file_name:?}"))?
                .to_string())
        };

        let mut out = stdout().lock();

        let full_span;
        {
            // Show a table with no data rows, for main titles
            let titles: Vec<_> = {
                let mut titles = vec![
                    "Insertion_time",
                    "S", // Status
                    "Prio",
                    "WD",
                    "Reason",
                ];
                match parameter_view {
                    ParameterView::Separated => {
                        titles.extend_from_slice(&[
                            "Commit_id",
                            "Target_name",
                            "Custom_parameters",
                        ]);
                    }
                    ParameterView::Path { kind } => {
                        titles.push(match kind.unwrap_or_default() {
                            ParameterPathKind::Relative => "Output_path",
                            ParameterPathKind::Full => "Output_path",
                            ParameterPathKind::Url => "Output_URL",
                        });
                    }
                }

                titles
                    .into_iter()
                    .map(|s| TerminalTableTitle {
                        text: Cow::Borrowed(s),
                        span: 1,
                    })
                    .collect()
            };
            full_span = titles.len();

            // Somehow have to move `out` in and out, `&mut out`
            // would not satisfy IsTerminal.
            let table = table_with_titles(
                &titles,
                // Note: in spite of `TERM=xterm-256color`, `watch
                // --color` still only supports system colors
                // 0..14!  (Can still not use `.rgb(10, 70, 140)`
                // nor `.fg(Color::Fixed(30))`, and watch 4.0.2
                // does not support `TERM=xterm-truecolor`.)
                Some(Style::new().fg(Color::Fixed(4)).italic().bold()),
                &terminal_table_opts,
                out,
                verbose,
                parameter_view,
            )?;
            out = table.finish()?;
        }

        let lock = working_directory_base_dir.lock("for SubCommand::List show_queue")?;

        let now = SystemTime::now();

        // Not kept in sync with what happens during for loop; but
        // then it is really about the status stored inside
        // `pool`, thus that doesn't even matter!
        let opt_current_working_directory = lock.read_current_working_directory()?;

        let show_queue = |i: &str, run_queue: &RunQueue, is_extra_queue: bool, out| -> Result<_> {
            let RunQueue {
                file_name,
                schedule_condition,
                queue,
            } = run_queue;

            // "Insertion time"
            // "R", "E", ""
            // priority
            // reason
            // "Commit id"
            // "Custom parameters"
            let titles = &[TerminalTableTitle {
                text: format!(
                    "{i}: queue {:?} ({schedule_condition}):",
                    file_name.as_str()
                )
                .into(),
                span: full_span,
            }];
            let mut table = table_with_titles(
                titles,
                None,
                &terminal_table_opts,
                out,
                verbose,
                parameter_view,
            )?;

            // We want the last view_jobs_max_len items, one more
            // if that's the complete list (the additional entry
            // then occupying the "entries skipped" line). Don't
            // want to collect the whole list first (leads to too
            // many open filehandles), don't want to go through it
            // twice (once for counting, once to skip); getting
            // them in reverse, taking the first n, collecting,
            // then reversing the list would be one way, but
            // cleaner is to use a two step approach, first get
            // the sorted collection of keys (cheap to hold in
            // memory and needs to be retrieved underneath
            // anyway), get the section we want, then use
            // resolve_entries to load the items still in
            // streaming fashion.  Note: this could show fewer
            // than limit items even after showing "skipped",
            // because items can vanish between getting
            // sorted_keys and resolve_entries. But that is really
            // no big deal.
            let limit = if is_extra_queue && !all {
                // Get 2 more since showing "skipped 1 entry" is
                // not economic, and we just look at number 0
                // after subtracting, i.e. include the equal case.
                conf.queues.view_jobs_max_len + 2
            } else {
                usize::MAX
            };
            let all_sorted_keys = queue.sorted_keys(false, None, false)?;
            let shown_sorted_keys;
            if let Some(num_skipped_2) = all_sorted_keys.len().checked_sub(limit) {
                let num_skipped = num_skipped_2 + 2;
                table.print(&format!("... ({num_skipped} entries skipped)\n"))?;
                shown_sorted_keys = &all_sorted_keys[num_skipped..];
            } else {
                shown_sorted_keys = &all_sorted_keys;
            }

            let mut row = Vec::new();
            for entry in queue.resolve_entries(shown_sorted_keys.into()) {
                let mut entry = entry?;
                let file_name = get_filename(&entry)?;
                let key = entry.key()?;
                let job = entry.get()?;
                let reason = if let Some(reason) = &job.public.reason {
                    reason.as_ref()
                } else {
                    ""
                };
                let (locking, is_locked) = if schedule_condition.is_inactive() {
                    ("", false)
                } else {
                    let lock_status = entry
                        .take_lockable_file()
                        .expect("not taken before")
                        .get_lock_status()?;
                    if lock_status == LockStatus::ExclusiveLock {
                        let s = if let Some(dir) = opt_current_working_directory {
                            let status = lock.read_working_directory_status(dir)?;
                            match status.status {
                                // CheckedOut wasn't planned
                                // to happen, but now happens
                                // for new working dir
                                // assignment
                                Status::CheckedOut => "R0",
                                Status::Processing => "R",  // running
                                Status::Error => "F",       // failure
                                Status::Finished => "E",    // evaluating
                                Status::Examination => "X", // manually marked
                            }
                        } else {
                            "R"
                        };
                        (s, true)
                    } else {
                        ("", false)
                    }
                };
                let priority = &*job.priority()?.to_string();
                let wd = if is_locked {
                    opt_current_working_directory
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "".into())
                } else {
                    job.state
                        .last_working_directory
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "".into())
                };

                let system_time = key.system_time();
                let is_older = {
                    let age = now.duration_since(system_time)?;
                    age > Duration::from_secs(3600 * 24)
                };
                let time = if verbose {
                    &*format!("{file_name} ({key})")
                } else {
                    let datetime: DateTime<Local> = system_time.into();
                    &*datetime.to_rfc3339()
                };
                row.extend_from_slice(&[time, locking, priority, &*wd, reason]);

                let commit_id;
                let custom_parameters;
                let key_dir;
                let path;
                match parameter_view {
                    ParameterView::Separated => {
                        commit_id = job.public.run_parameters.commit_id.to_string();
                        let target_name = job.public.command.target_name.as_str();
                        custom_parameters = job.public.run_parameters.custom_parameters.to_string();
                        row.extend_from_slice(&[&*commit_id, target_name, &*custom_parameters]);
                    }
                    ParameterView::Path { kind: _ } => {
                        let base = path_base
                            .as_ref()
                            .expect("initialized for ParameterView::Path");
                        key_dir = KeyDir::from_base_target_params(
                            base.clone_arc(),
                            job.public.command.target_name.clone(),
                            &job.public.run_parameters,
                        );
                        path = key_dir.to_path().to_string_lossy();
                        row.push(&path);
                    }
                }
                table.write_data_row(
                    &row,
                    if is_older {
                        // Note: need `TERM=xterm-256color`
                        // for `watch --color` to not turn
                        // this color to black!
                        Some(Style::new().bright_black())
                    } else {
                        None
                    },
                )?;
                if verbose {
                    let s = ron_to_string_pretty(&job)?;
                    table.print(&format!("{s}\n\n"))?;
                }

                row = {
                    row.clear();
                    // overwrite Vec with a version of itself that
                    // doesn't hold onto the lifetimes
                    row.into_iter().map(|_| unreachable!()).collect()
                };
            }
            Ok(table.finish()?)
        };

        let width = get_terminal_width(1);
        let bar_of = |c: &str| c.repeat(width);
        let (thin_bar, thick_bar) = if *UNICODE_IS_FINE {
            (bar_of("─"), bar_of("═"))
        } else {
            (bar_of("-"), bar_of("="))
        };

        for (i, run_queue) in queues.pipeline().iter().enumerate() {
            println!("{thin_bar}");
            out = show_queue(&(i + 1).to_string(), run_queue, false, out)?;
        }
        println!("{thick_bar}");
        let perhaps_show_extra_queue = |queue_name: &str,
                                        queue_field: &str,
                                        run_queue: Option<&RunQueue>,
                                        mut out|
         -> Result<_> {
            if let Some(run_queue) = run_queue {
                out = show_queue(queue_name, run_queue, true, out)?;
            } else {
                println!("No {queue_field} is configured")
            }
            Ok(out)
        };
        out = perhaps_show_extra_queue("done", "done_jobs_queue", queues.done_jobs_queue(), out)?;
        println!("{thin_bar}");
        _ = perhaps_show_extra_queue(
            "failures",
            "erroneous_jobs_queue",
            queues.erroneous_jobs_queue(),
            out,
        )?;
        println!("{thin_bar}");
        Ok(())
    }
}
