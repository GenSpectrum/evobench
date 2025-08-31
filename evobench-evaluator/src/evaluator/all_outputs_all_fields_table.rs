use std::{
    fs::File,
    io::{BufWriter, Write},
    ops::Deref,
    path::PathBuf,
};

use anyhow::{anyhow, bail, Result};
use run_git::path_util::{add_extension, AppendToPath};

use crate::{
    config_file::ron_to_file_pretty,
    evaluator::data::log_data_tree::LogDataTree,
    evaluator::options::TILE_COUNT,
    info,
    io_utils::tempfile_utils::TempfileOptions,
    join::KeyVal,
    stats::StatsField,
    tables::{excel_table_view::excel_file_write, table_view::TableView},
    tree::Tree,
    warn,
};

use super::{
    all_fields_table::{
        AllFieldsTable, AllFieldsTableKind, AllFieldsTableKindParams, KeyRuntimeDetails,
        SingleRunStats, SummaryStats,
    },
    options::{CheckedOutputOptionsMapCase, EvaluationOpts, OutputVariants},
};

pub struct AllFieldsTableWithOutputPathOrBase<Kind: AllFieldsTableKind> {
    aft: AllFieldsTable<Kind>,
    /// The path or base for where this file or set of files is to end up in
    output_path_or_base: PathBuf,
    /// Whether *this* aft is actually to be stored at the above path;
    /// false means, it's not processed to the final stage yet.
    is_final_file: bool,
}

/// A wrapper holding the table sets for all requested
/// outputs. (Wrapping since we want to have the same fields and
/// mapping methods. A type alias would currently lose the trait
/// restriction checks in Rust's type system.)
pub struct AllOutputsAllFieldsTable<Kind: AllFieldsTableKind>(
    OutputVariants<AllFieldsTableWithOutputPathOrBase<Kind>>,
);

