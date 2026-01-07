use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    num::NonZeroU32,
    path::PathBuf,
};

use anyhow::Result;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};

use crate::{
    dynamic_typing::{StatsOrCount, StatsOrCountOrSubStats},
    evaluator::{
        data::{
            log_data_tree::{LogDataTree, PathStringOptions, SpanId},
            log_message::Timing,
        },
        index_by_call_path::IndexByCallPath,
        options::TILE_COUNT,
    },
    join::{keyval_inner_join, KeyVal},
    rayon_util::ParRun,
    stats::{
        weighted::{WeightedValue, WEIGHT_ONE},
        Stats, StatsError, StatsField, ToStatsString,
    },
    tables::{
        table::{Table, TableKind},
        table_field_view::TableFieldView,
    },
    times::{MicroTime, NanoTime},
};

fn scopestats<K: KeyDetails>(
    log_data_tree: &LogDataTree,
    spans: &[SpanId],
) -> Result<Stats<K::ViewType, TILE_COUNT>, StatsError> {
    let vals: Vec<WeightedValue> = spans
        .into_iter()
        .filter_map(|span_id| -> Option<_> {
            let span = span_id.get_from_db(log_data_tree);
            let (start, end) = span.start_and_end()?;
            let value: u64 = K::timing_extract(end)?.into() - K::timing_extract(start)?.into();
            // Handle `EVOBENCH_SCOPE_EVERY` with `every_n > 1`
            let weight = NonZeroU32::try_from(start.n())
                .expect("num_runs is always at least 1 in the start Timing");
            Some(WeightedValue { value, weight })
        })
        .collect();
    Stats::from_values(vals)
}

fn pn_stats<K: KeyDetails>(
    log_data_tree: &LogDataTree,
    spans: &[SpanId],
    pn: &str,
) -> Result<KeyVal<Cow<'static, str>, StatsOrCountOrSubStats<K::ViewType, TILE_COUNT>>, StatsError>
{
    let r: Result<Stats<K::ViewType, TILE_COUNT>, StatsError> =
        scopestats::<K>(log_data_tree, spans);
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
    log_data_tree: &LogDataTree<'key>,
    index_by_call_path: &'key IndexByCallPath<'key>,
) -> Result<Table<'static, K, StatsOrCountOrSubStats<K::ViewType, TILE_COUNT>>> {
    let mut rows = Vec::new();

    // Add the bare probe names, not paths, to the table if desired
    if kind.show_probe_names() {
        for pn in log_data_tree.probe_names() {
            rows.push(pn_stats::<K>(
                log_data_tree,
                log_data_tree.spans_by_pn(&pn).unwrap(),
                pn,
            )?);
        }
    }

    for call_path in index_by_call_path.call_paths() {
        rows.push(pn_stats::<K>(
            log_data_tree,
            index_by_call_path.spans_by_call_path(call_path).unwrap(),
            call_path,
        )?);
    }

    Ok(Table { kind, rows })
}

/// How keys (in AllFieldsTable) are presented, and, unlike what the
/// name suggests, also what rows are generated, since the grouping of
/// the measurements depends on the set of generated key
/// strings. (This only contains the runtime data, but unlike what the
/// name suggests, actually there is no static data for the key
/// column?)
#[derive(Clone, PartialEq, Debug)]
pub struct KeyRuntimeDetails {
    /// The separators to use
    pub normal_separator: &'static str,
    pub reverse_separator: &'static str,
    /// Whether to use the probe names as keys (versus paths)
    pub show_probe_names: bool,
    pub show_paths_without_thread_number: bool,
    pub show_paths_with_thread_number: bool,
    pub show_paths_reversed_too: bool,
    pub key_column_width: Option<f64>,
    /// Override the standard prefixes--the same is used for all modes
    /// above!
    pub prefix: Option<&'static str>,
    /// Do not show process measurement (used for flamegraph)
    pub skip_process: bool,
}

