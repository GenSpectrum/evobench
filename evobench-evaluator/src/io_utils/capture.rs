use std::{
    borrow::Cow,
    ffi::OsStr,
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{Scope, ScopedJoinHandle},
};

use anyhow::{anyhow, Result};

use crate::{ctx, serde::date_and_time::DateTimeWithOffset};

use super::bash::cmd_as_bash_string;

// ETOOCOMPLICATED.
pub fn get_cmd_and_args(cmd: &Command) -> Vec<Cow<str>> {
    let prog_name = cmd.get_program().to_string_lossy();
    let mut args: Vec<_> = cmd
        .get_args()
        .map(|s: &OsStr| s.to_string_lossy())
        .collect();
    let mut cmd_and_args = vec![prog_name];
    cmd_and_args.append(&mut args);
    cmd_and_args
}

pub fn get_cmd_and_args_as_bash_string(cmd: &Command) -> String {
    cmd_as_bash_string(get_cmd_and_args(cmd))
}

pub fn new_proxy_thread<'scope, 'file, 'm, F: Read + Send + 'static>(
    scope: &'scope Scope<'scope, 'file>,
    child_output: F,
    main_file: Arc<Mutex<File>>,
    other_files: Arc<Mutex<Vec<Box<dyn Write + 'static + Send>>>>,
    source_indicator: Option<&'file str>,
    add_timestamp: bool,
) -> Result<ScopedJoinHandle<'scope, Result<()>>>
where
    'file: 'scope,
    'file: 'm,
    'm: 'scope,
{
    let mut child_output = BufReader::new(child_output);
    std::thread::Builder::new()
        .name("output proxy".into())
        .spawn_scoped(scope, move || -> Result<()> {
            // Have to use two buffers, because it's not possible to
            // prepare the timestamp in advance of the read_line call
            // since the latter is blocking.
            let mut input_line = String::new();
            let mut line = String::new();
            while child_output.read_line(&mut input_line)? > 0 {
                {
                    line.clear();
                    if let Some(source_indicator) = source_indicator.as_ref() {
                        line.push_str(source_indicator);
                        line.push_str("\t");
                    }
                    if add_timestamp {
                        line.push_str(&DateTimeWithOffset::now().into_string());
                        line.push_str("\t");
                    }
                    line.push_str(&input_line);
                    input_line.clear();
                }
                if !line.ends_with("\n") {
                    line.push_str("\n");
                }
                let mut output = main_file.lock().expect("no panics in proxy threads");
                output.write_all(line.as_bytes())?;
                {
                    let mut outputs = other_files.lock().expect("no panics in proxy threads");
                    for output in outputs.iter_mut() {
                        output.write_all(line.as_bytes())?;
                    }
                }
            }
            Ok(())
        })
        .map_err(move |e| anyhow!("{e}"))
}

#[derive(Clone, Debug)]
pub struct CaptureOpts {
    pub add_source_indicator: bool,
    pub add_timestamp: bool,
}

#[derive(Debug)]
pub struct OutFile {
    path: PathBuf,
    file: Arc<Mutex<File>>,
}

impl OutFile {
    pub fn create(path: &Path) -> Result<Self> {
        let file = File::create(path).map_err(ctx!("opening OutFile {path:?} for writing"))?;
        let path = path.to_owned();

        Ok(Self {
            path,
            file: Arc::new(Mutex::new(file)),
        })
    }

    /// The last `len` bytes, decoded as utf8 lossily, with "...\n"
    /// prepended if that is not the whole output in the file.
    pub fn last_part(&self, len: u16) -> Result<String> {
        let mut v = Vec::new();
        let have_all;
        {
            // Somehow self.file.seek(), with or without try_clone,
            // leads to "Bad file descriptor", hence open by path,
            // aha, because File::create does not open for read-write,
            // of course. No flush needed *currently* as we're not
            // using BufWriter (but have our own line buffering).
            let mut file =
                File::open(&self.path).map_err(ctx!("re-opening {:?} for reading", self.path))?;
            // SeekFrom::End leads to "Bad file descriptor"?, hence:
            // -- XX not anymore with File::open ?
            let meta = file.metadata().map_err(ctx!("metadata"))?;
            let existing_len = meta.len();
            let offset = if let Some(offset) = existing_len.checked_sub(u64::from(len)) {
                have_all = false;
                offset
            } else {
                have_all = true;
                0
            };
            file.seek(SeekFrom::Start(offset)).map_err(ctx!("seek"))?;
            file.read_to_end(&mut v)
                .map_err(ctx!("reading {:?}", self.path))?;
        }
        let s = String::from_utf8_lossy(&v);
        if have_all {
            Ok(s.into())
        } else {
            Ok(format!("...\n{s}"))
        }
    }

    /// Write a string to the file, without timestamps or prefixes or
    /// even checks for line endings. Used to store a header before
    /// calling `run_with_capture`.
    pub fn write_str(&self, s: &str) -> Result<()> {
        self.file
            .lock()
            .expect("no panics")
            .write_all(s.as_bytes())
            .map_err(ctx!("writing to {:?}", self.path))
    }

    /// Can give multiple output files, e.g. for on-disk and terminal.
    // Couldn't make it work with borrowing here, thus Arc. STUPID.
    pub fn run_with_capture<'a: 'file, 'file>(
        &self,
        mut cmd: Command,
        other_files: Arc<Mutex<Vec<Box<dyn Write + Send + 'static>>>>,
        opts: CaptureOpts,
    ) -> Result<ExitStatus> {
        let CaptureOpts {
            add_source_indicator,
            add_timestamp,
        } = opts;

        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ctx!("running {}", get_cmd_and_args_as_bash_string(&cmd)))?;

        std::thread::scope(move |scope| -> Result<ExitStatus> {
            let stdout_thread = new_proxy_thread(
                scope,
                child.stdout.take().expect("configured above"),
                self.file.clone(),
                other_files.clone(),
                if add_source_indicator {
                    Some("O")
                } else {
                    None
                },
                add_timestamp,
            )?;
            let stderr_thread = new_proxy_thread(
                scope,
                child.stderr.take().expect("configured above"),
                self.file.clone(),
                other_files.clone(),
                if add_source_indicator {
                    Some("E")
                } else {
                    None
                },
                add_timestamp,
            )?;

            let status = child.wait()?;

            stdout_thread
                .join()
                .map_err(|e| anyhow!("stdout proxy thread panicked: {e:?}"))?
                .map_err(ctx!("stdout proxy thread"))?;
            stderr_thread
                .join()
                .map_err(|e| anyhow!("stderr proxy thread panicked: {e:?}"))?
                .map_err(ctx!("stderr proxy thread"))?;

            Ok(status)
        })
    }
}
