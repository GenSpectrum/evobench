//! Using serde::date_and_time, but putting this code into another
//! module to keep the scope of the serde::* namespace narrow.

use std::{fmt::Display, str::FromStr};

use anyhow::{anyhow, bail};
use chrono::{DateTime, Days, Local, NaiveDate, TimeZone};

use crate::serde::date_and_time::LocalNaiveTime;

pub struct LocalNaiveTimeRange {
    pub from: LocalNaiveTime,
    pub to: LocalNaiveTime,
}

impl FromStr for LocalNaiveTimeRange {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('-').collect();
        match parts.as_slice() {
            &[from, to] => {
                let from = from.trim();
                let to = to.trim();
                let from = from
                    .parse()
                    .map_err(|e| anyhow!("from time {from:?}: {e}"))?;
                let to = to.parse().map_err(|e| anyhow!("to time {to:?}: {e}"))?;
                Ok(LocalNaiveTimeRange { from, to })
            }
            &[_] => {
                bail!("expecting exactly one '-', none given")
            }
            _ => {
                bail!("expecting exactly one '-', more than one given")
            }
        }
    }
}

impl From<(LocalNaiveTime, LocalNaiveTime)> for LocalNaiveTimeRange {
    fn from((from, to): (LocalNaiveTime, LocalNaiveTime)) -> Self {
        Self { from, to }
    }
}

impl From<&(LocalNaiveTime, LocalNaiveTime)> for LocalNaiveTimeRange {
    fn from((from, to): &(LocalNaiveTime, LocalNaiveTime)) -> Self {
        Self {
            from: *from,
            to: *to,
        }
    }
}

impl From<(&LocalNaiveTime, &LocalNaiveTime)> for LocalNaiveTimeRange {
    fn from((from, to): (&LocalNaiveTime, &LocalNaiveTime)) -> Self {
        Self {
            from: *from,
            to: *to,
        }
    }
}

impl LocalNaiveTimeRange {
    pub fn crosses_day_boundary(&self) -> bool {
        let Self { from, to } = self;
        to < from
    }

    /// Returns None if there is ambiguity (due to daylight savings
    /// time switches, or perhaps leap seconds?).
    pub fn with_start_date_as_unambiguous_locals(
        &self,
        nd: NaiveDate,
    ) -> Option<DateTimeRange<Local>> {
        let Self { from, to } = self;
        let from = from.with_date_as_unambiguous_local(nd)?;
        let nd_end = if self.crosses_day_boundary() {
            nd.checked_add_days(Days::new(1))?
        } else {
            nd
        };
        let to = to.with_date_as_unambiguous_local(nd_end)?;
        Some(DateTimeRange { from, to })
    }
}

impl Display for LocalNaiveTimeRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { from, to } = self;
        write!(f, "{from} - {to}")
    }
}

pub struct DateTimeRange<Tz: TimeZone> {
    pub from: DateTime<Tz>,
    pub to: DateTime<Tz>,
}

impl<Tz: TimeZone> Display for DateTimeRange<Tz> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { from, to } = self;
        write!(f, "{} - {}", from.to_rfc3339(), to.to_rfc3339())
    }
}

impl<Tz: TimeZone> DateTimeRange<Tz> {
    pub fn contains(&self, time: &DateTime<Tz>) -> bool {
        let Self { from, to } = self;
        from <= time && time < to
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn naive_to_locals(s: &str) -> Result<DateTimeRange<Local>> {
        let ltr = LocalNaiveTimeRange::from_str(s)?;
        ltr.with_start_date_as_unambiguous_locals(NaiveDate::from_ymd_opt(2024, 10, 23).unwrap())
            .ok_or_else(|| anyhow!("ambiguous: {ltr}"))
    }

    #[test]
    fn t_() -> Result<()> {
        assert_eq!(
            naive_to_locals("2:00-6:00")?.to_string(),
            "2024-10-23T02:00:00+02:00 - 2024-10-23T06:00:00+02:00"
        );
        assert_eq!(
            naive_to_locals("23:00-6:00")?.to_string(),
            "2024-10-23T23:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        Ok(())
    }
}
