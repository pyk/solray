//! Show the write functions exposed by a Solidity contract ABI.
//!
//! This module is the CLI-facing layer for the `hawk inspect entrypoints` command.

use std::path::Path;

use anyhow::Result;

use crate::EntrypointsResolver;
use crate::project::Project;

/// Run the entrypoint inspection for the given deployable contract.
pub fn run(deployable: &str, project_path: impl AsRef<Path>) -> Result<String> {
    let project = Project::open(project_path.as_ref());
    let resolver = EntrypointsResolver::new(project);
    resolver.resolve(deployable)
}
