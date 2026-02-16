//! Index spans at a call path from the top (excluding thread id,
//! i.e. across all threads and processes)

use std::{
    collections::{HashMap, hash_map},
    time::SystemTime,
};

use chj_rustbin::chunks::ChunksOp;
use rayon::iter::{ParallelBridge, ParallelIterator};

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

        let make_locals = || -> (String, String, HashMap<String, Vec<SpanId<'t>>>) {
            (Default::default(), Default::default(), Default::default())
        };
        let spans_by_call_path = ChunksOp::chunks(log_data_tree.span_ids(), 1000)
            .par_bridge()
            .map(|span_ids| {
                let (mut out_prefix, mut out_main, mut spans_by_call_path) = make_locals();
                for span_id in span_ids {
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
                        match spans_by_call_path.get_mut(path) {
                            Some(vec) => {
                                vec.push(span_id);
                            }
                            None => {
                                spans_by_call_path.insert(path.to_owned(), vec![span_id]);
                            }
                        }
                    }
                }
                spans_by_call_path
            })
            .reduce(
                || HashMap::new(),
                |mut spans_by_call_path, hm| {
                    for (k, mut v) in hm {
                        match spans_by_call_path.entry(k) {
                            hash_map::Entry::Occupied(occupied_entry) => {
                                occupied_entry.into_mut().append(&mut v);
                            }
                            hash_map::Entry::Vacant(vacant_entry) => {
                                vacant_entry.insert(v);
                            }
                        }
                    }
                    spans_by_call_path
                },
            );

        let t1 = SystemTime::now();
        eprintln!(
            "t IndexByCallPath::from_logdataindex: {} s",
            t1.duration_since(t0).unwrap().as_secs_f64()
        );

        Self { spans_by_call_path }
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
