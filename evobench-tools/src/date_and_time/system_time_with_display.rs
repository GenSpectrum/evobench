//! Wrap `SystemTime` so that it has `Display` showing RFC 3339 format
//! with the `evobench::serde::date_and_time::LOCAL_TIME` setting.

use std::{fmt::Display, ops::Deref, time::SystemTime};

use derive_more::From;

use crate::serde_types::date_and_time::system_time_to_rfc3339;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, From)]
pub struct SystemTimeWithDisplay(pub SystemTime);

impl Deref for SystemTimeWithDisplay {
    type Target = SystemTime;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for SystemTimeWithDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&system_time_to_rfc3339(self.0, None))
    }
}
