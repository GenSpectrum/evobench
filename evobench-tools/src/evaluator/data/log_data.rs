use std::{iter::FusedIterator, path::Path, sync::Mutex};

use anyhow::{Context, Result, anyhow, bail};
use kstring::KString;

use crate::{
    evaluator::data::log_message::{LogMessage, Metadata},
    io_utils::zstd_file::decompressed_file_mmap,
    utillib::auto_vivify::AutoVivify,
};

struct IterWithLineAndByteCount<'t, I: Iterator<Item = &'t [u8]>> {
    lines_iter: I,
    linenum: usize,
    bytepos: usize,
    path: &'t Path,
}

impl<'t, I: Iterator<Item = &'t [u8]>> IterWithLineAndByteCount<'t, I> {
    fn new(lines_iter: I, path: &'t Path) -> Self {
        Self {
            lines_iter,
            linenum: 0,
            bytepos: 0,
            path,
        }
    }
}

impl<'t, I: Iterator<Item = &'t [u8]>> Iterator for IterWithLineAndByteCount<'t, I> {
    type Item = Result<LogMessage>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(line) = self.lines_iter.next() {
            self.linenum += 1;
            self.bytepos += line.len() + 1;
            Some(
                serde_json::from_slice(line)
                    .with_context(|| anyhow!("parsing file {:?}:{}", self.path, self.linenum)),
            )
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct LogData {
    pub path: Box<Path>,
    pub messages: Box<[Box<[LogMessage]>]>,
    pub evobench_log_version: u32,
    pub evobench_version: KString,
    pub metadata: Metadata,
}

impl LogData {
    // Size of buffer for decompressed log data (for one chunk
    // processed in parallel)
    const CHUNK_SIZE_BYTES: usize = 20000000;

    pub fn messages(&self) -> impl DoubleEndedIterator<Item = &LogMessage> + FusedIterator {
        self.messages.iter().flatten()
    }

    /// `path` must end in `.log` or `.zstd`. Decompresses the latter
    /// transparently. Currently not doing streaming with the parsed
    /// results, the in-memory representation is larger than the
    /// file.
    // Note: you can find versions of this function reading from
    // `decompressed_file` instead of using mmap in the Git history,
    // in parallel and before-parallel versions.
    pub fn read_file(path: &Path) -> Result<Self> {
        let input = decompressed_file_mmap(path, Some("log"))?;

        let mut items = IterWithLineAndByteCount::new(input.split(|b| *b == b'\n'), path);

        let msg = items
            .next()
            .ok_or_else(|| anyhow!("missing the first message in {path:?}"))??;
        if let LogMessage::Start {
            evobench_log_version,
            evobench_version,
        } = msg
        {
            let msg = items
                .next()
                .ok_or_else(|| anyhow!("missing the second message in {path:?}"))??;
            if let LogMessage::Metadata(metadata) = msg {
                // Results from chunks processing as they come in
                let results: Mutex<Vec<Option<Result<Box<[LogMessage]>>>>> = Default::default();
                let results_ref = &results;

                rayon::scope(|scope| -> Result<()> {
                    let mut rest = &input[items.bytepos..];
                    let mut current_chunk_index = 0;

                    while !rest.is_empty() {
                        let buf = &rest[..Self::CHUNK_SIZE_BYTES.min(rest.len())];
                        // Find the last line break
                        let (i, _) = buf
                            .iter()
                            .rev()
                            .enumerate()
                            .find(|(_, b)| **b == b'\n')
                            .ok_or_else(|| {
                                anyhow!(
                                    "missing a line break in chunk {current_chunk_index} (size {}) \
                                     in file {path:?}",
                                    buf.len()
                                )
                            })?;
                        let cutoff = buf.len() - i;
                        let buf = &buf[0..cutoff];
                        rest = &rest[cutoff..];

                        scope.spawn({
                            let chunk_index = current_chunk_index;
                            move |_scope| {
                                let r = (|| -> Result<Box<[LogMessage]>> {
                                    let mut items = IterWithLineAndByteCount::new(
                                        buf.trim_ascii_end().split(|b| *b == b'\n'),
                                        path,
                                    );

                                    let mut messages = Vec::new();
                                    while let Some(msg) = items.next() {
                                        messages.push(msg?);
                                    }
                                    Ok(messages.into())
                                })();
                                let mut results = results_ref.lock().expect("no panics");
                                _ = results.auto_get_mut(chunk_index, || None).insert(r);
                            }
                        });
                        current_chunk_index += 1;
                    }
                    Ok(())
                })?;

                let messages: Vec<Box<[LogMessage]>> = results
                    .into_inner()?
                    .into_iter()
                    .enumerate()
                    .map(|(i, o)| {
                        if let Some(r) = o {
                            r.with_context(|| anyhow!("chunk {i}"))
                        } else {
                            bail!("chunk {i} has not reported a result, did it panic?")
                        }
                    })
                    .collect::<Result<_>>()?;

                let last = (&messages)
                    .last()
                    .ok_or_else(|| anyhow!("log file {path:?} contains no data, and misses TEnd"))?
                    .last()
                    .ok_or_else(|| {
                        anyhow!(
                            "processing log file {path:?}: missing a value in last chunk result"
                        )
                    })?;

                if let LogMessage::TEnd(_) = last {
                    // OK
                } else {
                    bail!("log file {path:?} does not end with TEnd, it was cut off")
                }

                Ok(LogData {
                    path: path.into(),
                    messages: messages.into(),
                    evobench_log_version,
                    evobench_version,
                    metadata,
                })
            } else {
                bail!("second message is not a `Metadata` message: {msg:?}")
            }
        } else {
            bail!("first message is not a `Start` message: {msg:?}")
        }
    }
}
