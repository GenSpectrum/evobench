//! Hack to get terminal width to allow making older Clap versions
//! auto-adapt to the current width.

use std::{fs::File, os::fd::AsRawFd};

use terminal_size::{terminal_size, terminal_size_using_fd, Height, Width};

/// Unlike `terminal_size::terminal_size()` which uses stdout, this
/// opens `/dev/tty` if possible, then falls back to the former.
pub fn terminal_size_using_tty() -> Option<(Width, Height)> {
    if let Ok(file) = File::open("/dev/tty") {
        terminal_size_using_fd(file.as_raw_fd())
    } else {
        terminal_size()
    }
}

/// Always return a width, fall back to a default value of 120.
pub fn get_terminal_width() -> usize {
    let default = 120;
    if let Some((terminal_size::Width(width), _height)) = terminal_size_using_tty() {
        usize::from(width).checked_sub(4).unwrap_or(default)
    } else {
        default
    }
}
