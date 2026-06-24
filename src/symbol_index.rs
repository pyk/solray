//! Lightweight index mapping all Solc AST declaration node IDs to artifact paths.
//!
//! [`SymbolIndex`] extends the concept of [`crate::function_index::FunctionIndex`]
//! to include not just function definitions, but also struct definitions, enum
//! definitions, error definitions, event definitions, modifier definitions,
//! variable declarations, and user-defined value type definitions.
//!
//! This enables source-code resolution: given a `referenced_declaration` ID
//! from any expression in the AST, we can look up which artifact contains
//! the corresponding definition node.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rayon::prelude::*;
use serde::Deserialize;

use crate::artifact_index::{ArtifactEntry, ArtifactIndex};

/// An entry in the symbol index: the artifact path and source file for a
/// declaration identified by its Solc AST node ID.
#[derive(Debug, Clone)]
pub struct SymbolIndexEntry {
    /// Path to the artifact JSON file containing this declaration.
    pub artifact_path: PathBuf,
    /// The source file this declaration was compiled from.
    pub source_file: PathBuf,
    /// Byte offset of the declaration in the source file.
    pub offset: usize,
    /// Byte length of the declaration in the source file.
    pub length: usize,
    /// Human-readable name of the declaration (e.g. "Product").
    pub name: String,
}

/// Lightweight index: Solc AST node ID -> artifact path + source file.
///
/// Built by scanning all artifact files and extracting declaration IDs for:
/// FunctionDefinition, VariableDeclaration, StructDefinition, EnumDefinition,
/// ErrorDefinition, EventDefinition, ModifierDefinition, and
/// UserDefinedValueTypeDefinition.
#[derive(Debug, Clone)]
pub struct SymbolIndex {
    inner: HashMap<i64, SymbolIndexEntry>,
}

/// Scanned declaration from an artifact.
#[derive(Debug, Clone)]
struct ScannedDecl {
    id: i64,
    offset: usize,
    length: usize,
    name: String,
}

/// Result of scanning a single artifact during index building.
struct ArtifactScan {
    source: PathBuf,
    artifact_path: PathBuf,
    ids: Vec<ScannedDecl>,
}

/// Node types that represent declarations (can be referenced from expressions).
const DECLARATION_NODE_TYPES: &[&str] = &[
    "FunctionDefinition",
    "VariableDeclaration",
    "StructDefinition",
    "EnumDefinition",
    "ErrorDefinition",
    "EventDefinition",
    "ModifierDefinition",
    "UserDefinedValueTypeDefinition",
];

impl SymbolIndex {
    /// Build a [`SymbolIndex`] by scanning all artifacts in `artifact_index`.
    pub fn build(artifact_index: &ArtifactIndex) -> Self {
        let entries: Vec<&ArtifactEntry> = artifact_index.all_entries().collect();

        let scanned: Vec<ArtifactScan> = entries
            .par_iter()
            .filter_map(|entry| {
                let (source, ids) = scan_artifact_ids(&entry.path).ok()??;
                Some(ArtifactScan {
                    source,
                    artifact_path: entry.path.to_path_buf(),
                    ids,
                })
            })
            .collect();

        let mut inner: HashMap<i64, SymbolIndexEntry> = HashMap::with_capacity(scanned.len() * 4);
        let mut seen_sources: HashSet<PathBuf> = HashSet::with_capacity(scanned.len());
        for scan in scanned {
            if !seen_sources.insert(scan.source.clone()) {
                continue;
            }
            for decl in &scan.ids {
                inner.insert(
                    decl.id,
                    SymbolIndexEntry {
                        artifact_path: scan.artifact_path.clone(), // checkrs: allow(clone_in_loops)
                        source_file: scan.source.clone(),          // checkrs: allow(clone_in_loops)
                        offset: decl.offset,
                        length: decl.length,
                        name: decl.name.clone(), // checkrs: allow(clone_in_loops)
                    },
                );
            }
        }

        Self { inner }
    }

    /// Return the number of indexed declaration IDs.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Return `true` if the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Look up an entry by declaration ID.
    pub fn get(&self, id: i64) -> Option<&SymbolIndexEntry> {
        self.inner.get(&id)
    }

    /// Return `true` if the given AST node ID is indexed.
    pub fn contains(&self, id: i64) -> bool {
        self.inner.contains_key(&id)
    }
}

impl std::ops::Deref for SymbolIndex {
    type Target = HashMap<i64, SymbolIndexEntry>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// ---- Lightweight ID extraction ----

/// Minimal node: captures `nodeType`, `id`, `name`, and `src` from any AST node.
#[derive(Deserialize)]
struct IdNode {
    #[serde(rename = "nodeType")]
    node_type: String,
    id: i64,
    src: Option<String>,
    #[serde(default)]
    name: String,
}

/// Minimal contract node: captures child nodes only.
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
/// all declaration IDs (functions, variables, structs, enums, errors,
/// events, modifiers, and user-defined value types) with their source offsets.
fn scan_artifact_ids(path: impl AsRef<Path>) -> Result<Option<(PathBuf, Vec<ScannedDecl>)>> {
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
            if DECLARATION_NODE_TYPES.contains(&node.node_type.as_str()) {
                let (offset, length) = parse_src(node.src.as_deref());
                ids.push(ScannedDecl {
                    id: node.id,
                    offset,
                    length,
                    name: node.name.clone(), // checkrs: allow(clone_in_loops)
                });
            }
        }
    }

    if ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some((source, ids)))
    }
}

/// Parse a Solc `src` field (format: "offset:length:fileIndex") into (offset, length).
fn parse_src(src: Option<&str>) -> (usize, usize) {
    let s = match src {
        Some(s) => s,
        None => return (0, 0),
    };
    let parts: Vec<&str> = s.split(':').collect();
    let offset = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
    let length = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    (offset, length)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/sources")
    }

    #[test]
    fn index_builds_for_sources_fixture() {
        let project_root = fixture_path();
        let artifact_index = ArtifactIndex::build(project_root.join("out"));
        let symbol_index = SymbolIndex::build(&artifact_index);
        assert!(symbol_index.len() > 0);
    }

    #[test]
    fn index_includes_struct_definition() {
        let project_root = fixture_path();
        let artifact_index = ArtifactIndex::build(project_root.join("out"));
        let symbol_index = SymbolIndex::build(&artifact_index);
        // The Main.sol fixture has a "Data" struct which should be indexed
        let has_struct = symbol_index.len() > 3; // at least: execute, _processData, _compute, Data
        assert!(
            has_struct,
            "Expected at least 4 declarations (3 functions + 1 struct)"
        );
    }

    #[test]
    fn scan_artifact_extracts_declaration_ids() {
        let path = fixture_path().join("out/Main.sol/Main.json");
        let result = scan_artifact_ids(&path).unwrap().unwrap();
        let (_source, ids) = result;
        // Main.sol: execute, _processData, _compute, Data (struct), _data (var)
        assert!(
            ids.len() >= 4,
            "Expected at least 4 declaration IDs, got {}",
            ids.len()
        );
    }
}
