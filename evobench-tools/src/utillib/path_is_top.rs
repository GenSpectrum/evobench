use std::path::Path;

use nix::NixPath;

pub trait PathIsTop {
    fn is_top(&self) -> bool;
}

impl PathIsTop for &Path {
    fn is_top(&self) -> bool {
        self.is_empty() || self == &AsRef::<Path>::as_ref("/")
    }
}
