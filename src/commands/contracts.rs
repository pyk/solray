//! List deployable contracts in a Foundry project.
//!
//! This module is the CLI-facing layer for the `hawk inspect contracts` command.
//! The core logic lives in [`crate::project::Project`].

use std::path::Path;

use anyhow::Result;

use crate::project::{Declaration, Project};

/// List deployable (non-abstract) contracts in the given Foundry project.
pub fn list(path: impl AsRef<Path>) -> Result<Vec<Declaration>> {
    let project = Project::open(path)?;
    project.deployable_contracts()
}
