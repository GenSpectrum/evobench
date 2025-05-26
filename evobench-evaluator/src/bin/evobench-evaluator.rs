use std::borrow::Cow;
use std::fmt::{Debug, Display};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_evaluator::excel_table_view::excel_file_write;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::index_by_call_path::IndexByCallPath;
use evobench_evaluator::join::{keyval_inner_join, KeyVal};
use evobench_evaluator::log_data_index::{LogDataIndex, PathStringOptions, SpanId};
use evobench_evaluator::log_file::LogData;
use evobench_evaluator::log_message::Timing;
use evobench_evaluator::stats::{Stats, StatsError, StatsField, SubStats, ToStatsString};
use evobench_evaluator::table::{StatsOrCount, Table, TableKind};
use evobench_evaluator::table_view::{TableView, TableViewRow};
use evobench_evaluator::times::{MicroTime, NanoTime};

include!("../../include/evobench_version.rs");

const PROGRAM_NAME: &str = "evobench-evaluator";

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
struct Opts {
    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    command: Command,
}

#[derive(clap::Args, Debug)]
struct EvaluationOpts {
    /// The width of the column with the probes path, in characters
    /// (as per Excel's definition of characters)
    #[clap(short, long, default_value = "100")]
    key_width: f64,

    /// Path to write Excel output to (currently required, as there is
    /// no other output format)
    #[clap(short, long)]
    excel: PathBuf,

    /// Include the internally-allocated thread number in call
    /// path strings in the output.
    #[clap(short, long)]
    show_thread_number: bool,
}

#[derive(clap::Args, Debug)]
struct FieldSelectorDimension3 {
    /// What stats field to select for the summary stats (i.e. of the
    /// 2nd dimension, for calculating the 3rd dimension in the data
    /// evaluation, after dimensions 1 (probe name) and 2 (stats
    /// fields)). Valid values: n|sum|average|median|sd or a floating
    /// point number between 0 and 1 for selecting a percentile.
    #[clap(long, default_value = "median")]
    summary_field: StatsField<TILE_COUNT>,
}

#[derive(clap::Args, Debug)]
struct FieldSelectorDimension4 {
    /// What stats field to select for the trend stats (i.e. of the
    /// 3rd dimension, for calculating the 4nd dimension in the data
    /// evaluation, after dimensions 1 (probe name), 2 (stats fields),
    /// 3 (stats of the field from dimension 2 selected by the
    /// --summary-field option)). See --summary-field docs for the
    /// valid values.
    #[clap(long, default_value = "median")]
    trend_field: StatsField<TILE_COUNT>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Print version
    Version,

    /// Show statistics for a single benchmarking log file
    Single {
        #[clap(flatten)]
        evaluation_opts: EvaluationOpts,

        /// The path that was provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        path: PathBuf,
    },

    /// Show statistics for a set of benchmarking log files, all for
    /// the same software version.
    Summary {
        #[clap(flatten)]
        evaluation_opts: EvaluationOpts,
        #[clap(flatten)]
        field_selector_dimension_3: FieldSelectorDimension3,

        /// The paths that were provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        paths: Vec<PathBuf>,
    },

    /// Show statistics across multiple sets of benchmarking log
    /// files, each group consisting of files for the same software
    /// version. Each group is enclosed with square brackets, e.g.:
    /// `trend [ a.log b.log ] [ c.log ] [ d.log e.log ]` has data for
    /// 3 software versions, the first and third version with data
    /// from two runs each.
    Trend {
        #[clap(flatten)]
        evaluation_opts: EvaluationOpts,
        #[clap(flatten)]
        field_selector_dimension_3: FieldSelectorDimension3,
        #[clap(flatten)]
        field_selector_dimension_4: FieldSelectorDimension4,

        /// The paths that were provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        grouped_paths: Vec<PathBuf>,
    },
}

// We use 101 buckets for percentiles instead of 100, so that we get
// buckets at positions 50, 25, 75 for exact matches, OK? (Although
// note that the `Stats` median is not based on those buckets
// (anymore).)
const TILE_COUNT: usize = 101;

