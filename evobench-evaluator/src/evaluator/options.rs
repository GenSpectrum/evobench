//! Options parameterizing the evaluation (excludes subcommands or
//! similar, those remain in src/bin/*.rs).

use std::path::PathBuf;

use anyhow::{bail, Result};

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

    /// Include the internally-allocated thread number in call
    /// path strings in the output.
    #[clap(short, long)]
    pub show_thread_number: bool,

    /// Show the call path so that the leaf instead of the root is on
    /// the left (only has an effect on tables (Excel), not
    /// flamegraphs).
    #[clap(short = 'r', long)]
    pub show_reversed: bool,
}

/// Private fields to enforce .check()
#[derive(clap::Args, Debug)]
pub struct OutputOpts {
    /// Path to write Excel output to
    #[clap(short, long)]
    excel: Option<PathBuf>,

    /// Base path to write flame graph SVG to; "-$type.svg" is
    /// appended, where type is "real", "cpu", "sys" or
    /// "ctx-switches".
    #[clap(short, long)]
    flame: Option<PathBuf>,

    /// What field to select for the flame graph.
    #[clap(long, default_value = "avg")]
    flame_field: StatsField<TILE_COUNT>,
}

/// OutputOpts split into checked `OutputVariants` and possibly other
/// options
pub struct CheckedOutputOpts {
    pub variants: OutputVariants<PathBuf>,
    pub flame_field: StatsField<TILE_COUNT>,
}

impl OutputOpts {
    pub fn check(self) -> Result<CheckedOutputOpts> {
        let Self {
            excel,
            flame,
            flame_field,
        } = self;

        let any_given = [excel.is_some(), flame.is_some()].iter().any(|b| *b);
        if !any_given {
            bail!("no output files were specified")
        }

        Ok(CheckedOutputOpts {
            variants: OutputVariants { excel, flame },
            flame_field,
        })
    }
}

/// Same as OutputOpts but at least one file is set; parameterized so
/// it can be used for pipelining via its `map` method.
#[derive(Clone)]
pub struct OutputVariants<T> {
    pub excel: Option<T>,
    pub flame: Option<T>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CheckedOutputOptsMapCase {
    Excel,
    Flame,
}

impl<T> OutputVariants<T> {
    /// get a field
    pub fn get(&self, case: CheckedOutputOptsMapCase) -> &Option<T> {
        match case {
            CheckedOutputOptsMapCase::Excel => &self.excel,
            CheckedOutputOptsMapCase::Flame => &self.flame,
        }
    }

    /// `f` is applied to all fields that are `Some`
    pub fn map<U>(self, f: impl Fn(CheckedOutputOptsMapCase, T) -> U) -> OutputVariants<U> {
        let Self { excel, flame } = self;
        OutputVariants {
            excel: excel.map(|v| f(CheckedOutputOptsMapCase::Excel, v)),
            flame: flame.map(|v| f(CheckedOutputOptsMapCase::Flame, v)),
        }
    }

    /// `f` is applied to all fields that are `Some`
    pub fn try_map<U, E>(
        self,
        f: impl Fn(CheckedOutputOptsMapCase, T) -> Result<U, E>,
    ) -> Result<OutputVariants<U>, E> {
        let Self { excel, flame } = self;
        Ok(OutputVariants {
            excel: excel
                .map(|v| f(CheckedOutputOptsMapCase::Excel, v))
                .transpose()?,
            flame: flame
                .map(|v| f(CheckedOutputOptsMapCase::Flame, v))
                .transpose()?,
        })
    }
}

#[derive(clap::Args, Debug)]
pub struct EvaluationAndOutputOpts {
    #[clap(flatten)]
    pub evaluation_opts: EvaluationOpts,
    #[clap(flatten)]
    pub output_opts: OutputOpts,
}

#[derive(clap::Args, Debug)]
pub struct FieldSelectorDimension3 {
    /// What stats field to select for the summary stats (i.e. of the
    /// 2nd dimension, for calculating the 3rd dimension in the data
    /// evaluation, after dimensions 1 (probe name) and 2 (stats
    /// fields)). Valid values: n|sum|average|median|sd or a floating
    /// point number between 0 and 1 for selecting a percentile.
    #[clap(long, default_value = "avg")]
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
