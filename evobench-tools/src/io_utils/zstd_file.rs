use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::Path,
    process::{ChildStdout, Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use cj_path_util::unix::polyfill::add_extension;
use memmap2::{Mmap, MmapOptions};
use ruzstd::{FrameDecoder, StreamingDecoder};

use crate::{ctx, io_utils::tempfile_utils::TempfileOptions};

// For decompression; compression is always done via tool.
const USING_EXTERNAL_TOOL: bool = false;

#[derive(Debug, PartialEq)]
enum Extension {
    ZStd,
    Other,
}

// Expect a file extension in `path`, return whether it is "zstd" or
// the `expected_suffix`. Anything else yields an error. If not given,
// accepts any suffix (but one is required).
fn file_extension<P: AsRef<Path>>(path: P, expected_suffix: Option<&str>) -> Result<Extension> {
    let path = path.as_ref();
    let ext = path.extension().ok_or_else(|| {
        let _hold;
        let extension_msg = if let Some(expected_suffix) = expected_suffix {
            _hold = format!("{expected_suffix:?}");
            &_hold
        } else {
            "any extension"
        };
        anyhow!("missing file extension, expecting {extension_msg} or \"zstd\": {path:?}")
    })?;

    match ext.to_string_lossy().as_ref() {
        "zstd" => {
            if let Some(expected_suffix) = expected_suffix {
                let stem = path.with_extension("");
                let ext2 = stem.extension().ok_or_else(|| {
                    anyhow!(
                        "missing second file extension, after \"zstd\", \
                     expecting {expected_suffix:?}: {path:?}"
                    )
                })?;
                match ext2.to_string_lossy().as_ref() {
                    s if &*s == expected_suffix => Ok(Extension::ZStd),
                    _ => bail!(
                        "unknown second file extension {ext2:?} after \"zstd\", \
                     expecting {expected_suffix:?}: {path:?}"
                    ),
                }
            } else {
                Ok(Extension::ZStd)
            }
        }
        ext_str => {
            if let Some(expected_suffix) = expected_suffix {
                if ext_str == expected_suffix {
                    Ok(Extension::Other)
                } else {
                    bail!(
                        "unknown file extension {ext:?}, expecting {expected_suffix:?} \
                     or \"zstd\": {path:?}"
                    )
                }
            } else {
                Ok(Extension::Other)
            }
        }
    }
}

#[test]
fn t_file_extension() {
    use Extension::*;
    let ok = |a: &str, b: &'static str| {
        file_extension(a, Some(b)).expect("test call should not give an error")
    };
    let err = |a: &str, b: &'static str| {
        file_extension(a, Some(b))
            .err()
            .expect("test call should give an error")
            .to_string()
    };
    assert_eq!(ok("foo.x", "x"), Other);
    assert_eq!(ok("foo.x.zstd", "x"), ZStd);
    assert_eq!(ok("foo.z.x", "x"), Other);
    assert_eq!(ok("foo.z.x.zstd", "x"), ZStd);
    assert_eq!(
        err("foo.x", "y"),
        "unknown file extension \"x\", expecting \"y\" or \"zstd\": \"foo.x\""
    );
    assert_eq!(
        err("foo.x.zstd", "y"),
        "unknown second file extension \"x\" after \"zstd\", expecting \"y\": \"foo.x.zstd\""
    );
    assert_eq!(
        err("foo.zstd", "y"),
        "missing second file extension, after \"zstd\", expecting \"y\": \"foo.zstd\""
    );
    assert_eq!(
        err("foo", "y"),
        "missing file extension, expecting \"y\" or \"zstd\": \"foo\""
    );
}

pub trait SendRead: Read + Send {}

impl SendRead for StreamingDecoder<std::fs::File, FrameDecoder> {}
impl SendRead for ChildStdout {}
impl SendRead for File {}

