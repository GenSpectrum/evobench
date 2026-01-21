use std::{fs::File, time::Duration};

use anyhow::Result;
use cj_path_util::path_util::AppendToPath;
use evobench_tools::{io_utils::temporary_file::TemporaryFile, run::run_job::bench_tmp_dir};
use nix::unistd::getpid;

fn main() -> Result<()> {
    let bench_tmp_dir = bench_tmp_dir()?;
    dbg!(&bench_tmp_dir);

    let pid = getpid();
    // File for evobench library output
    let evobench_log = TemporaryFile::from((&bench_tmp_dir).append(format!("evobench-{pid}.log")));
    // File for other output, for optional use by target application
    let bench_output_log =
        TemporaryFile::from((&bench_tmp_dir).append(format!("bench-output-{pid}.log")));

    dbg!(evobench_log.path());
    dbg!(bench_output_log.path());

    let p = evobench_log.path();
    File::create(p)?;

    let _ = std::fs::remove_file(evobench_log.path());
    let _ = std::fs::remove_file(bench_output_log.path());

    File::create(p)?;

    std::thread::sleep(Duration::from_secs(10));

    Ok(())
}
