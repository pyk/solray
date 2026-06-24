//! Lightweight name → artifact-path index for a Foundry project.
//!
//! [`ArtifactIndex`] is built by walking the `out/` directory without parsing
//! any JSON files. Each artifact is identified by its filename stem (the
//! declaration name).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// A resolved artifact entry: the path to the JSON file and the source file
/// it was compiled from.
#[derive(Debug, Clone)]
pub struct ArtifactEntry {
    /// Path to the artifact JSON, e.g. `out/DateTime.sol/DateTime.json`
    pub path: PathBuf,
    /// The source file, e.g. `src/DateTime.sol` (resolved lazily from the AST
    /// via build-info; populated on demand during call graph resolution).
    pub source_file: Option<PathBuf>,
}

/// A lightweight index mapping declaration names to artifact paths.
///
/// Built by scanning the `out/` directory. No JSON files are parsed during
/// construction.
#[derive(Debug, Clone)]
pub struct ArtifactIndex {
    inner: HashMap<String, Vec<ArtifactEntry>>,
}

impl ArtifactIndex {
    /// Walk `out_dir` and build a name → artifact-entry index.
    pub fn build(out_dir: impl AsRef<Path>) -> Self {
        let out_dir = out_dir.as_ref();
        let mut inner: HashMap<String, Vec<ArtifactEntry>> = HashMap::new();

        if !out_dir.exists() {
            return Self { inner };
        }

        for entry in WalkDir::new(out_dir)
            .min_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Only .json files.
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            // Skip build-info files.
            if path.to_string_lossy().contains("build-info") {
                continue;
            }

            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let entry = ArtifactEntry {
                    path: path.to_path_buf(),
                    source_file: None,
                };
                inner.entry(stem.to_string()).or_default().push(entry);
            }
        }

        Self { inner }
    }

    /// Look up artifact entries by declaration name.
    pub fn get(&self, name: &str) -> Option<&Vec<ArtifactEntry>> {
        self.inner.get(name)
    }

    /// Return `true` if the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate over all artifact entries across all declarations.
    pub fn all_entries(&self) -> impl Iterator<Item = &ArtifactEntry> {
        self.inner.values().flatten()
    }
}

impl std::ops::Deref for ArtifactIndex {
    type Target = HashMap<String, Vec<ArtifactEntry>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