/// Transparently decompress zstd files if they have a .zstd suffix;
/// after that, expecting the `expected_suffix` (which must be given
/// *without* a leading dot) if given.
pub fn decompressed_file(path: &Path, expected_suffix: Option<&str>) -> Result<Box<dyn SendRead>> {
    let ext = file_extension(path, expected_suffix)?;

    let file_open = || File::open(path).with_context(|| anyhow!("opening file {path:?}"));

    match ext {
        Extension::ZStd => {
            if USING_EXTERNAL_TOOL {
                let mut c = Command::new("zstd");
                let args: Vec<OsString> = vec!["-dcf".into(), "--".into(), path.into()];
                c.args(args);
                c.stdout(Stdio::piped());
                let child = c.spawn().map_err(ctx!("spawning command {c:?}"))?;
                Ok(Box::new(child.stdout.expect("present since configured")))
            } else {
                let input = file_open()?;
                Ok(Box::new(
                    StreamingDecoder::new(input).map_err(ctx!("zstd-decoding {path:?}"))?,
                ))
            }
        }
        Extension::Other => Ok(Box::new(file_open()?)),
    }
}

/// Open the file as a mmap. `.zstd` files are first decompressed to
/// `uncompressed_path` (or `.zstd.uncompressed` if not given) if the
/// path does not exist already. The MMap is created using 2MB
/// huge-pages on Linux. The usual caveats for memory maps applies:
/// modifications of the file while using the map can change the data
/// during parsing, which is safe or not depending on the
/// parser. Truncating the file while accessing it will segfault the
/// process. Leaving it marked safe here, for now.
pub fn decompressed_file_mmap(
    path: &Path,
    uncompressed_path: Option<&Path>,
    expected_suffix: Option<&str>,
) -> Result<Mmap> {
    let ext = file_extension(path, expected_suffix)?;

    let file_open =
        |path: &Path| File::open(path).with_context(|| anyhow!("opening file {path:?}"));

    let tmp;
    let uncompressed_path = match ext {
        Extension::ZStd => {
            let uncompressed_path = if let Some(uncompressed_path) = uncompressed_path {
                uncompressed_path.to_owned()
            } else {
                add_extension(path, "uncompressed")
                    .ok_or_else(|| anyhow!("appending extension to {path:?}"))?
            };
            if !uncompressed_path.exists() {
                let tmp = TempfileOptions {
                    target_path: uncompressed_path.clone(),
                    retain_tempfile: false,
                    migrate_access: false,
                }
                .tempfile()?;

                let mut c = Command::new("zstd");
                let args: Vec<OsString> = vec![
                    "-df".into(),
                    "--quiet".into(),
                    "-o".into(),
                    tmp.temp_path().into(),
                    "--".into(),
                    path.into(),
                ];
                c.args(args);
                let mut child = c.spawn().map_err(ctx!("spawning command {c:?}"))?;
                let status = child.wait()?;
                if !status.success() {
                    bail!("{c:?} failed: {status}");
                }
                tmp.finish()?;
            }
            tmp = uncompressed_path;
            &tmp
        }
        Extension::Other => path,
    };

    let input = file_open(&uncompressed_path)?;

    let meta = input.metadata()?;
    let size: usize = meta.size().try_into()?;
    unsafe {
        // As safe as the function docs says
        MmapOptions::new().huge(Some(21)).len(size).map(&input)
    }
    .map_err(ctx!("mmap for file {uncompressed_path:?}"))
}

/// If quiet is false, lets messaging by the `zstd` tool show up on
/// stdout/err. If true, silences reporting output but captures error
/// messages and reports those in the resulting error.
pub fn compress_file(source_path: &Path, target_path: &Path, quiet: bool) -> Result<()> {
    let mut c = Command::new("zstd");
    if quiet {
        c.arg("--quiet");
        c.stdout(Stdio::piped());
        c.stderr(Stdio::piped());
    }
    let args: &[&OsStr] = &[
        "-o".as_ref(),
        // XX: is this argument position safe against option injection?
        target_path.as_ref(),
        "--".as_ref(),
        source_path.as_ref(),
    ];
    c.args(args);
    let output = c.output().map_err(ctx!("running command {c:?}"))?;
    let status = output.status;
    if status.success() {
        Ok(())
    } else {
        let outputs = if quiet {
            let mut outputs = String::from_utf8_lossy(&output.stdout).into_owned();
            outputs.push_str(&String::from_utf8_lossy(&output.stderr));
            format!("{outputs:?}")
        } else {
            "not captured".into()
        };
        bail!("running zstd {args:?}: {status} with outputs {outputs}")
    }
}
