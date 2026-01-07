use std::io::Write;
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader},
};

use anyhow::{bail, Result};

pub fn ask_yn(question: &str) -> Result<bool> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(false);
    let opn = || opts.open("/dev/tty");
    let mut inp = BufReader::new(opn()?);
    let mut outp = opn()?;
    for n in (1..5).rev() {
        write!(outp, "{} (y/n) ", question)?;
        let mut ans = String::new();
        inp.read_line(&mut ans)?;
        if ans.len() > 1 && ans.starts_with("y") {
            return Ok(true);
        } else if ans.len() > 1 && ans.starts_with("n") {
            return Ok(false);
        }
        writeln!(outp, "Please answer with y or n, {} tries left", n)?;
    }
    bail!("Could not get an answer to the question {:?}", question)
}
