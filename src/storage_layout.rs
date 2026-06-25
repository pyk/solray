//! Storage layout resolution for Solidity contracts.
//!
//! [`StorageLayoutResolver`] resolves a deployable contract by name and emits the
//! formatted storage layout from the artifact `storageLayout` data.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use solc::ast::SourceUnit;

use crate::artifact_index::ArtifactIndex;
use crate::project::{Declaration, Project};

/// Resolves the storage layout for a deployable contract.
pub struct StorageLayoutResolver {
    project: Project,
    artifact_index: ArtifactIndex,
}

impl StorageLayoutResolver {
    /// Build a [`StorageLayoutResolver`] for the given project.
    pub fn new(project: Project) -> Self {
        let artifact_index = ArtifactIndex::build(project.out_dir());
        Self {
            project,
            artifact_index,
        }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Resolve a deployable contract and return the formatted storage layout.
    pub fn resolve(&self, deployable: &str) -> Result<String> {
        self.project.validate_storage_layout()?;
        let project_root_abs = std::path::absolute(self.project.path())?;
        let (name, maybe_file) = parse_target(deployable);
        let deployables = self.project.deployable_contracts()?;
        let found: Vec<Declaration> = deployables.into_iter().filter(|d| d.name == name).collect();

        let declaration = match (maybe_file, found.len()) {
            (Some(file_path), _) => found
                .iter()
                .find(|d| rel_path_str(&d.file, &project_root_abs) == file_path)
                .with_context(|| {
                    format!(
                        "\"{}\" not found.\n\nAvailable matches for \"{}\":",
                        deployable, name
                    )
                })?,
            (None, 0) => {
                let names = self.available_contract_names()?;
                bail!(
                    "\"{}\" not found.\n\nAvailable contracts: {}",
                    deployable,
                    names.join(", ")
                );
            }
            (None, 1) => &found[0],
            (None, n) => {
                let mut found = found;
                found.sort_by(|a, b| a.file.cmp(&b.file));

                let mut msg = format!("found {} \"{}\"\n\nSelect one of the following:\n", n, name);
                for d in &found {
                    let rp = rel_path_str(&d.file, &project_root_abs);
                    msg.push_str(&format!("\nhawk inspect storages {}:{}", rp, d.name));
                }
                msg.push('\n');
                bail!(msg);
            }
        };

        let artifact = self.load_contract_artifact(declaration)?;
        let storages: Vec<String> = artifact
            .storage_layout
            .storage
            .iter()
            .map(|storage| format_storage(storage, &artifact.storage_layout.types))
            .collect();

        Ok(format_output(&storages))
    }

    fn available_contract_names(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .project
            .deployable_contracts()?
            .into_iter()
            .map(|d| d.name)
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    fn load_contract_artifact(&self, declaration: &Declaration) -> Result<ContractArtifact> {
        let candidates = self.artifact_index.try_get(&declaration.name)?;

        if candidates.len() == 1 {
            let artifact = load_artifact(&candidates[0])?;
            let storage_layout = artifact.storage_layout.with_context(|| {
                format!(
                    "artifact `{}` is missing the storage layout; rebuild with `extra_output = [\"storageLayout\"]` in foundry.toml",
                    candidates[0].display()
                )
            })?;
            return Ok(ContractArtifact { storage_layout });
        }

        for candidate in candidates {
            let artifact = load_artifact(&candidate)?;
            if let Some(ast) = &artifact.ast
                && ast.absolute_path == declaration.file
            {
                let storage_layout = artifact.storage_layout.with_context(|| {
                    format!(
                        "artifact `{}` is missing the storage layout; rebuild with `extra_output = [\"storageLayout\"]` in foundry.toml",
                        candidate.display()
                    )
                })?;
                return Ok(ContractArtifact { storage_layout });
            }
        }

        bail!(
            "unable to resolve artifact for `{}` in `{}`",
            declaration.name,
            declaration.file.display()
        )
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    ast: Option<SourceUnit>,
    storage_layout: Option<StorageLayout>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageLayout {
    storage: Vec<StorageEntry>,
    types: HashMap<String, StorageType>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageEntry {
    label: String,
    slot: String,
    offset: u64,
    #[serde(rename = "type")]
    type_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageType {
    label: String,
}

struct ContractArtifact {
    storage_layout: StorageLayout,
}

fn load_artifact(path: impl AsRef<Path>) -> Result<Artifact> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn format_storage(storage: &StorageEntry, types: &HashMap<String, StorageType>) -> String {
    let ty = types
        .get(&storage.type_name)
        .map(|storage_type| storage_type.label.as_str())
        .unwrap_or(storage.type_name.as_str());

    format!(
        "{} (slot {}, offset {}, type {})",
        storage.label, storage.slot, storage.offset, ty
    )
}

fn format_output(storages: &[String]) -> String {
    let mut output = String::new();
    output.push_str(&format!("Found {} storages\n\n", storages.len()));
    for (i, storage) in storages.iter().enumerate() {
        output.push_str(&format!("{}. {}\n", i + 1, storage));
    }
    output
}

/// Split `deployable` into (name, optional_file_path).
fn parse_target(deployable: &str) -> (&str, Option<&str>) {
    match deployable.rsplit_once(':') {
        Some((path, name)) if !path.is_empty() && !name.is_empty() => (name, Some(path)),
        _ => (deployable, None),
    }
}

fn rel_path_str<'a>(file: &'a Path, project_root: &Path) -> Cow<'a, str> {
    file.strip_prefix(project_root)
        .unwrap_or(file)
        .to_string_lossy()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/storages")
    }

    fn entrypoints_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/entrypoints")
    }

    #[test]
    fn resolve_shows_storage_layout_for_a_unique_contract() {
        let project = Project::open(fixture_path());
        let resolver = StorageLayoutResolver::new(project);
        let result = resolver.resolve("ContractB").unwrap();
        assert_eq!(
            result,
            "\
Found 1 storages\n\n1. active (slot 0, offset 0, type bool)\n"
        );
    }

    #[test]
    fn resolve_shows_storage_layout_for_path_qualified_contract() {
        let project = Project::open(fixture_path());
        let resolver = StorageLayoutResolver::new(project);
        let result = resolver.resolve("src/Foo.sol:ContractA").unwrap();
        assert_eq!(
            result,
            "\
Found 2 storages\n\n1. count (slot 0, offset 0, type uint256)\n2. owner (slot 1, offset 0, type address)\n"
        );
    }

    #[test]
    fn resolve_errors_for_unknown_contract() {
        let project = Project::open(fixture_path());
        let resolver = StorageLayoutResolver::new(project);
        let result = resolver.resolve("Missing");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\
\"Missing\" not found.\n\nAvailable contracts: ContractA, ContractB"
        );
    }

    #[test]
    fn resolve_errors_for_ambiguous_contract() {
        let project = Project::open(fixture_path());
        let resolver = StorageLayoutResolver::new(project);
        let result = resolver.resolve("ContractA");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\
found 2 \"ContractA\"\n\nSelect one of the following:\n\nhawk inspect storages src/Bar.sol:ContractA\nhawk inspect storages src/Foo.sol:ContractA\n"
        );
    }

    #[test]
    fn validate_storage_layout_requires_extra_output() {
        let project = Project::open(entrypoints_fixture_path());
        let err = project.validate_storage_layout().unwrap_err().to_string();
        assert_eq!(
            err,
            format!(
                "`storageLayout` must be set in the [profile.default].extra_output section of {}",
                entrypoints_fixture_path().join("foundry.toml").display()
            )
        );
    }
}
