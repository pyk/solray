//! Parse Foundry build-info files for source ID resolution.
//!
//! Foundry produces build-info files during compilation that map source IDs
//! (from AST `src` fields) to file paths. With incremental builds, multiple
//! build-info files may exist; the correct one for a given artifact is the
//! latest build whose `source_id_to_path` entry matches the artifact's expected
//! source file.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use walkdir::WalkDir;

/// A parsed build-info file.
#[derive(Debug, Clone, Deserialize)]
pub struct BuildInfo {
    /// The build identifier (a hex hash).
    pub id: String,
    /// Maps source IDs (from AST node `src` fields) to file paths
    /// relative to the project root.
    pub source_id_to_path: HashMap<String, PathBuf>,
}

impl BuildInfo {
    /// Load all build-info files from the project's `out` directory.
    pub fn load_all(out_dir: impl AsRef<Path>) -> Vec<BuildInfo> {
        let out_dir = out_dir.as_ref();
        let build_info_dir = out_dir.join("build-info");
        if !build_info_dir.exists() {
            return Vec::new();
        }
        WalkDir::new(&build_info_dir)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .filter_map(|e| {
                let content = fs::read_to_string(e.path()).ok()?;
                serde_json::from_str::<BuildInfo>(&content).ok()
            })
            .collect()
    }

    /// Find the build-info that correctly maps the given `source_id` to the
    /// `expected_path`. Returns `None` if no build-info matches.
    ///
    /// This is used to disambiguate across incremental builds: an artifact from
    /// an earlier compilation may reference source IDs that only the earlier
    /// build-info can resolve.
    pub fn find_for_source<'a>(
        infos: &'a [BuildInfo],
        source_id: &str,
        expected_path: impl AsRef<Path>,
    ) -> Option<&'a BuildInfo> {
        let expected_path = expected_path.as_ref();
        infos.iter().find(|info| {
            let resolved = info.source_id_to_path.get(source_id);
            matches!(resolved, Some(p) if p == expected_path)
        })
    }

    /// Resolve a `source_id` to a file path using the provided build-infos.
    /// Tries each build-info in order until one maps the source ID.
    pub fn resolve_source_id<'a>(infos: &'a [BuildInfo], source_id: &str) -> Option<&'a Path> {
        for info in infos {
            if let Some(path) = info.source_id_to_path.get(source_id) {
                return Some(path.as_path());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_out() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritances/out")
    }

    /// Build two BuildInfo structs simulating an incremental build.
    ///
    /// The first ("old") contains all 6 source files.
    /// The second ("new") contains only the 3 files that changed (Base, Child, Middle),
    /// with reassigned source IDs.
    fn incremental_build_infos() -> Vec<BuildInfo> {
        let old = BuildInfo {
            id: "aaaaaaaaaaaaaaaa".into(),
            source_id_to_path: [
                ("0".into(), PathBuf::from("src/AnotherBase.sol")),
                ("1".into(), PathBuf::from("src/Base.sol")),
                ("2".into(), PathBuf::from("src/Child.sol")),
                ("3".into(), PathBuf::from("src/Middle.sol")),
                ("4".into(), PathBuf::from("src/MultiBase.sol")),
                ("5".into(), PathBuf::from("src/MultiChild.sol")),
            ]
            .into(),
        };
        let new = BuildInfo {
            id: "bbbbbbbbbbbbbbbb".into(),
            source_id_to_path: [
                ("0".into(), PathBuf::from("src/Base.sol")),
                ("1".into(), PathBuf::from("src/Child.sol")),
                ("2".into(), PathBuf::from("src/Middle.sol")),
            ]
            .into(),
        };
        vec![old, new]
    }

    #[test]
    fn load_all_finds_build_info_files() {
        let infos = BuildInfo::load_all(&fixture_out());
        assert!(!infos.is_empty(), "expected at least one build-info file");
        for info in &infos {
            assert!(!info.id.is_empty());
            assert!(!info.source_id_to_path.is_empty());
        }
    }

    #[test]
    fn find_for_source_picks_new_build_for_recompiled_file() {
        let infos = incremental_build_infos();
        // Base.sol was recompiled; source_id "0" now maps to it in the new build.
        let found = BuildInfo::find_for_source(&infos, "0", "src/Base.sol");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "bbbbbbbbbbbbbbbb");
    }

    #[test]
    fn find_for_source_picks_old_build_for_unchanged_file() {
        let infos = incremental_build_infos();
        // AnotherBase.sol was not recompiled; only the old build maps source_id "0" to it.
        let found = BuildInfo::find_for_source(&infos, "0", "src/AnotherBase.sol");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "aaaaaaaaaaaaaaaa");
    }

    #[test]
    fn find_for_source_returns_none_when_no_build_matches() {
        let infos = incremental_build_infos();
        // No build maps source_id "9" to anything.
        let found = BuildInfo::find_for_source(&infos, "9", "src/Nowhere.sol");
        assert!(found.is_none());
    }

    #[test]
    fn resolve_source_id_falls_back_across_builds() {
        let infos = incremental_build_infos();
        // source_id "4" only exists in the old build.
        let resolved = BuildInfo::resolve_source_id(&infos, "4");
        assert_eq!(resolved, Some(Path::new("src/MultiBase.sol")));
    }

    #[test]
    fn resolve_source_id_returns_none_for_unknown_id() {
        let infos = incremental_build_infos();
        let resolved = BuildInfo::resolve_source_id(&infos, "99");
        assert!(resolved.is_none());
    }
}
