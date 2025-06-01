use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::Path,
    sync::mpsc::sync_channel,
    thread,
};

use anyhow::{anyhow, bail, Context, Result};
use ruzstd::StreamingDecoder;

/// Transparently decompress zstd files if they have a .zstd suffix;
/// after that, expecting the `expected_suffix` (XX well, currently
/// not checking the sub-suffix if it has .zstd suffix). Does not
/// return individual lines, but groups of `num_lines_per_chunk`, and
/// a serial number of the line group.
pub fn decompressed_file_line_groups(
    path: &Path,
    expected_suffix: &str,
    max_file_size: Option<u64>,
    num_lines_per_chunk: usize,
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

    let (w, r) = sync_channel::<(usize, anyhow::Result<String>)>(100);

    thread::Builder::new()
        .name(format!("decompressed_file_lines"))
        .spawn({
            let path = path.to_owned();
            move || -> Result<()> {
                let send_off = |linegroup, group_num| -> Result<()> {
                    w.send((group_num, Ok(linegroup)))?;
                    Ok(())
                };
                let mut input = BufReader::new(uncompressed_input);
                let mut group_num = 0;
                loop {
                    // XX preallocate hopefully with right size,
                    // should take as arg if anything
                    let mut linegroup = String::with_capacity(300 * num_lines_per_chunk);
                    for _ in 0..num_lines_per_chunk {
                        match input.read_line(&mut linegroup) {
                            Ok(n) => {
                                if n == 0 {
                                    return send_off(linegroup, group_num);
                                } else {
                                    ()
                                }
                            }
                            Err(e) => {
                                w.send((
                                    group_num,
                                    Err(e).with_context(|| anyhow!("reading file {path:?}")),
                                ))?;
                                return Ok(());
                            }
                        }
                    }
                    send_off(linegroup, group_num)?;
                    group_num += 1;
                }
            }
        })?;

    Ok(r.into_iter())
}
