// XX I already did something like this somewhere, right?

use std::{os::unix::process::ExitStatusExt, process::ExitStatus};

pub trait ToExitCode {
    fn to_exit_code(&self) -> i32;
}

impl ToExitCode for ExitStatus {
    fn to_exit_code(&self) -> i32 {
        if self.success() {
            0
        } else {
            self.code()
                .or_else(|| self.signal().map(|x| x + 128))
                .unwrap_or(255)
        }
    }
}
