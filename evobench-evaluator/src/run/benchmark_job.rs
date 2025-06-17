use std::{collections::BTreeMap, path::PathBuf};

use anyhow::Result;

use crate::key::{CheckedRunParameters, RunParameters};

#[derive(Debug, PartialEq, Clone, clap::Args)]
pub struct BenchmarkJobOpts {
    /// The path to the working directory (git clone) of the project
    pub project_working_directory: PathBuf,

    #[clap(flatten)]
    pub run_parameters: RunParameters,
}

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkJob {
    pub project_working_directory: PathBuf,
    pub run_parameters: CheckedRunParameters,
}

impl BenchmarkJobOpts {
    pub fn checked(
        self,
        custom_parameters_required: &BTreeMap<String, bool>,
    ) -> Result<BenchmarkJob> {
        let Self {
            project_working_directory,
            run_parameters,
        } = self;

        Ok(BenchmarkJob {
            project_working_directory,
            run_parameters: run_parameters.checked(custom_parameters_required)?,
        })
    }
}
