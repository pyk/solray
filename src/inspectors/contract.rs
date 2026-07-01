//! Deployable contract inspection for Foundry projects.
//!
//! [`ContractInspector`] scans the artifact directory and produces structured
//! output for each deployable contract found.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::project::Project;
use crate::project::ProjectDirectories;

/// The directory category a [`Contract`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DirCategory {
    Src,
    Lib,
    Test,
    Other,
}

impl DirCategory {
    fn display(self) -> &'static str {
        match self {
            DirCategory::Src => "src",
            DirCategory::Lib => "lib",
            DirCategory::Test => "test",
            DirCategory::Other => "other",
        }
    }
}

/// Classifies artifact paths using the directory layout declared in the
/// project's `foundry.toml`.
struct DirectoryClassifier<'a> {
    directories: &'a ProjectDirectories,
}

impl<'a> DirectoryClassifier<'a> {
    fn new(directories: &'a ProjectDirectories) -> Self {
        Self { directories }
    }

    fn classify(&self, path: impl AsRef<Path>) -> DirCategory {
        let path = path.as_ref();
        if self
            .directories
            .libs
            .iter()
            .any(|lib| path.starts_with(lib))
        {
            DirCategory::Lib
        } else if path.starts_with(&self.directories.src) {
            DirCategory::Src
        } else if path.starts_with(&self.directories.test) {
            DirCategory::Test
        } else {
            DirCategory::Other
        }
    }
}

/// A single deployable contract.
pub struct Contract {
    /// The contract name.
    pub name: String,
    /// The source file path (relative to the project root).
    pub path: PathBuf,
}

impl Contract {
    /// Create a new [`Contract`] from a name and source file.
    fn new(name: &str, source_file: &Path, project_root: &Path) -> Self {
        let rel = source_file
            .strip_prefix(project_root)
            .unwrap_or(source_file);
        Self {
            name: name.to_string(),
            path: rel.to_path_buf(),
        }
    }
}

impl std::fmt::Display for Contract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

/// The output of a [`ContractInspector`] inspection.
pub struct ContractInspectorOutput {
    contracts: Vec<Contract>,
    directories: ProjectDirectories,
}

impl ContractInspectorOutput {
    /// Create a new [`ContractInspectorOutput`] from a list of contracts and
    /// the project's directory configuration.
    pub fn new(contracts: Vec<Contract>, directories: ProjectDirectories) -> Self {
        Self {
            contracts,
            directories,
        }
    }
}

impl std::fmt::Display for ContractInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let groups = self.grouped();

        writeln!(f, "summary:")?;
        for (dir, contracts) in &groups {
            writeln!(
                f,
                "- {} contracts in {} directory",
                contracts.len(),
                dir.display()
            )?;
        }
        writeln!(f, "- total {} contracts", self.contracts.len())?;

        for (dir, contracts) in &groups {
            writeln!(f, "\ncontracts in {} directory:", dir.display())?;
            for (i, contract) in contracts.iter().enumerate() {
                writeln!(
                    f,
                    "{}. {} (file: {})",
                    i + 1,
                    contract.name,
                    contract.path.display()
                )?;
            }
        }

        Ok(())
    }
}

impl ContractInspectorOutput {
    /// Return the contracts grouped by directory category.
    fn grouped(&self) -> BTreeMap<DirCategory, Vec<&Contract>> {
        let classifier = DirectoryClassifier::new(&self.directories);
        let mut groups: BTreeMap<DirCategory, Vec<&Contract>> = BTreeMap::new();
        for contract in &self.contracts {
            let dir = classifier.classify(&contract.path);
            groups.entry(dir).or_default().push(contract);
        }
        groups
    }
}

/// Inspect a Foundry project for deployable contracts.
pub struct ContractInspector {
    project: Project,
}

impl ContractInspector {
    /// Build a [`ContractInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the project and return all deployable contracts.
    pub fn inspect(&self) -> Result<ContractInspectorOutput> {
        self.project.validate()?;

        let project_root_abs = std::path::absolute(self.project.path())?;
        let directories = self.project.directories()?;
        let contracts = self.load_contracts(&project_root_abs)?;
        Ok(ContractInspectorOutput::new(contracts, directories))
    }

    /// Walk the artifact directory and extract only deployable contracts.
    fn load_contracts(&self, project_root: &Path) -> Result<Vec<Contract>> {
        let paths = self.artifact_paths();
        let mut contracts: Vec<Contract> = paths
            .into_par_iter()
            .filter_map(|path| parse_contract(&path, project_root).ok().flatten())
            .collect();
        contracts.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
        Ok(contracts)
    }

