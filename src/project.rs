//! Foundry project inspection.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail, ensure};
use rayon::prelude::*;
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

/// The directory configuration of a Foundry project, as declared in
/// `foundry.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDirectories {
    /// The source directory (configured via `src`; defaults to `"src"`).
    pub src: PathBuf,
    /// The test directory (configured via `test`; defaults to `"test"`).
    pub test: PathBuf,
    /// The library directories (configured via `libs`; defaults to
    /// `["lib", "node_modules"]`).
    pub libs: Vec<PathBuf>,
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
    /// Create a [`Project`] handle for the Foundry project at `path`.
    ///
    /// This simply records the project path and the expected `out/` directory.
    /// Call [`validate`](Self::validate) to check that the project is properly
    /// configured (e.g. `foundry.toml` exists and `ast = true` is set).
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let out = path.join("out");
        Project { path, out }
    }

    /// Validate that the project at [`self.path`](Self::path) is a properly
    /// configured Foundry project.
    ///
    /// Checks that `foundry.toml` exists and that `ast = true` is set in the
    /// default profile. This ensures artifacts can be inspected.
    pub fn validate(&self) -> Result<()> {
        let foundry_toml = self.path.join("foundry.toml");

        ensure!(
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

        ensure!(
            ast == Some(true),
            "`ast = true` must be set in the [profile.default] section of {}",
            foundry_toml.display()
        );

        Ok(())
    }

    /// Validate that `storageLayout` is enabled for the default profile.
    pub fn validate_storage_layout(&self) -> Result<()> {
        self.validate()?;

        let foundry_toml = self.path.join("foundry.toml");
        let config: toml::Value = toml::from_str(&fs::read_to_string(&foundry_toml)?)?;

        let storage_layout = config
            .get("profile")
            .and_then(|p| p.get("default"))
            .and_then(|d| d.get("extra_output"))
            .and_then(|extra_output| extra_output.as_array());
        let storage_layout_enabled = match storage_layout {
            Some(extra_output) => extra_output
                .iter()
                .any(|value| value.as_str() == Some("storageLayout")),
            None => false,
        };

        ensure!(
            storage_layout_enabled,
            "`storageLayout` must be set in the [profile.default].extra_output section of {}",
            foundry_toml.display()
        );

        Ok(())
    }

    /// Return the project root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the output directory path.
    pub fn out_dir(&self) -> &Path {
        &self.out
    }

    /// Read the directory configuration from the project's `foundry.toml`.
    ///
    /// Returns the configured `src`, `test`, and `libs` directories,
    /// falling back to Foundry defaults when a field is absent.
    pub fn directories(&self) -> Result<ProjectDirectories> {
        let foundry_toml = self.path.join("foundry.toml");
        let config: toml::Value = toml::from_str(&fs::read_to_string(&foundry_toml)?)?;
        let default_profile = config.get("profile").and_then(|p| p.get("default"));

        let src = default_profile
            .and_then(|d| d.get("src"))
            .and_then(|s| s.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("src"));

        let test = default_profile
            .and_then(|d| d.get("test"))
            .and_then(|t| t.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("test"));

        let libs = default_profile
            .and_then(|d| d.get("libs"))
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(PathBuf::from))
                    .collect()
            })
            .unwrap_or_else(|| vec![PathBuf::from("lib"), PathBuf::from("node_modules")]);

        Ok(ProjectDirectories { src, test, libs })
    }

    /// Collect all JSON artifact paths from the output directory,
    /// excluding `build-info` files.
    fn artifact_paths(&self) -> Vec<PathBuf> {
        WalkDir::new(&self.out)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let p = e.path();
                p.extension().and_then(|s| s.to_str()) == Some("json")
                    && !p.to_string_lossy().contains("build-info")
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    }

    /// Return all declarations found across all artifacts.
    pub fn declarations(&self) -> Result<Vec<Declaration>> {
        if !self.out.exists() {
            return Ok(Vec::new());
        }

        let paths = self.artifact_paths();
        let mut results: Vec<Declaration> = paths
            .into_par_iter()
            .filter_map(|path| process_artifact(path).ok().flatten())
            .collect();
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

    /// Find all declarations matching `name` exactly (case-sensitive).
    ///
    /// This is useful for detecting name collisions where the same declaration
    /// name appears in multiple files (e.g., a dependency and the project itself).
    pub fn find_declarations_by_name(&self, name: &str) -> Result<Vec<Declaration>> {
        let decls = self.declarations()?;
        Ok(decls.into_iter().filter(|d| d.name == name).collect())
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

/// Process a single artifact JSON file, extracting its [`Declaration`] if one
/// is found.
fn process_artifact(path: impl AsRef<Path>) -> Result<Option<Declaration>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
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
        Ok(Some(Declaration {
            name: cd.name,
            kind,
            file: source_file,
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/external-functions")
    }

    #[test]
    fn validate_storage_layout_requires_extra_output() {
        let project = Project::open(fixture_path());
        let err = project.validate_storage_layout().unwrap_err().to_string();
        assert_eq!(
            err,
            format!(
                "`storageLayout` must be set in the [profile.default].extra_output section of {}",
                fixture_path().join("foundry.toml").display()
            )
        );
    }
}
