use std::{
    borrow::Cow,
    cell::OnceCell,
    io::{self, IsTerminal, Write, stdout},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use ahtml::HtmlAllocator;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Local};

use crate::output_table::{CellValue, OutputStyle, OutputTable, OutputTableTitle};
use crate::{
    config_file::ron_to_string_pretty,
    key_val_fs::key_val::Entry,
    lockable_file::LockStatus,
    run::{
        config::RunConfig,
        output_directory::structure::{KeyDir, ToPath},
        run_queue::RunQueue,
        run_queues::RunQueues,
        working_directory::Status,
        working_directory_pool::WorkingDirectoryPoolBaseDir,
    },
    utillib::{arc::CloneArc, recycle::RecycleVec},
};
use crate::{
    output_table::{
        html::HtmlTable,
        terminal::{TerminalTable, TerminalTableOpts},
    },
    utillib::into_arc_path::IntoArcPath,
};

pub const TARGET_NAME_WIDTH: usize = 14;

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

impl ParameterView {
    fn titles(self) -> Vec<&'static str> {
        let mut titles = vec![
            "Insertion_time",
            "S", // Status
            "Prio",
            "WD",
            "Reason",
        ];
        match self {
            ParameterView::Separated => {
                titles.extend_from_slice(&["Commit_id", "Target_name", "Custom_parameters"]);
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
    }
}

#[derive(Debug, Clone, clap::Args)]
pub struct OutputTableOpts {
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

/// A text with optional link which is generated only when needed
/// (i.e. for HTML output)
#[derive(Clone, Copy)]
struct WithUrlOnDemand<'s> {
    text: &'s str,
    // dyn because different columns might want different links
    gen_url: Option<&'s dyn Fn() -> Option<String>>,
}

impl<'s> From<&'s str> for WithUrlOnDemand<'s> {
    fn from(text: &'s str) -> Self {
        WithUrlOnDemand {
            text,
            gen_url: None,
        }
    }
}

impl<'s> AsRef<str> for WithUrlOnDemand<'s> {
    fn as_ref(&self) -> &str {
        self.text
    }
}

impl<'s> CellValue for WithUrlOnDemand<'s> {
    fn perhaps_url(&self) -> Option<String> {
        if let Some(gen_url) = self.gen_url {
            gen_url()
        } else {
            None
        }
    }
}

impl OutputTableOpts {
    pub fn output_to_table<Table: OutputTable>(
        &self,
        mut table: Table,
        conf: &RunConfig,
        working_directory_base_dir: &Arc<WorkingDirectoryPoolBaseDir>,
        queues: &RunQueues,
    ) -> Result<Table::Output> {
        let Self {
            verbose,
            all,
            parameter_view,
        } = self;

        // The base of the path that's used for the `path` view
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

        let lock = working_directory_base_dir.lock("for SubCommand::List show_queue")?;

        {
            let titles: Vec<_> = parameter_view
                .titles()
                .into_iter()
                .map(|s| OutputTableTitle {
                    text: Cow::Borrowed(s),
                    span: 1,
                })
                .collect();

            let style = Some(OutputStyle {
                bold: true,
                italic: true,
                color: Some(4),
                ..Default::default()
            });

            table.write_title_row(&titles, style)?;
        }

        let full_span = table.num_columns();

        let now = SystemTime::now();

        // Not kept in sync with what happens during for loop; but
        // then it is really about the status stored inside
        // `pool`, thus that doesn't even matter!
        let opt_current_working_directory = lock.read_current_working_directory()?;

        let show_queue = |i: &str,
                          run_queue: &RunQueue,
                          is_extra_queue: bool,
                          table: &mut Table|
         -> Result<()> {
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
            let titles = &[OutputTableTitle {
                text: format!(
                    "{i}: queue {:?} ({schedule_condition}):",
                    file_name.as_str()
                )
                .into(),
                span: full_span,
            }];

            // It's OK to call this multiple times on the same table,
            // <th> are allowed in any table row; not sure about the
            // semantics, though.
            table.write_title_row(titles, None)?;

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

            let mut row: Vec<WithUrlOnDemand> = Vec::new();
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
                let time = if *verbose {
                    &*format!("{file_name} ({key})")
                } else {
                    let datetime: DateTime<Local> = system_time.into();
                    &*datetime.to_rfc3339()
                };
                row.extend_from_slice(&[
                    time.into(),
                    locking.into(),
                    priority.into(),
                    (&*wd).into(),
                    reason.into(),
                ]);

                let commit_id;
                let custom_parameters;
                let key_dir;
                let path;
                let gen_url_cache: OnceCell<Option<String>> = OnceCell::new();
                let gen_url = {
                    || -> Option<String> {
                        if let Some(url) = &conf.output_dir.url {
                            gen_url_cache
                                .get_or_init(|| {
                                    let key_dir = KeyDir::from_base_target_params(
                                        url.into_arc_path(),
                                        job.public.command.target_name.clone(),
                                        &job.public.run_parameters,
                                    );
                                    let url_as_path = key_dir.to_path();
                                    Some(url_as_path.to_string_lossy().to_string())
                                })
                                .clone()
                        } else {
                            None
                        }
                    }
                };
                match parameter_view {
                    ParameterView::Separated => {
                        commit_id = job.public.run_parameters.commit_id.to_string();
                        let target_name = job.public.command.target_name.as_str();
                        custom_parameters = job.public.run_parameters.custom_parameters.to_string();
                        row.extend_from_slice(&[
                            (&*commit_id).into(),
                            WithUrlOnDemand {
                                text: &target_name,
                                gen_url: Some(&gen_url),
                            },
                            WithUrlOnDemand {
                                text: &*custom_parameters,
                                gen_url: Some(&gen_url),
                            },
                        ]);
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
                        row.push(WithUrlOnDemand {
                            text: &*path,
                            gen_url: Some(&gen_url),
                        });
                    }
                }
                table.write_data_row(
                    &row,
                    if is_older {
                        Some(OutputStyle {
                            faded: true,
                            ..Default::default()
                        })
                    } else {
                        None
                    },
                )?;
                if *verbose {
                    let s = ron_to_string_pretty(&job)?;
                    table.print(&format!("{s}\n\n"))?;
                }

                row = row.recycle_vec();
            }
            Ok(())
        };

