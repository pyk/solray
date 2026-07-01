//! Abstract contract inspection for Foundry projects.
//!
//! [`AbstractInspector`] scans the artifact directory and produces structured
//! output for each abstract contract found.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::project::Project;

/// A single abstract contract.
pub struct Abstract {
    /// The contract name.
    pub name: String,
    /// The source file path (relative to the project root).
    pub path: PathBuf,
}

impl Abstract {
    /// Create a new [`Abstract`] from a name and source file.
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

impl std::fmt::Display for Abstract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

/// The output of an [`AbstractInspector`] inspection.
pub struct AbstractInspectorOutput {
    abstracts: Vec<Abstract>,
}

impl AbstractInspectorOutput {
    /// Create a new [`AbstractInspectorOutput`] from a list of abstracts.
    pub fn new(abstracts: Vec<Abstract>) -> Self {
        Self { abstracts }
    }
}

impl std::fmt::Display for AbstractInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "found {} abstracts\n", self.abstracts.len())?;
        for (i, abstract_) in self.abstracts.iter().enumerate() {
            writeln!(f, "{}. {}", i + 1, abstract_)?;
        }
        Ok(())
    }
}

/// Inspect a Foundry project for abstract contracts.
pub struct AbstractInspector {
    project: Project,
}

impl AbstractInspector {
    /// Build an [`AbstractInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the project and return all abstract contracts.
    pub fn inspect(&self) -> Result<AbstractInspectorOutput> {
        self.project.validate()?;

        let project_root_abs = std::path::absolute(self.project.path())?;
        let abstracts = self.load_abstracts(&project_root_abs)?;
        Ok(AbstractInspectorOutput::new(abstracts))
    }

    /// Walk the artifact directory and extract only abstract contracts.
    fn load_abstracts(&self, project_root: &Path) -> Result<Vec<Abstract>> {
        let paths = self.artifact_paths();
        let abstracts: Vec<Abstract> = paths
            .into_par_iter()
            .filter_map(|path| parse_abstract(&path, project_root).ok().flatten())
            .collect();
        Ok(abstracts)
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

/// Quick check whether the raw file bytes contain `"abstract":true`.
///
/// This lets us skip JSON deserialization entirely for the vast majority of
/// artifacts that contain no abstract contracts.
fn might_be_abstract(path: impl AsRef<Path>) -> Result<bool> {
    let bytes = fs::read(path)?;
    let pattern = b"\"abstract\":";
    Ok(bytes.windows(pattern.len()).any(|w| w == pattern))
}

/// Parse a single artifact JSON file, returning [`Some(Abstract)`] only if the
/// contract defined in the artifact is abstract.
fn parse_abstract(path: impl AsRef<Path>, project_root: &Path) -> Result<Option<Abstract>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
    };

    // Fast path: skip files that definitely contain no abstract contracts.
    if !might_be_abstract(path)? {
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
        .find_map(|n| n.is_abstract(contract_name))
    {
        return Ok(Some(Abstract::new(
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
/// abstract contracts. Heavy children (function bodies, events, errors) are
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
    /// Return the contract name if this node is an abstract contract matching
    /// `target_name`.
    fn is_abstract(&self, target_name: &str) -> Option<String> {
        if self.node_type != "ContractDefinition" {
            return None;
        }
        if self.contract_kind.as_deref() != Some("contract") {
            return None;
        }
        if !self.abstract_fallback {
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
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_project() -> Project {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/contracts");
        Project::open(path)
    }

    #[test]
    fn inspect_returns_abstract_contracts_only() {
        let inspector = AbstractInspector::new(fixture_project());
        let output = inspector.inspect().unwrap();

        assert_eq!(
            output
                .abstracts
                .iter()
                .map(|a: &Abstract| a.to_string())
                .collect::<Vec<String>>(),
            vec!["src/AbstractBase.sol:AbstractBase"]
        );
    }

    #[test]
    fn abstract_display_formats_as_path_colon_name() {
        let file = PathBuf::from("src/AbstractBase.sol");
        let abstract_ = Abstract::new("AbstractBase", &file, Path::new(""));
        assert_eq!(abstract_.to_string(), "src/AbstractBase.sol:AbstractBase");
    }

    #[test]
    fn output_display_numbers_each_abstract() {
        let file = PathBuf::from("src/AbstractBase.sol");
        let abstract_ = Abstract::new("AbstractBase", &file, Path::new(""));
        let output = AbstractInspectorOutput::new(vec![abstract_]);
        assert_eq!(
            output.to_string(),
            "found 1 abstracts\n\n1. src/AbstractBase.sol:AbstractBase\n"
        );
    }
}
