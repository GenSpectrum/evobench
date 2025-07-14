//! Using serde::date_and_time, but putting this code into another
//! module to keep the scope of the serde::* namespace narrow.

use std::{fmt::Display, str::FromStr};

use anyhow::{anyhow, bail};
use chrono::{DateTime, Days, Local, NaiveDate, TimeZone};

use crate::{debug, serde::date_and_time::LocalNaiveTime};

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
    /// time switches, or perhaps leap seconds?). *Note* that this
    /// literally just adds the given date as the date of the start
    /// time, then if necessary increments the date if the end time <
    /// start time (range crosses a day boundary). Taking the date
    /// from the current time and then passing it to this method means
    /// that the current time can be past the range, and it also means
    /// that even though the current time might be within the range,
    /// the result is in the future (i.e. if the start time of the
    /// range is before a day boundary, this method still resolves it
    /// to the given date, resulting in a time that is in the
    /// future). You probably want to use `after_datetime()` instead.
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

    /// Returns None if there is ambiguity (due to daylight savings
    /// time switches, or perhaps leap seconds?) (or if given a
    /// `datetime` for which no day can be added (max date?)). If
    /// `allow_time_inside_range` is true, picks the start date so
    /// that the resulting range contains `ndt` if possible, otherwise
    /// the resulting range starts >= `ndt`.  XX new desc: the time
    /// period using the `self` times, closest to `datetime`, around
    /// it if allowed or closest after it. Does not need to carry the
    /// same date!
    pub fn after_datetime(
        &self,
        datetime: &DateTime<Local>,
        allow_time_inside_range: bool,
    ) -> Option<DateTimeRange<Local>> {
        debug!("after_datetime({self}, {datetime}, {allow_time_inside_range}):");

        let Self { from, to } = self;

        let dtr_from_nd = |nd: NaiveDate| -> Option<_> {
            Some(DateTimeRange {
                from: from.with_date_as_unambiguous_local(nd)?,
                to: if self.crosses_day_boundary() {
                    to.with_date_as_unambiguous_local(nd.checked_add_days(Days::new(1))?)?
                } else {
                    to.with_date_as_unambiguous_local(nd)?
                },
            })
        };

        // Try with on the same day
        let nd = datetime.date_naive();
        let dtr_today = dtr_from_nd(nd)?;
        let contains = dtr_today.contains(datetime);
        debug!("    dtr_today={dtr_today}, dtr_today.contains(datetime) = {contains}");
        if contains {
            if allow_time_inside_range {
                debug!("        allowed inside -> dtr_today = {dtr_today}");
                Some(dtr_today)
            } else {
                let correct = dtr_from_nd(nd.checked_add_days(Days::new(1))?)?;
                debug!("        !allowed inside -> next day = {correct}");
                Some(correct)
            }
        } else {
            let range_is_in_past = dtr_today.to <= *datetime;
            debug!("        dtr_today.to <= datetime == {range_is_in_past}");
            if range_is_in_past {
                // Take the next day
                let next_day = dtr_from_nd(nd.checked_add_days(Days::new(1))?)?;
                debug!("            next_day = {next_day}");
                if !allow_time_inside_range && next_day.contains(datetime) {
                    todo!()
                } else {
                    Some(next_day)
                }
            } else {
                assert!(*datetime < dtr_today.from);
                // `dtr_today` is in the future. But check the
                // day before that, it might be closer.
                let prev_day = dtr_from_nd(nd.checked_sub_days(Days::new(1))?)?;
                debug!("            prev_day = {prev_day}");
                if prev_day.contains(datetime) {
                    if allow_time_inside_range {
                        Some(prev_day)
                    } else {
                        Some(dtr_today)
                    }
                } else {
                    if *datetime < prev_day.from {
                        Some(prev_day)
                    } else {
                        Some(dtr_today)
                    }
                }
            }
        }
    }
}

impl Display for LocalNaiveTimeRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { from, to } = self;
        write!(f, "{from} - {to}")
    }
}

