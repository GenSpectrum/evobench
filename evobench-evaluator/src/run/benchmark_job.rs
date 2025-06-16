use std::path::PathBuf;

use crate::key::RunParameters;

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize, clap::Args)]
pub struct BenchmarkJob {
    /// The path to the working directory (git clone) of the project
    pub project_working_directory: PathBuf,

    #[clap(flatten)]
    pub run_parameters: RunParameters,
}
