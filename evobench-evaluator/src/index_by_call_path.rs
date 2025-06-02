//! Index spans at a call path from the top (excluding thread id,
//! i.e. across all threads and processes)

use std::collections::HashMap;

use itertools::Itertools;

use crate::log_data_tree::{LogDataTree, PathStringOptions, SpanId};

#[derive(Debug, Default)]
pub struct IndexByCallPath<'t> {
    /// Spans at a call path from the top (excluding thread id,
    /// i.e. across all threads and processes)
    spans_by_call_path: HashMap<String, Vec<SpanId<'t>>>,
}

impl<'t> IndexByCallPath<'t> {
    pub fn from_logdataindex(
        db: &LogDataTree<'t>,
        path_string_optss: &[PathStringOptions],
    ) -> Self {
        let mut slf = Self::default();
        let mut out_prefix = String::new();
        let mut out_main = String::new();
        for span_id in db.span_ids() {
            let span = span_id.get_from_db(db);
            for opts in path_string_optss {
                out_prefix.clear();
                out_main.clear();
                span.path_string(&opts, db, &mut out_prefix, &mut out_main);
                out_prefix.push_str(&out_main);
                let path = &*out_prefix;
                match slf.spans_by_call_path.get_mut(path) {
                    Some(vec) => {
                        vec.push(span_id);
                    }
                    None => {
                        slf.spans_by_call_path
                            .insert(path.to_owned(), vec![span_id]);
                    }
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
