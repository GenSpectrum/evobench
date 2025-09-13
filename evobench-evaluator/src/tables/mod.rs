//! A flexible way to generate files with tabular data.
//!
//! [`table_view`](table_view.rs) defines `TableViewRow` and
//! `TableView` traits that declare a tabular data representation.
//!
//! [`table_field_view`](table_field_view.rs): XXX
//!
//! [`table`](table.rs): defines `Table`, a concrete implementation of
//! `TableView` that holds rows pairing a string key (representing the
//! first column) with a value that implements `TableViewRow`
//! (representing the remaining columns). `Table` is also
//! parameterized with a `TableKind` type for type safety and to carry
//! metadata (used to represent RealTime, CpuTime, SysTime and
//! CtxSwitches tables, see
//! [../evaluator/all_field_tables.rs](../evaluator/all_field_tables.rs)).
//!
//! [`change`](change.rs) is an abstraction for values that represent
//! change, with formatting indicating positive/negative change, used
//! by the `change()` method on `Table` to produce a table that
//! represents the change between two tables.
//!
//! [`excel_table_view`](excel_table_view.rs): take a sequence of
//! values implementing `TableView` and convert them to an Excel file
//! with a workbook for each.

pub mod change;
pub mod excel_table_view;
pub mod table;
pub mod table_field_view;
pub mod table_view;