impl<Kind: AllFieldsTableKind> Deref for AllOutputsAllFieldsTable<Kind> {
    type Target = OutputVariants<AllFieldsTableWithOutputPathOrBase<Kind>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn key_details_for(
    case: CheckedOutputOptionsMapCase,
    evaluation_opts: &EvaluationOpts,
) -> KeyRuntimeDetails {
    let EvaluationOpts {
        key_width,
        show_thread_number,
        show_reversed,
    } = evaluation_opts;

    let (
        normal_separator,
        reverse_separator,
        show_probe_names,
        show_paths_without_thread_number,
        show_paths_reversed_too,
        key_column_width,
        skip_process,
        prefix,
    );
    match case {
        CheckedOutputOptionsMapCase::Excel => {
            normal_separator = " > ";
            reverse_separator = " < ";
            show_probe_names = true;
            show_paths_without_thread_number = true;
            show_paths_reversed_too = *show_reversed;
            key_column_width = Some(*key_width);
            skip_process = false;
            prefix = None;
        }
        CheckedOutputOptionsMapCase::Flame => {
            normal_separator = ";";
            reverse_separator = ";";
            show_probe_names = false;
            show_paths_without_thread_number = !*show_thread_number;
            show_paths_reversed_too = false;
            key_column_width = None;
            skip_process = true;
            prefix = Some("");
        }
    }

    KeyRuntimeDetails {
        normal_separator,
        reverse_separator,
        show_probe_names,
        show_paths_without_thread_number,
        show_paths_with_thread_number: *show_thread_number,
        show_paths_reversed_too,
        key_column_width,
        skip_process,
        prefix,
    }
}

impl AllOutputsAllFieldsTable<SingleRunStats> {
    pub fn from_log_data_tree(
        log_data_tree: &LogDataTree,
        evaluation_opts: &EvaluationOpts,
        output_opts: OutputVariants<PathBuf>,
        is_final_file: bool,
    ) -> Result<Self> {
        let output_variants = output_opts.try_map(|case, path| -> Result<_> {
            Ok(AllFieldsTableWithOutputPathOrBase {
                aft: AllFieldsTable::from_log_data_tree(
                    log_data_tree,
                    AllFieldsTableKindParams {
                        source_path: log_data_tree.log_data().path.as_ref().into(),
                        key_details: key_details_for(case, evaluation_opts),
                    },
                )?,
                output_path_or_base: path,
                is_final_file,
            })
        })?;
        Ok(Self(output_variants))
    }
}

impl AllOutputsAllFieldsTable<SummaryStats> {
    pub fn summary_stats(
        aoafts: &[AllOutputsAllFieldsTable<SingleRunStats>],
        field_selector: StatsField<TILE_COUNT>,
        evaluation_opts: &EvaluationOpts,
        output_opts: OutputVariants<PathBuf>,
        is_final_file: bool,
    ) -> AllOutputsAllFieldsTable<SummaryStats> {
        // Split up the `aoafts` by field
        let lists_by_field = output_opts.clone().map(|case, _path| {
            aoafts
                .into_iter()
                .map(|aoaft| {
                    &aoaft
                        .get(case)
                        .as_ref()
                        .expect(
                            "same output_opts given in previous layer \
                             leading to same set of options",
                        )
                        .aft
                })
                .collect::<Vec<_>>()
        });
        let x = lists_by_field.map(|case, afts| AllFieldsTableWithOutputPathOrBase {
            aft: AllFieldsTable::summary_stats(
                afts.as_slice(),
                match case {
                    CheckedOutputOptionsMapCase::Excel => field_selector,
                    // Flame graphs always need the sums, thus ignore
                    // the user option for those
                    CheckedOutputOptionsMapCase::Flame => StatsField::Sum,
                },
                &key_details_for(case, evaluation_opts),
            ),
            output_path_or_base: output_opts.get(case).as_ref().expect("ditto").clone(),
            is_final_file,
        });
        Self(x)
    }
}

/// Get the sum of the children's values, and if those don't have a
/// value, their children's values recursively. XX Could be a bit
/// costly if there are many gaps!
fn node_children_sum<'key>(tree: &Tree<'key, u64>) -> u64 {
    tree.children
        .iter()
        .map(|(_, child)| child.value.unwrap_or_else(|| node_children_sum(child)))
        .sum()
}

/// Convert a tree where the value of a parent include the values of
/// the children (timings!) into one where the parent has only the
/// remainder after subtracting the original values of the
/// children.
fn fix_tree<'key>(tree: Tree<'key, u64>) -> Tree<'key, u64> {
    let value = tree.value.map(|orig_value| {
        let orig_children_total: u64 = node_children_sum(&tree);
        // orig_value - orig_children_total
        orig_value
            .checked_sub(orig_children_total)
            .unwrap_or_else(|| {
                eprintln!(
                    "somehow parent has lower value, {orig_value}, \
                     than sum of children, {orig_children_total}"
                );
                0
            })
    });
    Tree {
        value,
        children: tree
            .children
            .into_iter()
            .map(|(key, child)| (key, fix_tree(child)))
            .collect(),
    }
}

#[test]
fn t_fix_tree() {
    let vals = &[
        ("a", 2),
        ("a:b", 1),
        ("a:b:c", 1),
        ("c:d", 3),
        ("d:e:f", 4),
        ("d", 5),
    ];
    let tree = Tree::from_key_val(vals.into_iter().map(|(k, v)| (k.split(':'), *v)));
    dbg!(&tree);
    assert_eq!(tree.get("a".split(':')), Some(&2));
    assert_eq!(tree.get("a:b".split(':')), Some(&1));
    assert_eq!(tree.get("a:b:c".split(':')), Some(&1));
    let tree = fix_tree(tree);
    dbg!(&tree);
    assert_eq!(tree.get("a".split(':')), Some(&1));
    assert_eq!(tree.get("a:b".split(':')), Some(&0));
    assert_eq!(tree.get("a:b:c".split(':')), Some(&1));
    // panic!()
}

