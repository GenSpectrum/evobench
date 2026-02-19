use std::{
    collections::{HashMap, hash_map::Entry},
    env::temp_dir,
    fs::File,
    io::Write,
    mem::swap,
    ops::Deref,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
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
    utillib::{into_arc_path::IntoArcPath, linux_mounts::MountPoints, user::get_username},
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

fn _start_daemon(path: Arc<Path>, is_tmpfs: bool) -> Result<JoinHandle<()>, std::io::Error> {
    let sleep_time = Duration::from_millis(if is_tmpfs { 100 } else { 10000 });

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

                        std::thread::sleep(sleep_time);
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
            std::thread::sleep(sleep_time);
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

fn start_daemon(path: Arc<Path>, is_tmpfs: bool) -> Result<()> {
    let mut daemons = DAEMONS.lock().expect("no panics in this scope");
    let m = daemons.get_or_insert_with(|| HashMap::new());
    let r = match m.entry(path.clone()) {
        Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
        Entry::Vacant(vacant_entry) => vacant_entry.insert(_start_daemon(path, is_tmpfs)),
    };
    r.as_ref()
        .map(|_| ())
        .map_err(|e| anyhow!("start_daemon: {e:#}"))
}

/// Try to find the best place for putting the evobench.log, while
/// avoiding tmp file cleaners like systemd and staying portable. Also
/// returns if the path is pointing to a tmpfs.
pub fn get_fast_and_large_temp_dir_base() -> Result<(PathBuf, bool)> {
    // XX use src/installation/binaries_repo.rs from xmlhub-indexer
    // instead once that's separated?
    match std::env::consts::OS {
        "linux" => {
            let mount_points = MountPoints::read()?;

            if let Some(tmp) = mount_points.get_by_path("/tmp") {
                if tmp.is_tmpfs() {
                    let tmp_metadata = tmp.path_metadata()?;
                    if let Some(dev_shm) = mount_points.get_by_path("/dev/shm") {
                        let dev_shm_metadata = dev_shm.path_metadata()?;
                        if tmp_metadata.ino() == dev_shm_metadata.ino() {
                            return Ok((tmp.path_buf(), true));
                        }
                    }
                    // XX todo check if large enough
                    return Ok((tmp.path_buf(), true));
                } else {
                    if let Some(dev_shm) = mount_points.get_by_path("/dev/shm") {
                        // XX todo check Debian release? oldstable is OK, stable not.
                        return Ok((dev_shm.path_buf(), dev_shm.is_tmpfs()));
                    }
                    // XX ?
                    return Ok((tmp.path_buf(), false));
                }
            } else {
                if let Some(dev_shm) = mount_points.get_by_path("/dev/shm") {
                    // XX todo check Debian release? oldstable is OK, stable not.
                    return Ok((dev_shm.path_buf(), dev_shm.is_tmpfs()));
                }
                return Ok((temp_dir(), false));
            }
        }
        _ => Ok((temp_dir(), false)),
    }
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
    let (base, is_tmpfs) = get_fast_and_large_temp_dir_base()?;
    let user = get_username()?;
    let path = base.append(&user).into_arc_path();

    match std::fs::create_dir(&path) {
        Ok(()) => {
            start_daemon(path.clone(), is_tmpfs)?;
            Ok(BenchTmpDir { path })
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                let m = std::fs::metadata(&path)?;
                let dir_uid = m.uid();
                let uid: u32 = getuid().into();
                if dir_uid == uid {
                    start_daemon(path.clone(), is_tmpfs)?;
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
