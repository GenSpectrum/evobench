//! Index spans at a call path from the top (excluding thread id,
//! i.e. across all threads and processes)

use std::{collections::HashMap, time::SystemTime};

use crate::evaluator::data::log_data_tree::{LogDataTree, PathStringOptions, SpanId};

#[derive(Debug, Default)]
pub struct IndexByCallPath<'t> {
    /// Spans at a call path from the top (excluding thread id,
    /// i.e. across all threads and processes)
    spans_by_call_path: HashMap<String, Vec<SpanId<'t>>>,
}

impl<'t> IndexByCallPath<'t> {
    pub fn from_logdataindex(
        log_data_tree: &LogDataTree<'t>,
        path_string_optss: &[PathStringOptions],
    ) -> Self {
        let t0 = SystemTime::now();

        let mut slf = Self::default();
        let mut out_prefix = String::new();
        let mut out_main = String::new();
        for span_id in log_data_tree.span_ids() {
            let span = span_id.get_from_db(log_data_tree);
            for opts in path_string_optss {
                let path = {
                    // Calculate the path efficiently by reusing
                    // buffers
                    out_prefix.clear();
                    out_main.clear();
                    span.path_string(&opts, log_data_tree, &mut out_prefix, &mut out_main);
                    out_prefix.push_str(&out_main);
                    &*out_prefix
                };
                // Add span_id for the path
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
        let t1 = SystemTime::now();
        eprintln!(
            "t IndexByCallPath::from_logdataindex: {} s",
            t1.duration_since(t0).unwrap().as_secs_f64()
        );

        slf
    }

    pub fn call_paths(&self) -> Vec<&String> {
        let mut paths: Vec<&String> = self.spans_by_call_path.keys().collect();
        paths.sort();
        paths
    }

    pub fn spans_by_call_path(&self, call_path: &str) -> Option<&Vec<SpanId<'_>>> {
        self.spans_by_call_path.get(call_path)
    }
}
