//! Storage layout inspection for Foundry projects.
//!
//! [`StorageLayoutInspector`] reads a single artifact file and produces
//! structured output for the storage layout it defines.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::artifact_index::ArtifactIndex;
use crate::project::Project;

/// Identifies a storage layout by contract name and optional source file.
pub struct StorageLayoutId {
    /// The contract name (required).
    pub name: String,
    /// The source file path (optional).
    pub file: Option<String>,
}

impl StorageLayoutId {
    /// Parse a storage layout ID from a string like `Name` or `File.sol:Name`.
    pub fn new(id: &str) -> Self {
        match id.rsplit_once(':') {
            Some((path, name)) if !path.is_empty() && !name.is_empty() => Self {
                name: name.to_string(),
                file: Some(path.to_string()),
            },
            _ => Self {
                name: id.to_string(),
                file: None,
            },
        }
    }
}

/// A single storage entry.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageEntry {
    /// The storage slot label.
    pub label: String,
    /// The storage slot number.
    pub slot: String,
    /// The byte offset within the slot.
    pub offset: u64,
    /// The type identifier reference.
    #[serde(rename = "type")]
    pub type_name: String,
}

/// A storage type definition.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageType {
    /// The human-readable type label.
    pub label: String,
}

/// The parsed storage layout from an artifact.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLayout {
    /// The storage entries.
    pub storage: Vec<StorageEntry>,
    /// The type definitions referenced by storage entries.
    pub types: HashMap<String, StorageType>,
}

/// The output of a [`StorageLayoutInspector`] inspection.
#[derive(Debug)]
pub struct StorageLayoutInspectorOutput {
    storage_layout: StorageLayout,
}

impl StorageLayoutInspectorOutput {
    /// Create a new [`StorageLayoutInspectorOutput`] from a parsed
    /// [`StorageLayout`].
    pub fn new(storage_layout: StorageLayout) -> Self {
        Self { storage_layout }
    }
}

impl std::fmt::Display for StorageLayoutInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.storage_layout.storage.is_empty() {
            return writeln!(f, "No storage slots found.");
        }
        let slot_width = self
            .storage_layout
            .storage
            .iter()
            .map(|e| e.slot.len())
            .max()
            .unwrap_or(0);

        for entry in &self.storage_layout.storage {
            let ty = self
                .storage_layout
                .types
                .get(&entry.type_name)
                .map(|storage_type| storage_type.label.as_str())
                .unwrap_or(entry.type_name.as_str());
            writeln!(
                f,
                "slot {:>width$} offset {} {} {}",
                entry.slot,
                entry.offset,
                entry.label,
                ty,
                width = slot_width
            )?;
        }
        Ok(())
    }
}

/// Inspect a Foundry project for a single contract's storage layout.
pub struct StorageLayoutInspector {
    project: Project,
}

impl StorageLayoutInspector {
    /// Build a [`StorageLayoutInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Return the artifact path relative to the project root for error
    /// messages, falling back to the full path when it lies outside the
    /// project.
    fn display_artifact_path(&self, artifact_path: impl AsRef<Path>) -> String {
        let path = artifact_path.as_ref();
        path.strip_prefix(self.project.path())
            .unwrap_or(path)
            .display()
            .to_string()
    }

    /// Inspect the storage layout for the given [`StorageLayoutId`].
    pub fn inspect(&self, id: &StorageLayoutId) -> Result<StorageLayoutInspectorOutput> {
        let storage_layout = match &id.file {
            Some(file) => self.load_with_file(file, &id.name),
            None => self.load_without_file(&id.name),
        }?;
        Ok(StorageLayoutInspectorOutput::new(storage_layout))
    }

