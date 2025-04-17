//! Temporary overview over the dimensions we have. Mapping to polars
//! dataframes?

// use polars::prelude::*;

// What dimensions do we have? Or *selectors*. Choosing one value for
// each dimension yields a measurement value.
struct Dimension {
    name: &'static str,
    /// Key in JSON
    data_key: Option<&'static str>,
    type_name: &'static str,
    length: &'static str,
}

const DIMENSIONS: &[Dimension] = &[
    Dimension {
        name: "probe name",
        data_key: Some("pn"),
        type_name: "&str",
        length: "number of probes existing in code",
    },
    Dimension {
        name: "ThreadTiming field name",
        data_key: None,
        type_name: "&str",
        length: "number of fields on ThreadTiming",
    },
    Dimension {
        name: "thread id",
        data_key: None,
        type_name: "(usize, usize) or similar for (pid, tid)",
        length: "number of (processes, threads) started by the test run",
    },
];

// Then we can also have derived ones, from interdependencies?:

// - DAG of probes: parent vs. child relationship
// rather:
// - DAG of `KeyValue | probe` and other `KeyValue | probe`s

// - 1-level tree of process -> thread relationships
