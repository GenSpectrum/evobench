use std::{borrow::Cow, fmt::Debug};

use genawaiter::rc::Gen;
use itertools::{EitherOrBoth, Itertools};

use crate::{
    dynamic_typing::{StatsOrCount, StatsOrCountOrSubStats},
    evaluator::options::TILE_COUNT,
    join::KeyVal,
    resolution_unit::ResolutionUnit,
    stats::{Stats, StatsField, SubStats, ToStatsString},
    tables::{
        change::{Change, IsBetter},
        table_field_view::TableFieldView,
        table_view::{ColumnFormatting, Highlight, TableView, TableViewRow, Unit},
    },
};

pub trait TableKind: Clone {
    fn table_name(&self) -> Cow<'_, str>;
    /// The column title for the *key* field in the rows
    fn table_key_label(&self) -> Cow<'_, str>;
    /// Width of key column in number of characters (as per Excel),
    /// None == automatic.
    fn table_key_column_width(&self) -> Option<f64>;
}

pub struct Table<'key, K: TableKind, T> {
    pub kind: K,
    pub rows: Vec<KeyVal<Cow<'key, str>, T>>,
}

impl<'key, K: TableKind, T: TableViewRow<()>> TableView for Table<'key, K, T> {
    fn table_view_header(&self) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>> {
        let mut header = vec![(
            self.kind.table_key_label().to_string().into(),
            Unit::None,
            ColumnFormatting::String {
                width_chars: self.kind.table_key_column_width(),
            },
        )];
        let row_header = T::table_view_header(());
        for label in (*row_header).as_ref() {
            header.push((*label).clone());
        }
        Box::new(header)
    }

    fn table_name(&self) -> Cow<'_, str> {
        self.kind.table_name()
    }

    fn table_view_body<'s>(
        &'s self,
    ) -> Box<dyn Iterator<Item = Cow<'s, [(Cow<'s, str>, Highlight)]>> + 's> {
        Box::new(
            Gen::new(|co| async move {
                for KeyVal { key, val } in &self.rows {
                    // Can't re-use vals across yield calls for
                    // lifetime reasons (or I don't know how), so
                    // allocate a new one for every iteration.
                    let mut vals = Vec::new();
                    vals.push((key.clone(), Highlight::Neutral));
                    val.table_view_row(&mut vals);
                    co.yield_(vals.into()).await;
                }
            })
            .into_iter(),
        )
    }
}

impl<'key, K: TableKind, ViewType: Debug + ToStatsString + From<u64> + ResolutionUnit>
    TableFieldView<TILE_COUNT> for Table<'key, K, StatsOrCountOrSubStats<ViewType, TILE_COUNT>>
{
    fn table_key_vals<'s>(
        &'s self,
        stats_field: StatsField<TILE_COUNT>,
    ) -> Box<dyn Iterator<Item = KeyVal<&'s str, u64>> + 's> {
        Box::new(
            Gen::new(|co| async move {
                for KeyVal { key, val } in &self.rows {
                    let val = match val {
                        StatsOrCountOrSubStats::StatsOrCount(stats_or_count) => {
                            match stats_or_count {
                                StatsOrCount::Stats(stats) => stats.get(stats_field),
                                StatsOrCount::Count(_) => {
                                    // XX todo: I forgot: do I check
                                    // if stats_field is a count and
                                    // in that case give this value?
                                    continue;
                                }
                            }
                        }
                        StatsOrCountOrSubStats::SubStats(sub_stats) => match sub_stats {
                            SubStats::Count(stats) => stats.get(stats_field),
                            SubStats::ViewType(stats) => stats.get(stats_field),
                        },
                    };
                    co.yield_(KeyVal {
                        key: key.as_ref(),
                        val,
                    })
                    .await;
                }
            })
            .into_iter(),
        )
    }

    fn resolution_unit(&self) -> String {
        ViewType::RESOLUTION_UNIT_SHORT.into()
    }
}

impl<'key, K: TableKind, ViewType: Debug, const TILE_COUNT: usize>
    Table<'key, K, StatsOrCount<ViewType, TILE_COUNT>>
{
    /// Silently ignores rows with keys that only appear on one side.
    /// XX now take whole Groups.
    pub fn change<Better: IsBetter>(
        &self,
        to: &Self,
        extract: fn(&Stats<ViewType, TILE_COUNT>) -> u64,
    ) -> Table<'key, K, Change<Better>> {
        let mut rows: Vec<KeyVal<_, _>> = Vec::new();
        for either_or_both in self
            .rows
            .iter()
            .merge_join_by(&to.rows, |a, b| a.key.cmp(&b.key))
        {
            if let EitherOrBoth::Both(from, to) = either_or_both {
                match (&from.val, &to.val) {
                    (StatsOrCount::Stats(from_stats), StatsOrCount::Stats(to_stats)) => {
                        rows.push(KeyVal {
                            key: from.key.clone(), // OK, usually with a ref anyway?
                            val: Change::new(extract(from_stats), extract(to_stats)),
                        });
                        // XX but also pass data for significance!
                    }
                    (StatsOrCount::Count(_from), StatsOrCount::Count(_to)) => {
                        // Ignore bare counts for comparisons. -- XX
                        // or should it output the relation? Usually
                        // 1, but? But only when counts were asked! ->
                        // take this boolean info about the extract
                        // function as argument?
                    }
                    _ => panic!("not in sync, {from:?} vs. {to:?}"),
                }
            }
            // Silently ignore rows with keys that only appear on one
            // side.
        }
        Table {
            kind: self.kind.clone(), // XX?
            rows,
        }
    }
}
