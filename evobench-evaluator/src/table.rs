use std::{borrow::Cow, fmt::Debug};

use genawaiter::rc::Gen;
use itertools::{EitherOrBoth, Itertools};

use crate::{
    change::{Change, IsBetter},
    join::KeyVal,
    stats::{Stats, ToStatsString},
    table_view::{ColumnFormatting, Highlight, TableView, TableViewRow, Unit},
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

pub trait TableKind: Clone {
    fn table_name(&self) -> Cow<str>;
    /// The column title for the *key* field in the rows
    fn table_key_label(&self) -> Cow<str>;
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

    fn table_name(&self) -> Cow<str> {
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
