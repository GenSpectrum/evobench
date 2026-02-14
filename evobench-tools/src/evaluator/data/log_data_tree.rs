//! Build tree and index for making summaries.

//! `Timing` and contextual info remains in the parsed log file
//! (`Vec<LogMessage>`), the index just references into those.

//! `LogDataTree::from_logdata` both pairs the start and end timings
//! and builds up a tree of all spans.

use std::{
    collections::{HashMap, hash_map::Entry},
    fmt::{Display, Write},
    marker::PhantomData,
    num::NonZeroU32,
    ops::Deref,
};

use anyhow::{Result, anyhow, bail};

use crate::evaluator::data::{
    log_data::LogData,
    log_message::{DataMessage, KeyValue, PointKind, ThreadId, Timing},
};

#[derive(Debug)]
pub struct LogDataTree<'t> {
    log_data: &'t LogData,
    spans: Vec<Span<'t>>,
    /// For a probe name, all the spans in sequence as occurring in
    /// the log file (which isn't necessarily by time when multiple
    /// threads are running), regardless of thread and their `parent`
    /// inside the thread.
    spans_by_pn: HashMap<&'t str, Vec<SpanId<'t>>>,
}

macro_rules! def_log_data_index_id {
    { {$TId:tt $($TIdLifetime:tt)*} | $T:tt | $db_field:tt | $add_method:tt } => {
        #[derive(Debug, Clone, Copy)]
        pub struct $TId $($TIdLifetime)* {
            t: PhantomData<fn() -> $T $($TIdLifetime)* >,
            id: NonZeroU32,
        }

        impl $($TIdLifetime)* $TId $($TIdLifetime)* {
            fn new(index: usize) -> Self {
                let id: u32 = index.try_into().expect("index not outside u32 range");
                let id: NonZeroU32 = id.try_into().expect("index 1-based");
                Self { id, t: PhantomData::default() }
            }

            // We use len after insertion as the id, so that id 0 is
            // never used, so that Option is cheap.
            fn index(self) -> usize { usize::try_from(u32::from(self.id)).unwrap() - 1 }

            pub fn get_from_db<'d>(self, db: &'d LogDataTree<'t>) -> &'d $T$($TIdLifetime)*
                // XX are these even required or helpful?:
                where 't: 'd
            {
                &db.$db_field[self.index()]
            }

            pub fn get_mut_from_db<'d>(self, db: &'d mut LogDataTree<'t>) -> &'d mut $T$($TIdLifetime)*
                where 't: 'd
            {
                &mut db.$db_field[self.index()]
            }
        }

        impl<'t> LogDataTree<'t> {
            pub fn $add_method(&mut self, value: $T $($TIdLifetime)*) -> $TId $($TIdLifetime)* {
                self.$db_field.push(value);
                $TId::new(self.$db_field.len())
            }
        }
    }
}