        let mut first = true;
        for (i, run_queue) in queues.pipeline().iter().enumerate() {
            if first {
                table.write_thick_bar()?;
                first = false;
            } else {
                table.write_thin_bar()?;
            }
            show_queue(&(i + 1).to_string(), run_queue, false, &mut table)?;
        }
        table.write_thick_bar()?;
        let perhaps_show_extra_queue = |queue_name: &str,
                                        queue_field: &str,
                                        run_queue: Option<&RunQueue>,
                                        table: &mut Table|
         -> Result<()> {
            if let Some(run_queue) = run_queue {
                show_queue(queue_name, run_queue, true, table)?;
            } else {
                table.print(&format!("No {queue_field} is configured"))?;
            }
            Ok(())
        };
        perhaps_show_extra_queue(
            "done",
            "done_jobs_queue",
            queues.done_jobs_queue(),
            &mut table,
        )?;
        table.write_thin_bar()?;
        perhaps_show_extra_queue(
            "failures",
            "erroneous_jobs_queue",
            queues.erroneous_jobs_queue(),
            &mut table,
        )?;
        table.write_thin_bar()?;

        table.finish()
    }
}

#[derive(Debug, Clone, clap::Args)]
pub struct ListOpts {
    #[clap(flatten)]
    terminal_table_opts: TerminalTableOpts,

    /// Print table as HTML
    #[clap(long)]
    html: bool,

    #[clap(flatten)]
    output_table_opts: OutputTableOpts,
}

fn make_terminal_table<O: io::Write + IsTerminal>(
    terminal_table_opts: &TerminalTableOpts,
    out: O,
    verbose: bool,
    view: ParameterView,
) -> TerminalTable<O> {
    let insertion_time_width = if verbose { 82 } else { 37 };
    let widths =
    //     t                    R pr WD reason commit target
        &[insertion_time_width, 3, 6, 5, 25, 42, TARGET_NAME_WIDTH];
    let widths = match view {
        ParameterView::Separated => widths,
        ParameterView::Path { kind: _ } => &widths[0..5],
    };
    TerminalTable::new(widths, terminal_table_opts.clone(), out)
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
            output_table_opts,
            html,
        } = self;

        if html {
            let num_columns = output_table_opts.parameter_view.titles().len();
            let html = HtmlAllocator::new(1000000, Arc::new("list"));
            let table = HtmlTable::new(num_columns, &html);
            let body = output_table_opts.output_to_table(
                table,
                conf,
                working_directory_base_dir,
                queues,
            )?;
            let doc = html.html(
                [],
                [html.head([], [])?, html.body([], html.table([], body)?)?],
            )?;
            let mut out = stdout().lock();
            html.print_html_document(doc, &mut out)?;
            out.flush()?;
        } else {
            let out = stdout().lock();
            let table = make_terminal_table(
                &terminal_table_opts,
                out,
                output_table_opts.verbose,
                output_table_opts.parameter_view,
            );

            let mut out = output_table_opts.output_to_table(
                table,
                conf,
                working_directory_base_dir,
                queues,
            )?;

            out.flush()?;
        }

        Ok(())
    }
}