fn scopestats<K: KeyDetails>(
    log_data_index: &LogDataIndex,
    spans: &[SpanId],
) -> Result<Stats<K::ViewType, TILE_COUNT>, StatsError> {
    let vals: Vec<u64> = spans
        .into_iter()
        .filter_map(|span_id| -> Option<u64> {
            let span = span_id.get_from_db(log_data_index);
            let (start, end) = span.start_and_end()?;
            Some(K::timing_extract(end)?.into() - K::timing_extract(start)?.into())
        })
        .collect();
    Stats::from_values(vals)
}

fn pn_stats<K: KeyDetails>(
    log_data_index: &LogDataIndex,
    spans: &[SpanId],
    pn: &str,
) -> Result<KeyVal<Cow<'static, str>, StatsOrCountOrSubStats<K::ViewType, TILE_COUNT>>, StatsError>
{
    let r: Result<Stats<K::ViewType, TILE_COUNT>, StatsError> =
        scopestats::<K>(log_data_index, spans);
    match r {
        Ok(s) => Ok(KeyVal {
            key: pn.to_string().into(),
            val: StatsOrCount::Stats(s).into(),
        }),
        Err(StatsError::NoInputs) => {
            let count = spans.len();
            Ok(KeyVal {
                // Copy the keys to get a result with 'static lifetime
                key: pn.to_string().into(),
                val: StatsOrCount::Count(count).into(),
            })
        }
        Err(e) => Err(e),
    }
}

/// A table holding one field for all probes. We copy the keys (probe
/// names) to get a resulting Table with 'static lifetime.
fn table_for_field<'key, K: KeyDetails>(
    kind: K,
    log_data_index: &LogDataIndex<'key>,
    index_by_call_path: &'key IndexByCallPath<'key>,
) -> Result<Table<'static, K, StatsOrCountOrSubStats<K::ViewType, TILE_COUNT>>> {
    let mut rows = Vec::new();

    for pn in log_data_index.probe_names() {
        rows.push(pn_stats::<K>(
            log_data_index,
            log_data_index.spans_by_pn(&pn).unwrap(),
            pn,
        )?);
    }

    for call_path in index_by_call_path.call_paths() {
        rows.push(pn_stats::<K>(
            log_data_index,
            index_by_call_path.spans_by_call_path(call_path).unwrap(),
            call_path,
        )?);
    }

    Ok(Table { kind, rows })
}

#[derive(Clone, PartialEq, Debug)]
struct KeyRuntimeDetails {
    show_thread_number: bool,
    // XX and if reverse ordered
    key_column_width: Option<f64>,
}

impl KeyRuntimeDetails {
    fn key_label(&self) -> &str {
        if self.show_thread_number {
            "Probe name or path\n(A: across all threads, N: by thread number)"
        } else {
            "Probe name or path\n(A: across all threads)"
        }
    }
}

trait KeyDetails: TableKind {
    type ViewType: Into<u64> + From<u64> + ToStatsString + Debug + Display;
    fn new(det: KeyRuntimeDetails) -> Self;
    /// Extract a single value out of a `Timing`.
    fn timing_extract(timing: &Timing) -> Option<Self::ViewType>;
    /// Extract the statistics on these values out of an `AllFieldsTable`.
    fn all_fields_table_extract<'f>(
        aft: &'f AllFieldsTable<SingleRunStats>,
    ) -> &'f Table<'static, Self, StatsOrCountOrSubStats<Self::ViewType, TILE_COUNT>>;
}