impl<Kind: AllFieldsTableKind> AllOutputsAllFieldsTable<Kind> {
    /// Write to all output files originally specified; gives an error
    /// unless the `is_final_file` for this instance was true. (Taking
    /// ownership only because `try_map` currently requires so.)
    pub fn write_to_files(self, flame_field: StatsField<TILE_COUNT>) -> Result<()> {
        self.0.try_map(|case, aft| -> Result<()> {
            let AllFieldsTableWithOutputPathOrBase {
                aft,
                output_path_or_base,
                is_final_file,
            } = aft;
            if !is_final_file {
                bail!(
                    "trying to save a table that wasn't marked as \
                     the last stage in a processing chain"
                )
            }
            let tables = aft.tables();
            match case {
                CheckedOutputOptionsMapCase::Excel => {
                    excel_file_write(
                        tables.iter().map(|v| {
                            let v: &dyn TableView = *v;
                            v
                        }),
                        &output_path_or_base,
                    )?;
                }
                CheckedOutputOptionsMapCase::Flame => {
                    let curdir = PathBuf::from(".");
                    let flame_base_dir = output_path_or_base.parent().unwrap_or(&*curdir);
                    let flame_base_name = output_path_or_base
                        .file_name()
                        .ok_or_else(|| anyhow!("--flame option argument is missing a file name"))?
                        .to_string_lossy();

                    for table in tables {
                        if table.table_key_vals(flame_field).next().is_none() {
                            // The table has no rows. `inferno` is
                            // giving errors when attempting to
                            // generate flame graphs without data,
                            // thus skip this table
                            continue;
                        }

                        let lines: Vec<String> = {
                            let tree = Tree::from_key_val(
                                table
                                    .table_key_vals(flame_field)
                                    .map(|KeyVal { key, val }| (key.split(';'), val)),
                            );

                            let fixed_tree = fix_tree(tree);

                            fixed_tree
                                .into_joined_key_val(";")
                                .into_iter()
                                .map(|(path, val)| format!("{path} {val}"))
                                .collect()
                        };

                        // `inferno` is really fussy, apparently it
                        // gives a "No stack counts found" error
                        // whenever it's missing any line with a ";"
                        // in it, thus check:
                        if !lines.iter().any(|s| s.contains(';')) {
                            eprintln!(
                                "note: there are no lines with ';' to be fed to inferno, \
                                 thus do not attempt to generate flame graph"
                            );
                        } else {
                            let target_path = flame_base_dir
                                .append(format!("{flame_base_name}-{}.svg", table.table_name()));
                            if let Err(e) = (|| -> Result<()> {
                                let tempfile = TempfileOptions {
                                    target_path: target_path.clone(),
                                    retain_tempfile: true,
                                    migrate_access: false,
                                }
                                .tempfile()?;

                                let mut options = inferno::flamegraph::Options::default();
                                options.count_name = table.resolution_unit();
                                options.title = table.table_name().into();
                                // options.subtitle = Some("foo".into()); XX show inputs key

                                let mut out = BufWriter::new(File::create(&tempfile.temp_path)?);
                                inferno::flamegraph::from_lines(
                                    // why mut ??
                                    &mut options,
                                    lines.iter().map(|s| -> &str { s }),
                                    &mut out,
                                )?;
                                out.flush()?;
                                tempfile.finish()?;
                                Ok(())
                            })() {
                                warn!(
                                    "ignoring error creating flamegraph file \
                                     {target_path:?}: {e:#}"
                                );
                                let dump_path = add_extension(&target_path, "data")
                                    .expect("guaranteed to have file name");
                                ron_to_file_pretty(&lines, &dump_path, false, None)?;
                                info!(
                                    "wrote data to be used in {target_path:?} here: {dump_path:?}"
                                );
                            }
                        }
                    }
                }
            }
            Ok(())
        })?;
        Ok(())
    }
}
