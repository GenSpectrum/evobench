use std::{
    fs::File,
    ops::Deref,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use cj_path_util::path_util::AppendToPath;
use nix::unistd::getuid;
use rand::Rng;

use crate::{ctx, info, utillib::user::get_username};

/// The path to a temporary directory, and [on Linux (because of
/// systems using systemd--Debian from trixie onwards will delete
/// it),] a thread that keeps updating its mtime to prevent
/// deletion. Implements AsRef<Path> and Deref<Target = Path>.
#[derive(Debug, PartialEq, Eq)]
pub struct BenchTmpDir {
    path: Arc<Path>,
}

impl AsRef<Path> for BenchTmpDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Deref for BenchTmpDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

fn start_daemon(path: Arc<Path>) -> Result<()> {
    let th = std::thread::Builder::new().name("tmp-keep-alive".into());
    th.spawn(move || {
        let mut rnd = rand::thread_rng();
        while Arc::strong_count(&path) > 1 {
            // eprintln!("Helloworld");
            let n = rnd.gen_range(0..10000000);
            let path = path.append(format!(".{n}.tmp-keep-alive-dir"));
            if let Ok(_) = File::create_new(&path) {
                _ = std::fs::remove_file(&path);
            } else {
                info!("could not create touch file {path:?}");
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    })?;
    Ok(())
}

/// Returns the path to a temporary directory, creating it if
/// necessary and checking ownership if it already exists. The
/// directory is not unique for all processes, but shared for all
/// evobench instances--which is OK both because we only do 1 run
/// at the same time (and take a lock to ensure that), but also
/// because we're now currently actually also adding the pid to the
/// file paths inside. It is wrapped since it comes with a daemon that
/// keeps updating the directory mtime to prevent deletion by tmp
/// cleaners.
pub fn bench_tmp_dir() -> Result<BenchTmpDir> {
    // XX use src/installation/binaries_repo.rs from xmlhub-indexer
    // instead once that's separated?
    let user = get_username()?;
    match std::env::consts::OS {
        "linux" => {
            let path: PathBuf = format!("/dev/shm/{user}").into();
            let path: Arc<Path> = path.into();

            dbg!((&path, path.exists()));

            match std::fs::create_dir(&path) {
                Ok(()) => {
                    start_daemon(path.clone())?;
                    Ok(BenchTmpDir { path })
                }
                Err(e) => match e.kind() {
                    std::io::ErrorKind::AlreadyExists => {
                        let m = std::fs::metadata(&path)?;
                        let dir_uid = m.uid();
                        let uid: u32 = getuid().into();
                        if dir_uid == uid {
                            start_daemon(path.clone())?;
                            Ok(BenchTmpDir { path })
                        } else {
                            bail!(
                                "bench_tmp_dir: directory {path:?} should be owned by \
                                 the user {user:?} which is set in the USER env var, \
                                 but the uid owning that directory is {dir_uid} whereas \
                                 the current process is running as {uid}"
                            )
                        }
                    }
                    _ => Err(e).map_err(ctx!("create_dir {path:?}")),
                },
            }
        }
        _ => {
            let path: PathBuf = "./tmp".into();
            let path: Arc<Path> = path.into();
            std::fs::create_dir_all(&path).map_err(ctx!("create_dir_all {path:?}"))?;
            start_daemon(path.clone())?;
            Ok(BenchTmpDir { path })
        }
    }
}
