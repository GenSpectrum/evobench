//! Wrap `SystemTime` so that it has `Display` showing local time in
//! RFC 3339 format.

use std::{fmt::Display, ops::Deref, time::SystemTime};

use crate::serde::date_and_time::system_time_to_rfc3339;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SystemTimeWithDisplay(pub SystemTime);

impl Deref for SystemTimeWithDisplay {
    type Target = SystemTime;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for SystemTimeWithDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&system_time_to_rfc3339(self.0, true))
    }
}

impl From<SystemTime> for SystemTimeWithDisplay {
    fn from(value: SystemTime) -> Self {
        Self(value)
    }
}

impl From<&SystemTime> for SystemTimeWithDisplay {
    fn from(value: &SystemTime) -> Self {
        Self(*value)
    }
}
