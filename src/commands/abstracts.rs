//! List abstract contracts in a Foundry project.
//!
//! This module is the CLI-facing layer for the `hawk inspect abstracts` command.
//! The core logic lives in [`crate::project::Project`].

use std::path::Path;

use anyhow::Result;

use crate::project::Project;

/// List abstract contracts in the given Foundry project.
///
/// Returns each contract as `"file:name"` where `file` is relative to the
/// project root (as recorded in the AST).
pub fn list(path: impl AsRef<Path>) -> Result<Vec<String>> {
    let project = Project::open(path)?;
    let declarations = project.abstract_contracts()?;
    let lines: Vec<String> = declarations
        .iter()
        .map(|d| format!("{}:{}", d.file.display(), d.name))
        .collect();
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-contracts")
    }

    #[test]
    fn list_returns_abstract_contracts_only() {
        let contracts = list(fixture_path()).unwrap();

        assert_eq!(contracts, vec!["src/AbstractBase.sol:AbstractBase"]);
    }
}
