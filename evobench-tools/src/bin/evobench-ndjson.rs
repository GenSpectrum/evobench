use anyhow::{Context, Result, anyhow, bail};
use cj_path_util::{path_util::AppendToPath, unix::polyfill::add_extension};
use clap::Parser;
use evobench_tools::{
    config_file::backend_from_path,
    conslist::{List, cons},
    ctx,
    get_terminal_width::get_terminal_width,
    git::GitHash,
    io_utils::div::create_dir_if_not_exists,
    key::RunParameters,
    run::{
        benchmarking_job::{BenchmarkingJob, BenchmarkingJobPublic, BenchmarkingJobState},
        config::BenchmarkingCommand,
        custom_parameter::{CustomParameterType, CustomParameterValue},
        env_vars::AllowableCustomEnvVar,
    },
    serde::{allowed_env_var::AllowedEnvVar, priority::Priority},
    serde_util::CanonicalJson,
    silo::query::Query,
    util::integers::rounding_integer_division,
    utillib::{
        arc::CloneArc,
        logging::{LogLevelOpt, set_log_level},
    },
    warn,
};
use itertools::Itertools;
use kstring::KString;
use lazy_static::lazy_static;
use linfa::traits::Transformer;
use linfa_clustering::Dbscan;
use ndarray::{Array2, ArrayBase, Dim, ViewRepr};
use noisy_float::types::R64;
use num_traits::Float;
use regex::Regex;
use serde_json::{Number, Value};

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fmt::Display,
    fs::File,
    io::{BufWriter, Write},
    marker::PhantomData,
    ops::Range,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

type Weight = R64;

// The action type should be more important than other attributes, by
// this factor
const ACTION_TYPE_WEIGHT: f64 = 5.;

#[derive(Debug)]
struct KeyWeightsAndRanges<'t>(Vec<(Weight, Range<Weight>)>, PhantomData<&'t ()>);

/// A reference for a key path in `Queries`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyId(usize);

impl<'t> KeyWeightsAndRanges<'t> {
    /// Panics for invalid `KeyId`s
    fn key_weight_and_range(&self, key_id: KeyId) -> &(Weight, Range<Weight>) {
        &self.0[key_id.0]
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Debug)]
struct QueryWeights<'t> {
    #[allow(unused)]
    id: QueryId,
    weights: Vec<Weight>,
    _phantom: PhantomData<&'t ()>,
}

impl<'t> QueryWeights<'t> {
    fn copy_to_array<F: Float + Display>(
        &self,
        mut array: ArrayBase<ViewRepr<&mut F>, Dim<[usize; 1]>>,
    ) {
        for (i, w) in self.weights.iter().enumerate() {
            let w = F::from(*w).expect("Weight type is convertible to Float types");
            // / F::from(1.).expect("Weight::MAX is convertible to Float types");
            if w.is_nan() {
                panic!("why? w = {w}");
            }
            array[i] = w;
        }
    }
}

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
/// Schedule and query benchmarking jobs.
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpt,

    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(Debug, clap::Args)]
struct ParseFile {
    /// The ndjson file to parse
    path: PathBuf,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// Bring a queries file into canonical representation.
    Canonicalize {
        /// Whether to optimize filter expressions
        #[clap(long)]
        optimize: bool,

        /// Path to ndjson file with queries
        input: PathBuf,

        /// Path to where the same queries should be written, in the
        /// same order, but canonicalized (whitespace standardized,
        /// key order sorted alphabetically)
        output: PathBuf,
    },

    /// Build queries clusters out of a queries file (includes
    /// bringing queries into their canonical representation).
    Vectorize {
        /// Show values (with query id where it appears) with each key
        #[clap(short, long)]
        show_values: bool,

        /// Whether to optimize filter expressions
        #[clap(long)]
        optimize: bool,

        #[clap(flatten)]
        parse_file: ParseFile,

        /// Whether to write query (and job template) files
        #[clap(subcommand)]
        queries_output: Option<QueriesOutput>,
    },

