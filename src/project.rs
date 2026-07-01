//! Foundry project inspection.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rayon::prelude::*;
use serde::Deserialize;
use solc::ast::{ContractDefinition, ContractKind, SourceUnit, SourceUnitNode};
use walkdir::WalkDir;

use crate::inheritance::InheritanceNode;

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

/// Internal: contract info extracted from an artifact AST for inheritance resolution.
#[derive(Debug, Clone)]
struct ContractInfo {
    name: String,
    file: PathBuf,
    base_contracts: Vec<String>,
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

    /// Return only libraries.
    pub fn libraries(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::Library)
                .collect()
        })
    }

    /// Return only interfaces.
    pub fn interfaces(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::Interface)
                .collect()
        })
    }

    /// Find a declaration by exact name match (case-sensitive).
    pub fn find_declaration(&self, name: &str) -> Result<Option<Declaration>> {
        let decls = self.declarations()?;
        Ok(decls.into_iter().find(|d| d.name == name))
    }

    /// Find all declarations matching `name` exactly (case-sensitive).
    ///
    /// This is useful for detecting name collisions where the same declaration
    /// name appears in multiple files (e.g., a dependency and the project itself).
    pub fn find_declarations_by_name(&self, name: &str) -> Result<Vec<Declaration>> {
        let decls = self.declarations()?;
        Ok(decls.into_iter().filter(|d| d.name == name).collect())
    }

    /// Build the inheritance tree for a contract identified by name.
    ///
    /// Returns an [`InheritanceNode`] where `name` and `file` represent the
    /// root contract and `parents` contains the resolved base contracts
    /// recursively.
    pub fn inheritance_tree(&self, name: &str) -> Result<InheritanceNode> {
        let infos = self.load_contract_infos()?;
        let by_name: HashMap<&str, &ContractInfo> =
            infos.iter().map(|ci| (ci.name.as_str(), ci)).collect();

        let mut visited: HashSet<&str> = HashSet::new();
        build_tree(name, &by_name, &mut visited)
    }

    /// Build the inheritance tree for a contract identified by name and file path.
    ///
    /// Unlike [`inheritance_tree`], this disambiguates which contract to use
    /// as the root when multiple contracts share the same name. Base contracts
    /// are still resolved by name alone.
    pub fn inheritance_tree_by_path(
        &self,
        name: &str,
        file_path: impl AsRef<Path>,
    ) -> Result<InheritanceNode> {
        let file_path = file_path.as_ref();
        let infos = self.load_contract_infos()?;
        let by_name: HashMap<&str, &ContractInfo> =
            infos.iter().map(|ci| (ci.name.as_str(), ci)).collect();

        let root_info = infos
            .iter()
            .find(|ci| ci.name == name && ci.file == file_path)
            .with_context(|| {
                format!("contract `{}` not found in `{}`", name, file_path.display())
            })?;

        let mut visited: HashSet<&str> = HashSet::new();
        visited.insert(&root_info.name);

        let parents: Vec<InheritanceNode> = root_info
            .base_contracts
            .iter()
            .map(|base_name| build_tree(base_name, &by_name, &mut visited))
            .collect::<Result<Vec<InheritanceNode>>>()?;

        Ok(InheritanceNode {
            name: root_info.name.clone(),
            file: root_info.file.clone(),
            parents,
        })
    }

    /// Load contract info (name, file, base_contracts) from all artifacts.
    fn load_contract_infos(&self) -> Result<Vec<ContractInfo>> {
        if !self.out.exists() {
            return Ok(Vec::new());
        }
        let paths = self.artifact_paths();
        let results: Vec<ContractInfo> = paths
            .into_par_iter()
            .filter_map(|path| process_artifact_for_inheritance(path).ok().flatten())
            .collect();
        Ok(results)
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

/// Extract the base contract names from a list of inheritance specifiers.
fn base_contract_names(base_contracts: Vec<solc::ast::InheritanceSpecifier>) -> Vec<String> {
    base_contracts
        .into_iter()
        .map(|bc| bc.base_name.name)
        .collect()
}

/// Process a single artifact JSON file, returning a [`ContractInfo`] if a
/// contract definition is found.
fn process_artifact_for_inheritance(path: impl AsRef<Path>) -> Result<Option<ContractInfo>> {
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
        let bases = base_contract_names(cd.base_contracts);
        Ok(Some(ContractInfo {
            name: cd.name,
            file: source_file,
            base_contracts: bases,
        }))
    } else {
        Ok(None)
    }
}

/// Recursively build an [`InheritanceNode`] tree from a contract name.
fn build_tree<'a>(
    name: &'a str,
    by_name: &HashMap<&str, &'a ContractInfo>,
    visited: &mut HashSet<&'a str>,
) -> Result<InheritanceNode> {
    ensure!(
        visited.insert(name),
        "circular inheritance detected for `{}`",
        name
    );

    let info = by_name
        .get(name)
        .with_context(|| format!("contract `{}` not found in artifacts", name))?;

    let parents: Vec<InheritanceNode> = info
        .base_contracts
        .iter()
        .map(|base_name| build_tree(base_name, by_name, visited))
        .collect::<Result<Vec<InheritanceNode>>>()?;

    Ok(InheritanceNode {
        name: info.name.clone(),
        file: info.file.clone(),
        parents,
    })
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
