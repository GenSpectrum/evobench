use std::{
    collections::{HashMap, hash_map::Entry},
    fs::File,
    io::Write,
    mem::swap,
    ops::Deref,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use cj_path_util::path_util::AppendToPath;
use nix::unistd::{getpid, getuid};
use rand::Rng;

use crate::{
    ctx, info,
    utillib::{into_arc_path::IntoArcPath, user::get_username},
    warn,
};

/// The path to a temporary directory, and [on Linux (because of
/// systems using systemd--Debian from trixie onwards will delete
/// it),] a thread that keeps updating its mtime to prevent
/// deletion. Implements `AsRef<Path>` and `Deref<Target = Path>`.
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

fn _start_daemon(path: Arc<Path>) -> Result<JoinHandle<()>, std::io::Error> {
    let th = std::thread::Builder::new().name("tmp-keep-alive".into());
    th.spawn(move || -> () {
        let pid = getpid();

        let mut file_path_0 = path.append(format!(".{pid}.tmp-keep-alive-dir-0"));
        {
            if let Ok(mut f) = File::create(&file_path_0) {
                _ = f.write_all(
                    "This file stays to ensure there is always a file--or now replaced\n"
                        .as_bytes(),
                );
            } else {
                info!("could not create touch file {file_path_0:?}");
            }
        }
        let mut file_path = path.append(format!(".{pid}.tmp-keep-alive-dir"));
        _ = File::create(&file_path);
        let mut rnd = rand::thread_rng();
        while Arc::strong_count(&path) > 1 {
            if rnd.gen_range(0..2) == 0 {
                swap(&mut file_path_0, &mut file_path);
                _ = std::fs::remove_file(&file_path);
            }
            match File::create(&file_path) {
                Ok(mut f) => {
                    for _ in 0..20 {
                        let n = rnd.gen_range(0..10000000);
                        use std::io::Write;
                        if let Err(e) = write!(&mut f, "{n}\n") {
                            info!("could not write to touch file {file_path:?}: {e:#}");
                            break;
                        }

                        if !path.exists() {
                            // COPY-PASTE from below
                            match std::fs::create_dir(&path) {
                                Ok(_) => {
                                    warn!(
                                        "recreated directory {path:?}, worst of all \
                                         total hacks with race condition"
                                    );
                                }
                                Err(e) => {
                                    warn!("could not even recreate directory {path:?}: {e:#}");
                                    break;
                                }
                            }
                        }

                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Err(e) => {
                    warn!("could not create touch file {file_path:?}: {e:#}");
                    match std::fs::create_dir(&path) {
                        Ok(_) => {
                            warn!(
                                "recreated directory {path:?}, worst of all \
                                 total hacks with race condition"
                            );
                        }
                        Err(e) => {
                            warn!("could not even recreate directory {path:?}: {e:#}");
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Remove ourselves, right?
        let mut daemons_guard = DAEMONS.lock().expect("no panics in this scope");
        let daemons = daemons_guard
            .as_mut()
            .expect("already has hashmap since that happens before this thread is started");
        daemons.remove(&path);
    })
}

static DAEMONS: Mutex<Option<HashMap<Arc<Path>, Result<JoinHandle<()>, std::io::Error>>>> =
    Mutex::new(None);

fn start_daemon(path: Arc<Path>) -> Result<()> {
    let mut daemons = DAEMONS.lock().expect("no panics in this scope");
    let m = daemons.get_or_insert_with(|| HashMap::new());
    let r = match m.entry(path.clone()) {
        Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
        Entry::Vacant(vacant_entry) => vacant_entry.insert(_start_daemon(path)),
    };
    r.as_ref()
        .map(|_| ())
        .map_err(|e| anyhow!("start_daemon: {e:#}"))
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
            let path = format!("/dev/shm/{user}").into_arc_path();

            info!("bench_tmp_dir path, exists?: {:?}", (&path, path.exists()));

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
            let path = "./tmp".into_arc_path();
            std::fs::create_dir_all(&path).map_err(ctx!("create_dir_all {path:?}"))?;
            start_daemon(path.clone())?;
            Ok(BenchTmpDir { path })
        }
    }
}