#[derive(Debug, Clone)]
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

    fn naivedate_to_locals(s: &str, y: i32, m: u32, d: u32) -> Result<DateTimeRange<Local>> {
        let ltr = LocalNaiveTimeRange::from_str(s)?;
        ltr.with_start_date_as_unambiguous_locals(NaiveDate::from_ymd_opt(y, m, d).unwrap())
            .ok_or_else(|| anyhow!("ambiguous: {ltr}"))
    }

    #[test]
    fn t_with_start_date_as_unambiguous_locals() -> Result<()> {
        assert_eq!(
            naivedate_to_locals("2:00-6:00", 2024, 10, 23)?.to_string(),
            "2024-10-23T02:00:00+02:00 - 2024-10-23T06:00:00+02:00"
        );
        assert_eq!(
            naivedate_to_locals("23:00-6:00", 2024, 10, 23)?.to_string(),
            "2024-10-23T23:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        Ok(())
    }

    fn datetime_to_locals(
        s: &str,
        datetime_str: &str,
        allow_time_inside_range: bool,
    ) -> Result<DateTimeRange<Local>> {
        let datetime = DateTime::<Local>::from_str(datetime_str).expect("valid input");

        let ltr = LocalNaiveTimeRange::from_str(s)?;
        ltr.after_datetime(&datetime, allow_time_inside_range)
            .ok_or_else(|| anyhow!("ambiguous: {ltr}"))
    }

    #[test]
    fn t_after_datetime_within_first_day() -> Result<()> {
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-23T23:30:00+02:00", true)?.to_string(),
            "2024-10-23T23:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-23T23:30:00+02:00", false)?.to_string(),
            "2024-10-24T23:00:00+02:00 - 2024-10-25T06:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_within_first_day_on_start() -> Result<()> {
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-23T23:00:00+02:00", true)?.to_string(),
            "2024-10-23T23:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-23T23:00:00+02:00", false)?.to_string(),
            "2024-10-24T23:00:00+02:00 - 2024-10-25T06:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_within_first_day_before_end() -> Result<()> {
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-24T05:59:59+02:00", true)?.to_string(),
            "2024-10-23T23:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-24T05:59:59+02:00", false)?.to_string(),
            "2024-10-24T23:00:00+02:00 - 2024-10-25T06:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_within_first_day_on_end() -> Result<()> {
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-24T06:00:00+02:00", true)?.to_string(),
            "2024-10-24T23:00:00+02:00 - 2024-10-25T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-24T06:00:00+02:00", false)?.to_string(),
            "2024-10-24T23:00:00+02:00 - 2024-10-25T06:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_within_second_day() -> Result<()> {
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-24T02:00:00+02:00", true)?.to_string(),
            "2024-10-23T23:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("23:00-6:00", "2024-10-24T02:00:00+02:00", false)?.to_string(),
            "2024-10-24T23:00:00+02:00 - 2024-10-25T06:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_on_start_boundary() -> Result<()> {
        assert_eq!(
            datetime_to_locals("2:00-6:00", "2024-10-23T02:00:00+02:00", true)?.to_string(),
            "2024-10-23T02:00:00+02:00 - 2024-10-23T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("2:00-6:00", "2024-10-23T02:00:00+02:00", false)?.to_string(),
            "2024-10-24T02:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_starting_on_day_boundary() -> Result<()> {
        assert_eq!(
            datetime_to_locals("0:00-6:00", "2024-10-23T00:00:00+02:00", true)?.to_string(),
            "2024-10-23T00:00:00+02:00 - 2024-10-23T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("0:00-6:00", "2024-10-23T00:00:00+02:00", false)?.to_string(),
            "2024-10-24T00:00:00+02:00 - 2024-10-24T06:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_ending_on_day_boundary() -> Result<()> {
        assert_eq!(
            datetime_to_locals("6:00-0:00", "2024-10-23T00:00:00+02:00", true)?.to_string(),
            "2024-10-23T06:00:00+02:00 - 2024-10-24T00:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("6:00-0:00", "2024-10-23T00:00:00+02:00", false)?.to_string(),
            "2024-10-23T06:00:00+02:00 - 2024-10-24T00:00:00+02:00"
        );
        Ok(())
    }

    #[test]
    fn t_after_datetime_last_on_day_boundary() -> Result<()> {
        assert_eq!(
            datetime_to_locals("6:00-0:00", "2024-10-23T23:59:59+02:00", true)?.to_string(),
            "2024-10-23T06:00:00+02:00 - 2024-10-24T00:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("6:00-0:00", "2024-10-23T23:59:59+02:00", false)?.to_string(),
            "2024-10-24T06:00:00+02:00 - 2024-10-25T00:00:00+02:00"
        );
        Ok(())
    }

    // whole day? ah no, empty
    #[test]
    fn t_after_datetime_empty() -> Result<()> {
        assert_eq!(
            datetime_to_locals("6:00-6:00", "2024-10-23T00:00:00+02:00", true)?.to_string(),
            "2024-10-23T06:00:00+02:00 - 2024-10-23T06:00:00+02:00"
        );
        assert_eq!(
            datetime_to_locals("6:00-6:00", "2024-10-23T00:00:00+02:00", false)?.to_string(),
            "2024-10-23T06:00:00+02:00 - 2024-10-23T06:00:00+02:00"
        );
        Ok(())
    }
}