    /// Collect all JSON artifact paths from the output directory,
    /// excluding `build-info` files.
    fn artifact_paths(&self) -> Vec<PathBuf> {
        WalkDir::new(self.project.out_dir())
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
}

/// Quick check whether the raw file bytes contain `"contractKind":`.
///
/// This lets us skip JSON deserialization entirely for artifacts that contain
/// no contract definitions at all.
fn might_be_contract(path: impl AsRef<Path>) -> Result<bool> {
    let bytes = fs::read(path)?;
    let pattern = b"\"contractKind\":";
    Ok(bytes.windows(pattern.len()).any(|w| w == pattern))
}

/// Parse a single artifact JSON file, returning [`Some(Contract)`] only if the
/// contract defined in the artifact is deployable.
fn parse_contract(path: impl AsRef<Path>, project_root: &Path) -> Result<Option<Contract>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
    };

    // Fast path: skip files that definitely contain no contract definitions.
    if !might_be_contract(path)? {
        return Ok(None);
    }

    // Slow path: deserializing only the lightweight AST representation skips
    // heavy fields such as bytecode, deployedBytecode, and deep children
    // (function bodies, events, errors) inside contract definitions.
    let content = fs::read_to_string(path)?;
    let artifact: LightweightArtifact = serde_json::from_str(&content)?;

    let ast = artifact.ast.with_context(|| {
        format!(
            "artifact `{}` is missing the AST; rebuild with `ast = true` in foundry.toml",
            path.display()
        )
    })?;

    if let Some(name) = ast
        .nodes
        .into_iter()
        .find_map(|node| node.is_contract(contract_name))
    {
        return Ok(Some(Contract::new(
            &name,
            Path::new(&ast.absolute_path),
            project_root,
        )));
    }

    Ok(None)
}

/// Lightweight artifact representation that deserializes only the AST and
/// skips all heavy fields (bytecode, deployedBytecode, storageLayout, etc.).
#[derive(Deserialize)]
struct LightweightArtifact {
    #[serde(default)]
    ast: Option<LightweightSourceUnit>,
}

/// Lightweight source unit that deserializes only top-level node metadata.
#[derive(Deserialize)]
struct LightweightSourceUnit {
    #[serde(rename = "absolutePath")]
    absolute_path: PathBuf,
    #[serde(default)]
    nodes: Vec<LightweightNode>,
}

/// Lightweight node that only deserializes the fields needed to identify
/// deployable contracts. Heavy children (function bodies, events, errors) are
/// skipped by serde.
#[derive(Deserialize)]
struct LightweightNode {
    #[serde(rename = "nodeType")]
    node_type: String,
    name: Option<String>,
    #[serde(default, rename = "abstract")]
    abstract_fallback: bool,
    #[serde(rename = "contractKind")]
    contract_kind: Option<String>,
}

impl LightweightNode {
    /// Return the contract name if this node is a deployable contract matching
    /// `target_name`.
    fn is_contract(&self, target_name: &str) -> Option<String> {
        if self.node_type != "ContractDefinition" {
            return None;
        }
        if self.contract_kind.as_deref() != Some("contract") {
            return None;
        }
        if self.abstract_fallback {
            return None;
        }
        if self.name.as_deref() != Some(target_name) {
            return None;
        }
        self.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    use crate::project::Project;
    use crate::project::ProjectDirectories;

    fn default_directories() -> ProjectDirectories {
        ProjectDirectories {
            src: PathBuf::from("src"),
            test: PathBuf::from("test"),
            libs: vec![PathBuf::from("lib"), PathBuf::from("node_modules")],
        }
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/contracts")
    }

    #[test]
    fn inspect_returns_only_deployable_contracts() {
        let inspector = ContractInspector::new(Project::open(fixture_path()));
        let output = inspector.inspect().unwrap();
        let expected = include_str!("../../fixtures/contracts/expected/output.txt");
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn contract_display_formats_as_path_colon_name() {
        let file = PathBuf::from("src/Counter.sol");
        let contract = Contract::new("Counter", &file, Path::new(""));
        assert_eq!(contract.to_string(), "src/Counter.sol:Counter");
    }

    #[test]
    fn output_display_formats_structured_by_directory() {
        let src = Contract::new("Counter", &PathBuf::from("src/Counter.sol"), Path::new(""));
        let lib = Contract::new(
            "DependencyContract",
            &PathBuf::from("lib/mylib/src/DependencyContract.sol"),
            Path::new(""),
        );
        let node_modules = Contract::new(
            "ThirdPartyContract",
            &PathBuf::from("node_modules/some-pkg/src/ThirdPartyContract.sol"),
            Path::new(""),
        );
        let test = Contract::new(
            "TestTarget",
            &PathBuf::from("test/TestTarget.sol"),
            Path::new(""),
        );
        let output =
            ContractInspectorOutput::new(vec![test, lib, node_modules, src], default_directories());
        assert_eq!(
            output.to_string(),
            "summary:\n\
- 1 contracts in src directory\n\
- 2 contracts in lib directory\n\
- 1 contracts in test directory\n\
- total 4 contracts\n\
\n\
contracts in src directory:\n\
1. Counter (file: src/Counter.sol)\n\
\n\
contracts in lib directory:\n\
1. DependencyContract (file: lib/mylib/src/DependencyContract.sol)\n\
2. ThirdPartyContract (file: node_modules/some-pkg/src/ThirdPartyContract.sol)\n\
\n\
contracts in test directory:\n\
1. TestTarget (file: test/TestTarget.sol)\n"
        );
    }
}
