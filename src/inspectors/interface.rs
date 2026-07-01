//! Interface inspection for Foundry projects.
//!
//! [`InterfaceInspector`] scans the artifact directory and produces structured
//! output for each interface found.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::project::Project;
use crate::project::ProjectDirectories;

/// The directory category an [`Interface`] belongs to.
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

/// A single interface.
pub struct Interface {
    /// The interface name.
    pub name: String,
    /// The source file path (relative to the project root).
    pub path: PathBuf,
}

impl Interface {
    /// Create a new [`Interface`] from a name and source file.
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

impl std::fmt::Display for Interface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

/// The output of an [`InterfaceInspector`] inspection.
pub struct InterfaceInspectorOutput {
    interfaces: Vec<Interface>,
    directories: ProjectDirectories,
}

impl InterfaceInspectorOutput {
    /// Create a new [`InterfaceInspectorOutput`] from a list of interfaces and
    /// the project's directory configuration.
    pub fn new(interfaces: Vec<Interface>, directories: ProjectDirectories) -> Self {
        Self {
            interfaces,
            directories,
        }
    }
}

impl std::fmt::Display for InterfaceInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let groups = self.grouped();

        writeln!(f, "summary:")?;
        for (dir, interfaces) in &groups {
            writeln!(
                f,
                "- {} interfaces in {} directory",
                interfaces.len(),
                dir.display()
            )?;
        }
        writeln!(f, "- total {} interfaces", self.interfaces.len())?;

        for (dir, interfaces) in &groups {
            writeln!(f, "\ninterfaces in {} directory:", dir.display())?;
            for (i, interface) in interfaces.iter().enumerate() {
                writeln!(
                    f,
                    "{}. {} (file: {})",
                    i + 1,
                    interface.name,
                    interface.path.display()
                )?;
            }
        }

        Ok(())
    }
}

impl InterfaceInspectorOutput {
    /// Return the interfaces grouped by directory category.
    fn grouped(&self) -> BTreeMap<DirCategory, Vec<&Interface>> {
        let classifier = DirectoryClassifier::new(&self.directories);
        let mut groups: BTreeMap<DirCategory, Vec<&Interface>> = BTreeMap::new();
        for interface in &self.interfaces {
            let dir = classifier.classify(&interface.path);
            groups.entry(dir).or_default().push(interface);
        }
        groups
    }
}

/// Inspect a Foundry project for interfaces.
pub struct InterfaceInspector {
    project: Project,
}

impl InterfaceInspector {
    /// Build an [`InterfaceInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the project and return all interfaces.
    pub fn inspect(&self) -> Result<InterfaceInspectorOutput> {
        self.project.validate()?;

        let project_root_abs = std::path::absolute(self.project.path())?;
        let directories = self.project.directories()?;
        let interfaces = self.load_interfaces(&project_root_abs)?;
        Ok(InterfaceInspectorOutput::new(interfaces, directories))
    }

    /// Walk the artifact directory and extract only interfaces.
    fn load_interfaces(&self, project_root: &Path) -> Result<Vec<Interface>> {
        let paths = self.artifact_paths();
        let mut interfaces: Vec<Interface> = paths
            .into_par_iter()
            .filter_map(|path| parse_interface(&path, project_root).ok().flatten())
            .collect();
        interfaces.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
        Ok(interfaces)
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
fn might_be_interface(path: impl AsRef<Path>) -> Result<bool> {
    let bytes = fs::read(path)?;
    let pattern = b"\"contractKind\":";
    Ok(bytes.windows(pattern.len()).any(|w| w == pattern))
}

/// Parse a single artifact JSON file, returning [`Some(Interface)`] only if the
/// contract defined in the artifact is an interface.
fn parse_interface(path: impl AsRef<Path>, project_root: &Path) -> Result<Option<Interface>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
    };

    // Fast path: skip files that definitely contain no contract definitions.
    if !might_be_interface(path)? {
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
        .find_map(|n| n.is_interface(contract_name))
    {
        return Ok(Some(Interface::new(
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
/// interfaces. Heavy children (function bodies, events, errors) are skipped by
/// serde.
#[derive(Deserialize)]
struct LightweightNode {
    #[serde(rename = "nodeType")]
    node_type: String,
    name: Option<String>,
    #[serde(rename = "contractKind")]
    contract_kind: Option<String>,
}

impl LightweightNode {
    /// Return the contract name if this node is an interface matching
    /// `target_name`.
    fn is_interface(&self, target_name: &str) -> Option<String> {
        if self.node_type != "ContractDefinition" {
            return None;
        }
        if self.contract_kind.as_deref() != Some("interface") {
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/interfaces")
    }

    #[test]
    fn inspect_returns_only_interface_declarations() {
        let inspector = InterfaceInspector::new(Project::open(fixture_path()));
        let output = inspector.inspect().unwrap();
        let expected = include_str!("../../fixtures/interfaces/expected/output.txt");
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn interface_display_formats_as_path_colon_name() {
        let file = PathBuf::from("src/IPrimary.sol");
        let interface = Interface::new("IPrimary", &file, Path::new(""));
        assert_eq!(interface.to_string(), "src/IPrimary.sol:IPrimary");
    }

    #[test]
    fn output_display_formats_structured_by_directory() {
        let src = Interface::new(
            "IPrimary",
            &PathBuf::from("src/IPrimary.sol"),
            Path::new(""),
        );
        let lib = Interface::new(
            "IDependency",
            &PathBuf::from("lib/mylib/src/SomeLib.sol"),
            Path::new(""),
        );
        let node_modules = Interface::new(
            "IThirdParty",
            &PathBuf::from("node_modules/some-pkg/src/ThirdParty.sol"),
            Path::new(""),
        );
        let test = Interface::new(
            "ITestHelper",
            &PathBuf::from("test/TestInterface.sol"),
            Path::new(""),
        );
        let output = InterfaceInspectorOutput::new(
            vec![test, lib, node_modules, src],
            default_directories(),
        );
        assert_eq!(
            output.to_string(),
            "summary:\n\
- 1 interfaces in src directory\n\
- 2 interfaces in lib directory\n\
- 1 interfaces in test directory\n\
- total 4 interfaces\n\
\n\
interfaces in src directory:\n\
1. IPrimary (file: src/IPrimary.sol)\n\
\n\
interfaces in lib directory:\n\
1. IDependency (file: lib/mylib/src/SomeLib.sol)\n\
2. IThirdParty (file: node_modules/some-pkg/src/ThirdParty.sol)\n\
\n\
interfaces in test directory:\n\
1. ITestHelper (file: test/TestInterface.sol)\n\
"
        );
    }
}