    /// Load the storage layout from a specific artifact path.
    fn load_with_file(&self, file: &str, name: &str) -> Result<StorageLayout> {
        let direct = self
            .project
            .out_dir()
            .join(file)
            .join(format!("{name}.json"));
        let artifact_path = if direct.exists() {
            direct
        } else {
            let index = ArtifactIndex::build(self.project.out_dir());
            index.find_by_source_path(file, name).unwrap_or(direct)
        };

        let artifact = parse_artifact(&artifact_path)?;
        ensure_layout_available(&artifact, &artifact_path, self.project.path(), name)?;
        artifact.storage_layout.with_context(|| {
            format!(
                "artifact `{}` is missing the storage layout; rebuild with `extra_output = [\"storageLayout\"]` in foundry.toml",
                self.display_artifact_path(&artifact_path)
            )
        })
    }

    /// Load the storage layout by indexing all artifacts with the given name.
    fn load_without_file(&self, name: &str) -> Result<StorageLayout> {
        let index = ArtifactIndex::build(self.project.out_dir());
        let candidates = index.get(name).cloned().unwrap_or_default();

        match candidates.len() {
            0 => {
                bail!("\"{name}\" not found.");
            }
            1 => {
                let artifact = parse_artifact(&candidates[0])?;
                ensure_layout_available(&artifact, &candidates[0], self.project.path(), name)?;
                artifact.storage_layout.with_context(|| {
                    format!(
                        "artifact `{}` is missing the storage layout; rebuild with `extra_output = [\"storageLayout\"]` in foundry.toml",
                        self.display_artifact_path(&candidates[0])
                    )
                })
            }
            n => {
                let mut sorted = candidates;
                sorted.sort();

                let mut msg = format!("found {n} \"{name}\"\n\nSelect one of the following:\n");
                for candidate in &sorted {
                    let qualified = ArtifactIndex::qualified_name(candidate, name);
                    msg.push_str(&format!("\nsolray inspect storage-layout {qualified}"));
                }
                msg.push('\n');
                bail!(msg);
            }
        }
    }
}

/// Error when the artifact carries an empty storage layout even though the
/// contract declares storage variables. solc < 0.6 does not emit storage
/// layout output, so this turns a silent empty result into an explicit error.
fn ensure_layout_available(
    artifact: &Artifact,
    path: impl AsRef<Path>,
    base: &Path,
    name: &str,
) -> Result<()> {
    let path = path.as_ref();
    let display = path.strip_prefix(base).unwrap_or(path);
    let has_entries = match artifact.storage_layout.as_ref() {
        Some(layout) => !layout.storage.is_empty(),
        None => false,
    };
    if has_entries {
        return Ok(());
    }
    let declares_storage = match artifact.ast.as_ref() {
        Some(ast) => contract_declares_storage(ast, name),
        None => false,
    };
    if declares_storage {
        bail!(
            "artifact `{}` has an empty storage layout even though the contract declares storage variables; storage layout output is not available for solc < 0.6 builds",
            display.display()
        );
    }
    Ok(())
}

/// Return true when the contract (including its base contracts) declares at
/// least one non-constant state variable.
fn contract_declares_storage(ast: &LightweightSourceUnit, name: &str) -> bool {
    let by_id: HashMap<i64, &LightweightNode> =
        ast.nodes.iter().map(|node| (node.id, node)).collect();
    let Some(contract) = ast
        .nodes
        .iter()
        .find(|node| node.node_type == "ContractDefinition" && node.name.as_deref() == Some(name))
    else {
        return false;
    };

    let mut visited = HashSet::new();
    let mut pending = vec![contract];
    while let Some(node) = pending.pop() {
        if !visited.insert(node.id) {
            continue;
        }
        let declares = node.nodes.iter().any(|child| {
            child.node_type == "VariableDeclaration"
                && child.state_variable
                && !child.constant
                && child.mutability != "immutable"
        });
        if declares {
            return true;
        }
        for base in &node.base_contracts {
            if let Some(base_id) = base.base_name.referenced_declaration
                && let Some(base_node) = by_id.get(&base_id)
            {
                pending.push(base_node);
            }
        }
    }
    false
}

