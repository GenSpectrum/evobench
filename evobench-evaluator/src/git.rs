use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    path::Path,
    process::{Command, Stdio},
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use kstring::KString;

fn decode_hex_digit(b: u8) -> Result<u8> {
    if b >= b'0' && b <= b'9' {
        Ok(b - b'0')
    } else if b >= b'a' && b <= b'f' {
        Ok(b - b'a' + 10)
    } else if b >= b'A' && b <= b'F' {
        Ok(b - b'A' + 10)
    } else {
        bail!("byte is not a hex digit: {b}")
    }
}

fn decode_hex<const N: usize>(input: &[u8], output: &mut [u8; N]) -> Result<()> {
    let n2 = 2 * N;
    if input.len() != n2 {
        bail!(
            "wrong number of hex digits, expect {n2}, got {}",
            input.len()
        )
    }
    for i in 0..N {
        output[i] = decode_hex_digit(input[i * 2])? * 16 + decode_hex_digit(input[i * 2 + 1])?;
    }
    Ok(())
}

#[derive(Clone, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitHash([u8; 20]);

impl Debug for GitHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GitHash!(\"")?;
        Display::fmt(self, f)?;
        f.write_str("\")")
    }
}

#[macro_export]
macro_rules! GitHash {
    {$hash:expr} => { GitHash::try_from($hash.as_bytes()).unwrap() }
}

impl TryFrom<&[u8]> for GitHash {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        // let s = std::str::from_utf8(value)?;
        if value.len() != 40 {
            bail!(
                "not a git hash of 40 hex bytes: {:?}",
                String::from_utf8_lossy(value)
            )
        }
        let mut bytes = [0; 20];
        decode_hex(value, &mut bytes)?;
        Ok(Self(bytes))
    }
}

impl FromStr for GitHash {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.chars().count() != s.len() {
            bail!("not an ASCII string: {s:?}")
        }
        GitHash::try_from(s.as_bytes())
    }
}

impl Display for GitHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            f.write_fmt(format_args!("{:1x}{:1x}", b / 16, b & 15))?;
        }
        Ok(())
    }
}

#[test]
fn t_githash() -> Result<()> {
    let s = "18fdd1625c4d98526736ea8e5047a4ca818de0b4";
    let h1 = GitHash::try_from(s.as_bytes())?;
    let h = GitHash!(s);
    assert_eq!(h1, h);
    assert_eq!(h.0[0], 0x18);
    assert_eq!(h.0[1], 0xfd);
    assert_eq!(h.0[2], 0xd1);
    assert_eq!(format!("{h}"), s);
    Ok(())
}

#[derive(Debug)]
pub struct GitCommit {
    pub author_time: u64,
    pub parents: Vec<GitHash>,
}

#[derive(Debug)]
pub struct GitGraph {
    pub entry_reference: KString,
    pub entry_githash: Option<GitHash>,
    pub commits: HashMap<GitHash, GitCommit>,
}

impl GitGraph {
    pub fn new_dir_ref<D: AsRef<Path>>(in_directory: D, entry_reference: &str) -> Result<GitGraph> {
        let in_directory = in_directory.as_ref();
        let mut c = Command::new("git");
        c.args(&["log", "--pretty=%at,%H,%P"]);
        let str_from_bytes =
            |bs| std::str::from_utf8(bs).expect("git always gives ascii with given arguments");
        c.current_dir(in_directory);
        let mut commits = HashMap::new();
        c.stdout(Stdio::piped());
        let output = c
            .output()
            .with_context(|| anyhow!("in directory {in_directory:?}",))?;
        let mut entry_githash: Option<GitHash> = None;
        for line in output.stdout.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let items: Vec<_> = line.split(|b| *b == b',').collect();
            if let [author_time, hash, parents] = items.as_slice() {
                let author_time = u64::from_str_radix(str_from_bytes(author_time), 10)?;
                let hash = GitHash::try_from(*hash)?;
                if entry_githash.is_none() {
                    entry_githash = Some(hash.clone());
                }
                let parents: Vec<_> = parents
                    .split(|b| *b == b' ')
                    .into_iter()
                    .filter(|bs| !bs.is_empty())
                    .map(GitHash::try_from)
                    .collect::<Result<_>>()?;
                let commit = GitCommit {
                    author_time,
                    parents,
                };
                commits.insert(hash, commit);
            } else {
                unreachable!("3 fields from git")
            }
        }

        Ok(Self {
            entry_reference: KString::from_ref(entry_reference),
            entry_githash,
            commits,
        })
    }

    pub fn get(&self, h: &GitHash) -> Option<&GitCommit> {
        self.commits.get(h)
    }
}
