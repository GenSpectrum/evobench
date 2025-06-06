use std::path::Path;

use anyhow::Result;
use ouroboros::self_referencing;

use crate::{log_data_tree::LogDataTree, log_file::LogData};

#[self_referencing]
pub struct LogDataAndTree {
    log_data: LogData,
    #[borrows(log_data)]
    #[covariant]
    tree: LogDataTree<'this>,
}

impl LogDataAndTree {
    /// `LogData::read_file` combined with `LogDataTree::from_logdata`
    pub fn read_file(path: &Path) -> Result<Self> {
        let log_data = LogData::read_file(path)?;

        LogDataAndTree::try_new(log_data, |log_data| LogDataTree::from_logdata(log_data))
    }

    pub fn log_data(&self) -> &LogData {
        self.borrow_log_data()
    }

    pub fn tree(&self) -> &LogDataTree {
        self.borrow_tree()
    }
}
