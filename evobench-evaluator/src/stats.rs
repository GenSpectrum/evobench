use std::fmt::Display;
use std::io::Write;
use std::marker::PhantomData;

use num_traits::Zero;

use crate::{
    average::Average,
    times::{MicroTime, NanoTime, ToStringMilliseconds},
};

/// Convert a value to a string in a unit and corresponding
/// representation suitable for the stats here.
pub trait ToStatsString {
    const UNIT_SHORT: &str;

    fn to_stats_string(&self) -> String;
}

impl ToStatsString for MicroTime {
    const UNIT_SHORT: &str = "ms";

    fn to_stats_string(&self) -> String {
        self.to_string_ms()
    }
}

impl ToStatsString for NanoTime {
    const UNIT_SHORT: &str = "ms";

    fn to_stats_string(&self) -> String {
        self.to_string_ms()
    }
}

// XX define a Count ? Count<const &str>?
impl ToStatsString for u64 {
    const UNIT_SHORT: &str = "count";

    fn to_stats_string(&self) -> String {
        self.to_string()
    }
}

#[derive(Debug)]
pub struct Stats<ViewType, const TILES_COUNT: usize> {
    view_type: PhantomData<fn() -> ViewType>,
    pub num_values: usize,
    pub sum: u128,
    pub average: u64, // x.5 is rounded up
    pub median: u64,
    /// Percentiles or in `TILES_COUNT` number of sections. Sample
    /// count is the index, the sample value there is the value in the
    /// vector. `tiles[0]` is the mininum, `tiles[TILES_COUNT]` the
    /// maximum sample value.
    pub tiles: Vec<u64>,
}

#[derive(thiserror::Error, Debug)]
pub enum StatsError {
    #[error("no inputs given")]
    NoInputs,
    #[error("u128 saturated -- u64::MAX values summing up to near u128::MAX")]
    SaturatedU128,
}

impl<ViewType, const TILES_COUNT: usize> Stats<ViewType, TILES_COUNT> {
    /// `tiles_count` is how many 'tiles' to build, for percentiles
    /// give the number 101. (Needs to own `vals` for sorting,
    /// internally.)
    pub fn from_values(mut vals: Vec<u64>) -> Result<Self, StatsError> {
        let num_values = vals.len();
        if num_values.is_zero() {
            return Err(StatsError::NoInputs);
        }
        let sum: u128 = vals.iter().map(|v| u128::from(*v)).sum();
        let average = {
            let num_values = num_values as u128;
            sum.checked_add(num_values / 2)
                .ok_or(StatsError::SaturatedU128)?
                / num_values
        };
        vals.sort();

        // Calculate the median before making tiles, because for
        // uneven lengths, tiles do not precisely contain the median.
        let median = {
            let mid = vals.len() / 2;
            if 0 == vals.len() % 2 {
                // len is checked to be > 0, so we
                // must have at least 2 values here.
                (vals[mid - 1], vals[mid]).average()
            } else {
                vals[mid]
            }
        };

        let flen = num_values as f64;
        let mut tiles = Vec::new();
        let tiles_max = TILES_COUNT as f64;
        for i in 0..TILES_COUNT {
            let index = i as f64 / tiles_max * flen + 0.5;
            let val = vals[index as usize];
            tiles.push(val);
        }

        Ok(Stats {
            view_type: PhantomData::default(),
            num_values,
            sum,
            average: average.try_into().expect("always fits"),
            median,
            tiles,
        })
    }

    pub fn print_tsv_header(mut out: impl Write, key_names: &[&str]) -> Result<(), std::io::Error>
    where
        ViewType: ToStatsString,
    {
        for key_name in key_names {
            write!(out, "{key_name}\t")?;
        }
        let unit = ViewType::UNIT_SHORT;
        write!(out, "n\tsum {unit}\tavg {unit}\tmedian {unit}")?;

        // Add empty column before tiles:
        write!(out, "\t")?;

        for i in 0..TILES_COUNT {
            write!(out, "\ttile {i} ({unit})")?
        }
        writeln!(out, "")?;
        Ok(())
    }

    pub fn print_tsv_line(&self, mut out: impl Write, keys: &[&str]) -> Result<(), std::io::Error>
    where
        ViewType: ToStatsString + From<u64>,
    {
        let Self {
            view_type: _,
            num_values,
            sum,
            average,
            median,
            tiles,
        } = self;
        for key in keys {
            write!(out, "{key}\t")?;
        }
        write!(
            out,
            "{num_values}\t{}\t{}\t{}",
            ViewType::from(u64::try_from(*sum).expect("sum is larger than u64: {sum}"))
                .to_stats_string(),
            ViewType::from(*average).to_stats_string(),
            ViewType::from(*median).to_stats_string()
        )?;

        // Add empty column before tiles:
        write!(out, "\t")?;

        // *tiles
        for val in tiles {
            write!(out, "\t{}", ViewType::from(*val).to_stats_string())?;
        }
        writeln!(out, "")?;
        Ok(())
    }
}

impl<ViewType: From<u64> + Display, const TILES_COUNT: usize> Display
    for Stats<ViewType, TILES_COUNT>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            view_type: _,
            num_values,
            sum,
            average,
            median,
            tiles: _,
        } = self;
        write!(
            f,
            " {num_values} values \t sum {} \t average {} \t median {}",
            ViewType::from(u64::try_from(*sum).expect("sum is larger than u64: {sum}")),
            ViewType::from(*average),
            ViewType::from(*median)
        )
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn t_average_and_tiles_and_median() -> Result<()> {
        let data = vec![23, 4, 8, 30, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.average, 14); // 14.4
        assert_eq!(stats.tiles, [4, 7, 23, 30]); // 8 skipped
        assert_eq!(stats.median, 8);

        let data = vec![23, 4, 8, 31, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.average, 15); // 14.6
        assert_eq!(stats.tiles[0], 4);
        assert_eq!(stats.tiles.len(), 4);
        assert_eq!(stats.tiles[3], 31);
        assert_eq!(stats.tiles, [4, 7, 23, 31]); // 8 skipped
        assert_eq!(stats.median, 8);

        let data = vec![23, 4, 8, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.average, 11); // 10.5
        assert_eq!(stats.tiles, [4, 7, 8, 23]);
        assert_eq!(stats.median, 8); // 7.5 rounded up ? XX

        let data = vec![23, 8, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.average, 13); // 12.6666666666667
        assert_eq!(stats.median, 8);
        assert_eq!(stats.tiles, [7, 8, 23, 23]);
        // interesting that it duplicates 23 not 8; `let index = i as
        // f64 / tiles_max * flen + 0.499;` above leads to 8 being
        // duplicated, but then skips 23 in the test with 5 values
        // above.

        Ok(())
    }

    #[test]
    fn t_median() -> Result<()> {
        let data = vec![23, 4, 8, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.median, 8); // 7.5 rounded up

        let data = vec![23, 4, 9, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.median, 8); // 8.0

        let data = vec![23, 4, 10, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.median, 9); // 8.5

        let data = vec![23, 4, 10, 7];
        let stats = Stats::<u64, 2>::from_values(data)?;
        assert_eq!(stats.tiles, [4, 10]);
        // Calculated from original values, not tiles:
        assert_eq!(stats.median, 9); // 8.5

        let data = vec![23, 4, 7];
        let stats = Stats::<u64, 2>::from_values(data)?;
        assert_eq!(stats.median, 7);

        let data = vec![23, 4, 7];
        let stats = Stats::<u64, 3>::from_values(data)?;
        assert_eq!(stats.median, 7);

        let data = vec![23, 4, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.median, 7);

        Ok(())
    }
}
