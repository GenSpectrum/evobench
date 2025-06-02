//! Options parameterizing the evaluation (excludes subcommands or
//! similar, those remain in src/bin/*.rs).

use std::path::PathBuf;

use crate::stats::StatsField;

// We use 101 buckets for percentiles instead of 100, so that we get
// buckets at positions 50, 25, 75 for exact matches, OK? (Although
// note that the `Stats` median is not based on those buckets
// (anymore).)
pub const TILE_COUNT: usize = 101;

#[derive(clap::Args, Debug)]
pub struct EvaluationOpts {
    /// The width of the column with the probes path, in characters
    /// (as per Excel's definition of characters)
    #[clap(short, long, default_value = "100")]
    pub key_width: f64,

    /// Path to write Excel output to (currently required, as there is
    /// no other output format)
    #[clap(short, long)]
    pub excel: PathBuf,

    /// Include the internally-allocated thread number in call
    /// path strings in the output.
    #[clap(short, long)]
    pub show_thread_number: bool,

    /// Show the call path so that the leaf instead of the root is on
    /// the left.
    #[clap(short = 'r', long)]
    pub show_reversed: bool,
}

#[derive(clap::Args, Debug)]
pub struct FieldSelectorDimension3 {
    /// What stats field to select for the summary stats (i.e. of the
    /// 2nd dimension, for calculating the 3rd dimension in the data
    /// evaluation, after dimensions 1 (probe name) and 2 (stats
    /// fields)). Valid values: n|sum|average|median|sd or a floating
    /// point number between 0 and 1 for selecting a percentile.
    #[clap(long, default_value = "median")]
    pub summary_field: StatsField<TILE_COUNT>,
}

#[derive(clap::Args, Debug)]
pub struct FieldSelectorDimension4 {
    /// What stats field to select for the trend stats (i.e. of the
    /// 3rd dimension, for calculating the 4nd dimension in the data
    /// evaluation, after dimensions 1 (probe name), 2 (stats fields),
    /// 3 (stats of the field from dimension 2 selected by the
    /// --summary-field option)). See --summary-field docs for the
    /// valid values.
    #[clap(long, default_value = "median")]
    pub trend_field: StatsField<TILE_COUNT>,
}
