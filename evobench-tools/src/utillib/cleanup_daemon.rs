//! A daemon process that clean ups open files when the main process
//! is killed.

use std::{
    collections::HashSet,
    fs::{remove_dir_all, remove_file},
    ops::Deref,
    path::Path,
    process::{Command, exit},
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail};
use chj_unix_util::unix::easy_fork;
use derive_more::From;
use nix::unistd::{Pid, setsid};
use serde::{Deserialize, Serialize};

use crate::{
    debug, info,
    utillib::{
        arc::CloneArc,
        escaped_display::{AsEscapedString, DebugForDisplay},
        into_arc_path::IntoArcPath,
        ndjson_pipe::{NdJsonPipe, NdJsonPipeWriter},
    },
    warn,
};

trait RunCleanup {
    /// If successful, returns a string describing what was done (if
    /// nothing was to be done then None is returned; if there was an
    /// error carrying out what was to be done then an error is
    /// returned)
    fn run_cleanup(&self) -> Result<Option<String>>;
}

/// Note: paths need to be absolute (or canonical), or using chdir in
/// the app after starting the daemon will lead to breakage!  Use the
/// constructor functions rather than constructing this type directly!
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Deletion {
    /// A single file, deleted via `remove_file`, does not work for dirs
    File(Arc<Path>),
    /// A directory, deleted via `remove_dir_all`, also works on
    /// single files but is more dangerous.
    Dir(Arc<Path>),
}

impl Deref for Deletion {
    type Target = Arc<Path>;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::File(path) => path,
            Self::Dir(path) => path,
        }
    }
}

impl AsRef<Path> for Deletion {
    fn as_ref(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Dir(path) => path,
        }
    }
}

impl Deletion {
    /// Construct a single-file deletion action from a path; the path
    /// is made absolute based on the current cwd if needed, errors
    /// during that process are reported
    pub fn file(path: impl IntoArcPath + AsRef<Path>) -> Result<Self> {
        Ok(Self::File(std::path::absolute(path)?.into()))
    }

    /// Same as `file` but to delete dir trees
    pub fn dir(path: impl IntoArcPath + AsRef<Path>) -> Result<Self> {
        Ok(Self::Dir(std::path::absolute(path)?.into()))
    }

    /// Note that this could be a path to an executable, something to
    /// run, not delete!
    pub fn path(&self) -> &Arc<Path> {
        match self {
            Deletion::File(path) => path,
            Deletion::Dir(path) => path,
        }
    }
}

impl RunCleanup for Deletion {
    fn run_cleanup(&self) -> Result<Option<String>> {
        let raise_errors = |r: Result<(), std::io::Error>,
                            doing_msg: &str,
                            did_msg: &str,
                            path: &Path|
         -> Result<_> {
            match r {
                Ok(()) => Ok(Some(format!("{did_msg} {path:?}"))),
                Err(e) => match e.kind() {
                    std::io::ErrorKind::NotFound => Ok(None),
                    _ => {
                        bail!("{doing_msg} {path:?}: {e:#}");
                    }
                },
            }
        };
        match self {
            Deletion::File(path) => {
                raise_errors(remove_file(path), "deleting file", "deleted file", path)
            }
            Deletion::Dir(path) => raise_errors(
                remove_dir_all(path),
                "deleting dir tree",
                "deleted dir tree",
                path,
            ),
        }
    }
}

/// A command to run (with current working directory, but
/// otherwise unchanged environment from the time of starting the
/// daemon)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CleanupCommand {
    pub path: Arc<Path>,
    pub args: Arc<[String]>,
    pub cwd: Option<Arc<Path>>,
}

