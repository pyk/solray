//! Lightweight index mapping all Solc AST declaration node IDs to artifact paths.
//!
//! [`SymbolIndex`] extends the concept of a lightweight artifact index
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

use crate::artifact_index::ArtifactIndex;

/// Shared metadata for all declarations from the same artifact.
#[derive(Debug, Clone)]
pub struct ArtifactInfo {
    /// Path to the artifact JSON file.
    pub artifact_path: PathBuf,
    /// The source file this artifact was compiled from.
    pub source_file: PathBuf,
    /// The build-info identifier (hex hash) for the compilation unit.
    pub build_info_id: String,
}

/// An entry in the symbol index: links a declaration ID to its source
/// location and the shared artifact metadata.
#[derive(Debug, Clone)]
pub struct SymbolIndexEntry {
    /// Index into [`SymbolIndex::artifacts`].
    pub artifact_id: usize,
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
    /// Maps source file paths to build-info IDs for build-info scoping.
    source_to_build_info: HashMap<PathBuf, String>,
    /// Shared artifact metadata, indexed via [`SymbolIndexEntry::artifact_id`].
    pub artifacts: Vec<ArtifactInfo>,
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
    /// The first file index found in the artifact, used to resolve the build-info.
    first_file_index: String,
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

use crate::build_info::BuildInfo;

impl SymbolIndex {
    /// Build a [`SymbolIndex`] by scanning all artifacts in `artifact_index`.
    /// `build_infos` is used to scope each declaration to its compilation unit,
    /// preventing ID collisions across incremental builds.
    pub fn build(artifact_index: &ArtifactIndex, build_infos: &[BuildInfo]) -> Self {
        let artifact_paths: Vec<&PathBuf> = artifact_index.all_entries().collect();

        let scanned: Vec<ArtifactScan> = artifact_paths
            .par_iter()
            .filter_map(|artifact_path| {
                let result = scan_artifact_ids(artifact_path).ok()??;
                Some(ArtifactScan {
                    source: result.source,
                    artifact_path: artifact_path.to_path_buf(),
                    ids: result.ids,
                    first_file_index: result.first_file_index,
                })
            })
            .collect();

        let total_decls: usize = scanned.iter().map(|s| s.ids.len()).sum();
        let mut inner: HashMap<i64, SymbolIndexEntry> = HashMap::with_capacity(total_decls);
        let mut seen_sources: HashSet<PathBuf> = HashSet::with_capacity(scanned.len());
        let mut artifacts: Vec<ArtifactInfo> = Vec::with_capacity(scanned.len());

        for scan in scanned {
            // Dedup by source: only process the first artifact per source file
            // (all artifacts from the same source carry an identical AST).
            if !seen_sources.insert(scan.source.clone()) {
                continue;
            }

            // Resolve which build-info this artifact belongs to.
            let build_info_id =
                resolve_build_info(&scan.first_file_index, &scan.source, build_infos);

            // Consume the scan's owned fields into ArtifactInfo.
            let artifact_id = artifacts.len();
            artifacts.push(ArtifactInfo {
                artifact_path: scan.artifact_path,
                source_file: scan.source,
                build_info_id,
            });

            // Consume the declarations into the index (zero inner-loop clones).
            for decl in scan.ids {
                inner.insert(
                    decl.id,
                    SymbolIndexEntry {
                        artifact_id,
                        offset: decl.offset,
                        length: decl.length,
                        name: decl.name,
                    },
                );
            }
        }

        // Build the source-to-build-info lookup from the artifact vec.
        // These clones happen once per source outside the inner loop.
        let source_to_build_info: HashMap<PathBuf, String> = artifacts
            .iter()
            .map(|a| (a.source_file.clone(), a.build_info_id.clone()))
            .collect();

        Self {
            inner,
            source_to_build_info,
            artifacts,
        }
    }

    /// Return the build-info ID for a given source file, if known.
    pub fn build_info_for(&self, source: &Path) -> Option<&str> {
        self.source_to_build_info.get(source).map(|s| s.as_str())
    }

    /// Return the shared artifact metadata for a given artifact ID.
    pub fn artifact_info(&self, artifact_id: usize) -> &ArtifactInfo {
        &self.artifacts[artifact_id]
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

/// Result of scanning a single artifact.
struct ArtifactScanResult {
    source: PathBuf,
    ids: Vec<ScannedDecl>,
    first_file_index: String,
}

/// Scan a single artifact JSON file, extracting the source file path and
/// all declaration IDs (functions, variables, structs, enums, errors,
/// events, modifiers, and user-defined value types) with their source offsets.
fn scan_artifact_ids(path: impl AsRef<Path>) -> Result<Option<ArtifactScanResult>> {
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
    let mut first_file_index = String::new();
    for contract in &su.nodes {
        for node in &contract.nodes {
            if DECLARATION_NODE_TYPES.contains(&node.node_type.as_str()) {
                let src = parse_src(node.src.as_deref());
                if first_file_index.is_empty() {
                    first_file_index = src.file_index.clone(); // checkrs: allow(clone_in_loops)
                }
                ids.push(ScannedDecl {
                    id: node.id,
                    offset: src.offset,
                    length: src.length,
                    name: node.name.clone(), // checkrs: allow(clone_in_loops)
                });
            }
        }
    }

    if ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ArtifactScanResult {
            source,
            ids,
            first_file_index,
        }))
    }
}

/// Parsed source location.
struct ParsedSrc {
    offset: usize,
    length: usize,
    file_index: String,
}

/// Parse a Solc `src` field (format: "offset:length:fileIndex").
fn parse_src(src: Option<&str>) -> ParsedSrc {
    let s = match src {
        Some(s) => s,
        None => {
            return ParsedSrc {
                offset: 0,
                length: 0,
                file_index: String::new(),
            };
        }
    };
    let parts: Vec<&str> = s.split(':').collect();
    let offset = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
    let length = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let file_index = parts.get(2).map(|s| s.to_string()).unwrap_or_default();
    ParsedSrc {
        offset,
        length,
        file_index,
    }
}

/// Resolve which build-info an artifact belongs to by matching its
/// `file_index` against each build-info's `source_id_to_path` map.
fn resolve_build_info(file_index: &str, source: &Path, build_infos: &[BuildInfo]) -> String {
    for info in build_infos {
        if let Some(resolved) = info.source_id_to_path.get(file_index)
            && resolved == source
        {
            return info.id.clone(); // checkrs: allow(clone_in_loops)
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::artifact_index::ArtifactIndex;
    use crate::build_info::BuildInfo;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/function-source")
    }

    #[test]
    fn index_builds_for_sources_fixture() {
        let project_root = fixture_path();
        let artifact_index = ArtifactIndex::build(project_root.join("out"));
        let build_infos = BuildInfo::load_all(project_root.join("out"));
        let symbol_index = SymbolIndex::build(&artifact_index, &build_infos);
        assert!(symbol_index.len() > 0);
    }

    #[test]
    fn index_includes_struct_definition() {
        let project_root = fixture_path();
        let artifact_index = ArtifactIndex::build(project_root.join("out"));
        let build_infos = BuildInfo::load_all(project_root.join("out"));
        let symbol_index = SymbolIndex::build(&artifact_index, &build_infos);
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
        let (_source, ids) = (&result.source, &result.ids);
        // Main.sol: execute, _processData, _compute, Data (struct), _data (var)
        assert!(
            ids.len() >= 4,
            "Expected at least 4 declaration IDs, got {}",
            ids.len()
        );
    }
}
