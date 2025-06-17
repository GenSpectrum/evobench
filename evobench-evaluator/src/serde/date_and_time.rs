//! Date-and-time representations that can be nicely
//! serialized/deserialized (huh?)

use std::time::SystemTime;

use chrono::{DateTime, Local};

/// Stored in RFC 3339 format, with local time zone offset
#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct DateTimeWithOffset(String);

impl DateTimeWithOffset {
    pub fn now() -> Self {
        let now = SystemTime::now();
        let now: DateTime<Local> = DateTime::from(now);
        Self(now.to_rfc3339())
    }
}
