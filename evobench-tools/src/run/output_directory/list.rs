//! Generate the `list/index.html` file for accessing the output
//! directory. This uses the same code as the `evobench list`
//! subcommand.

use std::{
    fs::{File, create_dir_all},
    io::{BufWriter, Write},
    sync::Arc,
};

use ahtml::HtmlAllocator;
use anyhow::Result;
use cj_path_util::path_util::AppendToPath;

use crate::{
    ctx,
    io_utils::tempfile_utils::TempfileOptions,
    output_table::html::HtmlTable,
    run::{
        config::{RunConfig, RunConfigBundle},
        run_queues::RunQueues,
        sub_command::list::{OutputTableOpts, ParameterView},
        working_directory_pool::WorkingDirectoryPoolBaseDir,
    },
    utillib::arc::CloneArc,
};

// It's a bit of a mess: creating the table is in sub_command/list.rs,
// we get the OutputTableOpts and associated creation op from
// there. Currently. (I.e. it comes here from there and calls back to
// there .)

pub fn print_list(
    conf: &RunConfig,
    working_directory_base_dir: &Arc<WorkingDirectoryPoolBaseDir>,
    queues: &RunQueues,
    output_table_opts: &OutputTableOpts,
    mut out: impl Write,
) -> Result<()> {
    let num_columns = output_table_opts.parameter_view.titles().len();
    let html = HtmlAllocator::new(1000000, Arc::new("list"));
    let table = HtmlTable::new(num_columns, &html);
    let body =
        output_table_opts.output_to_table(table, conf, working_directory_base_dir, queues)?;
    let doc = html.html(
        [],
        [html.head([], [])?, html.body([], html.table([], body)?)?],
    )?;
    html.print_html_document(doc, &mut out)?;
    out.flush()?;
    Ok(())
}

/// Does not take a lock: just regenerates the file (via
/// tempfile-rename) with external values at least from now. For
/// savings, pass the optional values if you can.
pub fn regenerate_list(
    run_config_bundle: &RunConfigBundle,
    working_directory_base_dir: Option<&Arc<WorkingDirectoryPoolBaseDir>>,
    queues: Option<&RunQueues>,
) -> Result<()> {
    let conf = &run_config_bundle.run_config;

    // Copies from src/bin/evobench.rs; hacky.

    let tmp;
    let working_directory_base_dir = if let Some(d) = working_directory_base_dir {
        d
    } else {
        tmp = Arc::new(WorkingDirectoryPoolBaseDir::new(
            conf.working_directory_pool.base_dir.clone(),
            &|| {
                run_config_bundle
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
            run_config_bundle.run_config.queues.clone_arc(),
            true,
            &run_config_bundle.global_app_state_dir,
        )?;
        &tmp2
    };

    // / setup

    let output_table_opts = OutputTableOpts {
        verbose: false,
        all: false,
        parameter_view: ParameterView::Separated,
    };

    let list_dir = conf.output_dir.path.append("list");
    create_dir_all(&list_dir).map_err(ctx!("creating dir {list_dir:?}"))?;

    let target_path = list_dir.join("index.html");
    let tmp_file = TempfileOptions {
        target_path,
        retain_tempfile: false,
        migrate_access: false,
    }
    .tempfile()?;
    let temp_path = &tmp_file.temp_path;
    let out = BufWriter::new(File::create(temp_path).map_err(ctx!("creating file {temp_path:?}"))?);

    print_list(
        conf,
        working_directory_base_dir,
        queues,
        &output_table_opts,
        out,
    )?;

    tmp_file.finish()?;

    Ok(())
}
