//! Post-processing of the output files from a benchmark run, or a set
//! of benchmark runs belonging to the same 'key'.

use std::{
    collections::{hash_map::Entry, HashMap},
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Result};
use run_git::path_util::AppendToPath;

use crate::{
    ctx, info,
    path_util::add_extension,
    run::{
        command_log_file::CommandLogFile,
        config::{RunConfig, ScheduleCondition},
        output_directory_structure::{KeyDir, RunDir},
    },
    serde::{proper_dirname::ProperDirname, proper_filename::ProperFilename},
    utillib::logging::{log_level, LogLevel},
    zstd_file::compress_file,
};

/// `target_path` must include the `.zstd` extension. XX why does this
/// not always do .tmp and then rename, for safety? Rather, have an
/// `omit_rename` argument and then leave .tmp suffix in place?
pub fn compress_file_as(
    source_path: &Path,
    target_path: PathBuf,
    add_tmp_suffix: bool,
) -> Result<PathBuf> {
    let actual_target_path = if add_tmp_suffix {
        add_extension(&target_path, "tmp").expect("got filename")
    } else {
        target_path
    };
    compress_file(
        source_path,
        &actual_target_path,
        // be quiet when:
        log_level() < LogLevel::Info,
    )?;
    // Do *not* remove the source file here as
    // TemporaryFile::drop will do it.
    Ok(actual_target_path)
}

// XX here, *too*, do capture for consistency? XX: could do "nice" scheduling here.
pub fn evobench_evaluator(args: &[OsString]) -> Result<()> {
    let prog = "evobench-evaluator";
    let mut c = Command::new(prog);
    c.args(args);
    let mut child = c.spawn().map_err(ctx!("spawning command {c:?}"))?;
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        bail!("running {prog:?} with args {args:?}: {status}")
    }
}

fn generate_summary(
    key_dir: &Path,
    job_output_dirs: &[RunDir],
    selector: &str,        // "avg" or so
    target_type_opt: &str, // "--excel" or so
    file_base_name: &str,
) -> Result<()> {
    let mut args: Vec<OsString> = Vec::new();
    args.push("summary".into());

    args.push("--summary-field".into()); // XXX *is* right one right? OPEN
    args.push(selector.into());

    args.push(target_type_opt.into());
    args.push(key_dir.append(file_base_name).into());

    for job_output_dir in job_output_dirs {
        let evobench_log = job_output_dir.path().append("evobench.log.zstd");
        if std::fs::exists(&evobench_log).map_err(ctx!("checking path {evobench_log:?}"))? {
            args.push(evobench_log.into());
        } else {
            info!("missing file {evobench_log:?}, empty dir?");
        }
    }

    evobench_evaluator(&args)?;

    Ok(())
}

const SUMMARIES: &[(&str, &str, &str)] = &[
    ("sum", "--flame", ""),
    ("avg", "--excel", ".xlsx"),
    ("sum", "--excel", ".xlsx"),
];

/// Situation `None` means across all outputs; otherwise "night" etc.
pub fn generate_all_summaries_for_situation(
    situation: Option<&ProperFilename>,
    key_dir: &Path,
    job_output_dirs: &[RunDir],
) -> Result<()> {
    for (selector, target, suffix) in SUMMARIES {
        let mut basename = format!("{selector}-summary");
        if let Some(situation) = situation {
            basename = format!("{basename}-{}", situation.as_str());
        }
        basename.push_str(suffix);
        generate_summary(key_dir, job_output_dirs, selector, target, &basename)?;
    }
    Ok(())
}

