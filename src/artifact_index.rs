//! Lightweight name → artifact-path index for a Foundry project.
//!
//! [`ArtifactIndex`] is built by walking the `out/` directory without parsing
//! any JSON files. Each declaration name maps to one or more artifact paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use walkdir::WalkDir;

/// A lightweight index mapping declaration names to artifact paths.
///
/// Built by scanning the `out/` directory. No JSON files are parsed during
/// construction.
#[derive(Debug, Clone)]
pub struct ArtifactIndex {
    inner: HashMap<String, Vec<PathBuf>>,
}

impl ArtifactIndex {
    /// Walk `out_dir` and build a name → artifact-entry index.
    pub fn build(out_dir: impl AsRef<Path>) -> Self {
        let out_dir = out_dir.as_ref();
        let mut inner: HashMap<String, Vec<PathBuf>> = HashMap::new();

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
                inner
                    .entry(stem.to_string())
                    .or_default()
                    .push(path.to_path_buf());
            }
        }

        Self { inner }
    }

    /// Look up artifact entries by declaration name.
    pub fn get(&self, name: &str) -> Option<&Vec<PathBuf>> {
        self.inner.get(name)
    }

    /// Return `true` if the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Look up artifact entries by declaration name, returning an error if the
    /// name is not found or has no entries.
    pub fn try_get(&self, name: &str) -> Result<Vec<PathBuf>> {
        match self.inner.get(name) {
            Some(entries) if !entries.is_empty() => Ok(entries.clone()),
            _ => bail!("\"{}\" not found.", name),
        }
    }

    /// Iterate over all artifact entries across all declarations.
    pub fn all_entries(&self) -> impl Iterator<Item = &PathBuf> {
        self.inner.values().flatten()
    }

    /// Find an artifact whose AST source path matches `file`.
    ///
    /// This supports file-qualified IDs that use the source path recorded in
    /// the artifact (for example `src/Foo.sol:Contract`) instead of the
    /// artifact directory layout.
    pub fn find_by_source_path(&self, file: &str, name: &str) -> Option<PathBuf> {
        self.inner.get(name)?.iter().find_map(|path| {
            (Self::source_path(path).as_deref() == Some(file)).then(|| path.clone())
        })
    }

    /// Return the source path recorded in the artifact's AST, if available.
    pub fn source_path(artifact_path: impl AsRef<Path>) -> Option<String> {
        let file = std::fs::File::open(artifact_path).ok()?;
        let value: serde_json::Value = serde_json::from_reader(file).ok()?;
        value
            .get("ast")?
            .get("absolutePath")?
            .as_str()
            .map(ToOwned::to_owned)
    }

    /// Render an artifact as `path:name` for suggestions.
    ///
    /// Prefers the AST source path (for example `src/Foo.sol:Contract` or
    /// `lib/Foo.sol:Contract`) and falls back to the artifact directory name.
    pub fn qualified_name(artifact_path: impl AsRef<Path>, name: &str) -> String {
        let artifact_path = artifact_path.as_ref();
        match Self::source_path(artifact_path) {
            Some(source) => format!("{source}:{name}"),
            None => {
                let parent = artifact_path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                format!("{parent}:{name}")
            }
        }
    }
}

impl std::ops::Deref for ArtifactIndex {
    type Target = HashMap<String, Vec<PathBuf>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
