use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    path::PathBuf,
};

use anyhow::Result;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};

use crate::{
    evaluator::options::TILE_COUNT,
    index_by_call_path::IndexByCallPath,
    join::{keyval_inner_join, KeyVal},
    log_data_index::{LogDataIndex, PathStringOptions, SpanId},
    log_file::LogData,
    log_message::Timing,
    rayon_util::ParRun,
    stats::{Stats, StatsError, StatsField, SubStats, ToStatsString},
    table::{StatsOrCount, Table, TableKind},
    table_view::{ColumnFormatting, Highlight, TableView, TableViewRow, Unit},
    times::{MicroTime, NanoTime},
};

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
pub struct KeyRuntimeDetails {
    pub show_thread_number: bool,
    pub show_reversed: bool,
    pub key_column_width: Option<f64>,
}

impl KeyRuntimeDetails {
    fn key_label(&self) -> String {
        let mut cases = Vec::new();
        cases.push("A: across all threads");
        if self.show_thread_number {
            cases.push("N: by thread number");
        }
        if self.show_reversed {
            cases.push("..R: reversed");
        }
        format!("Probe name or path\n({})", cases.join(", ")).into()
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
        pub struct $T(KeyRuntimeDetails);
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
pub struct AllFieldsTableKindParams {
    pub path: PathBuf,
    pub key_details: KeyRuntimeDetails,
    pub key_width: f64,
}

/// Markers to designate what a `Stats` value represents.
pub trait AllFieldsTableKind {}

/// Marks a `Stats` representing a single benchmarking run.
pub struct SingleRunStats;
impl AllFieldsTableKind for SingleRunStats {}

/// Marks a `Stats` over multiple (or at least 1, anyway) identical
/// benchmarking runs, to gain statistical insights. `Stats.n`
/// represents the number of runs for these, not the number of calls.
pub struct SummaryStats;
impl AllFieldsTableKind for SummaryStats {}

/// Marks a `Stats` containing Change records, i.e. trend values /
/// lines, across SummaryStats.
pub struct Trend;
impl AllFieldsTableKind for Trend {}

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

pub struct AllFieldsTable<Kind: AllFieldsTableKind> {
    pub kind: Kind,
    /// The parameters this table set was created from/with, for cache
    /// keying purposes.
    pub params: AllFieldsTableKindParams,
    pub real_time: Table<'static, RealTime, StatsOrCountOrSubStats<NanoTime, TILE_COUNT>>,
    pub cpu_time: Table<'static, CpuTime, StatsOrCountOrSubStats<MicroTime, TILE_COUNT>>,
    pub sys_time: Table<'static, SysTime, StatsOrCountOrSubStats<MicroTime, TILE_COUNT>>,
    pub ctx_switches: Table<'static, CtxSwitches, StatsOrCountOrSubStats<u64, TILE_COUNT>>,
}

impl<Kind: AllFieldsTableKind> AllFieldsTable<Kind> {
    /// Return a list of tables, one for each field (real, cpu, sys
    /// times and ctx switches), to e.g. be output to excel.
    pub fn tables(&self) -> Vec<&dyn TableView> {
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
    pub fn from_logfile(params: AllFieldsTableKindParams) -> Result<Self> {
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
            let mut opts = vec![];
            opts.push(PathStringOptions {
                ignore_process: true,
                ignore_thread: true,
                include_thread_number_in_path: false,
                reversed: false,
                // "across threads / added up"
                prefix: "A:",
            });
            if key_details.show_reversed {
                opts.push(PathStringOptions {
                    ignore_process: true,
                    ignore_thread: true,
                    include_thread_number_in_path: false,
                    reversed: true,
                    prefix: "AR:",
                });
            }
            if key_details.show_thread_number {
                opts.push(PathStringOptions {
                    ignore_process: true,
                    ignore_thread: true,
                    include_thread_number_in_path: true,
                    reversed: false,
                    // "numbered threads"
                    prefix: "N:",
                });
                if key_details.show_reversed {
                    opts.push(PathStringOptions {
                        ignore_process: true,
                        ignore_thread: true,
                        include_thread_number_in_path: true,
                        reversed: true,
                        prefix: "NR:",
                    });
                }
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
        .par_iter()
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
        .into_par_iter()
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
                    StatsOrCountOrSubStats::SubStats(_sub_stats) => {
                        unreachable!("SingleRunStats cannot contain SubStats")
                    }
                })
                .collect();
            let maybe_val = match Stats::<K::ViewType, TILE_COUNT>::from_values_from_field(
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
            let val = maybe_val?;
            Some(KeyVal { key, val })
        })
        .collect();

    Table {
        kind: K::new(key_details.clone()), // XX ?*
        rows,
    }
}

impl AllFieldsTable<SummaryStats> {
    pub fn summary_stats(
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

        let (real_time, cpu_time, sys_time, ctx_switches) = (
            || summary_stats_for_field::<RealTime>(key_details, afts, field_selector),
            || summary_stats_for_field::<CpuTime>(key_details, afts, field_selector),
            || summary_stats_for_field::<SysTime>(key_details, afts, field_selector),
            || summary_stats_for_field::<CtxSwitches>(&key_details, afts, field_selector),
        )
            .par_run();

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
