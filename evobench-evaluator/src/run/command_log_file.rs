//! Log files holding benchmarking command output--not the structured
//! timing data, but stdout and stderr of the benchmarking target of
//! the target application.

use std::{borrow::Cow, path::Path};

use anyhow::Result;
use chrono::DateTime;

use crate::{ctx, key::BenchmarkingJobParameters};

/// Returns `(head, rest, rest_lineno)`, where `rest_lineno` is the
/// 1-based line number where `rest` starts.
fn split_off_log_file_params(s: &str) -> Option<(&str, &str, usize)> {
    // Should have added a separator to the files (now it outputs an
    // empty line, but have to deal with older files, too): scan until
    // finding the first timestamp, then assume the part before is the
    // head.
    let line_endings = s.char_indices().filter(|(_, c)| *c == '\n');
    let mut lineno = 0;
    for (i, _) in line_endings {
        lineno += 1;
        let rest = &s[i + 1..];
        if let Some((t, _)) = rest.split_once('\t') {
            if let Ok(_timestamp) = DateTime::parse_from_rfc3339(t) {
                let head = &s[0..i];
                return Some((head, rest, lineno));
            }
        }
    }
    None
}

/// A command log file, i.e. stderr and stdout of the benchmarking
/// target of the target application.
pub struct CommandLogFile<P: AsRef<Path>> {
    pub path: P,
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
        let logfile = self.path.as_ref();
        let log_contents =
            std::fs::read_to_string(logfile).map_err(ctx!("reading file {logfile:?}"))?;
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

    /// Parse the head (not cached)
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
