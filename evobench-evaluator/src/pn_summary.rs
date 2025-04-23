//! Indexing for making summaries (XX: rename file).

//! `Timing` and contextual info remains in the parsed log file
//! (`Vec<LogMessage>`), the index just references into those.

//! `LogDataIndex::from_logdata` both pairs the start and end timings
//! and builds up a tree of all spans.

use std::{
    collections::{hash_map::Entry, HashMap},
    marker::PhantomData,
    num::NonZeroU32,
};

use anyhow::{bail, Result};

use crate::{
    log_file::LogData,
    log_message::{DataMessage, KeyValue, PointKind, ThreadId, Timing},
};

#[derive(Debug, Default)]
pub struct LogDataIndex<'t> {
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

            pub fn get_from_db<'d>(self, db: &'d LogDataIndex<'t>) -> &'d $T$($TIdLifetime)*
                // XX are these even required or helpful?:
                where 't: 'd
            {
                &db.$db_field[self.index()]
            }

            pub fn get_mut_from_db<'d>(self, db: &'d mut LogDataIndex<'t>) -> &'d mut $T$($TIdLifetime)*
                where 't: 'd
            {
                &mut db.$db_field[self.index()]
            }
        }

        impl<'t> LogDataIndex<'t> {
            pub fn $add_method(&mut self, value: $T $($TIdLifetime)*) -> $TId $($TIdLifetime)* {
                self.$db_field.push(value);
                $TId::new(self.$db_field.len())
            }
        }
    }
}

def_log_data_index_id! {{SpanId<'t>} | Span | spans | add_span }

#[derive(Debug, Clone, Copy)]
pub enum ScopeKind {
    Process,
    Thread,
    Scope,
}

#[derive(Debug, Clone, Copy)]
pub enum SpanKind<'t> {
    /// Process and tread creation and destruction, as well as
    /// `EVOBENCH_SCOPE`, end by message from destructor
    Scope {
        kind: ScopeKind,
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
    pub kind: SpanKind<'t>,
}

impl<'t> Span<'t> {
    pub fn end_mut(&mut self) -> Option<&mut Option<&'t Timing>> {
        match &mut self.kind {
            SpanKind::Scope {
                kind: _,
                start: _,
                end,
            } => Some(end),
            SpanKind::KeyValue(_) => None,
        }
    }

    pub fn check(&self) {
        match &self.kind {
            SpanKind::Scope {
                kind: _,
                start,
                end,
            } => {
                assert_eq!(start.pn, end.unwrap().pn)
            }
            SpanKind::KeyValue(_) => todo!(),
        }
    }

    pub fn pn(&self) -> Option<&'t str> {
        match &self.kind {
            SpanKind::Scope {
                kind: _,
                start,
                end: _,
            } => Some(&start.pn),
            SpanKind::KeyValue(_) => None,
        }
    }

    pub fn path_string(&self, db: &LogDataIndex<'t>) -> String {
        let mut out = if let Some(parent_id) = self.parent {
            let parent = parent_id.get_from_db(db);
            let mut out = parent.path_string(db);
            out.push_str(" > ");
            out
        } else {
            String::new()
        };
        match &self.kind {
            SpanKind::Scope {
                kind,
                start,
                end: _,
            } => {
                match kind {
                    ScopeKind::Process => out.push_str("P:"),
                    ScopeKind::Thread => out.push_str("T:"),
                    ScopeKind::Scope => (),
                }
                let pn = &start.pn;
                out.push_str(pn);
            }
            SpanKind::KeyValue(KeyValue { tid: _, k, v }) => {
                out.push_str(k);
                out.push_str("=");
                out.push_str(v);
            }
        }
        out
    }

    pub fn start_and_end(&self) -> Option<(&'t Timing, &'t Timing)> {
        match &self.kind {
            SpanKind::Scope {
                kind: _,
                start,
                end,
            } => Some((*start, end.expect("properly balanced spans"))),
            SpanKind::KeyValue(_) => None,
        }
    }
}

impl<'t> LogDataIndex<'t> {
    pub fn from_logdata(data: &'t LogData) -> Result<Self> {
        let mut slf = Self::default();
        // XXX ThreadId needs local id for safety
        let mut start_by_thread: HashMap<ThreadId, Vec<SpanId<'t>>> = HashMap::new();

        for message in &data.messages {
            match message.data_message() {
                DataMessage::KeyValue(kv) => {
                    // Make it a Span
                    let mut span_with_parent = |parent| -> SpanId<'t> {
                        slf.add_span(Span {
                            kind: SpanKind::KeyValue(kv),
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
                            bail!("KeyValue must be below some span (but creating a thread counts, too)")
                        }
                    }
                }
                DataMessage::Timing(kind, timing) => {
                    match kind {
                        // Scope start
                        PointKind::TS => {
                            let mut scope_with_parent = |parent| -> SpanId<'t> {
                                slf.add_span(Span {
                                    kind: SpanKind::Scope {
                                        kind: ScopeKind::Scope,
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

                        // Scope end
                        PointKind::TE => match start_by_thread.entry(timing.tid) {
                            Entry::Occupied(mut e) => loop {
                                let span_id = e
                                    .get_mut()
                                    .pop()
                                    .expect("always have TS before TE for the same thread");
                                let span = span_id.get_mut_from_db(&mut slf);
                                if let Some(end) = span.end_mut() {
                                    *end = Some(timing);
                                    span.check();

                                    let pn = span.pn().expect("scopes have a pn");
                                    match slf.spans_by_pn.entry(pn) {
                                        Entry::Occupied(mut e) => {
                                            e.get_mut().push(span_id);
                                        }
                                        Entry::Vacant(e) => {
                                            e.insert(vec![span_id]);
                                        }
                                    }

                                    break;
                                }
                            },
                            Entry::Vacant(_e) => {
                                panic!("should never happen as TS comes before TE")
                            }
                        },

                        PointKind::T => (), // XX

                        PointKind::TThreadStart => (), // XX safety ThreadId
                        PointKind::TThreadEnd => (),   // XX safety ThreadId
                        PointKind::TStart => (),       // XX
                        PointKind::TEnd => (),         // XX
                        PointKind::TIO => (),          // XX
                    }
                }
            }
        }
        Ok(slf)
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
