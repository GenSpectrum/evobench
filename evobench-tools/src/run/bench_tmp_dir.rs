use std::{os::unix::fs::MetadataExt, path::PathBuf};

use anyhow::{Result, bail};
use nix::unistd::getuid;

use crate::{ctx, utillib::user::get_username};

/// Returns the path to a temporary directory, creating it if
/// necessary and checking ownership if it already exists. The
/// directory is not unique for all processes, but shared for all
/// evobench-jobs instances--which is OK both because we only do 1 run
/// at the same time (and take a lock to ensure that), but also
/// because we're now currently actually also adding the pid to the
/// file paths inside.
pub fn bench_tmp_dir() -> Result<PathBuf> {
    // XX use src/installation/binaries_repo.rs from xmlhub-indexer
    // instead once that's separated?
    let user = get_username()?;
    match std::env::consts::OS {
        "linux" => {
            let tmp: PathBuf = format!("/dev/shm/{user}").into();

            dbg!((&tmp, tmp.exists()));

            match std::fs::create_dir(&tmp) {
                Ok(()) => Ok(tmp),
                Err(e) => match e.kind() {
                    std::io::ErrorKind::AlreadyExists => {
                        let m = std::fs::metadata(&tmp)?;
                        let dir_uid = m.uid();
                        let uid: u32 = getuid().into();
                        if dir_uid == uid {
                            Ok(tmp)
                        } else {
                            bail!(
                                "bench_tmp_dir: directory {tmp:?} should be owned by \
                                 the user {user:?} which is set in the USER env var, \
                                 but the uid owning that directory is {dir_uid} whereas \
                                 the current process is running as {uid}"
                            )
                        }
                    }
                    _ => Err(e).map_err(ctx!("create_dir {tmp:?}")),
                },
            }
        }
        _ => {
            let tmp: PathBuf = "./tmp".into();
            std::fs::create_dir_all(&tmp).map_err(ctx!("create_dir_all {tmp:?}"))?;
            Ok(tmp)
        }
    }
}
