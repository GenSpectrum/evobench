use std::marker::PhantomData;
use std::{borrow::Cow, str::FromStr};

use anyhow::bail;
use num_traits::{Pow, Zero};

use crate::{
    average::Average,
    table_view::{ColumnFormatting, Highlight, TableViewRow, Unit},
    times::{MicroTime, NanoTime, ToStringMilliseconds},
};

/// Selects a field of `Stats`, e.g. to calculate the stats for one of
/// the Stats fields.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum StatsField<const TILE_COUNT: usize> {
    N,
    Sum,
    Average,
    Median,
    SD,
    Tile(u32),
}

impl<const TILE_COUNT: usize> FromStr for StatsField<TILE_COUNT> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use StatsField::*;
        match s {
            "n" | "N" => Ok(N),
            "sum" | "Sum" => Ok(Sum),
            "average" | "Average" | "avg" | "mean" => Ok(Average),
            "median" | "Median" => Ok(Median),
            "sd" | "SD" | "stdev" => Ok(SD),
            _ => match f64::from_str(s) {
                Ok(x) => {
                    if x >= 0. && x <= 1.00000000001 {
                        let i = (TILE_COUNT as f64 * x + 0.5).floor() as u32;
                        Ok(Tile(i))
                    } else {
                        bail!(
                            "expecting one of n|sum|average|median|sd or a floating \
                             point number between 0 and 1, got: {x}"
                        )
                    }
                }
                Err(e) => bail!(
                    "expecting one of n|sum|average|median|sd or a floating \
                     point number between 0 and 1, floating point parse error for {s:?}: {e}"
                ),
            },
        }
    }
}

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

/// `ViewType` is perhaps a bit of a misnomer: simply the type that
/// the statistics is made for, e.g. `NanoTime`. But that type must,
/// for full functionality, support conversion to and from u64, and
/// `ToStatsString` for viewing.
#[derive(Debug)]
pub struct Stats<ViewType, const TILE_COUNT: usize> {
    view_type: PhantomData<fn() -> ViewType>,
    pub num_values: usize,
    pub sum: u128,
    /// x.5 is rounded up
    pub average: u64,
    /// Interpolated and rounded up for even numbers of input values.
    pub median: u64,
    /// mean squared difference from the mean
    pub variance: f64,
    /// Percentiles or in `TILE_COUNT` number of sections. Sample
    /// count is the index, the sample value there is the value in the
    /// vector. `tiles[0]` is the mininum, `tiles[TILE_COUNT]` the
    /// maximum sample value. Cut-offs are inclusive, i.e. the index
    /// is rounded up (shows the value of the next item if it falls at
    /// least the distance 0.5 between items).
    pub tiles: Vec<u64>,
}

#[derive(thiserror::Error, Debug)]
pub enum StatsError {
    #[error("no inputs given")]
    NoInputs,
    #[error("u128 saturated -- u64::MAX values summing up to near u128::MAX")]
    SaturatedU128,
}

impl<ViewType, const TILE_COUNT: usize> Stats<ViewType, TILE_COUNT> {
    /// sqrt(variance) as u64, since our conversions to ms etc. are on
    /// that type; bummer to lose f64 precision, though. Rounded.
    pub fn standard_deviation_u64(&self) -> u64 {
        // What about number overflows from f64 ? Can't happen,
        // though, right?
        (self.variance.sqrt() + 0.5) as u64
    }

    /// Get the value for the given field (and ending up untyped! But
    /// from_values was always untyped, too.)
    pub fn get(&self, field: StatsField<TILE_COUNT>) -> u64 {
        match field {
            StatsField::N => self
                .num_values
                .try_into()
                .expect("hopefully in range -- realistically it should be"),
            StatsField::Sum => self
                .sum
                .try_into()
                .expect("hopefully in range -- realistically it should be"),
            StatsField::Average => self.average,
            StatsField::Median => self.median,
            StatsField::SD => self.standard_deviation_u64(),
            StatsField::Tile(i) => {
                self.tiles[usize::try_from(i).expect("u32 always works as index")]
            }
        }
    }

