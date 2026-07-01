//! Storage layout inspection for Foundry projects.
//!
//! [`StorageLayoutInspector`] reads a single artifact file and produces
//! structured output for the storage layout it defines.

use std::collections::HashMap;
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
        writeln!(f, "Found {} storages\n", self.storage_layout.storage.len())?;
        for (i, entry) in self.storage_layout.storage.iter().enumerate() {
            let ty = self
                .storage_layout
                .types
                .get(&entry.type_name)
                .map(|storage_type| storage_type.label.as_str())
                .unwrap_or(entry.type_name.as_str());
            writeln!(
                f,
                "{}. {} (slot {}, offset {}, type {})",
                i + 1,
                entry.label,
                entry.slot,
                entry.offset,
                ty
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
        let artifact_path = self
            .project
            .out_dir()
            .join(file)
            .join(format!("{name}.json"));

        let artifact = parse_artifact(&artifact_path)?;
        artifact.storage_layout.with_context(|| {
            format!(
                "artifact `{}` is missing the storage layout; rebuild with `extra_output = [\"storageLayout\"]` in foundry.toml",
                artifact_path.display()
            )
        })
    }

    /// Load the storage layout by indexing all artifacts with the given name.
    fn load_without_file(&self, name: &str) -> Result<StorageLayout> {
        let index = ArtifactIndex::build(self.project.out_dir());
        let candidates = index.get(name).cloned().unwrap_or_default();

        match candidates.len() {
            0 => {
                let mut names: Vec<String> = index
                    .keys()
                    .filter(|k| k.as_str() != name)
                    .cloned()
                    .collect();
                names.sort();
                bail!(
                    "\"{name}\" not found.\n\nAvailable contracts: {}",
                    names.join(", ")
                );
            }
            1 => {
                let artifact = parse_artifact(&candidates[0])?;
                artifact.storage_layout.with_context(|| {
                    format!(
                        "artifact `{}` is missing the storage layout; rebuild with `extra_output = [\"storageLayout\"]` in foundry.toml",
                        candidates[0].display()
                    )
                })
            }
            n => {
                let mut sorted = candidates;
                sorted.sort();

                let mut msg = format!("found {n} \"{name}\"\n\nSelect one of the following:\n");
                for candidate in &sorted {
                    let parent = candidate
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    msg.push_str(&format!("\nhawk inspect storage-layout {parent}:{name}"));
                }
                msg.push('\n');
                bail!(msg);
            }
        }
    }
}

/// Artifact representation that deserializes only the storage layout.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    storage_layout: Option<StorageLayout>,
}

fn parse_artifact(path: impl AsRef<Path>) -> Result<Artifact> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/storages")
    }

    #[test]
    fn inspect_shows_storage_layout_for_a_unique_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("ContractB");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Found 1 storages\n\n1. active (slot 0, offset 0, type bool)\n"
        );
    }

    #[test]
    fn inspect_shows_storage_layout_for_path_qualified_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("Foo.sol:ContractA");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Found 2 storages\n\n1. count (slot 0, offset 0, type uint256)\n2. owner (slot 1, offset 0, type address)\n"
        );
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("Missing");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            "\"Missing\" not found.\n\nAvailable contracts: ContractA, ContractB"
        );
    }

    #[test]
    fn inspect_errors_for_ambiguous_contract() {
        let inspector = StorageLayoutInspector::new(Project::open(fixture_path()));
        let id = StorageLayoutId::new("ContractA");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            "found 2 \"ContractA\"\n\nSelect one of the following:\n\nhawk inspect storage-layout Bar.sol:ContractA\nhawk inspect storage-layout Foo.sol:ContractA\n"
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
