use std::{
    collections::{hash_map::Entry, HashMap},
};

use anyhow::Result;

use crate::{
    log_file::LogData,
    log_message::{DataMessage, PointKind, ThreadId, Timing},
    scope::Scope,
};

#[derive(Debug)]
pub struct ByScope<'t> {
    /// For a probe name of a scoped probe, all the Scope { start, end
    /// } values in sequence.
    pub by_pn: HashMap<&'t str, Vec<Scope<'t>>>,
}

impl<'t> ByScope<'t> {
    pub fn from_logdata(data: &'t LogData) -> Result<Self> {
        // XXX ThreadId needs safety local id
        let mut start_by_thread: HashMap<ThreadId, Vec<&Timing>> = HashMap::new();
        let mut by_pn: HashMap<&str, Vec<Scope>> = HashMap::new();

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
                                e.get_mut().push(timing);
                            }
                            Entry::Vacant(e) => {
                                e.insert(vec![timing]);
                            }
                        },
                        PointKind::TE => match start_by_thread.entry(timing.tid) {
                            Entry::Occupied(mut e) => {
                                let start = e
                                    .get_mut()
                                    .pop()
                                    .expect("always have TS before TE for the same thread");
                                let scope = Scope::new(start, timing)?;
                                match by_pn.entry(scope.pn()) {
                                    Entry::Occupied(mut e) => {
                                        e.get_mut().push(scope);
                                    }
                                    Entry::Vacant(e) => {
                                        e.insert(vec![scope]);
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
        Ok(Self { by_pn })
    }

    pub fn probe_names(&self) -> Vec<&'t str> {
        let mut probe_names: Vec<&'t str> = self.by_pn.keys().copied().collect();
        probe_names.sort();
        probe_names
    }

    pub fn scopes_by_pn(&self, pn: &str) -> Option<&Vec<Scope>> {
        self.by_pn.get(pn)
    }
}
