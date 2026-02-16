//! Generate the HTML files at the top of the output directory for
//! easy access of the outputs. This uses the same code as the
//! `evobench list` subcommand, and some more.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    io::Write,
    sync::Arc,
};

use ahtml::{ASlice, HtmlAllocator, Node};
use anyhow::Result;
use kstring::KString;

use crate::{
    io_utils::tempfile_utils::tempfile,
    output_table::{CellValue, OutputTable, OutputTableTitle, html::HtmlTable},
    run::{
        config::{RunConfig, ShareableConfig},
        output_directory::structure::{ParametersDir, ToPath},
        run_queues::RunQueues,
        sub_command::list::{OutputTableOpts, ParameterView},
        working_directory_pool::WorkingDirectoryPoolBaseDir,
    },
    utillib::{arc::CloneArc, into_arc_path::IntoArcPath, invert_index::invert_index_by_ref},
};

fn print_html_document(
    body: ASlice<Node>,
    html: &HtmlAllocator,
    mut out: impl Write,
) -> Result<()> {
    let doc = html.html(
        [],
        [html.head([], [])?, html.body([], html.table([], body)?)?],
    )?;
    html.print_html_document(doc, &mut out)?;
    out.flush()?;
    Ok(())
}

// It's a bit of a mess: creating the table is in sub_command/list.rs,
// we get the OutputTableOpts and associated creation op from
// there. Currently. (I.e. it comes here from there and calls back to
// there .)

pub fn print_list(
    conf: &RunConfig,
    working_directory_base_dir: &Arc<WorkingDirectoryPoolBaseDir>,
    queues: &RunQueues,
    output_table_opts: &OutputTableOpts,
    html: Option<&HtmlAllocator>,
    link_skipped: Option<&str>,
    out: impl Write,
) -> Result<()> {
    let tmp;
    let html = if let Some(html) = html {
        html
    } else {
        tmp = HtmlAllocator::new(1000000, Arc::new("list"));
        &tmp
    };
    let num_columns = output_table_opts.parameter_view.titles().len();
    let table = HtmlTable::new(num_columns, &html);
    let body = output_table_opts.output_to_table(
        table,
        conf,
        link_skipped,
        working_directory_base_dir,
        queues,
    )?;
    print_html_document(body.as_slice(), html, out)
}

fn write_2_column_table_file<'url, T1: CellValue<'url>, T2: CellValue<'url>>(
    file_name: &str,
    titles: &[&str],
    index: &BTreeMap<T1, BTreeSet<T2>>,
    conf: &RunConfig,
    html: &HtmlAllocator,
) -> Result<()> {
    // let title_style = Some(OutputStyle {
    //     font_size: Some(FontSize::Large),
    //     ..Default::default()
    // });

    let titles: Vec<_> = titles
        .iter()
        .map(|title| OutputTableTitle {
            text: (*title).into(),
            span: 1,
        })
        .collect();

    let (tmp_file, out) = tempfile(conf.output_dir.path.join(file_name), false)?;
    let num_columns = titles.len();
    let mut table = HtmlTable::new(num_columns, &html);
    table.write_title_row(&titles, None)?;

    for (k, vs) in index {
        // let mut items = html.new_vec();
        // for v in vs {
        //     items.push(html.text(v.as_ref())?)?;
        //     items.push(html.br([],[])?)?;
        // }

        // Show k once only, with the first v
        let mut vs = vs.iter();
        let v = vs
            .next()
            .expect("only adding BtreeSet with a value and never removing values");
        let row: &[&dyn CellValue] = &[k, v];
        table.write_data_row(row, None)?;

        for v in vs {
            let row: &[&dyn CellValue] = &[&"", v];
            table.write_data_row(row, None)?;
        }
    }

    print_html_document(table.finish()?.as_slice(), &html, out)?;
    tmp_file.finish()?;
    Ok(())
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ParametersCellValue {
    dir: ParametersDir,
    s: String,
}

impl From<ParametersDir> for ParametersCellValue {
    fn from(dir: ParametersDir) -> Self {
        let s = format!(
            "{} -> {}",
            dir.target_name().as_str(),
            dir.custom_parameters()
        );
        Self { dir, s }
    }
}

impl AsRef<str> for ParametersCellValue {
    fn as_ref(&self) -> &str {
        &self.s
    }
}

impl<'url> CellValue<'url> for ParametersCellValue {
    fn perhaps_url(&self) -> Option<Cow<'static, str>> {
        Some(self.dir.to_path().to_string_lossy().to_string().into())
    }
    fn perhaps_anchor_name(&self) -> Option<&KString> {
        None
    }
}

