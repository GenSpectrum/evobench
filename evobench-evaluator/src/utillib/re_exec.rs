use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

pub fn re_exec_with_existing_args_and_env(executable_path: PathBuf) -> std::io::Error {
    let mut args = std::env::args_os();
    let arg0 = args.next();
    let mut cmd = Command::new(&executable_path);
    cmd.args(args);
    if let Some(a0) = arg0 {
        cmd.arg0(a0);
    }
    cmd.exec()
}
