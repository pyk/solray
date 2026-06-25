//! Show the storage layout exposed by a Solidity contract.
//!
//! This module is the CLI-facing layer for the `hawk inspect storages` command.

use std::path::Path;

use anyhow::Result;

use crate::StorageLayoutResolver;
use crate::project::Project;

/// Run the storage inspection for the given deployable contract.
pub fn run(deployable: &str, project_path: impl AsRef<Path>) -> Result<String> {
    let project = Project::open(project_path.as_ref());
    let resolver = StorageLayoutResolver::new(project);
    resolver.resolve(deployable)
}
