//! External function inspection for Foundry projects.
//!
//! [`ExternalFunctionInspector`] reads a single artifact file and produces
//! structured output for every external function it exposes.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use solc::abi::{Abi, AbiItem, Function};

use crate::artifact_index::ArtifactIndex;
use crate::inspectors::artifact_id::ArtifactId;
use crate::project::Project;

fn external_function_signature(function: &Function) -> String {
    format!(
        "{}({})",
        function.name,
        function
            .inputs
            .iter()
            .map(|p| p.r#type.as_str())
            .collect::<Vec<&str>>()
            .join(",")
    )
}

/// The output of an [`ExternalFunctionInspector`] inspection.
#[derive(Debug)]
pub struct ExternalFunctionInspectorOutput {
    functions: Vec<String>,
}

impl ExternalFunctionInspectorOutput {
    /// Create a new [`ExternalFunctionInspectorOutput`] from a list of
    /// external function signatures.
    pub fn new(functions: Vec<String>) -> Self {
        Self { functions }
    }
}

impl std::fmt::Display for ExternalFunctionInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Found {} external functions", self.functions.len())?;
        writeln!(f)?;
        for (i, func) in self.functions.iter().enumerate() {
            writeln!(f, "{}. {}", i + 1, func)?;
        }
        Ok(())
    }
}

/// Inspect a Foundry project for a single contract's external functions.
pub struct ExternalFunctionInspector {
    project: Project,
}

impl ExternalFunctionInspector {
    /// Build an [`ExternalFunctionInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the external functions for the given [`ArtifactId`].
    pub fn inspect(&self, id: &ArtifactId) -> Result<ExternalFunctionInspectorOutput> {
        let abi = match &id.file {
            Some(file) => self.load_with_file(file, &id.name),
            None => self.load_without_file(&id.name),
        }?;

        let functions: Vec<String> = abi
            .items
            .iter()
            .filter_map(|item| match item {
                AbiItem::Function(function) => Some(external_function_signature(function)),
                _ => None,
            })
            .collect();

        Ok(ExternalFunctionInspectorOutput::new(functions))
    }

    /// Load the ABI from a specific artifact path.
    fn load_with_file(&self, file: &str, name: &str) -> Result<Abi> {
        let artifact_path = self
            .project
            .out_dir()
            .join(file)
            .join(format!("{name}.json"));

        let artifact = parse_artifact(&artifact_path)?;
        artifact
            .abi
            .with_context(|| format!("artifact `{}` is missing the ABI", artifact_path.display()))
    }

    /// Load the ABI by indexing all artifacts with the given name.
    fn load_without_file(&self, name: &str) -> Result<Abi> {
        let index = ArtifactIndex::build(self.project.out_dir());
        let candidates = index.get(name).cloned().unwrap_or_default();

        match candidates.len() {
            0 => {
                let mut names: Vec<String> = index.keys().cloned().collect();
                names.sort();
                bail!(
                    "\"{name}\" not found.\n\nAvailable contracts: {}",
                    names.join(", ")
                );
            }
            1 => {
                let artifact = parse_artifact(&candidates[0])?;
                artifact.abi.with_context(|| {
                    format!("artifact `{}` is missing the ABI", candidates[0].display())
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
                    msg.push_str(&format!(
                        "\nhawk inspect external-functions {parent}:{name}"
                    ));
                }
                msg.push('\n');
                bail!(msg);
            }
        }
    }
}

/// Artifact representation that deserializes only the ABI.
#[derive(Deserialize)]
struct Artifact {
    abi: Option<Abi>,
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/entrypoints")
    }

    #[test]
    fn inspect_shows_external_functions_for_a_unique_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ContractB");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Found 3 external functions\n\n1. charge()\n2. count()\n3. update(address)\n"
        );
    }

    #[test]
    fn inspect_shows_external_functions_for_path_qualified_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Foo.sol:ContractA");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Found 3 external functions\n\n1. entrypointOne(string)\n2. payMe()\n3. readOnly()\n"
        );
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Missing");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            "\"Missing\" not found.\n\nAvailable contracts: ContractA, ContractB"
        );
    }

    #[test]
    fn inspect_errors_for_ambiguous_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ContractA");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            "found 2 \"ContractA\"\n\nSelect one of the following:\n\nhawk inspect external-functions Bar.sol:ContractA\nhawk inspect external-functions Foo.sol:ContractA\n"
        );
    }

    #[test]
    fn artifact_id_parses_name_only() {
        let id = ArtifactId::new("MyContract");
        assert_eq!(id.name, "MyContract");
        assert_eq!(id.file, None);
    }

    #[test]
    fn artifact_id_parses_file_and_name() {
        let id = ArtifactId::new("Foo.sol:MyContract");
        assert_eq!(id.name, "MyContract");
        assert_eq!(id.file, Some("Foo.sol".to_string()));
    }
}
