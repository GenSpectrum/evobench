//! A daemon process that clean ups open files when the main process
//! is killed.

use std::{
    collections::HashMap,
    fs::{remove_dir_all, remove_file},
    ops::Deref,
    path::Path,
    process::exit,
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail};
use chj_unix_util::unix::easy_fork;
use nix::unistd::{Pid, setsid};
use serde::{Deserialize, Serialize};

use crate::{
    debug, info,
    utillib::{
        arc::CloneArc,
        ndjson_pipe::{NdJsonPipe, NdJsonPipeWriter},
    },
    warn,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeletionItem {
    /// A single file, deleted via `remove_file`, does not work for dirs
    File(Arc<Path>),
    /// A directory, deleted via `remove_dir_all`, also works on
    /// single files but is more dangerous.
    Dir(Arc<Path>),
}

impl AsRef<Path> for DeletionItem {
    fn as_ref(&self) -> &Path {
        match self {
            DeletionItem::File(path) => path,
            DeletionItem::Dir(path) => path,
        }
    }
}

impl DeletionItem {
    fn as_arc_path(&self) -> &Arc<Path> {
        match self {
            DeletionItem::File(path) => path,
            DeletionItem::Dir(path) => path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CleanupMessage {
    /// Add a path for deletion on exit, and whether it is to a single
    /// file or a directory
    Add(DeletionItem),
    /// Remove a path from being deleted (regardless of whether dir or
    /// file)
    Cancel(Arc<Path>),
}

#[derive(Debug)]
pub struct CleanupDaemon {
    _child_pid: Pid,
    writer: NdJsonPipeWriter<CleanupMessage>,
}

impl CleanupDaemon {
    /// Create a daemon that can receive information about open
    /// files. This forks a separate process into a new unix session
    /// so that it is not killed. Must be run while there are no
    /// additional threads, otherwise this panics! When the process
    /// holding the `CleanupDaemon` struct (the parent) exits (in
    /// whatever way), the forked daemon process detects the pipe
    /// filehandles closing, and then deletes all files that haven't
    /// been cancelled.
    pub fn start() -> Result<Self> {
        let pipe = NdJsonPipe::<CleanupMessage>::new()?;

        if let Some(_child_pid) = easy_fork()? {
            let writer = pipe.into_writer();
            Ok(Self { _child_pid, writer })
        } else {
            // Child
            let r = (|| -> Result<()> {
                // Do we have to double fork? Things work without so
                // far--the parent will get signals, but that may
                // actually be interesting to register daemon errors.

                setsid()?;

                let reader = pipe.into_reader();
                // True means it's dir deletion
                let mut files: HashMap<Arc<Path>, bool> = HashMap::new();
                for msg in reader {
                    let msg = msg?;
                    debug!("got message {msg:?}");
                    match msg {
                        CleanupMessage::Add(item) => {
                            match item {
                                DeletionItem::File(path) => files.insert(path, false),
                                DeletionItem::Dir(path) => files.insert(path, true),
                            };
                        }
                        CleanupMessage::Cancel(path) => {
                            files.remove(&path);
                        }
                    }
                }
                for (path, is_dir) in files {
                    if is_dir {
                        match remove_dir_all(&path) {
                            Ok(()) => {
                                info!("deleted dir tree {path:?}");
                            }
                            Err(e) => match e.kind() {
                                std::io::ErrorKind::NotFound => (),
                                _ => {
                                    warn!("could not delete dir tree {path:?}: {e:#}");
                                }
                            },
                        }
                    } else {
                        match remove_file(&path) {
                            Ok(()) => {
                                info!("deleted file {path:?}");
                            }
                            Err(e) => match e.kind() {
                                std::io::ErrorKind::NotFound => (),
                                _ => {
                                    warn!("could not delete file {path:?}: {e:#}");
                                }
                            },
                        }
                    }
                }
                Ok(())
            })();
            match r {
                Ok(()) => {
                    debug!("exiting cleanly");
                    exit(0);
                }
                Err(e) => {
                    warn!("terminating due to error: {e:#}");
                    exit(1);
                }
            }
        }
    }

    /// Warning: only send absolute paths, or if you don't, make sure
    /// that you don't use chdir in the parent after starting the
    /// daemon!
    pub fn send(&mut self, message: CleanupMessage) -> Result<()> {
        self.writer.send(message)
    }
}

#[derive(Debug, Clone)]
pub struct FileCleanupHandler {
    daemon: Arc<Mutex<CleanupDaemon>>,
}

impl FileCleanupHandler {
    /// See the warning on `CleanupDaemon::start`, i.e. you must only
    /// run this while there are no additional threads or this will
    /// panic!
    pub fn start() -> Result<Self> {
        let daemon = Mutex::new(CleanupDaemon::start()?).into();
        Ok(Self { daemon })
    }

    /// Paths that are not absolute are canonicalized. Note: if you
    /// are looking for auto-cleanup you want to use
    /// `register_temporary_file` instead. (Should this method even be
    /// public?)
    pub fn register_cleanup(&self, item: DeletionItem) -> Result<PathWithCleanup> {
        let path = item.as_ref();
        let canonicalized = if path.is_absolute() {
            None
        } else {
            Some(path.canonicalize()?.into())
        };
        {
            let mut daemon = self.daemon.lock().expect("no panics");
            daemon.send(CleanupMessage::Add(item.clone()))?;
        }
        Ok(PathWithCleanup {
            item,
            canonicalized,
            daemon: self.daemon.clone_arc(),
        })
    }

    pub fn register_temporary_file(&self, item: DeletionItem) -> Result<TemporaryFile> {
        Ok(TemporaryFile(Some(self.register_cleanup(item)?)))
    }
}

pub struct PathWithCleanup {
    item: DeletionItem,
    // Only if `path` is not absolute
    canonicalized: Option<Arc<Path>>,
    daemon: Arc<Mutex<CleanupDaemon>>,
}

impl PathWithCleanup {
    pub fn cancel_cleanup(self) -> Result<DeletionItem> {
        let Self {
            item,
            canonicalized,
            daemon,
        } = self;
        let abs_path = canonicalized.unwrap_or_else(|| item.as_arc_path().clone_arc());
        {
            let mut daemon = daemon.lock().expect("no panics");
            daemon.send(CleanupMessage::Cancel(abs_path))?;
        }
        Ok(item)
    }

    /// Does *not* cancel the cleanup! Is this unintuitive, I guess?
    /// Use `cancel_cleanup` for when you want to get rid of the
    /// cleanup "wrapping".
    pub fn into_inner(self) -> DeletionItem {
        self.item
    }

    /// Deletes the file (if present) then cancels the cleanup
    pub fn cleanup_now(self) -> Result<DeletionItem> {
        let raise_errors = |r: Result<(), std::io::Error>, msg: &str, path: &Path| -> Result<()> {
            match r {
                Ok(()) => Ok(()),
                Err(e) => match e.kind() {
                    std::io::ErrorKind::NotFound => Ok(()),
                    _ => {
                        bail!("{msg} {path:?}: {e:#}");
                    }
                },
            }
        };

        // OK to use the original paths, not the canonical ones?
        match &self.item {
            DeletionItem::File(path) => {
                raise_errors(remove_file(path), "deleting file", path)?;
            }
            DeletionItem::Dir(path) => {
                raise_errors(remove_dir_all(path), "deleting dir tree", path)?;
            }
        }
        self.cancel_cleanup()
    }
}

// Have to provide Drop *separate* from a type that offers
// ownership-accepting methods that expect to destructure the type
// (and its Drop to not be called any more). I.e. layer the
// concerns. Thus, TemporaryFile.

pub struct TemporaryFile(Option<PathWithCleanup>);

impl Deref for TemporaryFile {
    type Target = Arc<Path>;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("only Drop can take it out and then Deref can't be called anymore")
            .item
            .as_arc_path()
    }
}

impl AsRef<Arc<Path>> for TemporaryFile {
    fn as_ref(&self) -> &Arc<Path> {
        &**self
    }
}

impl AsRef<Path> for TemporaryFile {
    fn as_ref(&self) -> &Path {
        &**self
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if let Some(pwc) = self.0.take() {
            match pwc.cleanup_now() {
                Ok(_) => (),
                Err(e) => {
                    warn!("TemporaryFile: error in drop: {e:#}");
                }
            }
        }
    }
}