def_log_data_index_id! {{SpanId<'t>} | Span | spans | add_span }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScopeKind {
    Process,
    Thread,
    Scope,
}

/// How a point log message should be handled
enum PointDispatch {
    Scope { kind: ScopeKind, is_ending: bool },
    T,
    TIO,
}

impl PointDispatch {
    pub fn from_point_kind(kind: PointKind) -> Self {
        use ScopeKind::*;
        match kind {
            PointKind::TStart => PointDispatch::Scope {
                kind: Process,
                is_ending: false,
            },
            PointKind::T => PointDispatch::T,
            PointKind::TS => PointDispatch::Scope {
                kind: Scope,
                is_ending: false,
            },
            PointKind::TE => PointDispatch::Scope {
                kind: Scope,
                is_ending: true,
            },
            PointKind::TThreadStart => PointDispatch::Scope {
                kind: Thread,
                is_ending: false,
            },
            PointKind::TThreadEnd => PointDispatch::Scope {
                kind: Thread,
                is_ending: true,
            },
            PointKind::TEnd => PointDispatch::Scope {
                kind: Process,
                is_ending: true,
            },
            PointKind::TIO => PointDispatch::TIO,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SpanData<'t> {
    /// Process and tread creation and destruction, as well as
    /// `EVOBENCH_SCOPE`, end by message from destructor
    Scope {
        kind: ScopeKind,
        /// The internally-allocated thread number, 0-based
        thread_number: ThreadNumber,
        start: &'t Timing,
        /// Option just because we allocate the Scope before we get the
        /// closing Timing, as we need it as parent for inner scopes. All
        /// `end` fields should be set by the time `from_logdata`
        /// finishes.
        end: Option<&'t Timing>,
    },
    /// `EVOBENCH_KEY_VALUE`, scoped from issue to the next end of a
    /// `EVOBENCH_SCOPE`
    KeyValue(&'t KeyValue),
}

#[derive(Debug)]
pub struct Span<'t> {
    pub parent: Option<SpanId<'t>>,
    pub children: Vec<SpanId<'t>>,
    pub data: SpanData<'t>,
}

pub struct PathStringOptions {
    pub normal_separator: &'static str,
    pub reverse_separator: &'static str,
    /// Stop when reaching a `ScopeKind::Process`
    pub ignore_process: bool,
    /// Skip showing the process completely (`ignore_process` just
    /// stops before it and shows a placeholder)
    pub skip_process: bool,
    /// Stop when reaching a `ScopeKind::Thread`
    pub ignore_thread: bool,
    /// Add thread number (0..) in path strings
    pub include_thread_number_in_path: bool,
    /// Whether to show the top of the tree left (default) or the leafs left (reversed)
    pub reversed: bool,
    /// A prefix to distinguish this kind of path from others (feel
    /// free to use ""). Only used with `ignore_process` and
    /// `ignore_thread`!
    pub prefix: &'static str,
}

impl<'t> Span<'t> {
    /// Also returns the `ScopeKind`, since you want to verify that at
    /// the same time as mutating the `end` field.
    pub fn end_mut(&mut self) -> Option<(&mut Option<&'t Timing>, ScopeKind)> {
        match &mut self.data {
            SpanData::Scope {
                kind,
                start: _,
                thread_number: _,
                end,
            } => Some((end, *kind)),
            SpanData::KeyValue(_) => None,
        }
    }

    /// Checks that the `pn` on the start and end timings
    /// match. Panics if they don't.
    pub fn assert_consistency(&self) {
        match &self.data {
            SpanData::Scope {
                kind: _,
                start,
                thread_number: _,
                end,
            } => {
                assert_eq!(start.pn, end.unwrap().pn)
            }
            SpanData::KeyValue(_) => todo!(),
        }
    }

    pub fn pn(&self) -> Option<&'t str> {
        match &self.data {
            SpanData::Scope {
                kind: _,
                start,
                thread_number: _,
                end: _,
            } => Some(&start.pn),
            SpanData::KeyValue(_) => None,
        }
    }

    /// Show the path to a node in the tree (towards the right, show
    /// the child node; can also be in reverse (via opts): towards the
    /// right, show the parents up the tree). `out_prefix` receives
    /// the prefix (always meant to be shown on the left), `out_main`
    /// receives the main part of the path (in reversed or normal
    /// form). The outputs are *not* cleared by this method! The idea
    /// is to `out_prefix.push_str(&out_main)` after this call, then
    /// clear both buffers before re-using them.
    pub fn path_string(
        &self,
        opts: &PathStringOptions,
        db: &LogDataTree<'t>,
        out_prefix: &mut String,
        out_main: &mut String,
    ) {
        //
        let PathStringOptions {
            ignore_process,
            skip_process,
            ignore_thread,
            include_thread_number_in_path,
            reversed,
            prefix,
            normal_separator,
            reverse_separator,
        } = opts;
        // Stop recursion via opts?--XX how useful is this even, have
        // display below, too ("P:" etc.).
        match &self.data {
            SpanData::Scope {
                kind,
                thread_number,
                start: _,
                end: _,
            } => match kind {
                ScopeKind::Process => {
                    if *skip_process {
                        return;
                    }
                    if *ignore_process {
                        // Show this as "main thread", not "process",
                        // because Timing currently still contains
                        // `RUSAGE_THREAD` data in this context, too!
                        // And there is no thread start message for
                        // that thread, too, so data would be missing
                        // if not using that as main thread data.
                        out_prefix.push_str(prefix);
                        out_main.push_str("main thread");
                        return;
                    }
                }
                ScopeKind::Thread => {
                    if *ignore_thread {
                        out_prefix.push_str(prefix);
                        if *include_thread_number_in_path {
                            out_main
                                .write_fmt(format_args!("{thread_number}"))
                                .expect("string writes don't fail");
                        } else {
                            out_main.push_str("thread");
                        };
                        return;
                    }
                }
                ScopeKind::Scope => (),
            },
            SpanData::KeyValue(_) => (),
        }

        let push_self = |out_prefix: &mut String, out_main: &mut String| {
            match &self.data {
                SpanData::Scope {
                    kind,
                    thread_number,
                    start,
                    end: _,
                } => {
                    match kind {
                        ScopeKind::Process => {
                            out_prefix.push_str("P:");
                        }
                        ScopeKind::Thread => {
                            // Push to out_prefix ? But, we're not at the
                            // end, so no--XX or what options do we have?
                            out_main.push_str("T:");
                            if *include_thread_number_in_path {
                                out_main.push_str(&thread_number.to_string());
                            }
                        }
                        ScopeKind::Scope => (),
                    }
                    let pn = &start.pn;
                    out_main.push_str(pn);
                }
                SpanData::KeyValue(KeyValue { tid: _, k, v }) => {
                    out_main.push_str(k);
                    out_main.push_str("=");
                    out_main.push_str(v);
                }
            }
        };

        if let Some(parent_id) = self.parent {
            let parent = parent_id.get_from_db(db);
            if *reversed {
                push_self(out_prefix, out_main);
                out_main.push_str(reverse_separator);
                parent.path_string(opts, db, out_prefix, out_main);
            } else {
                parent.path_string(opts, db, out_prefix, out_main);
                out_main.push_str(normal_separator);
                push_self(out_prefix, out_main);
            }
        } else {
            push_self(out_prefix, out_main);
        }
    }

    #[inline]
    pub fn start_and_end(&self) -> Option<(&'t Timing, &'t Timing)> {
        match &self.data {
            SpanData::Scope {
                kind: _,
                thread_number: _,
                start,
                end,
            } => Some((*start, end.expect("properly balanced spans"))),
            SpanData::KeyValue(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadNumber(u32);

impl Deref for ThreadNumber {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for ThreadNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("thread{:02}", self.0))
    }
}

/// Map from thread id to internally-allocated thread_number, both for
/// correctness (in case a tid is re-used), as well as for more
/// consistent output (try to have the same numbers across benchmark
/// runs, although this depends on the order of thread creation
/// (including their initialization messages) remaining the
/// same).
struct ThreadIdMapper {
    current_thread_number: u32,
    // Mappings are removed here when a thread ends! To enforce a
    // new mapping when the same ThreadId shows up again.
    thread_number_by_thread_id: HashMap<ThreadId, ThreadNumber>,
}

impl ThreadIdMapper {
    fn new() -> Self {
        Self {
            current_thread_number: 0,
            thread_number_by_thread_id: HashMap::new(),
        }
    }

    /// Automatically inserts a mapping if there is none yet
    fn to_thread_number(&mut self, thread_id: ThreadId) -> ThreadNumber {
        match self.thread_number_by_thread_id.entry(thread_id) {
            Entry::Occupied(occupied_entry) => *occupied_entry.get(),
            Entry::Vacant(vacant_entry) => {
                let thread_number = ThreadNumber(self.current_thread_number);
                self.current_thread_number += 1;
                vacant_entry.insert(thread_number);
                thread_number
            }
        }
    }

    /// NOTE: mappings are to be removed during parsing when a thread
    /// ends, so that when the same ThreadId is re-used, it gets a new
    /// mapping
    fn remove_thread_id(&mut self, thread_id: ThreadId) -> Option<ThreadNumber> {
        self.thread_number_by_thread_id.remove(&thread_id)
    }
}

impl<'t> LogDataTree<'t> {
    pub fn from_logdata(log_data: &'t LogData) -> Result<Self> {
        let mut slf = Self {
            log_data,
            spans: Default::default(),
            spans_by_pn: Default::default(),
        };

        let mut thread_id_mapper = ThreadIdMapper::new();
        let mut start_by_thread: HashMap<ThreadId, Vec<SpanId<'t>>> = HashMap::new();

        for message in log_data.messages() {
            match message.data_message() {
                DataMessage::KeyValue(kv) => {
                    // Make it a Span
                    let mut span_with_parent = |parent| -> SpanId<'t> {
                        slf.add_span(Span {
                            data: SpanData::KeyValue(kv),
                            parent,
                            children: Default::default(),
                        })
                    };
                    match start_by_thread.entry(kv.tid) {
                        Entry::Occupied(mut e) => {
                            let opt_parent_id: Option<SpanId<'t>> = e.get().last().copied();
                            let span_id = span_with_parent(opt_parent_id);
                            if let Some(parent_id) = opt_parent_id {
                                // Add us, span_id, to the parent's child list.
                                let parent = parent_id.get_mut_from_db(&mut slf);
                                parent.children.push(span_id);
                            }
                            e.get_mut().push(span_id);
                        }
                        Entry::Vacant(_e) => {
                            bail!(
                                "KeyValue must be below some span (but creating a thread counts, too)"
                            )
                        }
                    }
                }
                DataMessage::Timing(kind, timing) => {
                    match PointDispatch::from_point_kind(kind) {
                        // Process / thread / scope start
                        PointDispatch::Scope {
                            kind,
                            is_ending: false,
                        } => {
                            let mut scope_with_parent = |parent| -> SpanId<'t> {
                                let thread_number = thread_id_mapper.to_thread_number(timing.tid);
                                slf.add_span(Span {
                                    data: SpanData::Scope {
                                        kind,
                                        thread_number,
                                        start: timing,
                                        end: None,
                                    },
                                    parent,
                                    children: Default::default(),
                                })
                            };
                            match start_by_thread.entry(timing.tid) {
                                Entry::Occupied(mut e) => {
                                    let parent: Option<SpanId<'t>> = e.get().last().copied();
                                    e.get_mut().push(scope_with_parent(parent));
                                }
                                Entry::Vacant(e) => {
                                    e.insert(vec![scope_with_parent(None)]);
                                }
                            }
                        }

                        // Process / thread / scope end
                        PointDispatch::Scope {
                            kind,
                            is_ending: true,
                        } => match start_by_thread.entry(timing.tid) {
                            Entry::Occupied(mut e) => loop {
                                let span_id = e.get_mut().pop().ok_or_else(|| {
                                    anyhow!("missing messages incl. TS before TE for thread")
                                })?;
                                let span = span_id.get_mut_from_db(&mut slf);

                                if let Some((end, opening_scope_kind)) = span.end_mut() {
                                    if opening_scope_kind != kind {
                                        // XX line location report
                                        bail!(
                                            "expected closing of scope kind \
                                             {opening_scope_kind:?}, \
                                             but got {kind:?} ({span:?} vs. message \
                                             {message:?})"
                                        )
                                    }

                                    *end = Some(timing);
                                    span.assert_consistency();

                                    let pn = span.pn().expect("scopes have a pn");
                                    match slf.spans_by_pn.entry(pn) {
                                        Entry::Occupied(mut e) => {
                                            e.get_mut().push(span_id);
                                        }
                                        Entry::Vacant(e) => {
                                            e.insert(vec![span_id]);
                                        }
                                    }

                                    if kind == ScopeKind::Thread {
                                        thread_id_mapper.remove_thread_id(timing.tid);
                                    }

                                    break;
                                }
                                // else: it was no Scope, go on pop
                                // the next frame in the next loop
                                // iteration.
                            },
                            Entry::Vacant(_e) => {
                                // XX line location report
                                bail!("should never happen as TS comes before TE")
                            }
                        },

                        PointDispatch::T => (),   // XX
                        PointDispatch::TIO => (), // XX
                    }
                }
            }
        }
        Ok(slf)
    }

    pub fn log_data(&self) -> &'t LogData {
        self.log_data
    }

    pub fn probe_names(&self) -> Vec<&'t str> {
        let mut probe_names: Vec<&'t str> = self.spans_by_pn.keys().copied().collect();
        probe_names.sort();
        probe_names
    }

    pub fn spans(&self) -> &[Span<'t>] {
        &self.spans
    }

    pub fn span_ids(&self) -> impl Iterator<Item = SpanId<'t>> {
        (1..=self.spans.len()).map(SpanId::new)
    }

    pub fn spans_by_pn(&self, pn: &str) -> Option<&Vec<SpanId<'t>>> {
        self.spans_by_pn.get(pn)
    }
}