    /// Copy contents of multiple queries dirs into a single new one
    /// (does *not* run query canonicalization); creates a new
    /// template file from scratch.
    MergeClustersTo {
        /// Path to new dir (no extension is added here!)
        new_queries_dir: PathBuf,

        /// Paths to existing queries dirs
        queries_dirs: Vec<PathBuf>,
        // Not adding `reason` override here; just rely on the
        // default, OK?
    },
}

#[derive(clap::Subcommand, Debug)]
enum QueriesOutput {
    ToFiles {
        /// Write queries files with the given path, with `.$n.ndjson`
        /// appended, where `$n` is the number of the cluster. There
        /// is also one file with `.unclustered.ndjson` appended, that
        /// contains queries that did not cluster with any other
        /// query.
        file_base_path: PathBuf,
    },
    ToFolders {
        /// The reason put into the benchmarking job template. By
        /// default, takes the name of the folder in which the
        /// template is created (i.e. of `folder_base_path` after
        /// appending the cluster id).
        #[clap(long)]
        reason: Option<String>,

        /// Write queries files one to newly created folders each,
        /// with `.$n` appended to the path, where `$n` is the number
        /// of the cluster, as `queries.ndjson` files. Also creates a
        /// symlink `ignore_queries_for_checksum_regex.txt ->
        /// ../queries/ignore_queries_for_checksum_regex.txt`, and a
        /// file `job-template.json5` that can be used with
        /// `evobench-jobs insert-file` to run a job with that queries
        /// file.  There is also one dir with `.unclustered` appended,
        /// that contains queries that did not cluster with any other
        /// query (only if that set is not empty).
        folder_base_path: PathBuf,
    },
}

fn write_queries_file<'a>(
    queries_file_path: &Path,
    queries: &mut dyn Iterator<Item = &'a Arc<str>>,
) -> Result<()> {
    (|| -> Result<()> {
        let mut out = BufWriter::new(File::create(queries_file_path)?);
        for query in queries {
            out.write_all(query.trim_end().as_bytes())?;
            out.write_all(b"\n")?;
        }
        out.flush()?;
        Ok(())
    })()
    .with_context(|| anyhow!("writing query file {queries_file_path:?}"))
}

// *Without* the .ndjson extension, that is added by the backend itself
fn extension_for_cluster_id(cluster_id: Option<ClusterId>) -> String {
    if let Some(cluster_id) = cluster_id {
        format!("{cluster_id:03}")
    } else {
        format!("unclustered")
    }
}