macro_rules! def_key_details {
    { $T:tt: $ViewType:tt, $table_name:tt, $timing_extract:expr, $aft_extract:expr, } => {
        #[derive(Clone)]
        struct $T(KeyRuntimeDetails);
        impl TableKind for $T {
            fn table_name(&self) -> Cow<str> {
                $table_name.into()
            }
            fn table_key_label(&self) -> Cow<str> {
                self.0.key_label().into()
            }
            fn table_key_column_width(&self) -> Option<f64> {
                self.0.key_column_width
            }
        }
        impl KeyDetails for $T {
            type ViewType = $ViewType;
            fn new(det: KeyRuntimeDetails) -> Self { Self(det) }
            fn timing_extract(timing: &Timing) -> Option<Self::ViewType> {
                ($timing_extract)(timing)
            }
            fn all_fields_table_extract<'f>(
                aft: &'f AllFieldsTable<SingleRunStats>,
            ) -> &'f Table<'static, Self, StatsOrCountOrSubStats<Self::ViewType, TILE_COUNT>>{
                ($aft_extract)(aft)
            }
        }
    }
}

def_key_details! {
    RealTime:
    NanoTime, "real time",
    |timing: &Timing| Some(timing.r),
    |aft: &'f AllFieldsTable<SingleRunStats>| &aft.real_time,
}
def_key_details! {
    CpuTime:
    MicroTime, "cpu time",
    |timing: &Timing| Some(timing.u),
    |aft: &'f AllFieldsTable<SingleRunStats>| &aft.cpu_time,
}
def_key_details! {
    SysTime:
    MicroTime, "sys time",
    |timing: &Timing| Some(timing.s),
    |aft: &'f AllFieldsTable<SingleRunStats>| &aft.sys_time,
}
def_key_details! {
    CtxSwitches:
    u64, "ctx switches",
    |timing: &Timing| Some(timing.nvcsw()? + timing.nivcsw()?),
    |aft: &'f AllFieldsTable<SingleRunStats>| &aft.ctx_switches,
}

#[derive(Clone, Debug)]
struct AllFieldsTableKindParams {
    path: PathBuf,
    key_details: KeyRuntimeDetails,
    key_width: f64,
}

/// Markers to designate what a `Stats` value represents.
trait AllFieldsTableKind {}

/// Marks a `Stats` representing a single benchmarking run.
struct SingleRunStats;
impl AllFieldsTableKind for SingleRunStats {}

/// Marks a `Stats` over multiple (or at least 1, anyway) identical
/// benchmarking runs, to gain statistical insights. `Stats.n`
/// represents the number of runs for these, not the number of calls.
struct SummaryStats;
impl AllFieldsTableKind for SummaryStats {}

/// Marks a `Stats` containing Change records, i.e. trend values /
/// lines, across SummaryStats.
struct Trend;
impl AllFieldsTableKind for Trend {}

// XX can one do the same via GATs? For now just do a runtime switch.
enum StatsOrCountOrSubStats<ViewType: Debug, const TILE_COUNT: usize> {
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
    fn table_view_header(
        _: (),
    ) -> Box<
        dyn AsRef<
            [(
                Cow<'static, str>,
                evobench_evaluator::table_view::Unit,
                evobench_evaluator::table_view::ColumnFormatting,
            )],
        >,
    > {
        // XXX oh, which one to choose from? Are they the same?-- now
        // could base this on TableViewRow type parameter
        StatsOrCount::<ViewType, TILE_COUNT>::table_view_header(())
    }

    fn table_view_row(&self, out: &mut Vec<(Cow<str>, evobench_evaluator::table_view::Highlight)>) {
        match self {
            StatsOrCountOrSubStats::StatsOrCount(stats_or_count) => {
                stats_or_count.table_view_row(out)
            }
            StatsOrCountOrSubStats::SubStats(sub_stats) => sub_stats.table_view_row(out),
        }
    }
}

struct AllFieldsTable<Kind: AllFieldsTableKind> {
    kind: Kind,
    /// The parameters this table set was created from/with, for cache
    /// keying purposes.
    params: AllFieldsTableKindParams,
    real_time: Table<'static, RealTime, StatsOrCountOrSubStats<NanoTime, TILE_COUNT>>,
    cpu_time: Table<'static, CpuTime, StatsOrCountOrSubStats<MicroTime, TILE_COUNT>>,
    sys_time: Table<'static, SysTime, StatsOrCountOrSubStats<MicroTime, TILE_COUNT>>,
    ctx_switches: Table<'static, CtxSwitches, StatsOrCountOrSubStats<u64, TILE_COUNT>>,
}