/// Artifact representation that deserializes only the storage layout and the
/// lightweight AST needed to detect storage-declaring contracts.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    storage_layout: Option<StorageLayout>,
    ast: Option<LightweightSourceUnit>,
}

/// Lightweight source unit that only deserializes contract definitions.
#[derive(Deserialize)]
struct LightweightSourceUnit {
    #[serde(default)]
    nodes: Vec<LightweightNode>,
}

/// Lightweight contract node that only deserializes the fields needed to
/// detect storage-declaring contracts.
#[derive(Deserialize)]
struct LightweightNode {
    #[serde(rename = "nodeType")]
    node_type: String,
    id: i64,
    name: Option<String>,
    #[serde(default)]
    nodes: Vec<LightweightChild>,
    #[serde(default, rename = "baseContracts")]
    base_contracts: Vec<LightweightBase>,
}

/// Lightweight contract child that only deserializes variable declaration
/// metadata.
#[derive(Deserialize)]
struct LightweightChild {
    #[serde(rename = "nodeType")]
    node_type: String,
    #[serde(default, rename = "stateVariable")]
    state_variable: bool,
    #[serde(default)]
    constant: bool,
    #[serde(default)]
    mutability: String,
}

/// Lightweight base contract specifier.
#[derive(Deserialize)]
struct LightweightBase {
    #[serde(rename = "baseName")]
    base_name: LightweightTypeName,
}

/// Lightweight user-defined type name.
#[derive(Deserialize)]
struct LightweightTypeName {
    #[serde(rename = "referencedDeclaration")]
    referenced_declaration: Option<i64>,
}

fn parse_artifact(path: impl AsRef<Path>) -> Result<Artifact> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse artifact `{}`", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-storage-layout")
    }

    fn solc_0_5_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-storage-layout-solc-0.5")
    }

    #[test]
    fn inspect_errors_for_empty_layout_with_storage_variables() {
        let inspector = StorageLayoutInspector::new(Project::open(solc_0_5_fixture_path()));
        let id = StorageLayoutId::new("Store");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../fixtures/inspect-storage-layout-solc-0.5/expected/inspect_errors_for_empty_layout_with_storage_variables.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_empty_layout_with_inherited_storage() {
        let inspector = StorageLayoutInspector::new(Project::open(solc_0_5_fixture_path()));
        let id = StorageLayoutId::new("Child");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../fixtures/inspect-storage-layout-solc-0.5/expected/inspect_errors_for_empty_layout_with_inherited_storage.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_no_storage_message_for_storage_less_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(solc_0_5_fixture_path()));
        let id = StorageLayoutId::new("Empty");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../fixtures/inspect-storage-layout-solc-0.5/expected/inspect_shows_no_storage_message_for_storage_less_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_no_storage_message_for_immutable_only_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("ImmutableOnly");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../fixtures/inspect-storage-layout/expected/inspect_shows_no_storage_message_for_immutable_only_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_storage_layout_for_a_unique_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("ContractB");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../fixtures/inspect-storage-layout/expected/inspect_shows_storage_layout_for_a_unique_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_storage_layout_for_path_qualified_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("Foo.sol:ContractA");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../fixtures/inspect-storage-layout/expected/inspect_shows_storage_layout_for_path_qualified_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("Missing");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../fixtures/inspect-storage-layout/expected/inspect_errors_for_unknown_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_ambiguous_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("ContractA");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../fixtures/inspect-storage-layout/expected/inspect_errors_for_ambiguous_contract.txt"
            )
        );
    }

    #[test]
    fn storage_layout_id_parses_name_only() {
        let id = StorageLayoutId::new("MyContract");
        assert_eq!(id.name, "MyContract");
        assert_eq!(id.file, None);
    }

    #[test]
    fn storage_layout_id_parses_file_and_name() {
        let id = StorageLayoutId::new("src/Foo.sol:MyContract");
        assert_eq!(id.name, "MyContract");
        assert_eq!(id.file, Some("src/Foo.sol".to_string()));
    }
}
