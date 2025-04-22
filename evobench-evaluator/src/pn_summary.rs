//! Indexing for making summaries (XX: rename file).

//! `Timing` and contextual info remains in the parsed log file
//! (`Vec<LogMessage>`), the index just references into those.

//! `LogDataIndex::from_logdata` both pairs the start and end timings
//! and builds up a tree of all scopes.

use std::{
    collections::{hash_map::Entry, HashMap},
    marker::PhantomData,
};

use anyhow::Result;

use crate::{
    log_file::LogData,
    log_message::{DataMessage, PointKind, ThreadId, Timing},
};

#[derive(Debug, Default)]
pub struct LogDataIndex<'t> {
    scopes: Vec<Scope<'t>>,
    /// For a probe name, all the `Scope`s in sequence as occurring in
    /// the log file (which isn't necessarily by time when multiple
    /// threads are running), regardless of thread and their `parent`
    /// inside the thread.
    scopes_by_pn: HashMap<&'t str, Vec<ScopeId<'t>>>,
}

macro_rules! def_log_data_index_id {
    { {$TId:tt $($TIdLifetime:tt)*} | $T:tt | $db_field:tt | $add_method:tt } => {
        #[derive(Debug, Clone, Copy)]
        pub struct $TId $($TIdLifetime)* {
            t: PhantomData<fn() -> $T $($TIdLifetime)* >,
            id: u32,
        }

        impl $($TIdLifetime)* $TId $($TIdLifetime)* {
            fn new(index: usize) -> Self {
                let id: u32 = index.try_into().expect("index not outside u32 range");
                Self { id, t: PhantomData::default() }
            }

            // We use len after insertion as the id, so that id 0 is
            // never used, so that Option is cheap.
            fn index(self) -> usize { usize::try_from(self.id).unwrap() - 1 }

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

def_log_data_index_id! {{ScopeId<'t>} | Scope | scopes | add_scope }

#[derive(Debug)]
pub struct Scope<'t> {
    pub parent: Option<ScopeId<'t>>,
    pub start: &'t Timing,
    /// Option just because we allocate the Scope before we get the
    /// closing Timing, as we need it as parent for inner scopes. All
    /// `end` fields should be set by the time `from_logdata`
    /// finishes.
    pub end: Option<&'t Timing>,
}

impl<'t> Scope<'t> {
    pub fn start(&self) -> &'t Timing {
        self.start
    }
    /// Panics if `.end` is not set
    pub fn end(&self) -> &'t Timing {
        self.end.expect("`from_logdata` properly finished")
    }

    pub fn check(&self) {
        assert_eq!(self.start().pn, self.end().pn)
    }

    pub fn pn(&self) -> &'t str {
        &self.start.pn
    }
}

impl<'t> LogDataIndex<'t> {
    pub fn from_logdata(data: &'t LogData) -> Result<Self> {
        let mut slf = Self::default();
        // XXX ThreadId needs local id for safety
        let mut start_by_thread: HashMap<ThreadId, Vec<ScopeId<'t>>> = HashMap::new();

        for message in &data.messages {
            match message.data_message() {
                DataMessage::KeyValue(kv) => {
                    // println!("XX keyvalue {kv:?}");
                }
                DataMessage::Timing(kind, timing) => {
                    match kind {
                        PointKind::TStart => (), // XX
                        PointKind::T => (),      // XX
                        PointKind::TS => match start_by_thread.entry(timing.tid) {
                            Entry::Occupied(mut e) => {
                                let parent: Option<ScopeId<'t>> = e.get().last().copied();
                                let scope_id: ScopeId<'t> = slf.add_scope(Scope {
                                    parent,
                                    start: timing,
                                    end: None,
                                });
                                e.get_mut().push(scope_id);
                            }
                            Entry::Vacant(e) => {
                                let scope_id: ScopeId<'t> = slf.add_scope(Scope {
                                    parent: None,
                                    start: timing,
                                    end: None,
                                });
                                e.insert(vec![scope_id]);
                            }
                        },
                        PointKind::TE => match start_by_thread.entry(timing.tid) {
                            Entry::Occupied(mut e) => {
                                let scope_id = e
                                    .get_mut()
                                    .pop()
                                    .expect("always have TS before TE for the same thread");
                                let scope = scope_id.get_mut_from_db(&mut slf);
                                scope.end = Some(timing);
                                scope.check();

                                let pn = scope.pn();
                                match slf.scopes_by_pn.entry(pn) {
                                    Entry::Occupied(mut e) => {
                                        e.get_mut().push(scope_id);
                                    }
                                    Entry::Vacant(e) => {
                                        e.insert(vec![scope_id]);
                                    }
                                }
                            }
                            Entry::Vacant(_e) => {
                                panic!("should never happen as TS comes before TE")
                            }
                        },
                        PointKind::TThreadStart => (), // XX safety ThreadId
                        PointKind::TThreadEnd => (),   // XX safety ThreadId
                        PointKind::TEnd => (),         // XX
                        PointKind::TIO => (),          // XX
                    }
                }
            }
        }
        Ok(slf)
    }

    pub fn probe_names(&self) -> Vec<&'t str> {
        let mut probe_names: Vec<&'t str> = self.scopes_by_pn.keys().copied().collect();
        probe_names.sort();
        probe_names
    }

    pub fn scopes_by_pn(&self, pn: &str) -> Option<&Vec<ScopeId<'t>>> {
        self.scopes_by_pn.get(pn)
    }
}
