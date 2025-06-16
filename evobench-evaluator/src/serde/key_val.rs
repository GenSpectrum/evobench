//! A key/val pair that supports serde and command line parsing for clap.

use std::str::FromStr;

use anyhow::anyhow;

// #[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize, clap::Parser)]
// pub struct KeyVal<V: FromStr>
// where
//     V::Err: Into<Box<dyn Error + Send + Sync + 'static>>,
// {
//     pub key: String,
//     pub val: V,
// }

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize, clap::Parser)]
pub struct KeyVal {
    pub key: String,
    pub val: String,
}

impl FromStr for KeyVal {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (key, val) = s
            .split_once('=')
            .ok_or_else(|| anyhow!("missing '=' in key-value pair {s:?}"))?;

        Ok(KeyVal {
            key: key.into(),
            val: val.into(),
        })
    }
}