impl KeyRuntimeDetails {
    fn key_label(&self) -> String {
        let mut cases = Vec::new();
        cases.push("A: across all threads");
        if self.show_paths_with_thread_number {
            cases.push("N: by thread number");
        }
        if self.show_paths_reversed_too {
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
    /// Whether probe *names* (not paths) are part of the table
    fn show_probe_names(&self) -> bool;
}

macro_rules! def_key_details {
    { $T:tt: $ViewType:tt, $table_name:tt, $timing_extract:expr, $aft_extract:expr, } => {
        #[derive(Clone)]
        pub struct $T(KeyRuntimeDetails);
        impl TableKind for $T {
            fn table_name(&self) -> Cow<'_, str> {
                $table_name.into()
            }
            fn table_key_label(&self) -> Cow<'_, str> {
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
            fn show_probe_names(&self) -> bool {
                self.0.show_probe_names
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
    pub source_path: PathBuf,
    pub key_details: KeyRuntimeDetails,
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
pub struct TrendStats;
impl AllFieldsTableKind for TrendStats {}

/// A group of 4 tables, one per real/cpu/sys time and ctx switches,
/// rows representing probe points, although the exact rows depend on
/// `params.key_details`
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

impl<Kind: AllFieldsTableKind> AsRef<AllFieldsTable<Kind>> for AllFieldsTable<Kind> {
    fn as_ref(&self) -> &AllFieldsTable<Kind> {
        self
    }
}

impl<Kind: AllFieldsTableKind> AllFieldsTable<Kind> {
    /// Return a list of tables, one for each field (real, cpu, sys
    /// times and ctx switches), to e.g. be output to excel.
    pub fn tables(&self) -> Vec<&dyn TableFieldView<TILE_COUNT>> {
        let mut tables: Vec<&dyn TableFieldView<TILE_COUNT>> = vec![];
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
    pub fn from_log_data_tree(
        log_data_tree: &LogDataTree,
        params: AllFieldsTableKindParams,
    ) -> Result<Self> {
        let AllFieldsTableKindParams {
            key_details,
            // the whole `params` will be used below
            source_path: _,
        } = &params;

        let KeyRuntimeDetails {
            normal_separator,
            reverse_separator,
            show_paths_without_thread_number,
            show_paths_with_thread_number,
            show_paths_reversed_too,
            skip_process,
            prefix,
            // show_probe_names and key_column_width are passed to
            // `table_for_field` inside its `kind` argument
            show_probe_names: _,
            key_column_width: _,
        } = key_details;
        let skip_process = *skip_process;

        let index_by_call_path = {
            // Note: it's important to give prefixes here, to
            // avoid getting rows that have the scopes counted
            // *twice* (currently just "main thread"). (Could
            // handle that in `IndexByCallPath::from_logdataindex`
            // (by using a set instead of Vec), but having 1 entry
            // that only counts thing once, but is valid for both
            // kinds of groups, would surely still be confusing.)
            let mut opts = vec![];
            if *show_paths_without_thread_number {
                opts.push(PathStringOptions {
                    normal_separator,
                    reverse_separator,
                    ignore_process: true,
                    skip_process,
                    ignore_thread: true,
                    include_thread_number_in_path: false,
                    reversed: false,
                    // "across threads / added up"
                    prefix: prefix.unwrap_or("A:"),
                });
            }
            // XX should this be nested in the above, like for
            // show_paths_with_thread_number, or rather really not?
            // Really should make separate options for ALL of
            // those. Currently IIRC the logic is that the user's
            // option is passed down only once, in
            // show_paths_reversed_too, and we deal with it in this
            // contorted way for that reason.
            if *show_paths_reversed_too {
                opts.push(PathStringOptions {
                    normal_separator,
                    reverse_separator,
                    ignore_process: true,
                    skip_process,
                    ignore_thread: true,
                    include_thread_number_in_path: false,
                    reversed: true,
                    prefix: prefix.unwrap_or("AR:"),
                });
            }
            if *show_paths_with_thread_number {
                opts.push(PathStringOptions {
                    normal_separator,
                    reverse_separator,
                    ignore_process: true,
                    skip_process,
                    ignore_thread: true,
                    include_thread_number_in_path: true,
                    reversed: false,
                    // "numbered threads"
                    prefix: prefix.unwrap_or("N:"),
                });
                if *show_paths_reversed_too {
                    opts.push(PathStringOptions {
                        normal_separator,
                        reverse_separator,
                        ignore_process: true,
                        skip_process,
                        ignore_thread: true,
                        include_thread_number_in_path: true,
                        reversed: true,
                        prefix: prefix.unwrap_or("NR:"),
                    });
                }
            }
            IndexByCallPath::from_logdataindex(&log_data_tree, &opts)
        };

        let real_time = table_for_field(
            RealTime(key_details.clone()),
            &log_data_tree,
            &index_by_call_path,
        )?;
        let cpu_time = table_for_field(
            CpuTime(key_details.clone()),
            &log_data_tree,
            &index_by_call_path,
        )?;
        let sys_time = table_for_field(
            SysTime(key_details.clone()),
            &log_data_tree,
            &index_by_call_path,
        )?;
        let ctx_switches = table_for_field(
            CtxSwitches(key_details.clone()),
            &log_data_tree,
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
    afts: &[impl AsRef<AllFieldsTable<SingleRunStats>> + Sync],
    extract_stats_field: StatsField<TILE_COUNT>, // XX add to cache key somehow !
) -> Table<'static, K, StatsOrCountOrSubStats<K::ViewType, TILE_COUNT>>
where
    K::ViewType: 'static,
{
    let mut rowss: Vec<_> = afts
        .par_iter()
        .map(|aft| {
            Some(K::all_fields_table_extract(aft.as_ref()).rows.iter().map(
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
            // XX using WeightedValue here just because Stats requires
            // it now! Kinda ugly? Make separate Stats methods?
            let vals: Vec<WeightedValue> = val
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
                .map(|value| WeightedValue {
                    value,
                    weight: WEIGHT_ONE,
                })
                .collect();
            let maybe_val = match Stats::<K::ViewType, TILE_COUNT>::from_values_from_field(
                extract_stats_field,
                vals,
            ) {
                Ok(val) => Some(val.into()),
                Err(e) => match e {
                    StatsError::NoInputs => {
                        // This does happen, even after 'at least 1 table':
                        // sure, if only a Count happened I guess?  So,
                        // eliminate the row completely?
                        None
                    }
                    StatsError::SaturatedU128 => {
                        unreachable!("expecting to never see values > u64")
                    }
                    StatsError::VirtualCountDoesNotFitUSize => unreachable!("on 64bit archs"),
                    StatsError::VirtualSumDoesNotFitU96 => panic!("stats error: {e}"),
                },
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
        afts: &[impl AsRef<AllFieldsTable<SingleRunStats>> + Sync],
        field_selector: StatsField<TILE_COUNT>,
        key_details: &KeyRuntimeDetails,
    ) -> AllFieldsTable<SummaryStats> {
        // XX panic happy everywhere...
        let params = afts[0].as_ref().params.clone();
        for aft in afts {
            if params.key_details != aft.as_ref().params.key_details {
                panic!(
                    "unequal key_details in params: {:?} vs. {:?}",
                    params,
                    aft.as_ref().params
                );
            }
        }

        let (real_time, cpu_time, sys_time, ctx_switches) = (
            || summary_stats_for_field::<RealTime>(key_details, afts, field_selector),
            || summary_stats_for_field::<CpuTime>(key_details, afts, field_selector),
            || summary_stats_for_field::<SysTime>(key_details, afts, field_selector),
            || summary_stats_for_field::<CtxSwitches>(key_details, afts, field_selector),
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