impl<Kind: AllFieldsTableKind> AllFieldsTable<Kind> {
    fn tables(&self) -> Vec<&dyn TableView> {
        let mut tables: Vec<&dyn TableView> = vec![];
        let Self {
            kind: _,
            params: _,
            real_time,
            cpu_time,
            sys_time,
            ctx_switches,
        } = self;
        tables.push(real_time);
        tables.push(cpu_time);
        tables.push(sys_time);
        tables.push(ctx_switches);
        tables
    }
}

impl AllFieldsTable<SingleRunStats> {
    fn from_logfile(params: AllFieldsTableKindParams) -> Result<Self> {
        let AllFieldsTableKindParams {
            path,
            key_width: _, // the whole `params` will be used below
            key_details,
        } = &params;

        let data = LogData::read_file(path, None)?;
        let log_data_index = LogDataIndex::from_logdata(&data)?;

        let index_by_call_path = {
            // Note: it's important to give prefixes here, to
            // avoid getting rows that have the scopes counted
            // *twice* (currently just "main thread"). (Could
            // handle that in `IndexByCallPath::from_logdataindex`
            // (by using a set instead of Vec), but having 1 entry
            // that only counts thing once, but is valid for both
            // kinds of groups, would surely still be confusing.)
            let mut opts = vec![PathStringOptions {
                ignore_process: true,
                ignore_thread: true,
                include_thread_number_in_path: false,
                // "across threads / added up"
                prefix: "A:",
            }];
            if key_details.show_thread_number {
                opts.push(PathStringOptions {
                    ignore_process: true,
                    ignore_thread: true,
                    include_thread_number_in_path: true,
                    // "numbered threads"
                    prefix: "N:",
                });
            }
            IndexByCallPath::from_logdataindex(&log_data_index, &opts)
        };

        let real_time = table_for_field(
            RealTime(key_details.clone()),
            &log_data_index,
            &index_by_call_path,
        )?;
        let cpu_time = table_for_field(
            CpuTime(key_details.clone()),
            &log_data_index,
            &index_by_call_path,
        )?;
        let sys_time = table_for_field(
            SysTime(key_details.clone()),
            &log_data_index,
            &index_by_call_path,
        )?;
        let ctx_switches = table_for_field(
            CtxSwitches(key_details.clone()),
            &log_data_index,
            &index_by_call_path,
        )?;

        Ok(AllFieldsTable {
            kind: SingleRunStats,
            params,
            real_time,
            cpu_time,
            sys_time,
            ctx_switches,
        })
    }
}

/// `K::all_fields_table_extract` extracts the field out of
/// `AllFieldsTable` (e.g. cpu time, or ctx switches),
/// `extract_stats_field` the kind of statistical value (e.g. median,
/// average, counts, etc.)
fn summary_stats_for_field<'t, K: KeyDetails + 'static>(
    key_details: &KeyRuntimeDetails,
    afts: &[AllFieldsTable<SingleRunStats>],
    extract_stats_field: StatsField<TILE_COUNT>, // XX add to cache key somehow !
) -> Table<'static, K, StatsOrCountOrSubStats<K::ViewType, TILE_COUNT>>
where
    K::ViewType: 'static,
{
    let mut rowss: Vec<_> = afts
        .iter()
        .map(|aft| {
            Some(K::all_fields_table_extract(aft).rows.iter().map(
                |KeyVal { key, val }| -> KeyVal<Cow<'static, str>, _> {
                    KeyVal {
                        key: key.clone(),
                        val,
                    }
                },
            ))
        })
        .collect();
    let rows_merged: Vec<_> = keyval_inner_join(&mut rowss)
        .expect("at least 1 table")
        .collect();
    let rows: Vec<_> = rows_merged
        .into_iter()
        .filter_map(|KeyVal { key, val }| {
            let vals: Vec<u64> = val
                .iter()
                .filter_map(|s| match s {
                    StatsOrCountOrSubStats::StatsOrCount(stats_or_count) => match stats_or_count {
                        StatsOrCount::Stats(stats) => Some(stats.get(extract_stats_field)),
                        StatsOrCount::Count(c) => {
                            if extract_stats_field == StatsField::N {
                                Some(u64::try_from(*c).expect("hopefully in range, here, too"))
                            } else {
                                None
                            }
                        }
                    },
                    StatsOrCountOrSubStats::SubStats(sub_stats) => todo!(),
                })
                .collect();
            let val = match Stats::<K::ViewType, TILE_COUNT>::from_values_from_field(
                extract_stats_field,
                vals,
            ) {
                Ok(val) => Some(val.into()),
                Err(StatsError::NoInputs) => {
                    // This does happen, even after 'at least 1 table':
                    // sure, if only a Count happened I guess?  So,
                    // eliminate the row completely?
                    None
                }
                Err(StatsError::SaturatedU128) => {
                    unreachable!("expecting to never see values > u64")
                }
            };
            let val = val?;
            Some(KeyVal { key, val })
        })
        .collect();

    Table {
        kind: K::new(key_details.clone()), // XX ?*
        rows,
    }
}

