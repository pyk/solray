//! Library contract inspection for Foundry projects.
//!
//! [`LibraryInspector`] scans the artifact directory and produces structured
//! output for each library found.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::project::Project;
use crate::project::ProjectDirectories;

/// The directory category a [`Library`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DirCategory {
    Src,
    Lib,
    Test,
    Other,
}

impl DirCategory {
    fn display(self) -> &'static str {
        match self {
            DirCategory::Src => "src",
            DirCategory::Lib => "lib",
            DirCategory::Test => "test",
            DirCategory::Other => "other",
        }
    }
}

/// Classifies artifact paths using the directory layout declared in the
/// project's `foundry.toml`.
struct DirectoryClassifier<'a> {
    directories: &'a ProjectDirectories,
}

impl<'a> DirectoryClassifier<'a> {
    fn new(directories: &'a ProjectDirectories) -> Self {
        Self { directories }
    }

    fn classify(&self, path: impl AsRef<Path>) -> DirCategory {
        let path = path.as_ref();
        if self
            .directories
            .libs
            .iter()
            .any(|lib| path.starts_with(lib))
        {
            DirCategory::Lib
        } else if path.starts_with(&self.directories.src) {
            DirCategory::Src
        } else if path.starts_with(&self.directories.test) {
            DirCategory::Test
        } else {
            DirCategory::Other
        }
    }
}

/// A single library.
pub struct Library {
    /// The library name.
    pub name: String,
    /// The source file path (relative to the project root).
    pub path: PathBuf,
}

impl Library {
    /// Create a new [`Library`] from a name and source file.
    fn new(name: &str, source_file: &Path, project_root: &Path) -> Self {
        let rel = source_file
            .strip_prefix(project_root)
            .unwrap_or(source_file);
        Self {
            name: name.to_string(),
            path: rel.to_path_buf(),
        }
    }
}

impl std::fmt::Display for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

/// The output of a [`LibraryInspector`] inspection.
pub struct LibraryInspectorOutput {
    libraries: Vec<Library>,
    directories: ProjectDirectories,
}

impl LibraryInspectorOutput {
    /// Create a new [`LibraryInspectorOutput`] from a list of libraries and
    /// the project's directory configuration.
    pub fn new(libraries: Vec<Library>, directories: ProjectDirectories) -> Self {
        Self {
            libraries,
            directories,
        }
    }
}

impl std::fmt::Display for LibraryInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let groups = self.grouped();

        writeln!(f, "summary:")?;
        for (dir, libraries) in &groups {
            writeln!(
                f,
                "- {} libraries in {} directory",
                libraries.len(),
                dir.display()
            )?;
        }
        writeln!(f, "- total {} libraries", self.libraries.len())?;

        for (dir, libraries) in &groups {
            writeln!(f, "\nlibraries in {} directory:", dir.display())?;
            for (i, lib) in libraries.iter().enumerate() {
                writeln!(f, "{}. {} (file: {})", i + 1, lib.name, lib.path.display())?;
            }
        }

        Ok(())
    }
}

impl LibraryInspectorOutput {
    /// Return the libraries grouped by directory category.
    fn grouped(&self) -> BTreeMap<DirCategory, Vec<&Library>> {
        let classifier = DirectoryClassifier::new(&self.directories);
        let mut groups: BTreeMap<DirCategory, Vec<&Library>> = BTreeMap::new();
        for lib in &self.libraries {
            let dir = classifier.classify(&lib.path);
            groups.entry(dir).or_default().push(lib);
        }
        groups
    }
}

/// Inspect a Foundry project for libraries.
pub struct LibraryInspector {
    project: Project,
}

impl LibraryInspector {
    /// Build a [`LibraryInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the project and return all libraries.
    pub fn inspect(&self) -> Result<LibraryInspectorOutput> {
        self.project.validate()?;

        let project_root_abs = std::path::absolute(self.project.path())?;
        let directories = self.project.directories()?;
        let libraries = self.load_libraries(&project_root_abs)?;
        Ok(LibraryInspectorOutput::new(libraries, directories))
    }

    /// Walk the artifact directory and extract only libraries.
    fn load_libraries(&self, project_root: &Path) -> Result<Vec<Library>> {
        let paths = self.artifact_paths();
        let mut libraries: Vec<Library> = paths
            .into_par_iter()
            .filter_map(|path| parse_library(&path, project_root).ok().flatten())
            .collect();
        libraries.sort_by(|a, b| a.path.cmp(&b.path).then(a.name.cmp(&b.name)));
        Ok(libraries)
    }

    /// Collect all JSON artifact paths from the output directory,
    /// excluding `build-info` files.
    fn artifact_paths(&self) -> Vec<PathBuf> {
        WalkDir::new(self.project.out_dir())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let p = e.path();
                p.extension().and_then(|s| s.to_str()) == Some("json")
                    && !p.to_string_lossy().contains("build-info")
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    }
}

