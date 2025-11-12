//! Log files holding benchmarking command output--not the structured
//! timing data, but stdout and stderr of the benchmarking target of
//! the target application.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use anyhow::Result;
use chrono::DateTime;

use crate::{
    ctx, io_utils::capture::OutFile, key::BenchmarkingJobParameters, zstd_file::decompressed_file,
};

/// Returns `(head, rest, rest_lineno)`, where `rest_lineno` is the
/// 1-based line number where `rest` starts. Returns None if either
/// head or rest are empty.
fn split_off_log_file_params(s: &str) -> Option<(&str, &str, usize)> {
    // Should have added a separator to the files (now it outputs an
    // empty line, but have to deal with older files, too): scan until
    // finding the first timestamp, then assume the part before is the
    // head.
    let mut line_endings = s.char_indices().filter(|(_, c)| *c == '\n').map(|(i, _)| i);
    let mut lineno = 1;
    let mut i = 0; // the start of the next line
    loop {
        let rest = &s[i..];
        if let Some((t, _)) = rest.split_once('\t') {
            if let Ok(_timestamp) = DateTime::parse_from_rfc3339(t) {
                if i == 0 {
                    return None;
                }
                let head = &s[0..i - 1];
                return Some((head, rest, lineno));
            }
        }
        if let Some(i2) = line_endings.next() {
            lineno += 1;
            i = i2 + 1;
        } else {
            return None;
        }
    }
}

/// A command log file, i.e. stderr and stdout of the benchmarking
/// target of the target application.
pub struct CommandLogFile<P: AsRef<Path>> {
    pub path: P,
}

// XX should wrap OutFile to represent command log files while
// writing, then translate from *that* to CommandLogFile (which is the
// non-writing state).
impl From<OutFile> for CommandLogFile<PathBuf> {
    fn from(value: OutFile) -> Self {
        Self {
            path: value.into_path(),
        }
    }
}

impl<P: AsRef<Path>> From<P> for CommandLogFile<P> {
    fn from(path: P) -> Self {
        Self { path }
    }
}

/// The contents of a command log file, split into head and rest if
/// possible (old versions of those files didn't have a head; probably
/// should require one at some point).
#[ouroboros::self_referencing]
pub struct CommandLog<'l, P: AsRef<Path>> {
    pub log_file: &'l CommandLogFile<P>,
    pub contents: String,
    #[borrows(contents)]
    #[covariant]
    /// Only if a head is present; otherwise, borrow `contents` as the
    /// rest.
    pub head_and_rest: Option<(&'this str, &'this str, usize)>,
}

impl<P: AsRef<Path>> CommandLogFile<P> {
    /// Read the file contents and split it into head and rest if it
    /// has a detectable head.
    pub fn command_log<'l>(&'l self) -> Result<CommandLog<'l, P>> {
        let log_path = self.path.as_ref();
        let input = decompressed_file(log_path, None)?;
        let log_contents =
            std::io::read_to_string(input).map_err(ctx!("reading file {log_path:?}"))?;
        Ok(CommandLog::new(self, log_contents, |contents| {
            split_off_log_file_params(contents)
        }))
    }
}

impl<'l, P: AsRef<Path>> CommandLog<'l, P> {
    pub fn path(&self) -> &Path {
        self.borrow_log_file().path.as_ref()
    }

    pub fn path_string_lossy<'s>(&'s self) -> Cow<'s, str> {
        self.path().to_string_lossy()
    }

    /// Parse the head (not cached). Option because of compatibility
    /// with older log files that didn't have the head (should perhaps
    /// be changed at some point soon, and give an error instead?)
    pub fn parse_log_file_params(&self) -> Result<Option<BenchmarkingJobParameters>> {
        self.borrow_head_and_rest()
            .map(|(head, _rest, _lineno)| -> Result<_> {
                let params: BenchmarkingJobParameters = serde_yml::from_str(head)?;
                Ok(params)
            })
            .transpose()
    }

    /// The part of the file contents after the head, together with
    /// the 1-based line number where it starts. If there was no head
    /// detected, just give the whole file contents.
    pub fn log_contents_rest(&self) -> (&str, usize) {
        if let Some((_, rest, lineno)) = self.borrow_head_and_rest() {
            (rest, *lineno)
        } else {
            (self.borrow_contents(), 1)
        }
    }
}