impl<'url> CellValue<'url> for &ParametersCellValue {
    fn perhaps_url(&self) -> Option<Cow<'static, str>> {
        Some(self.dir.to_path().to_string_lossy().to_string().into())
    }
    fn perhaps_anchor_name(&self) -> Option<&KString> {
        None
    }
}

impl<'url> CellValue<'url> for KString {
    fn perhaps_url(&self) -> Option<Cow<'static, str>> {
        None
    }
    fn perhaps_anchor_name(&self) -> Option<&KString> {
        None
    }
}

impl<'url> CellValue<'url> for &KString {
    fn perhaps_url(&self) -> Option<Cow<'static, str>> {
        None
    }
    fn perhaps_anchor_name(&self) -> Option<&KString> {
        None
    }
}

/// Does not take a lock: just regenerates the file (via
/// tempfile-rename) with external values at least from now. For
/// savings, pass the optional values if you can.
pub fn regenerate_index_files(
    shareable_config: &ShareableConfig,
    working_directory_base_dir: Option<&Arc<WorkingDirectoryPoolBaseDir>>,
    queues: Option<&RunQueues>,
) -> Result<()> {
    let conf = &shareable_config.run_config;

    // Copies from src/bin/evobench.rs; hacky.

    let tmp;
    let working_directory_base_dir = if let Some(d) = working_directory_base_dir {
        d
    } else {
        tmp = Arc::new(WorkingDirectoryPoolBaseDir::new(
            conf.working_directory_pool.base_dir.clone(),
            &|| {
                shareable_config
                    .global_app_state_dir
                    .working_directory_pool_base()
            },
        )?);
        &tmp
    };

    let tmp2;
    let queues = if let Some(q) = queues {
        q
    } else {
        tmp2 = RunQueues::open(
            shareable_config.run_config.queues.clone_arc(),
            true,
            &shareable_config.global_app_state_dir,
            // No need to signal changes, not going to mutate anything
            None,
        )?;
        &tmp2
    };

    // / setup

    let mut html = HtmlAllocator::new(1000000, Arc::new("regenerate_index_files"));

    let write_jobs_list =
        |html: &HtmlAllocator, file_name: &str, all: bool, link: Option<&str>| -> Result<()> {
            let output_table_opts = OutputTableOpts {
                verbose: false,
                all,
                n: None,
                parameter_view: ParameterView::Separated,
            };

            let (tmp_file, out) = tempfile(conf.output_dir.path.join(file_name), false)?;

            print_list(
                conf,
                working_directory_base_dir,
                queues,
                &output_table_opts,
                Some(html),
                link,
                out,
            )?;

            tmp_file.finish()?;

            Ok(())
        };

    // Write the jobs list with the default limit
    write_jobs_list(&html, "list.html", false, Some("list-unlimited.html"))?;
    html.clear();
    // And again with "all" jobs; to avoid confusion, do not use
    // "list-all.html" since "list-all" is a different evobench
    // subcommand.
    write_jobs_list(&html, "list-unlimited.html", true, None)?;
    html.clear();

    // parameter lists
    if let Some(base_url) = &conf.output_dir.url {
        let paths_with_names = {
            let mut paths_with_names = BTreeMap::new();
            for (name, templates) in &conf.job_template_lists {
                for template in &**templates {
                    let dir = ParametersCellValue::from(
                        template.to_parameters_dir(base_url.into_arc_path()),
                    );
                    match paths_with_names.entry(dir) {
                        Entry::Vacant(vacant_entry) => {
                            let mut m = BTreeSet::new();
                            m.insert(name.clone());
                            vacant_entry.insert(m);
                        }
                        Entry::Occupied(mut occupied_entry) => {
                            occupied_entry.get_mut().insert(name.clone());
                        }
                    }
                }
            }
            paths_with_names
        };

        write_2_column_table_file(
            "by_parameters.html",
            &["Parameter", "Templates names"],
            &paths_with_names,
            conf,
            &html,
        )?;
        html.clear();

        let names_with_paths = invert_index_by_ref(&paths_with_names);
        write_2_column_table_file(
            "by_templates_name.html",
            &["Templates name", "Parameters"],
            &names_with_paths,
            conf,
            &html,
        )?;
        html.clear();
    }

    Ok(())
}