impl RunDir {
    /// Produce the "single" extract files, as well as other
    /// configured derivatives. After the standard "single" extracts
    /// succeeded, `evaluating_benchmark_file_succeeded` is run; it
    /// should remove the file at `evobench_log_path` if this is the
    /// initial run and `evobench_log_path` pointed to e.g. a
    /// tmpfs. Pass a no-op if calling later on. If
    /// `evobench_log_path` is None, then the standard location is
    /// used.
    pub fn post_process_single(
        &self,
        evobench_log_path: Option<&Path>,
        evaluating_benchmark_file_succeeded: impl FnOnce() -> Result<()>,
        target_name: &ProperDirname,
        standard_log_path: &Path,
        run_config: &RunConfig,
    ) -> Result<()> {
        info!("evaluating benchmark file");

        let default_path_;
        let evobench_log_path = if let Some(path) = evobench_log_path {
            path
        } else {
            default_path_ = self.evobench_log_path();
            &default_path_
        };

        // Doing this *before* possibly renaming the file via
        // `evaluating_benchmark_file_succeeded`, as a way to ensure
        // that no invalid files end up in the results pool!
        evobench_evaluator(&vec![
            "single".into(),
            evobench_log_path.into(),
            "--show-thread-number".into(),
            "--excel".into(),
            self.append_str("single.xlsx")?.into(),
        ])?;

        // It's a bit inefficient to read the $EVOBENCH_LOG twice, but
        // currently can't change the options (--show-thread-number)
        // without a separate run. (Will be low cost once caching is
        // done.)
        evobench_evaluator(&vec![
            "single".into(),
            evobench_log_path.into(),
            "--flame".into(),
            self.append_str("single")?.into(),
        ])?;

        evaluating_benchmark_file_succeeded()?;
        // The above may have unlinked evobench_log_path, thus prevent further use:
        #[allow(unused)]
        let evobench_log_path = ();

        // Find the `LogExtract`s for the `target_name`
        if let Some(target) = run_config.targets.get(target_name) {
            if let Some(log_extracts) = &target.log_extracts {
                if !log_extracts.is_empty() {
                    info!("performing log extracts");

                    let command_log_file = CommandLogFile::from(standard_log_path);
                    let command_log = command_log_file.command_log()?;

                    for log_extract in log_extracts {
                        log_extract.extract_seconds_from(&command_log, self.path())?;
                    }
                }
            } else {
                info!("no log extracts are configured");
            }
        } else {
            info!(
                "haven't found target {target_name:?}, old job before \
                 configuration change?"
            );
        }

        Ok(())
    }
}

impl KeyDir {
    pub fn generate_summaries_for_key_dir(&self) -> Result<()> {
        let key_dir = self.path();
        info!("(re-)evaluating the summary files across all results in key dir {key_dir:?}");

        let run_dirs = self.run_dirs()?;

        generate_all_summaries_for_situation(None, key_dir, &run_dirs)?;

        {
            let mut job_output_dirs_by_situation: HashMap<ProperFilename, Vec<RunDir>> =
                HashMap::new();
            for run_dir in &run_dirs {
                let schedule_condition_path = run_dir.path().append("schedule_condition.ron");
                match std::fs::read_to_string(&schedule_condition_path) {
                    Ok(s) => {
                        let schedule_condition: ScheduleCondition = ron::from_str(&s)
                            .map_err(ctx!("reading file {schedule_condition_path:?}"))?;
                        if let Some(situation) = schedule_condition.situation() {
                            // XX it's just too long, proper abstraction pls?
                            match job_output_dirs_by_situation.entry(situation.clone()) {
                                Entry::Occupied(mut occupied_entry) => {
                                    occupied_entry.get_mut().push(run_dir.clone());
                                }
                                Entry::Vacant(vacant_entry) => {
                                    vacant_entry.insert(vec![run_dir.clone()]);
                                }
                            }
                        }
                    }
                    Err(e) => match e.kind() {
                        std::io::ErrorKind::NotFound => (),
                        _ => Err(e).map_err(ctx!("reading file {schedule_condition_path:?}"))?,
                    },
                }
            }

            for (situation, job_output_dirs) in job_output_dirs_by_situation.iter() {
                generate_all_summaries_for_situation(
                    Some(situation),
                    &key_dir,
                    job_output_dirs.as_slice(),
                )?;
            }
        }
        Ok(())
    }
}