    // XXX add tests: compare to table_view_header's .0 field
    pub fn column_of_field(field: StatsField<TILE_COUNT>) -> usize {
        match field {
            StatsField::N => 0,
            StatsField::Sum => 1,
            StatsField::Average => 2,
            StatsField::Median => 3,
            StatsField::SD => 4,
            StatsField::Tile(i) => 6 + usize::try_from(i).expect("u32 always works as index"),
        }
    }

    // If it's not count (u64), then it's ViewType. XXX add tests:
    // compare to table_view_header's Unit field
    pub fn field_type_is_count(field: StatsField<TILE_COUNT>) -> bool {
        match field {
            StatsField::N => true,
            StatsField::Sum => false,
            StatsField::Average => false,
            StatsField::Median => false,
            StatsField::SD => false,
            StatsField::Tile(_) => false,
        }
    }

    /// Make stats from values from field `field`: this determines the
    /// ViewType of the resulting `Stats` struct: count or ViewType.
    pub fn from_values_from_field(
        field: StatsField<TILE_COUNT>,
        vals: Vec<u64>,
    ) -> Result<SubStats<ViewType, TILE_COUNT>, StatsError> {
        if Self::field_type_is_count(field) {
            Ok(SubStats::Count(Stats::from_values(vals)?))
        } else {
            Ok(SubStats::ViewType(Stats::from_values(vals)?))
        }
    }

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

        let variance = {
            let num_values = num_values as f64;
            let average: f64 = sum as f64 / num_values;
            let sum_squared_error: f64 = vals.iter().map(|v| (*v as f64 - average).pow(2)).sum();
            sum_squared_error / num_values
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

        // As an example, a TILE_COUNT of 3 means, we group into
        // [min, med, max] (we get percentiles via a TILE_COUNT of
        // 101, not 100, if we want a median bucket!). We need to go 2
        // distances. If `vals` has 4 elements, that's 3 distances to
        // go. The tile position 2 needs to go to vals index 3, the
        // tile position 1 to vals index 1.5 -> round up to index 2.
        let vals_distances = (num_values - 1) as f64;
        let tiles_distances = (TILE_COUNT - 1) as f64;
        let mut tiles = Vec::new();
        for i in 0..TILE_COUNT {
            let index = i as f64 / tiles_distances * vals_distances + 0.5;
            let val = vals[index as usize];
            tiles.push(val);
        }

        assert_eq!(vals.first(), tiles.first());
        assert_eq!(vals.last(), tiles.last());

        Ok(Stats {
            view_type: PhantomData::default(),
            num_values,
            sum,
            average: average.try_into().expect("always fits"),
            median,
            variance,
            tiles,
        })
    }
}

/// A way to give full access to `Stats` structs with the possible
/// parameterizations, unlike the `dyn TableViewRow` approach which
/// only gives access to that interface. Also avoid the need for
/// boxing, perhaps performance relevant. (But then, somewhat
/// funnily?, we need to implement `TableViewRow` for this, too.)
pub enum SubStats<ViewType, const TILE_COUNT: usize> {
    Count(Stats<u64, TILE_COUNT>),
    ViewType(Stats<ViewType, TILE_COUNT>),
}

#[derive(Debug, Clone, Copy)]
pub enum SubStatsKind {
    Count,
    ViewType,
}

impl<ViewType: From<u64> + ToStatsString, const TILE_COUNT: usize> TableViewRow<SubStatsKind>
    for SubStats<ViewType, TILE_COUNT>
{
    fn table_view_header(
        ctx: SubStatsKind,
    ) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>> {
        eprintln!("XX weirdly this is never used??");
        match ctx {
            SubStatsKind::Count => Stats::<u64, TILE_COUNT>::table_view_header(()),
            SubStatsKind::ViewType => Stats::<ViewType, TILE_COUNT>::table_view_header(()),
        }
    }

    fn table_view_row(&self, out: &mut Vec<(Cow<str>, Highlight)>) {
        match self {
            SubStats::Count(stats) => stats.table_view_row(out),
            SubStats::ViewType(stats) => stats.table_view_row(out),
        }
    }
}

