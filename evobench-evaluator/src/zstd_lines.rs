use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
    sync::mpsc::channel,
    thread,
};

use anyhow::{anyhow, bail, Context, Result};
use ruzstd::StreamingDecoder;

/// Transparently decompress zstd files if they have a .zstd suffix;
/// after that, expecting the `expected_suffix` (XX well, currently
/// not checking the sub-suffix if it has .zstd suffix).
pub fn decompressed_file_lines(
    path: &Path,
    expected_suffix: &str,
    max_file_size: Option<u64>,
) -> Result<impl Iterator<Item = (usize, anyhow::Result<String>)>> {
    let ext = path.extension().ok_or_else(|| {
        anyhow!("missing file extension, expecting {expected_suffix:?} or \".zstd\": {path:?}")
    })?;

    let is_compressed = match ext.to_string_lossy().as_ref() {
        "zstd" => true,
        s if &*s == expected_suffix => false,
        _ => bail!("unknown file extension {ext:?}, expecting .log or .zstd: {path:?}"),
    };

    let input = File::open(path).with_context(|| anyhow!("opening file {path:?}"))?;

    if let Some(max_file_size) = max_file_size {
        let m = input.metadata()?;
        if m.len() > max_file_size {
            bail!("currently assuming that you don't read files larger than {max_file_size}")
        }
    }

    // XX does matching extension bytes work on Windows?
    let uncompressed_input: Box<dyn Read + Send + 'static> = if is_compressed {
        Box::new(StreamingDecoder::new(input).with_context(|| anyhow!("zstd-decoding {path:?}"))?)
    } else {
        Box::new(input)
    };

    let input = BufReader::new(uncompressed_input);

    // XXX need to use bounded channel
    let (w, r) = channel();

    thread::Builder::new()
        .name(format!("decompressed_file_lines"))
        .spawn({
            let path = path.to_owned();
            move || -> Result<()> {
                for (line_num, line) in input.lines().enumerate() {
                    let line = line.with_context(|| anyhow!("reading file {path:?}"));
                    w.send((line_num, line))?;
                }
                Ok(())
            }
        })?;

    Ok(r.into_iter())
}
