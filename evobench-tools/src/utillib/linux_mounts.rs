use std::{
    collections::HashMap,
    fs::Metadata,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, anyhow};
use kstring::KString;

use crate::{ctx, utillib::into_arc_path::IntoArcPath};

pub struct MountPoint {
    pub device_name: KString,
    pub path: KString,
    pub fstype: KString,
    pub options: KString,
    // Ignore remaining fields
}

impl MountPoint {
    pub fn is_tmpfs(&self) -> bool {
        &self.fstype == "tmpfs"
    }

    pub fn path(&self) -> Arc<Path> {
        self.path.as_str().into_arc_path()
    }

    pub fn path_buf(&self) -> PathBuf {
        self.path.as_str().into()
    }

    pub fn path_metadata(&self) -> Result<Metadata> {
        let path: &Path = self.path.as_str().as_ref();
        path.metadata()
            .map_err(ctx!("getting metadata for {path:?}"))
    }
}

pub struct MountPoints {
    mount_points: Vec<MountPoint>,
    by_path: HashMap<KString, usize>,
}

impl MountPoints {
    pub fn read() -> Result<Self> {
        let input =
            std::fs::read_to_string("/proc/mounts").map_err(ctx!("reading /proc/mounts"))?;
        let mount_points: Vec<MountPoint> = input
            .trim_end()
            .split("\n")
            .map(|line| -> Result<MountPoint> {
                let mut items = line.split(" ");
                macro_rules! let_get {
                { $name:tt } =>  {
                    let $name = KString::from_ref(items.next().ok_or_else(
                        || anyhow!("missing {:?}", stringify!( $name)))?);
                }
            }
                let_get!(device_name);
                let_get!(path);
                let_get!(fstype);
                let_get!(options);
                Ok(MountPoint {
                    device_name,
                    path,
                    fstype,
                    options,
                })
            })
            .collect::<Result<_>>()?;

        let by_path = mount_points
            .iter()
            .enumerate()
            .map(|(i, p)| (p.path.clone(), i))
            .collect();

        Ok(MountPoints {
            mount_points,
            by_path,
        })
    }

    pub fn get_by_path(&self, path: &str) -> Option<&MountPoint> {
        self.by_path.get(path).map(|i| &self.mount_points[*i])
    }
}
