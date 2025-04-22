//! Index spans at a call path from the top (excluding thread id,
//! i.e. across all threads and processes)

use std::collections::{hash_map::Entry, HashMap};

use itertools::Itertools;

use crate::pn_summary::{LogDataIndex, SpanId};

#[derive(Debug, Default)]
pub struct IndexByCallPath<'t> {
    /// Spans at a call path from the top (excluding thread id,
    /// i.e. across all threads and processes)
    spans_by_call_path: HashMap<String, Vec<SpanId<'t>>>,
}

impl<'t> IndexByCallPath<'t> {
    pub fn from_logdataindex(db: &LogDataIndex<'t>) -> Self {
        let mut slf = Self::default();
        for span_id in db.span_ids() {
            let span = span_id.get_from_db(db);
            let path = span.path_string(db);
            match slf.spans_by_call_path.entry(path) {
                Entry::Occupied(mut e) => {
                    e.get_mut().push(span_id);
                }
                Entry::Vacant(e) => {
                    e.insert(vec![span_id]);
                }
            }
        }
        slf
    }

    pub fn call_paths(&self) -> impl Iterator<Item = &String> {
        self.spans_by_call_path.keys().sorted()
    }

    pub fn spans_by_call_path(&self, call_path: &str) -> Option<&Vec<SpanId>> {
        self.spans_by_call_path.get(call_path)
    }
}