impl QueriesOutput {
    // `extension` is the part *without* the .ndjson extension (that
    // is added by the backend itself if targetting a file); None
    // means nothing is added to the "base" paths in `self`.
    // `queries` is/can be strings without the end of line character
    // (any whitespace at the end is stripped before re-adding a
    // newline).
    fn write_queries<'a>(
        &self,
        extension: Option<String>,
        queries: &mut dyn ExactSizeIterator<Item = &'a Arc<str>>,
    ) -> Result<()> {
        // Get this before mutating `queries`!
        let queries_len = queries.len();

        match self {
            QueriesOutput::ToFiles { file_base_path } => {
                let queries_file_path = if let Some(extension) = extension {
                    let extension = format!("{extension}.ndjson");

                    &add_extension(file_base_path, extension).ok_or_else(|| {
                        anyhow!(
                            "to-files requires a path to which a file \
                             extension can be added"
                        )
                    })?
                } else {
                    file_base_path
                };
                write_queries_file(queries_file_path, queries)
            }
            QueriesOutput::ToFolders {
                folder_base_path,
                reason,
            } => {
                let folder_path = if let Some(extension) = extension {
                    &add_extension(folder_base_path, extension).ok_or_else(|| {
                        anyhow!(
                            "to-folders requires a path to which a file \
                         extension can be added"
                        )
                    })?
                } else {
                    folder_base_path
                };

                create_dir_if_not_exists(folder_path, "folder for queries file")?;

                // File with queries
                {
                    let queries_file_path = folder_path.append("queries.ndjson");
                    write_queries_file(&queries_file_path, queries)?;
                }

                // Symlink
                {
                    let symlink_path = folder_path.append("ignore_queries_for_checksum_regex.txt");
                    match symlink(
                        "../queries/ignore_queries_for_checksum_regex.txt",
                        &symlink_path,
                    ) {
                        Ok(()) => (),
                        Err(e) => match e.kind() {
                            std::io::ErrorKind::AlreadyExists => (),
                            _ => Err(e).map_err(ctx!("creating symlink at {symlink_path:?}"))?,
                        },
                    }
                }

                // Job template file
                {
                    let template_path = folder_path.append("job-template.json5");
                    let reason: &str = if let Some(reason) = reason {
                        reason
                    } else {
                        let folder_base_name = (&folder_base_path)
                        .file_name()
                            .ok_or_else(||anyhow!(
                                "expect a to-folder base path from which the last element can be taken"))?
                        .to_string_lossy();

                        &format!("t_{folder_base_name}")
                    };

                    let repeat = {
                        // XX hack hard-coded; can't just take the
                        // total over all clusters, because the ndjson
                        // input file may already be a subset of the
                        // original count!
                        // wc -l silo-benchmark-datasets/SC2open/v0.9.0/queries/queries.ndjson
                        let original_len = 33126;

                        if queries_len > original_len {
                            bail!(
                                "queries_len {queries_len} > hard-coded original_len {original_len}"
                            );
                        }

                        rounding_integer_division(original_len, queries_len)
                    };

                    let custom_parameters: Vec<(
                        AllowedEnvVar<AllowableCustomEnvVar>,
                        CustomParameterValue,
                    )> = {
                        let folder_name = folder_path
                            .file_name()
                            .expect("can get back file name of path to which a suffix was added")
                            .to_string_lossy();

                        let var = |k: &str, t: CustomParameterType, v: &str| -> Result<_> {
                            Ok((
                                k.parse()?,
                                CustomParameterValue::checked_from(t, &KString::from_ref(v))?,
                            ))
                        };

                        use CustomParameterType::*;
                        // XX hack hard-coded
                        vec![
                            var("CONCURRENCY", NonZeroU32, "120")?,
                            var("DATASET", Dirname, "SC2open")?,
                            // Heh, still using the Filename type here!
                            // But then, now using a suffix, thus actually
                            // appropriate?
                            var("QUERIES", Filename, &folder_name)?,
                            var("RANDOMIZED", Bool, "1")?,
                            var("REPEAT", NonZeroU32, &repeat.to_string())?,
                            var("SORTED", Bool, "0")?,
                        ]
                    };
                    let custom_parameters: BTreeMap<
                        AllowedEnvVar<AllowableCustomEnvVar>,
                        CustomParameterValue,
                    > = BTreeMap::from_iter(custom_parameters.into_iter());

                    let template = BenchmarkingJob::new(
                        BenchmarkingJobPublic {
                            reason: Some((*reason).to_owned()),
                            run_parameters: Arc::new(RunParameters {
                                commit_id: GitHash::from_str(
                                    "a71209b88a91d6ac3fcdb5b9c41062d06a170376",
                                )
                                .expect("hash"),
                                custom_parameters: Arc::new(custom_parameters.into()),
                            }),
                            command: Arc::new(BenchmarkingCommand {
                                target_name: "api".parse().expect("ok"),
                                subdir: "benchmarking".into(),
                                command: "make".into(),
                                arguments: vec!["api".into()],
                            }),
                        },
                        BenchmarkingJobState {
                            remaining_count: 8,
                            remaining_error_budget: 2,
                            last_working_directory: None,
                        },
                        Priority::LOW,
                        Priority::NORMAL,
                    );
                    let backend = backend_from_path(&template_path)?;
                    backend.save_config_file(&template_path, &template)?;
                }

                Ok(())
            }
        }
    }
}

