pub trait ExitStatusExt {
    fn status_and_outputs(&self) -> (std::process::ExitStatus, String);
}

impl ExitStatusExt for std::process::Output {
    fn status_and_outputs(&self) -> (std::process::ExitStatus, String) {
        let stdout = String::from_utf8_lossy(&self.stdout);
        let stderr = String::from_utf8_lossy(&self.stderr);
        let mut outputs = Vec::new();
        if !stdout.is_empty() {
            let need_newline = !stdout.ends_with("\n");
            outputs.push(stdout);
            if need_newline {
                outputs.push("\n".into())
            }
        }
        if !stderr.is_empty() {
            let need_newline = !stderr.ends_with("\n");
            outputs.push(stderr);
            if need_newline {
                outputs.push("\n".into())
            }
        }
        (self.status, outputs.join(""))
    }
}
