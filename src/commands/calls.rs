//! Show the call graph of a Solidity function.
//!
//! This module is the CLI-facing layer for the `hawk inspect calls` command.

use std::path::Path;

use anyhow::Result;

use crate::call_graph::CallGraphResolver;
use crate::project::Project;

/// Run the call graph inspection for the given function ID.
///
/// `function_id` should be in the format `Contract::function`.
pub fn run(project_path: impl AsRef<Path>, function_id: &str) -> Result<String> {
    let project = Project::open(project_path.as_ref());
    project.validate()?;
    let resolver = CallGraphResolver::new(project);
    Ok(resolver.resolve(function_id)?.to_string())
}