impl AllFieldsTable<SummaryStats> {
    fn summary_stats(
        field_selector: StatsField<TILE_COUNT>,
        key_details: &KeyRuntimeDetails,
        afts: &[AllFieldsTable<SingleRunStats>],
    ) -> AllFieldsTable<SummaryStats> {
        // XX panic happy everywhere...
        let params = afts[0].params.clone();
        for aft in afts {
            if params.key_details != aft.params.key_details {
                panic!(
                    "unequal key_details in params: {:?} vs. {:?}",
                    params, aft.params
                );
            }
        }

        let real_time = summary_stats_for_field::<RealTime>(key_details, afts, field_selector);
        let cpu_time = summary_stats_for_field::<CpuTime>(key_details, afts, field_selector);
        let sys_time = summary_stats_for_field::<SysTime>(key_details, afts, field_selector);
        let ctx_switches =
            summary_stats_for_field::<CtxSwitches>(&key_details, afts, field_selector);
        AllFieldsTable {
            kind: SummaryStats,
            params,
            real_time,
            cpu_time,
            sys_time,
            ctx_switches,
        }
    }
}

fn main() -> Result<()> {
    let Opts { command } = Opts::parse();
    match command {
        Command::Version => println!("{PROGRAM_NAME} version {EVOBENCH_VERSION}"),

        Command::Single {
            evaluation_opts:
                EvaluationOpts {
                    key_width,
                    excel,
                    show_thread_number,
                },
            path,
        } => {
            let aft = AllFieldsTable::from_logfile(AllFieldsTableKindParams {
                path,
                key_width,
                key_details: KeyRuntimeDetails {
                    show_thread_number,
                    key_column_width: Some(key_width),
                },
            })?;
            excel_file_write(&aft.tables(), &excel)?;
        }

        Command::Summary {
            evaluation_opts:
                EvaluationOpts {
                    key_width,
                    excel,
                    show_thread_number,
                },
            paths,
            field_selector_dimension_3: FieldSelectorDimension3 { summary_field },
        } => {
            let afts: Vec<AllFieldsTable<SingleRunStats>> = paths
                .iter()
                .map(|path| {
                    AllFieldsTable::from_logfile(AllFieldsTableKindParams {
                        path: path.into(),
                        key_width,
                        key_details: KeyRuntimeDetails {
                            show_thread_number,
                            key_column_width: Some(key_width),
                        },
                    })
                })
                .collect::<Result<_>>()?;
            let aft = AllFieldsTable::<SummaryStats>::summary_stats(
                summary_field,
                &KeyRuntimeDetails {
                    show_thread_number,
                    key_column_width: Some(key_width),
                },
                &afts,
            );
            excel_file_write(&aft.tables(), &excel)?;
        }

        Command::Trend {
            evaluation_opts,
            grouped_paths,
            field_selector_dimension_3: FieldSelectorDimension3 { summary_field },
            field_selector_dimension_4: FieldSelectorDimension4 { trend_field },
        } => todo!(),
    }

    Ok(())
}