/// Quick check whether the raw file bytes contain `"contractKind":`.
///
/// This lets us skip JSON deserialization entirely for artifacts that contain
/// no contract definitions at all.
fn might_be_library(path: impl AsRef<Path>) -> Result<bool> {
    let bytes = fs::read(path)?;
    let pattern = b"\"contractKind\":";
    Ok(bytes.windows(pattern.len()).any(|w| w == pattern))
}

/// Parse a single artifact JSON file, returning [`Some(Library)`] only if the
/// contract defined in the artifact is a library.
fn parse_library(path: impl AsRef<Path>, project_root: &Path) -> Result<Option<Library>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
    };

    // Fast path: skip files that definitely contain no contract definitions.
    if !might_be_library(path)? {
        return Ok(None);
    }

    // Slow path: deserializing only the lightweight AST representation skips
    // heavy fields such as bytecode, deployedBytecode, and deep children
    // (function bodies, events, errors) inside contract definitions.
    let content = fs::read_to_string(path)?;
    let artifact: LightweightArtifact = serde_json::from_str(&content)?;

    let ast = artifact.ast.with_context(|| {
        format!(
            "artifact `{}` is missing the AST; rebuild with `ast = true` in foundry.toml",
            path.display()
        )
    })?;

    if let Some(name) = ast
        .nodes
        .into_iter()
        .find_map(|n| n.is_library(contract_name))
    {
        return Ok(Some(Library::new(
            &name,
            Path::new(&ast.absolute_path),
            project_root,
        )));
    }

    Ok(None)
}

/// Lightweight artifact representation that deserializes only the AST and
/// skips all heavy fields (bytecode, deployedBytecode, storageLayout, etc.).
#[derive(Deserialize)]
struct LightweightArtifact {
    #[serde(default)]
    ast: Option<LightweightSourceUnit>,
}

/// Lightweight source unit that deserializes only top-level node metadata.
#[derive(Deserialize)]
struct LightweightSourceUnit {
    #[serde(rename = "absolutePath")]
    absolute_path: PathBuf,
    #[serde(default)]
    nodes: Vec<LightweightNode>,
}

/// Lightweight node that only deserializes the fields needed to identify
/// libraries. Heavy children (function bodies, events, errors) are
/// skipped by serde.
#[derive(Deserialize)]
struct LightweightNode {
    #[serde(rename = "nodeType")]
    node_type: String,
    name: Option<String>,
    #[serde(rename = "contractKind")]
    contract_kind: Option<String>,
}

impl LightweightNode {
    /// Return the contract name if this node is a library matching
    /// `target_name`.
    fn is_library(&self, target_name: &str) -> Option<String> {
        if self.node_type != "ContractDefinition" {
            return None;
        }
        if self.contract_kind.as_deref() != Some("library") {
            return None;
        }
        if self.name.as_deref() != Some(target_name) {
            return None;
        }
        self.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    use crate::project::Project;
    use crate::project::ProjectDirectories;

    fn default_directories() -> ProjectDirectories {
        ProjectDirectories {
            src: PathBuf::from("src"),
            test: PathBuf::from("test"),
            libs: vec![PathBuf::from("lib"), PathBuf::from("node_modules")],
        }
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/libraries")
    }

    #[test]
    fn inspect_returns_only_library_declarations() {
        let inspector = LibraryInspector::new(Project::open(fixture_path()));
        let output = inspector.inspect().unwrap();
        let expected = include_str!("../../fixtures/libraries/expected/output.txt");
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn library_display_formats_as_path_colon_name() {
        let file = PathBuf::from("src/MathLib.sol");
        let library = Library::new("MathLib", &file, Path::new(""));
        assert_eq!(library.to_string(), "src/MathLib.sol:MathLib");
    }

    #[test]
    fn output_display_formats_structured_by_directory() {
        let src = Library::new("ArrayUtils", &PathBuf::from("src/Mixed.sol"), Path::new(""));
        let lib = Library::new(
            "LibUtils",
            &PathBuf::from("lib/mylib/src/SomeLib.sol"),
            Path::new(""),
        );
        let test = Library::new(
            "TestHelpers",
            &PathBuf::from("test/TestLib.sol"),
            Path::new(""),
        );
        let output = LibraryInspectorOutput::new(vec![test, lib, src], default_directories());
        assert_eq!(
            output.to_string(),
            "summary:\n\
- 1 libraries in src directory\n\
- 1 libraries in lib directory\n\
- 1 libraries in test directory\n\
- total 3 libraries\n\
\nlibraries in src directory:\n\
1. ArrayUtils (file: src/Mixed.sol)\n\
\nlibraries in lib directory:\n\
1. LibUtils (file: lib/mylib/src/SomeLib.sol)\n\
\nlibraries in test directory:\n\
1. TestHelpers (file: test/TestLib.sol)\n\
"
        );
    }
}
