use std::{borrow::Cow, fmt::Debug, marker::PhantomData};

use genawaiter::rc::Gen;
use itertools::{EitherOrBoth, Itertools};

use crate::{
    stats::{Change, IsBetter, Stats, ToStatsString},
    table_view::{ColumnFormatting, Highlight, TableView, TableViewRow, Unit},
};

fn opt_max<T: PartialOrd>(a: Option<T>, b: Option<T>) -> Option<T> {
    let a = a?;
    let b = b?;
    Some(if a > b { a } else { b })
}

#[derive(Debug)]
pub enum StatsOrCount<ViewType: Debug, const TILES_COUNT: usize> {
    Stats(Stats<ViewType, TILES_COUNT>),
    Count(usize),
}

impl<ViewType: From<u64> + ToStatsString + Debug, const TILES_COUNT: usize> TableViewRow
    for StatsOrCount<ViewType, TILES_COUNT>
{
    fn table_view_header() -> impl AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]> {
        Stats::<ViewType, TILES_COUNT>::table_view_header()
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

#[derive(Debug)]
pub struct KeyVal<K, V> {
    pub key: K,
    pub val: V,
}

/// `KEY_LABEL`: the column title for the key in the rows. (This needs
/// to be parameterized on the type level because `TableViewHeader`
/// must work for rows, and tables must be able to retrieve those
/// headers for the rows even if there are no rows (and if there were
/// rows, which runtime value would determine the headers?).)
pub trait TableKeyLabel {
    const KEY_LABEL: &str;
}

pub struct Table<'s, T, KeyLabel: TableKeyLabel> {
    // key_label is pub only to allow construction with other fields.
    pub key_label: PhantomData<KeyLabel>,
    /// Width of key column in number of characters (as per Excel),
    /// None == automatic.
    pub key_column_width: Option<f64>,
    /// Table name
    pub name: Cow<'s, str>,
    pub rows: Vec<KeyVal<Cow<'s, str>, T>>,
}

impl<'t, T: TableViewRow + TableViewRow, KeyLabel: TableKeyLabel> TableView
    for Table<'t, T, KeyLabel>
{
    fn table_view_header(&self) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>> {
        let mut header = vec![(
            KeyLabel::KEY_LABEL.into(),
            Unit::None,
            ColumnFormatting::String {
                width_chars: self.key_column_width,
            },
        )];
        let row_header = T::table_view_header();
        for label in row_header.as_ref() {
            header.push((*label).clone());
        }
        Box::new(header)
    }

    fn table_name(&self) -> &str {
        self.name.as_ref()
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

impl<'s, ViewType: Debug, const TILES_COUNT: usize, KeyLabel: TableKeyLabel>
    Table<'s, StatsOrCount<ViewType, TILES_COUNT>, KeyLabel>
{
    /// Silently ignores rows with keys that only appear on one side.
    /// XX now take whole Groups.
    pub fn change<Better: IsBetter>(
        &self,
        to: &Self,
        extract: fn(&Stats<ViewType, TILES_COUNT>) -> u64,
    ) -> Table<'s, Change<Better>, KeyLabel> {
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
            key_label: self.key_label,
            name: format!("from {} to {}", self.name, to.name).into(),
            rows,
            key_column_width: opt_max(self.key_column_width, to.key_column_width),
        }
    }
}