/// Original value from the query json; will be converted to use the
/// u32 range once all values are known and min and max can be
/// derived.
#[derive(Debug)]
enum LeafValue {
    // `Null` would only be useful if the key does not contain the
    // type declaration `:Null`, but if it does, we just drop the
    // value, thus None is used instead.
    Bool(bool),
    Number(Number),
    /// When the value does not appear relevant, e.g. for position
    None,
}

#[derive(Debug)]
struct Queries {
    query_strings: Vec<Arc<str>>,
    // id of the above entries
    query_strings_index: BTreeMap<Arc<str>, usize>,

    // Which queries have that vector; the key string here is the
    // vector string (the json path alike), not the query string. The
    // `Weight` is a multiplier, can give higher or lower weight for
    // this dimension.
    vectors: BTreeMap<String, (Weight, BTreeMap<QueryId, LeafValue>)>,
}

#[derive(Debug, PartialEq, Eq)]
enum UplistEntry<'t> {
    /// in the middle of paths, to signal the presence of a map key
    /// .type="foo"
    MapType(String),
    MapKey(&'t str),
    Array,
    String(&'t str),
    // Null,
    // Bool(bool),
    // Number(Number),
    LeafType(&'static str),
}

fn _uplist_to_string(uplist: &List<UplistEntry>, out: &mut String) {
    match uplist {
        List::Pair(val, list) => {
            _uplist_to_string(list, out);
            match val {
                UplistEntry::MapKey(s) => {
                    out.push_str(".");
                    out.push_str(s);
                }
                UplistEntry::String(s) => {
                    use std::fmt::Write;
                    let _ = write!(out, "={s:?}");
                }
                UplistEntry::Array => {
                    out.push_str("[*]");
                }
                // UplistEntry::Null => {
                //     out.push_str("=");
                //     out.push_str("null");
                // }
                // UplistEntry::Bool(b) => {
                //     out.push_str("=");
                //     out.push_str(if *b { "true" } else { "false" });
                // }
                // UplistEntry::Number(number) => {
                //     use std::fmt::Write;
                //     let _ = write!(out, "={number}");
                // }
                UplistEntry::LeafType(s) => {
                    out.push(':');
                    out.push_str(s);
                }
                UplistEntry::MapType(s) => {
                    out.push_str("{type:");
                    // missing escaping, like in some other places, insecure
                    out.push_str(s);
                    out.push_str("}");
                }
            }
        }
        List::Null => (),
    }
}

fn uplist_to_string(uplist: &List<UplistEntry>) -> String {
    let mut out = String::new();
    _uplist_to_string(uplist, &mut out);
    out
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct ClusterId(usize);

impl Display for ClusterId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (self.0 + 1).fmt(f)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct QueryId(usize);

impl Display for QueryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (self.0 + 1).fmt(f)
    }
}

impl Queries {
    fn new() -> Queries {
        Queries {
            query_strings: Default::default(),
            query_strings_index: Default::default(),
            vectors: Default::default(),
        }
    }

    /// panics for invalid ids
    fn query_by_id(&self, id: QueryId) -> &Arc<str> {
        &self.query_strings[id.0]
    }

    /// Returns Null if query is already in it
    fn insert_query(&mut self, query: Arc<str>) -> Option<QueryId> {
        match self.query_strings_index.entry(query.clone_arc()) {
            Entry::Occupied(_occupied_entry) => None,
            Entry::Vacant(vacant_entry) => {
                let i = self.query_strings.len();
                vacant_entry.insert(i);
                self.query_strings.push(query);
                Some(QueryId(i))
            }
        }
    }

    /// `weight` is only used when this is the first entry of `key`
    fn add_vector_entry(&mut self, id: QueryId, key: String, weight: Weight, val: LeafValue) {
        match self.vectors.entry(key) {
            Entry::Occupied(mut query_ids) => {
                let r = query_ids.get_mut();
                assert_eq!(r.0, weight);
                r.1.insert(id, val);
            }
            Entry::Vacant(vacant_entry) => {
                let mut hs = BTreeMap::new();
                hs.insert(id, val);
                vacant_entry.insert((weight, hs));
            }
        }
    }

    /// How much the distance at each key should contribute
    fn key_weights_and_ranges(&self) -> KeyWeightsAndRanges<'_> {
        KeyWeightsAndRanges(
            self.vectors
                .iter()
                .map(|(_k, vals)| -> (Weight, Range<Weight>) {
                    // ^ originally wanted to use k.len() for
                    // weighting; not anymore, getting it in f64^ now.
                    let (min, max) = vals
                        .1
                        .values()
                        .filter_map(|leafvalue| match leafvalue {
                            LeafValue::Bool(_) => None,
                            LeafValue::Number(number) => {
                                if let Some(x) = number.as_f64() {
                                    Some(x)
                                } else {
                                    warn!(
                                        "got a Number value that cannot be represented \
                                         as f64: {number}"
                                    );
                                    None
                                }
                            }
                            LeafValue::None => None,
                        })
                        .minmax()
                        .into_option()
                        // .expect("always have some values if we have the
                        // key"); wrong, it happens, of course, if we
                        // only have None items. XX What to use?
                        .unwrap_or_else(|| (0., 1.));
                    let range = Weight::from_f64(min)..Weight::from_f64(max);
                    (vals.0, range)
                })
                .collect(),
            PhantomData,
        )
    }

    fn query_ids(&self) -> impl Iterator<Item = QueryId> {
        (0..self.query_strings.len()).map(|i| QueryId(i))
    }

    fn query_ids_count(&self) -> usize {
        self.query_strings.len()
    }

    fn vector_values(
        &self,
    ) -> impl Iterator<Item = (KeyId, &(Weight, BTreeMap<QueryId, LeafValue>))> {
        self.vectors
            .values()
            .enumerate()
            .map(|(i, v)| (KeyId(i), v))
    }

    fn weights_for_query(
        &self,
        id: QueryId,
        key_weights_and_ranges: &KeyWeightsAndRanges,
    ) -> QueryWeights<'_> {
        QueryWeights {
            id,
            weights: self
                .vector_values()
                .map(|(key_id, v)| -> Weight {
                    let (key_weight, range) = key_weights_and_ranges.key_weight_and_range(key_id);
                    let x = if let Some(val) = v.1.get(&id) {
                        match val {
                            LeafValue::Bool(b) => {
                                if *b {
                                    Weight::from_f64(1.)
                                } else {
                                    Weight::from_f64(0.)
                                }
                            }
                            LeafValue::Number(n) => {
                                let range_len = range.end - range.start;
                                if range_len == 0. {
                                    // Only should get here if there
                                    // is only 1 distinct value. ('No
                                    // values' should get us to 0.0
                                    // .. 1.0 instead.)  -- XX ? still not the 20.0 place.
                                    Weight::from_f64(1.)
                                } else {
                                    let x = n.as_f64().expect(
                                        "really need the number to be representable as f64, XX",
                                    );
                                    let x =
                                        Weight::try_from(x).expect("user does not feed NaN or inf");
                                    // ^ XX sigh. user-exposed panic!
                                    let d = x - range.start;
                                    let d01 = d / range_len;
                                    d01
                                }
                            }
                            // presence should be the median 'normal
                            // value', vs. MIN for absence, OK?
                            LeafValue::None => Weight::from_f64(0.5),
                        }
                    } else {
                        Weight::from_f64(0.)
                    };
                    x * *key_weight
                })
                .collect(),
            _phantom: PhantomData,
        }
    }

    fn _add_vectors(&mut self, id: QueryId, value: &Value, uplist: &List<UplistEntry>) {
        let (uplist, leaf_value) = match value {
            // Leaf positions
            Value::Null => {
                // ignore entry: null is for an Option::None, and we
                // don't know the type, it's best to just omit the
                // path. That will yield a default value (0.)
                // for that field later on.
                return;
            }
            Value::Bool(b) => (
                &cons(UplistEntry::LeafType("Bool"), uplist),
                LeafValue::Bool(*b),
            ),
            Value::Number(x) => {
                let prev = uplist.first();
                let val = if prev == Some(&UplistEntry::MapKey("position")) {
                    LeafValue::None
                } else if prev == Some(&UplistEntry::MapKey("numberOfMatchers")) {
                    LeafValue::None
                } else {
                    LeafValue::Number(x.clone())
                };
                (&cons(UplistEntry::LeafType("Number"), uplist), val)
            }
            Value::String(s) => {
                lazy_static! {
                    static ref DATE_DAY: Regex = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
                    static ref SINGLE_WORD: Regex = Regex::new(r"^[a-zA-Z]+$").unwrap();
                    static ref SPACED_WORDS: Regex =
                        Regex::new(r"^[a-zA-Z]+(?: [a-zA-Z]+)+$").unwrap();
                    static ref LINEAGE: Regex = Regex::new(r"^[a-zA-Z]+(?:\.\d+)+$").unwrap();
                    static ref WORDNUM: Regex = Regex::new(r"^[a-zA-Z]+\d+$").unwrap();
                    static ref NUMWORD: Regex = Regex::new(r"^\d+[a-zA-Z]+$").unwrap();
                    static ref WORDNUMWORD: Regex = Regex::new(r"^[a-zA-Z]+\d+[a-zA-Z]+$").unwrap();
                }

                let prev = uplist.first();
                let entry = if prev == Some(&UplistEntry::MapKey("symbol")) {
                    UplistEntry::LeafType("String")
                } else if prev == Some(&UplistEntry::MapKey("from"))
                    || prev == Some(&UplistEntry::MapKey("to"))
                {
                    if DATE_DAY.is_match(s) {
                        UplistEntry::LeafType("DateDayString")
                    } else {
                        UplistEntry::String(s)
                    }
                } else if prev == Some(&UplistEntry::MapKey("value"))
                    || prev == Some(&UplistEntry::MapKey("sequenceName"))
                {
                    if SINGLE_WORD.is_match(s) {
                        UplistEntry::LeafType("SingleWordString")
                    } else if WORDNUM.is_match(s) {
                        UplistEntry::LeafType("WordnumString")
                    } else if NUMWORD.is_match(s) {
                        UplistEntry::LeafType("NumwordString")
                    } else if WORDNUMWORD.is_match(s) {
                        UplistEntry::LeafType("WordnumwordString")
                    } else if SPACED_WORDS.is_match(s) {
                        UplistEntry::LeafType("SpacedWordsString")
                    } else if LINEAGE.is_match(s) {
                        UplistEntry::LeafType("LineageString")
                    } else if s.contains('*') {
                        UplistEntry::LeafType("StringContainingStar")
                    } else if s.contains('∗') {
                        UplistEntry::LeafType("StringContainingSpecialStar")
                    } else {
                        dbg!(s);
                        UplistEntry::String(s)
                    }
                } else {
                    UplistEntry::String(s)
                };
                (&cons(entry, uplist), LeafValue::None)
            }

            // Non-leaf positions
            Value::Array(values) => {
                let uplist = cons(UplistEntry::Array, uplist);
                for value in values {
                    self._add_vectors(id, value, &uplist);
                }
                return;
            }
            Value::Object(map) => {
                let (uplist, ignore_type, weight) = if let Some(val) = map.get("type") {
                    match val {
                        Value::String(s) => {
                            let weight = if uplist.any(|entry| match entry {
                                // We don't have `"type": "Action"`;
                                // we have `"action":{... "type": x}`;
                                // i.e. x might be a *subtype* of
                                // Action; but if we are under an
                                // "action" field key, then it *must*
                                // be an Action subtype. Saves us
                                // checking for all subtypes.
                                UplistEntry::MapKey(n) => *n == "action",
                                _ => false,
                            }) {
                                ACTION_TYPE_WEIGHT
                            } else {
                                1.
                            };
                            (
                                &cons(UplistEntry::MapType(s.to_owned()), uplist),
                                true,
                                weight,
                            )
                        }
                        _ => {
                            warn!("expecting val for 'type' key to be a string, but got: {val}");
                            (uplist, false, 1.)
                        }
                    }
                } else {
                    (uplist, false, 1.)
                };

                // If we have a `type` entry, add the path on its own,
                // for the weight if it is an Action type, but should
                // be in there anyway, right?
                if ignore_type {
                    let s = uplist_to_string(uplist);
                    // dbg!((&s, weight)); -- this is OK
                    self.add_vector_entry(
                        id,
                        s,
                        Weight::from_f64(weight),
                        LeafValue::Bool(true), // XXX grusig
                    );
                }

                for (k, v) in map {
                    if ignore_type && k == "type" {
                        continue;
                    }
                    let uplist = cons(UplistEntry::MapKey(k), uplist);
                    self._add_vectors(id, v, &uplist);
                }
                return;
            }
        };
        self.add_vector_entry(
            id,
            uplist_to_string(uplist),
            Weight::from_f64(1.),
            leaf_value,
        );
    }
    fn add_vectors(&mut self, id: QueryId, value: &Value) {
        self._add_vectors(id, value, &List::Null)
    }
}