impl RunCleanup for CleanupCommand {
    fn run_cleanup(&self) -> Result<Option<String>> {
        let Self { path, args, cwd } = self;
        let mut cmd = Command::new(&**path);
        cmd.args(&**args);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        match cmd.status() {
            Ok(status) => {
                if status.success() {
                    Ok(Some(format!("successfully executed command {cmd:?}")))
                } else {
                    bail!("error: command {cmd:?} exited with status {status}")
                }
            }
            Err(e) => {
                bail!("error: could not run command {path:?}: {e:#}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, From)]
pub enum CleanupItem {
    Deletion(Deletion),
    CleanupCommand(CleanupCommand),
}

impl RunCleanup for CleanupItem {
    fn run_cleanup(&self) -> Result<Option<String>> {
        match self {
            CleanupItem::Deletion(d) => d.run_cleanup(),
            CleanupItem::CleanupCommand(c) => c.run_cleanup(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CleanupMessage {
    /// Add an item for clean up on exit
    Add(CleanupItem),
    /// Remove an item from being cleaned up
    Cancel(CleanupItem),
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
                let mut items: HashSet<CleanupItem> = Default::default();
                for msg in reader {
                    let msg = msg?;
                    debug!("got message {msg:?}");
                    match msg {
                        CleanupMessage::Add(item) => {
                            items.insert(item);
                        }
                        CleanupMessage::Cancel(path) => {
                            items.remove(&path);
                        }
                    }
                }
                for item in items {
                    match item.run_cleanup() {
                        Ok(None) => (),
                        Ok(Some(did)) => {
                            info!("{did}")
                        }
                        Err(e) => {
                            warn!("{e:#}")
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
pub struct CleanupHandler {
    daemon: Arc<Mutex<CleanupDaemon>>,
}

impl CleanupHandler {
    /// See the warning on `CleanupDaemon::start`, i.e. you must only
    /// run this while there are no additional threads or this will
    /// panic!
    pub fn start() -> Result<Self> {
        let daemon = Mutex::new(CleanupDaemon::start()?).into();
        Ok(Self { daemon })
    }

    /// Note: if you are looking for auto-cleanup you want to use
    /// `register_temporary_file` or `register_temporary_command`
    /// instead. (Should this method even be public?)
    fn register_cleanup<Item: Clone + Into<CleanupItem>>(
        &self,
        item: Item,
    ) -> Result<ItemWithCleanup<Item>> {
        {
            let mut daemon = self.daemon.lock().expect("no panics");
            daemon.send(CleanupMessage::Add(item.clone().into()))?;
        }
        Ok(ItemWithCleanup {
            item,
            daemon: self.daemon.clone_arc(),
        })
    }

    pub fn register_temporary_file(&self, deletion: Deletion) -> Result<TemporaryFile> {
        Ok(TemporaryFile(Some(self.register_cleanup(deletion)?)))
    }

    pub fn register_temporary_command(&self, deletion: CleanupCommand) -> Result<TemporaryCommand> {
        Ok(TemporaryCommand(Some(self.register_cleanup(deletion)?)))
    }
}

struct ItemWithCleanup<Item: Into<CleanupItem>> {
    item: Item,
    daemon: Arc<Mutex<CleanupDaemon>>,
}

impl<Item: Into<CleanupItem> + RunCleanup> ItemWithCleanup<Item> {
    fn cancel_cleanup(self) -> Result<()> {
        let Self { item, daemon } = self;
        {
            let mut daemon = daemon.lock().expect("no panics");
            daemon.send(CleanupMessage::Cancel(item.into()))?;
        }
        Ok(())
    }

    // /// Does *not* cancel the cleanup! Is this unintuitive, I guess?
    // /// Use `cancel_cleanup` for when you want to get rid of the
    // /// cleanup "wrapping".
    // fn into_inner(self) -> Item {
    //     self.item
    // }

    /// Deletes the file (if present) then cancels the cleanup
    fn cleanup_now(self) -> Result<Option<String>> {
        let did = self.item.run_cleanup()?;
        // XX should we ignore errors here? OK for now.
        self.cancel_cleanup()?;
        Ok(did)
    }
}

// Have to provide Drop *separate* from a type that offers
// ownership-accepting methods that expect to destructure the type
// (and its Drop to not be called any more). I.e. layer the
// concerns. Thus, TemporaryFile.

pub struct TemporaryFile(Option<ItemWithCleanup<Deletion>>);

impl Deref for TemporaryFile {
    type Target = Arc<Path>;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("only Drop can take it out and then Deref can't be called anymore")
            .item
            .deref()
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

impl AsEscapedString for TemporaryFile {
    type ViewableType<'t>
        = DebugForDisplay<&'t Path>
    where
        Self: 't;

    fn as_escaped_string<'s>(&'s self) -> Self::ViewableType<'s> {
        DebugForDisplay(self.as_ref())
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if let Some(iwc) = self.0.take() {
            match iwc.cleanup_now() {
                Ok(_) => (),
                Err(e) => {
                    warn!("TemporaryFile: error in drop: {e:#}");
                }
            }
        }
    }
}

pub struct TemporaryCommand(Option<ItemWithCleanup<CleanupCommand>>);

impl Drop for TemporaryCommand {
    fn drop(&mut self) {
        if let Some(iwc) = self.0.take() {
            match iwc.cleanup_now() {
                Ok(_) => (),
                Err(e) => {
                    warn!("TemporaryCommand: error in drop: {e:#}");
                }
            }
        }
    }
}
