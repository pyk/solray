//! Pre-built index mapping AST node IDs to function information.
//!
//! [`FunctionIndex`] is built eagerly by parsing all artifact JSON files,
//! replacing the slow lazy-loading pattern in call graph resolution. Looking
//! up a function by its Solc AST node ID becomes O(1).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rayon::prelude::*;
use serde::Deserialize;
use solc::ast::{
    ContractDefinitionNode, FunctionDefinition, SourceUnitNode, VariableDeclaration, Visibility,
};

use crate::artifact_index::{ArtifactEntry, ArtifactIndex};

/// Function information extracted from an artifact AST for call graph resolution.
#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub id: i64,
    pub name: String,
    pub contract_name: String,
    pub file: PathBuf,
    pub parameters: Vec<VariableDeclaration>,
    pub visibility: Visibility,
    pub definition: FunctionDefinition,
}

/// Minimal artifact wrapper for extracting the AST on demand.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<solc::ast::SourceUnit>,
}

/// A pre-built index mapping Solc AST node IDs to [`FunctionInfo`].
///
/// Built eagerly by parsing all artifact files in a Foundry project. This
/// makes lookups O(1) instead of the previous pattern where each unresolved
/// call required a linear scan through all artifacts.
#[derive(Debug, Clone)]
pub struct FunctionIndex {
    inner: HashMap<i64, FunctionInfo>,
}

impl FunctionIndex {
    /// Build a [`FunctionIndex`] by parsing all artifacts in `artifact_index`
    /// in parallel using rayon.
    #[tracing::instrument(skip_all)]
    pub fn build(artifact_index: &ArtifactIndex) -> Self {
        let entries: Vec<&ArtifactEntry> = artifact_index.all_entries().collect();
        tracing::trace!(total_entries = entries.len(), "building function index");

        let results: Vec<Vec<FunctionInfo>> = entries
            .par_iter()
            .filter_map(|entry| process_artifact_for_functions(&entry.path).ok().flatten())
            .collect();

        let mut inner: HashMap<i64, FunctionInfo> = HashMap::new();
        for funcs in results {
            for fi in funcs {
                inner.insert(fi.id, fi);
            }
        }

        tracing::trace!(total_functions = inner.len(), "function index built");
        Self { inner }
    }

    /// Return the number of indexed functions.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Return `true` if the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Look up a function by its Solc AST node ID.
    pub fn get(&self, id: i64) -> Option<&FunctionInfo> {
        self.inner.get(&id)
    }

    /// Return `true` if the given AST node ID is indexed.
    pub fn contains(&self, id: i64) -> bool {
        self.inner.contains_key(&id)
    }
}

impl std::ops::Deref for FunctionIndex {
    type Target = HashMap<i64, FunctionInfo>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Process a single artifact JSON file, returning all [`FunctionInfo`] entries
/// found across all contracts in the AST.
#[tracing::instrument(skip_all)]
fn process_artifact_for_functions(path: impl AsRef<Path>) -> Result<Option<Vec<FunctionInfo>>> {
    let path = path.as_ref();
    tracing::trace!(?path);

    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)?;

    let ast = match artifact.ast {
        None => return Ok(None),
        Some(ast) => ast,
    };

    let source_file = ast.absolute_path;
    let mut functions = Vec::new();

    for node in ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node {
            functions.extend(extract_contract_functions(cd, &source_file));
        }
    }

    if functions.is_empty() {
        Ok(None)
    } else {
        Ok(Some(functions))
    }
}

/// Extract all implemented functions from a contract definition.
fn extract_contract_functions(
    cd: solc::ast::ContractDefinition,
    source_file: &Path,
) -> Vec<FunctionInfo> {
    let contract_name = cd.name;
    let file = source_file.to_path_buf();
    cd.nodes
        .into_iter()
        .filter_map(|inner| {
            let ContractDefinitionNode::FunctionDefinition(fd) = inner else {
                return None;
            };
            if !fd.implemented {
                return None;
            }
            Some(FunctionInfo {
                id: fd.id,
                name: fd.name.clone(),
                contract_name: contract_name.clone(),
                file: file.clone(),
                parameters: fd.parameters.parameters.clone(),
                visibility: fd.visibility.clone(),
                definition: fd,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/calls")
    }

    #[test]
    fn index_builds_for_calls_fixture() {
        let project_root = fixture_path();
        let artifact_index = ArtifactIndex::build(project_root.join("out"));
        let function_index = FunctionIndex::build(&artifact_index);
        assert!(function_index.len() > 0);
    }

    #[test]
    fn index_contains_functions_from_all_contracts() {
        let project_root = fixture_path();
        let artifact_index = ArtifactIndex::build(project_root.join("out"));
        let function_index = FunctionIndex::build(&artifact_index);

        // FunctionInfo entries should include contract_name from all contracts.
        let contract_names: HashSet<&str> = function_index
            .values()
            .map(|fi| fi.contract_name.as_str())
            .collect();
        assert!(contract_names.contains("Main"));
        assert!(contract_names.contains("Helper"));
    }
}