fn canonicalize(optimize: bool, line: &str, line0: usize, path: &Path) -> Result<(Value, String)> {
    let possibly_optimized = if optimize {
        let query: Query = serde_json::from_str(line).map_err(ctx!(
            "parsing query line at {}:{}",
            path.to_string_lossy(),
            line0 + 1
        ))?;
        let optimized = query.optimize();
        &serde_json::to_string(&optimized)?
    } else {
        line
    };

    let value: Value = serde_json::from_str(possibly_optimized).map_err(ctx!(
        "parsing line at {}:{}",
        path.to_string_lossy(),
        line0 + 1
    ))?;
    let canonical_query = CanonicalJson(&value).to_string();
    Ok((value, canonical_query))
}

fn read_queries_file_step_1(input: &Path) -> Result<String> {
    std::fs::read_to_string(&input).map_err(ctx!("reading ndjson file {input:?}"))
}

fn read_queries_file_step_2(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.trim_end().split("\n").enumerate()
}

fn main() -> Result<()> {
    let Opts {
        log_level,
        subcommand,
    } = Opts::parse();

    set_log_level(log_level.try_into()?);

    match subcommand {
        SubCommand::Canonicalize {
            optimize,
            input,
            output,
        } => {
            // ~copy paste from SubCommand::Vectorize
            let s = read_queries_file_step_1(&input)?;
            let mut out = BufWriter::new(
                File::create(&output).map_err(ctx!("opening output file {output:?}"))?,
            );
            for (i, line) in read_queries_file_step_2(&s) {
                let (_value, canonical_query) = canonicalize(optimize, line, i, &input)?;
                (|| -> Result<()> {
                    out.write_all(canonical_query.as_bytes())?;
                    out.write_all(b"\n")?;
                    Ok(())
                })()
                .with_context(|| anyhow!("writing to output file {output:?}"))?;
            }
            out.flush()
                .with_context(|| anyhow!("writing to output file {output:?}"))?;
        }
        SubCommand::Vectorize {
            parse_file: ParseFile { path },
            show_values,
            optimize,
            queries_output,
        } => {
            let mut queries = Queries::new();
            let s = read_queries_file_step_1(&path)?;
            for (i, line) in read_queries_file_step_2(&s) {
                let (value, canonical_query) = canonicalize(optimize, line, i, &path)?;
                if let Some(id) = queries.insert_query(canonical_query.into()) {
                    queries.add_vectors(id, &value);
                }
            }
            // dbg!(queries);
            for (key, (weight, idvals)) in &queries.vectors {
                println!("{weight}\t{key}");
                if show_values {
                    for (id, val) in idvals {
                        match val {
                            LeafValue::Bool(b) => println!("    {id} => {b:?}"),
                            LeafValue::Number(number) => println!("    {id} => {number:?}"),
                            LeafValue::None => (),
                        }
                    }
                }
            }

            let key_weights_and_ranges = queries.key_weights_and_ranges();
            dbg!(&key_weights_and_ranges);
            let n_queries = queries.query_ids_count();
            let n_weights = key_weights_and_ranges.len();
            let mut records = Array2::<f64>::zeros((n_queries, n_weights));

            for query_id in queries.query_ids() {
                let qw = queries.weights_for_query(query_id, &key_weights_and_ranges);
                // dbg!(&qw);
                qw.copy_to_array(records.row_mut(query_id.0));
            }
            dbg!(records.row(0));
            dbg!(records.row(1));

            let min_points = 2;
            let clusters = Dbscan::params(min_points)
                .tolerance(1.9)
                .transform(&records)?;
            // dbg!(clusters);

            assert_eq!(clusters.len(), n_queries);

            // (XX is there actually no reason to use Set vs Vec here?
            // Ordering enforced to be by id in either case?)
            let mut easy_clusters: BTreeMap<ClusterId, BTreeSet<Arc<str>>> = BTreeMap::new();
            let mut non_clustered: Vec<Arc<str>> = Vec::new();

            for (query_id, cluster) in clusters.iter().enumerate() {
                let query_id = QueryId(query_id);
                let query = queries.query_by_id(query_id).clone_arc();
                if let Some(cluster_id) = cluster {
                    let cluster_id = ClusterId(*cluster_id);
                    match easy_clusters.entry(cluster_id) {
                        Entry::Vacant(vacant_entry) => {
                            let mut set = BTreeSet::new();
                            set.insert(query);
                            vacant_entry.insert(set);
                        }
                        Entry::Occupied(mut occupied_entry) => {
                            occupied_entry.get_mut().insert(query);
                        }
                    }
                } else {
                    non_clustered.push(query);
                }
            }

            if let Some(queries_output) = queries_output {
                for (cluster_id, cluster) in &easy_clusters {
                    queries_output.write_queries(
                        Some(extension_for_cluster_id(Some(*cluster_id))),
                        &mut cluster.iter(),
                    )?;
                }

                if !non_clustered.is_empty() {
                    queries_output.write_queries(
                        Some(extension_for_cluster_id(None)),
                        &mut non_clustered.iter(),
                    )?;
                }
            } else {
                dbg!(easy_clusters);
                dbg!(non_clustered);
            }
        }

        SubCommand::MergeClustersTo {
            new_queries_dir,
            queries_dirs,
        } => {
            let mut queries = Vec::new();

            for queries_dir in queries_dirs {
                let queries_file_path = queries_dir.append("queries.ndjson");
                let s = read_queries_file_step_1(&queries_file_path)?;
                for (_i, line) in read_queries_file_step_2(&s) {
                    queries.push(Arc::from(line.to_owned()));
                }
            }

            // Re-use QueriesOutput; a bit hacky.
            let queries_output = QueriesOutput::ToFolders {
                reason: None,
                folder_base_path: new_queries_dir,
            };
            queries_output.write_queries(None, &mut queries.iter())?;
        }
    }

    Ok(())
}
