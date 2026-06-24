//! Foundry project inspection.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Deserialize;
use solc::ast::{ContractDefinition, ContractKind, SourceUnit, SourceUnitNode};
use walkdir::WalkDir;

/// A single Solidity source-level declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub kind: DeclarationKind,
    pub file: PathBuf,
}

/// The kind of a top-level Solidity declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Contract,
    AbstractContract,
    Interface,
    Library,
}

/// A Foundry project opened for inspection.
#[derive(Debug)]
pub struct Project {
    path: PathBuf,
    out: PathBuf,
}

/// Minimal artifact wrapper for extracting the AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

impl Project {
    /// Open the Foundry project at `path`.
    ///
    /// Looks for `foundry.toml` to locate the project root and validates
    /// that `ast = true` is set in the default profile.
    ///
    /// Requires artifacts to have been built with `forge build`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let foundry_toml = path.join("foundry.toml");

        anyhow::ensure!(
            foundry_toml.exists(),
            "not a Foundry project: {} not found",
            foundry_toml.display()
        );

        let config: toml::Value = toml::from_str(&fs::read_to_string(&foundry_toml)?)?;

        let ast = config
            .get("profile")
            .and_then(|p| p.get("default"))
            .and_then(|d| d.get("ast"))
            .and_then(|a| a.as_bool());

        anyhow::ensure!(
            ast == Some(true),
            "`ast = true` must be set in the [profile.default] section of {}",
            foundry_toml.display()
        );

        let out = path.join("out");
        Ok(Project { path, out })
    }

    /// Return the project root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return all declarations found across all artifacts.
    pub fn declarations(&self) -> Result<Vec<Declaration>> {
        if !self.out.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for entry in WalkDir::new(&self.out).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if path.to_string_lossy().contains("build-info") {
                continue;
            }

            let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(name) => name,
                None => continue,
            };

            let content = fs::read_to_string(path)?;
            let artifact: Artifact = serde_json::from_str(&content)?;

            let ast = match artifact.ast {
                None => bail!(
                    "artifact `{}` is missing the AST; rebuild with `ast = true` in foundry.toml",
                    path.display()
                ),
                Some(ast) => ast,
            };

            // Take ownership of nodes to avoid cloning the name.
            let source_file = ast.absolute_path;
            if let Some(cd) = ast.nodes.into_iter().find_map(|node| {
                if let SourceUnitNode::ContractDefinition(cd) = node
                    && cd.name == contract_name
                {
                    return Some(cd);
                }
                None
            }) {
                let kind = classify_contract(&cd);
                results.push(Declaration {
                    name: cd.name,
                    kind,
                    file: source_file,
                });
            }
        }

        results.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(results)
    }

    /// Return only deployable (non-abstract, non-interface, non-library) contracts.
    pub fn deployable_contracts(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::Contract)
                .collect()
        })
    }

    /// Return only abstract contracts.
    pub fn abstract_contracts(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::AbstractContract)
                .collect()
        })
    }

    /// Return only libraries.
    pub fn libraries(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::Library)
                .collect()
        })
    }
}

fn classify_contract(cd: &ContractDefinition) -> DeclarationKind {
    match cd.contract_kind {
        ContractKind::Contract if cd.r#abstract => DeclarationKind::AbstractContract,
        ContractKind::Contract => DeclarationKind::Contract,
        ContractKind::Interface => DeclarationKind::Interface,
        ContractKind::Library => DeclarationKind::Library,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-contracts")
    }

    #[test]
    fn project_open_valid() {
        let project = Project::open(fixture_path());
        assert!(project.is_ok());
    }

    #[test]
    fn project_open_invalid() {
        let project = Project::open("/tmp/nonexistent");
        assert!(project.is_err());
    }

    #[test]
    fn project_open_missing_ast_errors() {
        let no_ast =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-contracts-no-ast");
        let err = Project::open(no_ast).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ast = true"));
    }

    #[test]
    fn deployable_contracts_from_fixture() {
        let project = Project::open(fixture_path()).unwrap();
        let contracts = project.deployable_contracts().unwrap();

        let names: Vec<&str> = contracts.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Counter"));
        assert!(names.contains(&"DataStore"));
        assert!(!names.contains(&"AbstractBase"));
        assert!(!names.contains(&"MathLib"));
    }

    #[test]
    fn abstract_contracts_from_fixture() {
        let project = Project::open(fixture_path()).unwrap();
        let abstracts = project.abstract_contracts().unwrap();

        let names: Vec<&str> = abstracts.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"AbstractBase"));
        assert!(!names.contains(&"Counter"));
    }

    #[test]
    fn libraries_from_fixture() {
        let project = Project::open(fixture_path()).unwrap();
        let libs = project.libraries().unwrap();

        let names: Vec<&str> = libs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"MathLib"));
        assert!(!names.contains(&"Counter"));
    }
}
