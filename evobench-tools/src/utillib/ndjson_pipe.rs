use std::{
    io::{BufRead, BufReader, PipeReader, PipeWriter, Write, pipe},
    marker::PhantomData,
};

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};

pub struct NdJsonPipe<T: Serialize + DeserializeOwned> {
    _phantom: PhantomData<fn() -> T>,
    p: (PipeReader, PipeWriter),
}

#[derive(Debug)]
pub struct NdJsonPipeWriter<T: Serialize + DeserializeOwned> {
    _phantom: PhantomData<fn() -> T>,
    w: PipeWriter,
}

impl<T: Serialize + DeserializeOwned> NdJsonPipeWriter<T> {
    /// Immediately sends `msg` to the daemon process (there is no
    /// buffering).
    pub fn send(&mut self, msg: T) -> Result<()> {
        let mut s = serde_json::to_string(&msg)?;
        s.push('\n');
        self.w.write_all(s.as_bytes())?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct NdJsonPipeReader<T: Serialize + DeserializeOwned> {
    _phantom: PhantomData<fn() -> T>,
    line: String,
    reader: BufReader<PipeReader>,
}

impl<T: Serialize + DeserializeOwned> Iterator for NdJsonPipeReader<T> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        (|| -> Result<Option<T>> {
            let nread = self.reader.read_line(&mut self.line)?;
            if nread == 0 {
                Ok(None)
            } else {
                let val: T = serde_json::from_str(&self.line)?;
                self.line.clear();
                Ok(Some(val))
            }
        })()
        .transpose()
    }
}

impl<T: Serialize + DeserializeOwned> NdJsonPipe<T> {
    pub fn new() -> Result<Self> {
        let p = pipe()?;
        Ok(Self {
            _phantom: PhantomData,
            p,
        })
    }

    /// Note: if the borrow checker doesn't allow you to use both
    /// `into_*` methods, then revert the git commit introducing this
    /// comment.
    pub fn into_reader(self) -> NdJsonPipeReader<T> {
        let (r, _w) = self.p;
        NdJsonPipeReader {
            _phantom: PhantomData,
            line: String::new(),
            reader: BufReader::new(r),
        }
    }

    /// Note: if the borrow checker doesn't allow you to use both
    /// `into_*` methods, then revert the git commit introducing this
    /// comment.
    pub fn into_writer(self) -> NdJsonPipeWriter<T> {
        let (_r, w) = self.p;
        NdJsonPipeWriter {
            _phantom: PhantomData,
            w,
        }
    }
}
