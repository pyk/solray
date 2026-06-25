//! Show the inheritance graph of a Solidity declaration.
//!
//! This module is the CLI-facing layer for the `hawk inspect inheritances` command.

use std::path::Path;

use anyhow::Result;

use crate::InheritanceResolver;
use crate::project::Project;

/// Run the inheritance inspection for the given declaration.
pub fn run(decl: &str, path: impl AsRef<Path>) -> Result<String> {
    let project = Project::open(path);
    let resolver = InheritanceResolver::new(project);
    resolver.resolve(decl)
}
