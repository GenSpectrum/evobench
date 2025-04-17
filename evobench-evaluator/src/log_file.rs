use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::{anyhow, bail, Context, Result};
use kstring::KString;

use crate::log_message::{LogMessage, Metadata};

const MAX_FILE_SIZE: u64 = 300_000_000;

#[derive(Debug)]
pub struct LogData {
    pub path: Box<Path>,
    pub messages: Vec<LogMessage>,
    pub evobench_log_version: u32,
    pub evobench_version: KString,
    pub metadata: Metadata,
}

impl LogData {
    pub fn read_file(path: &Path) -> Result<Self> {
        let input = File::open(path)?;
        let m = input.metadata()?;
        // Currently not doing streaming with the parsed results
        // anyway, so...
        if m.len() > MAX_FILE_SIZE {
            bail!("currently assuming that you don't read files larger than {MAX_FILE_SIZE}")
        }

        let mut input = BufReader::new(input);

        let mut line = String::new();
        let mut linenum = 0;
        let mut messages = Vec::new();

        // ugly in-line 'iterator' that also updates linenum
        macro_rules! let_next {
            { $var:ident or $($err:tt)* } => {
                if input.read_line(&mut line)? == 0 {
                    $($err)*
                }
                linenum += 1;
                let $var: LogMessage = serde_json::from_str(&line)
                    .with_context(|| anyhow!("parsing file {path:?}:{linenum}"))?;
                line.clear();
            }
        }

        let_next!(msg or bail!("missing the first message in {path:?}"));
        if let LogMessage::Start {
            evobench_log_version,
            evobench_version,
        } = msg
        {
            let_next!(msg or bail!("missing the second message in {path:?}"));
            if let LogMessage::Metadata(metadata) = msg {
                loop {
                    let_next!(msg or break);
                    messages.push(msg);
                }

                let last = (&messages).last().ok_or_else(|| {
                    anyhow!("log file {path:?} contains no data, and misses TEnd")
                })?;
                if let LogMessage::TEnd(_) = last {
                    // OK
                } else {
                    bail!("log file {path:?} does not end with TEnd, it was cut off")
                }

                Ok(LogData {
                    path: path.into(),
                    messages,
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
