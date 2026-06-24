//! Show the inheritance graph of a Solidity declaration.
//!
//! This module is the CLI-facing layer for the `hawk inspect inheritances` command.
//! The core logic lives in [`crate::project::Project`].

use std::path::Path;

use anyhow::{Result, bail};

use crate::project::Project;

/// Run the inheritance inspection for the given declaration name.
///
/// Returns the formatted output on success. Returns an error with a
/// user-friendly message when the declaration is not found.
pub fn run(decl: &str, path: impl AsRef<Path>) -> Result<String> {
    let project = Project::open(path)?;

    let found = project.find_declaration(decl)?;
    if found.is_none() {
        let decls = project.declarations()?;
        let names: Vec<String> = decls.into_iter().map(|d| d.name).collect();
        bail!(
            "\"{}\" not found.\n\nAvailable declarations: {}",
            decl,
            names.join(", ")
        );
    }

    let tree = project.inheritance_tree(decl)?;
    let sources = tree.flatten_sources();
    let mut output = String::new();

    output.push_str("Inheritance graph:\n\n");
    output.push_str(&tree.to_string());

    output.push_str("\nSources:\n\n");
    for (i, (file, name)) in sources.iter().enumerate() {
        output.push_str(&format!("{}. {}:{}\n", i + 1, file, name));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritances")
    }

    #[test]
    fn run_errors_for_unknown_declaration() {
        let result = run("Nonexistent", fixture_path());
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\
\"Nonexistent\" not found.

Available declarations: AnotherBase, Base, Child, Middle, MultiBase, MultiChild",
        );
    }

    #[test]
    fn run_shows_inheritance_for_contract_with_no_parents() {
        let result = run("Base", fixture_path()).unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:

Base

Sources:

1. src/Base.sol:Base
",
        );
    }

    #[test]
    fn run_shows_inheritance_chain() {
        let result = run("Child", fixture_path()).unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:

Child
\u{2514}\u{2500}\u{2500} Middle
    \u{2514}\u{2500}\u{2500} Base

Sources:

1. src/Child.sol:Child
2. src/Middle.sol:Middle
3. src/Base.sol:Base
",
        );
    }

    #[test]
    fn run_shows_multiple_inheritance() {
        let result = run("MultiChild", fixture_path()).unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:

MultiChild
\u{251c}\u{2500}\u{2500} MultiBase
\u{2514}\u{2500}\u{2500} AnotherBase

Sources:

1. src/MultiChild.sol:MultiChild
2. src/MultiBase.sol:MultiBase
3. src/AnotherBase.sol:AnotherBase
",
        );
    }
}