impl<ViewType: From<u64> + ToStatsString, const TILE_COUNT: usize> TableViewRow<()>
    for Stats<ViewType, TILE_COUNT>
{
    fn table_view_header(_: ()) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>> {
        let mut cols = vec![
            ("n".into(), Unit::Count, ColumnFormatting::Number),
            (
                "sum".into(),
                Unit::ViewType(ViewType::UNIT_SHORT),
                ColumnFormatting::Number,
            ),
            (
                "avg".into(),
                Unit::ViewType(ViewType::UNIT_SHORT),
                ColumnFormatting::Number,
            ),
            (
                "median".into(),
                Unit::ViewType(ViewType::UNIT_SHORT),
                ColumnFormatting::Number,
            ),
            (
                "SD".into(),
                Unit::ViewType(ViewType::UNIT_SHORT),
                ColumnFormatting::Number,
            ),
            ("".into(), Unit::None, ColumnFormatting::Spacer),
        ];

        for i in 0..TILE_COUNT {
            cols.push((
                format!("{:.2}", (i as f64) / ((TILE_COUNT - 1) as f64)).into(),
                Unit::ViewType(ViewType::UNIT_SHORT),
                ColumnFormatting::Number,
            ));
        }
        Box::new(cols)
    }

    fn table_view_row(&self, out: &mut Vec<(Cow<str>, Highlight)>) {
        let Self {
            view_type: _,
            num_values,
            sum,
            average,
            // using standard_deviation_u64() instead
            variance: _,
            median,
            tiles,
        } = self;

        out.push((num_values.to_string().into(), Highlight::Neutral));
        out.push((
            ViewType::from(u64::try_from(*sum).expect("sum must fit in u64 range"))
                .to_stats_string()
                .into(),
            Highlight::Neutral,
        ));
        out.push((
            ViewType::from(*average).to_stats_string().into(),
            Highlight::Neutral,
        ));
        out.push((
            ViewType::from(*median).to_stats_string().into(),
            Highlight::Neutral,
        ));
        out.push((
            // bummer, float precision is lost here, but doesn't
            // matter in our ns or us units
            ViewType::from(self.standard_deviation_u64())
                .to_stats_string()
                .into(),
            Highlight::Neutral,
        ));

        out.push(("".into(), Highlight::Spacer));

        for val in tiles {
            out.push((
                ViewType::from(*val).to_stats_string().into(),
                Highlight::Neutral,
            ));
        }
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
        assert_eq!(stats.variance, 104.24000000000001);
        assert_eq!(stats.standard_deviation_u64(), 10); // 10.2097992144802

        let data = vec![23, 4, 8, 31, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.average, 15); // 14.6
        assert_eq!(stats.tiles[0], 4);
        assert_eq!(stats.tiles.len(), 4);
        assert_eq!(stats.tiles[3], 31);
        assert_eq!(stats.tiles, [4, 7, 23, 31]); // 8 skipped
        assert_eq!(stats.median, 8);
        assert_eq!(stats.variance, 110.64000000000001);
        assert_eq!(stats.standard_deviation_u64(), 11); // 10.5185550338438

        let data = vec![23, 4, 8, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.average, 11); // 10.5
        assert_eq!(stats.tiles, [4, 7, 8, 23]);
        assert_eq!(stats.median, 8); // 7.5 rounded up ? XX

        let data = vec![23, 8, 7];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.average, 13); // 12.6666666666667
        assert_eq!(stats.median, 8);
        assert_eq!(stats.tiles, [7, 8, 8, 23]);

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
        assert_eq!(stats.tiles, [4, 23]);
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

        let data = vec![23, 4];
        let stats = Stats::<u64, 4>::from_values(data)?;
        assert_eq!(stats.median, 14); // 13.5

        Ok(())
    }
}
