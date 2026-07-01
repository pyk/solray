//! Abstract contract inspection for Foundry projects.
//!
//! [`AbstractInspector`] scans the artifact directory and produces structured
//! output for each abstract contract found.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::project::Project;
use crate::project::ProjectDirectories;

/// The directory category an [`Abstract`] belongs to.
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

/// A single abstract contract.
pub struct Abstract {
    /// The contract name.
    pub name: String,
    /// The source file path (relative to the project root).
    pub path: PathBuf,
}

impl Abstract {
    /// Create a new [`Abstract`] from a name and source file.
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

impl std::fmt::Display for Abstract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

/// The output of an [`AbstractInspector`] inspection.
pub struct AbstractInspectorOutput {
    abstracts: Vec<Abstract>,
    directories: ProjectDirectories,
}

impl AbstractInspectorOutput {
    /// Create a new [`AbstractInspectorOutput`] from a list of abstracts and
    /// the project's directory configuration.
    pub fn new(abstracts: Vec<Abstract>, directories: ProjectDirectories) -> Self {
        Self {
            abstracts,
            directories,
        }
    }
}

impl std::fmt::Display for AbstractInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let groups = self.grouped();

        writeln!(f, "summary:")?;
        for (dir, abstracts) in &groups {
            writeln!(
                f,
                "- {} abstracts in {} directory",
                abstracts.len(),
                dir.display()
            )?;
        }
        writeln!(f, "- total {} abstracts", self.abstracts.len())?;

        for (dir, abstracts) in &groups {
            writeln!(f, "\nabstracts in {} directory:", dir.display())?;
            for (i, abstract_) in abstracts.iter().enumerate() {
                writeln!(
                    f,
                    "{}. {} (file: {})",
                    i + 1,
                    abstract_.name,
                    abstract_.path.display()
                )?;
            }
        }

        Ok(())
    }
}

impl AbstractInspectorOutput {
    /// Return the abstracts grouped by directory category.
    fn grouped(&self) -> BTreeMap<DirCategory, Vec<&Abstract>> {
        let classifier = DirectoryClassifier::new(&self.directories);
        let mut groups: BTreeMap<DirCategory, Vec<&Abstract>> = BTreeMap::new();
        for abstract_ in &self.abstracts {
            let dir = classifier.classify(&abstract_.path);
            groups.entry(dir).or_default().push(abstract_);
        }
        groups
    }
}

/// Inspect a Foundry project for abstract contracts.
pub struct AbstractInspector {
    project: Project,
}

impl AbstractInspector {
    /// Build an [`AbstractInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the project and return all abstract contracts.
    pub fn inspect(&self) -> Result<AbstractInspectorOutput> {
        self.project.validate()?;

        let project_root_abs = std::path::absolute(self.project.path())?;
        let directories = self.project.directories()?;
        let abstracts = self.load_abstracts(&project_root_abs)?;
        Ok(AbstractInspectorOutput::new(abstracts, directories))
    }

    /// Walk the artifact directory and extract only abstract contracts.
    fn load_abstracts(&self, project_root: &Path) -> Result<Vec<Abstract>> {
        let paths = self.artifact_paths();
        let mut abstracts: Vec<Abstract> = paths
            .into_par_iter()
            .filter_map(|path| parse_abstract(&path, project_root).ok().flatten())
            .collect();
        abstracts.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
        Ok(abstracts)
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

/// Quick check whether the raw file bytes contain `"abstract":true`.
///
/// This lets us skip JSON deserialization entirely for the vast majority of
/// artifacts that contain no abstract contracts.
fn might_be_abstract(path: impl AsRef<Path>) -> Result<bool> {
    let bytes = fs::read(path)?;
    let pattern = b"\"abstract\":";
    Ok(bytes.windows(pattern.len()).any(|w| w == pattern))
}

/// Parse a single artifact JSON file, returning [`Some(Abstract)`] only if the
/// contract defined in the artifact is abstract.
fn parse_abstract(path: impl AsRef<Path>, project_root: &Path) -> Result<Option<Abstract>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
    };

    // Fast path: skip files that definitely contain no abstract contracts.
    if !might_be_abstract(path)? {
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
        .find_map(|n| n.is_abstract(contract_name))
    {
        return Ok(Some(Abstract::new(
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
/// abstract contracts. Heavy children (function bodies, events, errors) are
/// skipped by serde.
#[derive(Deserialize)]
struct LightweightNode {
    #[serde(rename = "nodeType")]
    node_type: String,
    name: Option<String>,
    #[serde(default, rename = "abstract")]
    abstract_fallback: bool,
    #[serde(rename = "contractKind")]
    contract_kind: Option<String>,
}

impl LightweightNode {
    /// Return the contract name if this node is an abstract contract matching
    /// `target_name`.
    fn is_abstract(&self, target_name: &str) -> Option<String> {
        if self.node_type != "ContractDefinition" {
            return None;
        }
        if self.contract_kind.as_deref() != Some("contract") {
            return None;
        }
        if !self.abstract_fallback {
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/abstracts")
    }

    #[test]
    fn inspect_returns_only_abstract_declarations() {
        let inspector = AbstractInspector::new(Project::open(fixture_path()));
        let output = inspector.inspect().unwrap();
        let expected = include_str!("../../fixtures/abstracts/expected/output.txt");
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn abstract_display_formats_as_path_colon_name() {
        let file = PathBuf::from("src/AbstractBase.sol");
        let abstract_ = Abstract::new("AbstractBase", &file, Path::new(""));
        assert_eq!(abstract_.to_string(), "src/AbstractBase.sol:AbstractBase");
    }

    #[test]
    fn output_display_formats_structured_by_directory() {
        let src = Abstract::new(
            "OnlyOwnerBase",
            &PathBuf::from("src/OnlyOwnerBase.sol"),
            Path::new(""),
        );
        let lib = Abstract::new(
            "TestBase",
            &PathBuf::from("lib/forge-std/src/Base.sol"),
            Path::new(""),
        );
        let node_modules = Abstract::new(
            "NodeModuleFoo",
            &PathBuf::from("node_modules/some-pkg/src/Foo.sol"),
            Path::new(""),
        );
        let test = Abstract::new(
            "Assertions",
            &PathBuf::from("test/Assertions.sol"),
            Path::new(""),
        );
        let output =
            AbstractInspectorOutput::new(vec![test, lib, node_modules, src], default_directories());
        assert_eq!(
            output.to_string(),
            "summary:\n\
- 1 abstracts in src directory\n\
- 2 abstracts in lib directory\n\
- 1 abstracts in test directory\n\
- total 4 abstracts\n\
\n\
abstracts in src directory:\n\
1. OnlyOwnerBase (file: src/OnlyOwnerBase.sol)\n\
\n\
abstracts in lib directory:\n\
1. TestBase (file: lib/forge-std/src/Base.sol)\n\
2. NodeModuleFoo (file: node_modules/some-pkg/src/Foo.sol)\n\
\n\
abstracts in test directory:\n\
1. Assertions (file: test/Assertions.sol)\n\
"
        );
    }
}
