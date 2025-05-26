//! A representation of tables (and individual rows) as title row and
//! body rows of strings and formatting instructions, independent of
//! serialisation format.

use std::borrow::Cow;

#[derive(Debug, Clone, Copy)]
pub enum Unit {
    /// No unit, e.g. for spacer columns
    None,
    /// E.g. factors, could be floats
    DimensionLess,
    /// Integers
    Count,
    /// From the ViewType the container is parameterized with
    ViewType(&'static str),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Highlight {
    /// Used for spacer columns, i.e. no value is there.
    Spacer,
    /// No special formatting, normal number display
    Neutral,
    /// "Bad"
    Red,
    /// "Good"
    Green,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnFormatting {
    /// Spacercolumn, should have no values
    Spacer,
    /// Values are numbers: right-adjusted, and auto-width
    Number,
    /// Values are (potentially long) strings, left-adjusted, fixed
    /// width
    String {
        /// In Excel widths. None == automatic.
        width_chars: Option<f64>,
    },
}

pub trait TableViewRow<Context> {
    /// Column names and unit. Not dyn compatible, must be static
    /// because it needs to be available for tables in the absense of
    /// rows. But to accommodate for dynamically decided changes,
    /// takes a context argument (which could be ()).
    fn table_view_header(
        ctx: Context,
    ) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>>;

    /// Write the given row to `out`, matching the columns in the
    /// `TableViewHeader`. Do *not* clear out inside this method!
    fn table_view_row(&self, out: &mut Vec<(Cow<str>, Highlight)>);
}

/// A full table. dyn compatible.
pub trait TableView {
    fn table_name(&self) -> Cow<str>;

    /// Column names and unit.
    fn table_view_header(&self) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>>;

    fn table_view_body<'s>(
        &'s self,
    ) -> Box<dyn Iterator<Item = Cow<'s, [(Cow<'s, str>, Highlight)]>> + 's>;
}
