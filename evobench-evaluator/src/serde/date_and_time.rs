//! Date-and-time representations that can be nicely
//! serialized/deserialized

use std::{fmt::Display, str::FromStr, time::SystemTime};

use chrono::{
    DateTime, FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
    Timelike,
};
use serde::de::Visitor;

/// Stored in RFC 3339 format, with local time zone offset -- CAREFUL,
/// if specified as the wrong string in a file, no check is done on
/// deserialization!
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, serde::Serialize, serde::Deserialize)]
pub struct DateTimeWithOffset(String);

pub fn system_time_to_rfc3339(t: SystemTime) -> String {
    let t: DateTime<Local> = DateTime::from(t);
    t.to_rfc3339()
}

impl DateTimeWithOffset {
    pub fn now() -> Self {
        Self(system_time_to_rfc3339(SystemTime::now()))
    }

    pub fn to_datetime(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(&self.0)
            .expect("field is result of to_rfc3339 hence always parseable")
    }

    pub fn to_systemtime(&self) -> SystemTime {
        let dt = self.to_datetime();
        dt.into()
    }
}

impl Display for DateTimeWithOffset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for DateTimeWithOffset {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = DateTime::parse_from_rfc3339(s)?;
        // XX lol, parse and format back, but it's needed for error
        // checking and uniform storage. It's why it should maintain
        // DateTime internally, and provide custom serde
        // implementations
        Ok(Self(t.to_rfc3339()))
    }
}

/// Without offset, but representing time in the local time zone.
#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord)]
pub struct LocalNaiveTime {
    /// 24-hour based
    pub hour: u8,
    pub minute: u8,
    /// second 60 is allowed to represent leap seconds
    pub second: u8,
}

impl Display for LocalNaiveTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            hour,
            minute,
            second,
        } = self;
        write!(f, "{hour:02}:{minute:02}:{second:02}")
    }
}

impl LocalNaiveTime {
    pub fn hour(&self) -> u32 {
        self.hour.into()
    }
    pub fn minute(&self) -> u32 {
        self.minute.into()
    }
    pub fn second(&self) -> u32 {
        self.second.into()
    }
    pub fn with_date(&self, date: NaiveDate) -> NaiveDateTime {
        NaiveDateTime::new(date, self.into())
    }
    pub fn with_date_as_local(&self, date: NaiveDate) -> LocalResult<DateTime<Local>> {
        Local.from_local_datetime(&self.with_date(date))
    }
    pub fn with_date_as_unambiguous_local(&self, date: NaiveDate) -> Option<DateTime<Local>> {
        match self.with_date_as_local(date) {
            LocalResult::None => None,
            LocalResult::Single(v) => Some(v),
            LocalResult::Ambiguous(_, _) => None,
        }
    }
}

impl From<NaiveTime> for LocalNaiveTime {
    fn from(value: NaiveTime) -> Self {
        let hour = value.hour().try_into().expect("fits");
        let minute = value.minute().try_into().expect("fits");
        let second = value.second().try_into().expect("fits");
        // XX is everything guaranteed in range now ?
        Self {
            hour,
            minute,
            second,
        }
    }
}

impl From<&LocalNaiveTime> for NaiveTime {
    fn from(value: &LocalNaiveTime) -> Self {
        NaiveTime::from_hms_opt(value.hour(), value.minute(), value.second())
            .expect("fitting range")
    }
}

impl serde::Serialize for LocalNaiveTime {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Self {
            hour,
            minute,
            second,
        } = self;
        let s = format!("{hour:02}:{minute:02}:{second:02}");
        serializer.serialize_str(&s)
    }
}

fn parse_max(s: &str, max_incl: u8, field_name: &str) -> Result<u8, String> {
    let val = s
        .parse()
        .map_err(|_| format!("{field_name} must be an integer 0..{}", max_incl))?;
    if val <= max_incl {
        Ok(val)
    } else {
        Err(format!("{field_name} must be an integer 0..{}", max_incl))
    }
}

impl FromStr for LocalNaiveTime {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((h, m_s)) = s.split_once(':') {
            let hour = parse_max(h, 23, "hour")?;
            if let Some((m, s)) = m_s.split_once(':') {
                let minute = parse_max(m, 59, "minute")?;
                // Allow for leap second, OK?
                let second = parse_max(s, 60, "second")?;
                Ok(LocalNaiveTime {
                    hour,
                    minute,
                    second,
                })
            } else {
                let minute = parse_max(m_s, 59, "minute")?;
                let second = 0;
                Ok(LocalNaiveTime {
                    hour,
                    minute,
                    second,
                })
            }
        } else {
            Err("need at least one ':'".into())
        }
    }
}

struct LocalNaiveTimeVisitor;
impl<'de> Visitor<'de> for LocalNaiveTimeVisitor {
    type Value = LocalNaiveTime;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a string hh:mm:ss or h:mm:ss, or with the :ss left out")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        LocalNaiveTime::from_str(v).map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for LocalNaiveTime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(LocalNaiveTimeVisitor)
    }
}
