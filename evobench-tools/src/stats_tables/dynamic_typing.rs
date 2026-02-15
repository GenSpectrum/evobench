//! Wrappers to allow to use various types generically at runtime

use std::{borrow::Cow, fmt::Debug};

use super::{
    stats::{Stats, SubStats, ToStatsString},
    tables::table_view::{ColumnFormatting, Highlight, TableViewRow, Unit},
};

#[derive(Debug)]
pub enum StatsOrCount<ViewType: Debug, const TILE_COUNT: usize> {
    Stats(Stats<ViewType, TILE_COUNT>),
    Count(usize),
}

impl<ViewType: From<u64> + ToStatsString + Debug, const TILE_COUNT: usize> TableViewRow<()>
    for StatsOrCount<ViewType, TILE_COUNT>
{
    fn table_view_header(_: ()) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>> {
        Stats::<ViewType, TILE_COUNT>::table_view_header(())
    }
    fn table_view_row(&self, out: &mut Vec<(Cow<str>, Highlight)>) {
        match self {
            StatsOrCount::Stats(stats) => stats.table_view_row(out),
            StatsOrCount::Count(count) => {
                out.push((
                    count.to_string().into(),
                    // XX?
                    Highlight::Neutral,
                ));
            }
        }
    }
}

// XX can one do the same via GATs? For now just do a runtime switch.
pub enum StatsOrCountOrSubStats<ViewType: Debug, const TILE_COUNT: usize> {
    StatsOrCount(StatsOrCount<ViewType, TILE_COUNT>),
    SubStats(SubStats<ViewType, TILE_COUNT>),
}

impl<ViewType: Debug, const TILE_COUNT: usize> From<StatsOrCount<ViewType, TILE_COUNT>>
    for StatsOrCountOrSubStats<ViewType, TILE_COUNT>
{
    fn from(value: StatsOrCount<ViewType, TILE_COUNT>) -> Self {
        Self::StatsOrCount(value)
    }
}

impl<ViewType: Debug, const TILE_COUNT: usize> From<SubStats<ViewType, TILE_COUNT>>
    for StatsOrCountOrSubStats<ViewType, TILE_COUNT>
{
    fn from(value: SubStats<ViewType, TILE_COUNT>) -> Self {
        Self::SubStats(value)
    }
}

impl<ViewType: Debug + From<u64> + ToStatsString, const TILE_COUNT: usize> TableViewRow<()>
    for StatsOrCountOrSubStats<ViewType, TILE_COUNT>
{
    fn table_view_header(_: ()) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>> {
        // XXX oh, which one to choose from? Are they the same?-- now
        // could base this on TableViewRow type parameter
        StatsOrCount::<ViewType, TILE_COUNT>::table_view_header(())
    }

    fn table_view_row(&self, out: &mut Vec<(Cow<str>, Highlight)>) {
        match self {
            StatsOrCountOrSubStats::StatsOrCount(stats_or_count) => {
                stats_or_count.table_view_row(out)
            }
            StatsOrCountOrSubStats::SubStats(sub_stats) => sub_stats.table_view_row(out),
        }
    }
}
