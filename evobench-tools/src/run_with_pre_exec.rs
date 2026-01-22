use std::{borrow::Cow, ffi::OsStr, fmt::Display, path::Path, process::Command};

use crate::io_utils::bash::bash_string_from_program_path_and_args;

#[derive(Debug, Clone, Copy)]
pub enum BashSettingsLevel {
    None,
    SetMEU,
    SetMEUPipefail,
}

impl BashSettingsLevel {
    pub fn str(self) -> &'static str {
        match self {
            BashSettingsLevel::None => "",
            BashSettingsLevel::SetMEU => "set -meu",
            BashSettingsLevel::SetMEUPipefail => "set -meuo pipefail",
        }
    }
}

pub struct BashSettings {
    pub level: BashSettingsLevel,
    pub set_ifs: bool,
}

impl Display for BashSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\n{}",
            self.level.str(),
            if self.set_ifs { "IFS=\n" } else { "" }
        )
    }
}

pub struct RunWithPreExec<'t> {
    pub pre_exec_bash_code: Cow<'t, str>,
    pub bash_settings: BashSettings,
    pub bash_path: Option<Cow<'t, Path>>,
}

impl<'t> RunWithPreExec<'t> {
    /// Return a command that executes the given command and args
    /// directly if `pre_exec_bash_code` is empty (after trim), or
    /// executes "bash" or the given `bash_path` with the given
    /// pre-exec code plus `exec` to the given command and args.  The
    /// `command_path` has to be a str since it must be possible to
    /// represent it as unicode to be made part of bash code.
    pub fn command<S: AsRef<OsStr> + AsRef<str>>(
        &self,
        command_path: &str,
        args: impl AsRef<[S]>,
    ) -> Command {
        let Self {
            pre_exec_bash_code,
            bash_settings,
            bash_path,
        } = self;

        if pre_exec_bash_code.trim().is_empty() {
            let mut command = Command::new(command_path);
            command.args(args.as_ref());
            command
        } else {
            let mut command = {
                let bash: &Path = "bash".as_ref();
                let bash = bash_path.as_ref().map(AsRef::as_ref).unwrap_or(bash);
                Command::new(bash)
            };
            command.arg("-c");
            {
                let mut code = bash_settings.to_string();
                code.push_str(&pre_exec_bash_code);
                code.push_str("\n\nexec ");
                code.push_str(&bash_string_from_program_path_and_args(
                    command_path,
                    args.as_ref(),
                ));
                command.arg(code);
            }
            command
        }
    }
}

pub fn join_pre_exec_bash_code(a: &str, b: &str) -> String {
    let mut s = String::new();
    if !a.trim().is_empty() {
        s.push_str(a);
        s.push_str("\n\n");
    }
    if !b.trim().is_empty() {
        s.push_str(b);
        s.push_str("\n\n");
    }
    s
}
