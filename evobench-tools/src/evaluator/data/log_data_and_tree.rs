use std::{path::Path, time::SystemTime};

use anyhow::Result;
use ouroboros::self_referencing;

use crate::evaluator::data::{log_data::LogData, log_data_tree::LogDataTree};

#[self_referencing]
pub struct LogDataAndTree {
    log_data: LogData,
    #[borrows(log_data)]
    #[covariant]
    tree: LogDataTree<'this>,
}

impl LogDataAndTree {
    /// `LogData::read_file` combined with `LogDataTree::from_logdata`
    pub fn read_file(path: &Path, uncompressed_path: Option<&Path>) -> Result<Self> {
        let t0 = SystemTime::now();
        let log_data = LogData::read_file(path, uncompressed_path)?;

        let t1 = SystemTime::now();
        let r = LogDataAndTree::try_new(log_data, |log_data| LogDataTree::from_logdata(log_data));
        let t2 = SystemTime::now();

        eprintln!(
            "t LogData::read_file: {} s",
            t1.duration_since(t0)?.as_secs_f64()
        );
        eprintln!(
            "t LogDataAndTree::try_new: {} s",
            t2.duration_since(t1)?.as_secs_f64()
        );

        r
    }

    pub fn log_data(&self) -> &LogData {
        self.borrow_log_data()
    }

    pub fn tree(&self) -> &LogDataTree<'_> {
        self.borrow_tree()
    }
}
