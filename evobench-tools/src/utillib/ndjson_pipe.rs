use std::{
    cell::Cell,
    io::{BufRead, BufReader, PipeReader, PipeWriter, Write, pipe},
    marker::PhantomData,
};

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};

pub struct NdJsonPipe<T: Serialize + DeserializeOwned> {
    _phantom: PhantomData<fn() -> T>,
    // Cell and Option because the borrow checker doesn't understand
    // `fork` thus we have to panic at runtime instead on usage
    // errors.
    p: Cell<Option<(PipeReader, PipeWriter)>>,
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
            p: Some(p).into(),
            _phantom: PhantomData,
        })
    }

    /// You can only use *one* of the `into_*` functions, once,
    /// otherwise you get a panic.
    pub fn into_reader(&self) -> NdJsonPipeReader<T> {
        let (r, _w) = self
            .p
            .take()
            .expect("only call once in each of parent and child");
        NdJsonPipeReader {
            _phantom: PhantomData,
            line: String::new(),
            reader: BufReader::new(r),
        }
    }

    /// You can only use *one* of the `into_*` functions, once,
    /// otherwise you get a panic.
    pub fn into_writer(&self) -> NdJsonPipeWriter<T> {
        let (_r, w) = self
            .p
            .take()
            .expect("only call once in each of parent and child");
        NdJsonPipeWriter {
            _phantom: PhantomData,
            w,
        }
    }
}
