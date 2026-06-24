//! Lightweight index mapping Solc AST node IDs to artifact paths.
//!
//! [`FunctionIndex`] is built by scanning all artifact files and extracting
//! just the function definition IDs using a minimal serde struct -- no full
//! AST deserialization. This makes it fast even for large flattened files.
//!
//! Artifacts from the same source file are deduplicated during scanning,
//! since all artifacts compiled from the same `.sol` source contain an
//! identical AST.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rayon::prelude::*;
use serde::Deserialize;

use crate::artifact_index::ArtifactIndex;

/// An entry in the function index: the artifact path and source file for a
/// function definition identified by its Solc AST node ID.
#[derive(Debug, Clone)]
pub struct FunctionIndexEntry {
    /// Path to the artifact JSON file containing this function.
    pub artifact_path: PathBuf,
    /// The source file this function was compiled from.
    pub source_file: PathBuf,
}

/// Lightweight index: Solc AST node ID -> artifact path + source file.
///
/// Built by scanning all artifact files and extracting just the function
/// definition IDs. No full AST deserialization is performed, making this
/// fast even for projects with many large flattened artifacts.
#[derive(Debug, Clone)]
pub struct FunctionIndex {
    inner: HashMap<i64, FunctionIndexEntry>,
}

/// Result of scanning a single artifact during index building.
struct ArtifactScan {
    source: PathBuf,
    artifact_path: PathBuf,
    ids: Vec<i64>,
}

impl FunctionIndex {
    /// Build a [`FunctionIndex`] by scanning all artifacts in `artifact_index`.
    ///
    /// Only function definition IDs are extracted (using a minimal serde
    /// struct), avoiding the cost of full AST deserialization. Artifacts
    /// from the same source file are deduplicated.
    #[tracing::instrument(skip_all)]
    pub fn build(artifact_index: &ArtifactIndex) -> Self {
        let artifact_paths: Vec<&PathBuf> = artifact_index.all_entries().collect();
        tracing::trace!(
            total_entries = artifact_paths.len(),
            "building function index"
        );

        // Parallel scan: extract (source, artifact_path, ids) from all
        // artifacts concurrently. No Mutex needed since each task produces
        // independent results.
        let scanned: Vec<ArtifactScan> = artifact_paths
            .par_iter()
            .filter_map(|artifact_path| {
                let (source, ids) = scan_artifact_ids(artifact_path).ok()??;
                Some(ArtifactScan {
                    source,
                    artifact_path: artifact_path.to_path_buf(),
                    ids,
                })
            })
            .collect();

        // Serial dedup: keep only the first artifact per source file, then
        // flatten into the ID -> entry map.
        let mut inner: HashMap<i64, FunctionIndexEntry> = HashMap::with_capacity(scanned.len() * 4);
        let mut seen_sources: HashSet<PathBuf> = HashSet::with_capacity(scanned.len());
        for scan in scanned {
            if !seen_sources.insert(scan.source.clone()) {
                continue;
            }
            for id in scan.ids {
                inner.insert(
                    id,
                    FunctionIndexEntry {
                        artifact_path: scan.artifact_path.clone(), // checkrs: allow(clone_in_loops)
                        source_file: scan.source.clone(),          // checkrs: allow(clone_in_loops)
                    },
                );
            }
        }

        tracing::trace!(total_functions = inner.len(), "function index built");
        Self { inner }
    }

    /// Return the number of indexed function IDs.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Return `true` if the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Look up an artifact entry by function definition ID.
    pub fn get(&self, id: i64) -> Option<&FunctionIndexEntry> {
        self.inner.get(&id)
    }

    /// Return `true` if the given AST node ID is indexed.
    pub fn contains(&self, id: i64) -> bool {
        self.inner.contains_key(&id)
    }
}

impl std::ops::Deref for FunctionIndex {
    type Target = HashMap<i64, FunctionIndexEntry>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// ---- Lightweight ID extraction ----

/// Minimal node: captures `nodeType` and `id` from any AST node.
/// All Solc AST nodes have `nodeType` and `id`.
#[derive(Deserialize)]
struct IdNode {
    #[serde(rename = "nodeType")]
    node_type: String,
    id: i64,
}

/// Minimal contract node: captures child nodes only.
/// `nodeType` is not needed and silently skipped by serde.
#[derive(Deserialize)]
struct ContractNodes {
    #[serde(default)]
    nodes: Vec<IdNode>,
}

/// Minimal source unit: captures source path and top-level nodes.
#[derive(Deserialize)]
struct SourceUnitNodes {
    #[serde(default)]
    nodes: Vec<ContractNodes>,
    #[serde(rename = "absolutePath")]
    absolute_path: Option<PathBuf>,
}

/// Minimal artifact: only deserializes the AST skeleton.
#[derive(Deserialize)]
struct ArtifactSkeleton {
    ast: Option<SourceUnitNodes>,
}

/// Scan a single artifact JSON file, extracting the source file path and
/// all function definition IDs. Uses a minimal serde struct (no full AST).
fn scan_artifact_ids(path: impl AsRef<Path>) -> Result<Option<(PathBuf, Vec<i64>)>> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let artifact: ArtifactSkeleton = serde_json::from_str(&content)?;

    let su = match artifact.ast {
        None => return Ok(None),
        Some(su) => su,
    };

    let source = match su.absolute_path {
        None => return Ok(None),
        Some(p) => p,
    };

    let mut ids = Vec::new();
    for contract in &su.nodes {
        for node in &contract.nodes {
            if node.node_type == "FunctionDefinition" {
                ids.push(node.id);
            }
        }
    }

    if ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some((source, ids)))
    }
}

#[cfg(test)]
mod tests {
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
    fn index_contains_functions_from_all_sources() {
        let project_root = fixture_path();
        let artifact_index = ArtifactIndex::build(project_root.join("out"));
        let function_index = FunctionIndex::build(&artifact_index);
        assert!(function_index.len() > 0);
    }

    #[test]
    fn scan_artifact_extracts_function_ids() {
        let path = fixture_path().join("out/Main.sol/Main.json");
        let result = scan_artifact_ids(&path).unwrap().unwrap();
        let (_source, ids) = result;
        // Main.sol has helper(), execute() - should find at least 3 functions.
        assert!(
            ids.len() >= 3,
            "Expected at least 3 function IDs, got {}",
            ids.len()
        );
    }
}
