use anyhow::Result;
use evobench_evaluator::run::run_job::bench_tmp_dir;

fn main() -> Result<()> {
    let bench_tmp_dir = bench_tmp_dir()?;
    dbg!(bench_tmp_dir);
    Ok(())
}
