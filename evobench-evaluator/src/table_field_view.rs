//! Stats related, e.g. for flamegraph

use crate::{join::KeyVal, stats::StatsField, table_view::TableView};

/// A full table, key and single value. dyn compatible. Combine with
/// TableView for `table_name`.
pub trait TableFieldView<const TILE_COUNT: usize>: TableView {
    /// Access to the list of (key / selected value)
    fn table_key_vals<'s>(
        &'s self,
        stats_field: StatsField<TILE_COUNT>,
    ) -> Box<dyn Iterator<Item = KeyVal<String, u64>> + 's>;
}
