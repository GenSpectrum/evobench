use std::fmt::Display;
use std::io::Write;
use std::marker::PhantomData;

use num_traits::Zero;

use crate::times::{MicroTime, NanoTime, ToStringMilliseconds};

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
    pub average: u64, // rounded down
    /// Percentiles or in whatever number of sections you asked:
    /// sample count is the index, the sample value there is the value
    /// in the vector.
    pub tiles: Vec<u64>,
}

#[derive(thiserror::Error, Debug)]
pub enum StatsError {
    #[error("no inputs given")]
    NoInputs,
}

impl<ViewType, const TILES_COUNT: usize> Stats<ViewType, TILES_COUNT> {
    /// `tiles_count` is how many 'tiles' to build, for percentiles
    /// give the number 101.
    pub fn from_values(mut vals: Vec<u64>) -> Result<Self, StatsError> {
        let num_values = vals.len();
        if num_values.is_zero() {
            return Err(StatsError::NoInputs);
        }
        let sum: u128 = vals.iter().map(|v| u128::from(*v)).sum();
        let average = sum / (num_values as u128);
        vals.sort();

        let flen = (num_values - 1) as f64;
        let mut tiles = Vec::new();
        let tiles_max = TILES_COUNT as f64;
        for i in 0..TILES_COUNT {
            let index = i as f64 / tiles_max * flen;
            let val = vals[index as usize];
            tiles.push(val);
        }

        // dbg!(vals.first());
        // dbg!(vals.last());

        Ok(Stats {
            view_type: PhantomData::default(),
            num_values,
            sum,
            average: average.try_into().expect("always fit"),
            tiles,
        })
    }

    /// Uses the values from `tiles`; panics if you gave an even
    /// tiles_count (must be odd so the middle is present)
    pub fn median(&self) -> u64 {
        assert!(0 != self.tiles.len() % 2);
        self.tiles[self.tiles.len() / 2]
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
            ViewType::from(self.median()).to_stats_string()
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
            tiles: _,
        } = self;
        write!(
            f,
            " {num_values} values \t sum {} \t average {} \t median {}",
            ViewType::from(u64::try_from(*sum).expect("sum is larger than u64: {sum}")),
            ViewType::from(*average),
            ViewType::from(self.median())
        )
    }
}
