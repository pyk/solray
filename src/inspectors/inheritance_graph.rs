//! Inheritance graph inspection for Foundry projects.
//!
//! [`InheritanceGraphInspector`] reads a single artifact file and resolves
//! the full inheritance chain of the contract it defines.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use solc::ast::{ContractKind, SourceUnit, SourceUnitNode};
use tracing::debug;

use crate::artifact_index::ArtifactIndex;
use crate::inspectors::artifact_id::ArtifactId;
use crate::project::Project;

/// A node in an inheritance tree.
#[derive(Debug, Clone)]
struct InheritanceNode {
    name: String,
    file: String,
    parents: Vec<InheritanceNode>,
}

impl InheritanceNode {
    /// Flatten the tree into a depth-first list of `(file, name)` references.
    fn flatten_sources(&self) -> Vec<(&str, &str)> {
        let mut result = Vec::new();
        self.collect_recursive(&mut result);
        result
    }

    fn collect_recursive<'a>(&'a self, out: &mut Vec<(&'a str, &'a str)>) {
        out.push((&self.file, &self.name));
        for parent in &self.parents {
            parent.collect_recursive(out);
        }
    }
}

impl fmt::Display for InheritanceNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn fmt_children(
            children: &[InheritanceNode],
            f: &mut fmt::Formatter<'_>,
            prefix: &str,
        ) -> fmt::Result {
            let len = children.len();
            for (i, child) in children.iter().enumerate() {
                let is_last = i == len - 1;
                let connector = if is_last {
                    "\u{2514}\u{2500}\u{2500} "
                } else {
                    "\u{251c}\u{2500}\u{2500} "
                };
                let continuation = if is_last { "    " } else { "\u{2502}   " };

                writeln!(f, "{}{}{}", prefix, connector, child.name)?;
                if !child.parents.is_empty() {
                    let child_prefix = format!("{}{}", prefix, continuation);
                    fmt_children(&child.parents, f, &child_prefix)?;
                }
            }
            Ok(())
        }

        writeln!(f, "{}", self.name)?;
        fmt_children(&self.parents, f, "")
    }
}

/// The output of an [`InheritanceGraphInspector`] inspection.
#[derive(Debug)]
pub struct InheritanceGraphInspectorOutput {
    root: InheritanceNode,
}

impl fmt::Display for InheritanceGraphInspectorOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Inheritance graph:\n")?;
        write!(f, "{}", self.root)?;

        let mut sources = self.root.flatten_sources();
        sources.sort_by(|(file_a, name_a), (file_b, name_b)| {
            name_a.cmp(name_b).then(file_a.cmp(file_b))
        });
        writeln!(f, "\nSources:\n")?;
        for (i, (file, name)) in sources.iter().enumerate() {
            writeln!(f, "{}. {} (file: {})", i + 1, name, file)?;
        }
        Ok(())
    }
}

/// Context for resolving inheritance graphs from artifact files.
struct ResolutionContext {
    project_root: PathBuf,
    out_dir: PathBuf,
}

/// Inspect a Foundry project for a single contract's inheritance graph.
pub struct InheritanceGraphInspector {
    project: Project,
}

impl InheritanceGraphInspector {
    /// Build an [`InheritanceGraphInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the inheritance graph for the given [`ArtifactId`].
    pub fn inspect(&self, id: &ArtifactId) -> Result<InheritanceGraphInspectorOutput> {
        debug!(name = %id.name, project = %self.project.path().display(), "starting inheritance graph inspection");
        self.project.validate()?;
        let ctx = ResolutionContext {
            project_root: self.project.path().to_path_buf(),
            out_dir: self.project.out_dir().to_path_buf(),
        };

        let artifact_path = match &id.file {
            Some(file) => ctx.resolve_with_file(file, &id.name),
            None => ctx.resolve_without_file(&id.name),
        }?;
        debug!(path = %artifact_path.display(), "resolved root artifact");

        let root = ctx.build_inheritance_tree(&artifact_path, &id.name)?;
        Ok(InheritanceGraphInspectorOutput { root })
    }
}

impl ResolutionContext {
    /// Resolve the artifact path when a file is specified.
    fn resolve_with_file(&self, file: &str, name: &str) -> Result<PathBuf> {
        let artifact_path = self.out_dir.join(file).join(format!("{name}.json"));
        ensure!(
            artifact_path.exists(),
            "artifact `{}` not found",
            artifact_path.display()
        );
        Ok(artifact_path)
    }

    /// Resolve the artifact path when only a name is given.
    fn resolve_without_file(&self, name: &str) -> Result<PathBuf> {
        let index = ArtifactIndex::build(&self.out_dir);
        let candidates = index.get(name).cloned().unwrap_or_default();
        debug!(name, candidates = ?candidates, "looked up root artifact candidates");

        match candidates.len() {
            0 => {
                bail!("\"{name}\" not found.");
            }
            1 => Ok(candidates[0].clone()),
            n => {
                let mut sorted = candidates;
                sorted.sort();

                let mut msg = format!("found {n} \"{name}\"\n\nSelect one of the following:\n");
                for candidate in &sorted {
                    let relative = self.relative_artifact_path(candidate);
                    msg.push_str(&format!("\nhawk inspect inheritance-graph {relative}"));
                }
                msg.push('\n');
                bail!(msg);
            }
        }
    }

    /// Return the artifact path as a `file:name` string for error messages.
    ///
    /// For an artifact at `out/Foo.sol/Contract.json`, this returns
    /// `"Foo.sol:Contract"`. For `out/lib/Foo.sol/Contract.json`, this
    /// returns `"lib/Foo.sol:Contract"`.
    fn relative_artifact_path(&self, artifact_path: impl AsRef<Path>) -> String {
        let artifact_path = artifact_path.as_ref();
        let name = artifact_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let file = artifact_path
            .strip_prefix(&self.out_dir)
            .unwrap_or(artifact_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        format!("{file}:{name}")
    }

    /// Build the full inheritance tree rooted at the given artifact.
    fn build_inheritance_tree(
        &self,
        artifact_path: impl AsRef<Path>,
        name: &str,
    ) -> Result<InheritanceNode> {
        let artifact_path = artifact_path.as_ref();
        debug!(name, path = %artifact_path.display(), "building inheritance tree");
        let sources = self.parse_contract_sources(artifact_path)?;
        debug!(
            name,
            sources = ?sources.keys().collect::<Vec<&String>>(),
            "loaded contract sources"
        );
        sources.get(name).with_context(|| {
            format!(
                "contract `{name}` not found in `{}`",
                artifact_path.display()
            )
        })?;

        let mut visited: HashSet<String> = HashSet::new();
        self.resolve_tree(name, &sources, &mut visited)
    }

    /// Recursively resolve an inheritance node for a contract name.
    fn resolve_tree(
        &self,
        name: &str,
        sources: &HashMap<String, ContractSource>,
        visited: &mut HashSet<String>,
    ) -> Result<InheritanceNode> {
        ensure!(
            visited.insert(name.to_string()),
            "circular inheritance detected for `{name}`"
        );

        let source = sources
            .get(name)
            .with_context(|| format!("contract `{name}` not found in artifacts"))?;
        debug!(name, bases = ?source.bases, "resolving inheritance node");

        let parents: Vec<InheritanceNode> = source
            .bases
            .iter()
            .map(|base| self.resolve_tree(base, sources, visited))
            .collect::<Result<Vec<InheritanceNode>>>()?;
        visited.remove(name);

        Ok(InheritanceNode {
            name: source.name.clone(),
            file: source.file.clone(),
            parents,
        })
    }

    /// Parse contract name, file, and base names from a single artifact file.
    fn parse_contract_sources(
        &self,
        artifact_path: impl AsRef<Path>,
    ) -> Result<HashMap<String, ContractSource>> {
        let artifact_path = artifact_path.as_ref();
        let contract_name = artifact_path
            .file_stem()
            .and_then(|s| s.to_str())
            .with_context(|| format!("invalid artifact path `{}`", artifact_path.display()))?;
        debug!(path = %artifact_path.display(), contract_name, "parsing artifact AST");

        let content = fs::read_to_string(artifact_path)?;
        let artifact: Artifact = serde_json::from_str(&content)?;

        let ast = artifact.ast.with_context(|| {
            format!(
                "artifact `{}` is missing the AST; rebuild with `ast = true` in foundry.toml",
                artifact_path.display()
            )
        })?;

        let mut sources: HashMap<String, ContractSource> = HashMap::new();
        let file = self.rel_path_str(&ast.absolute_path);
        debug!(path = %artifact_path.display(), source_file = %file, node_count = ast.nodes.len(), "parsed artifact AST");

        for node in &ast.nodes {
            if let SourceUnitNode::ContractDefinition(cd) = node
                && (cd.contract_kind == ContractKind::Contract
                    || cd.contract_kind == ContractKind::Interface)
            {
                let bases: Vec<String> = cd
                    .base_contracts
                    .iter()
                    .map(|bc| bc.base_name.name.clone()) // checkrs: allow(clone_in_iterator, clone_in_loops)
                    .collect();

                sources.insert(
                    cd.name.clone(), // checkrs: allow(clone_in_loops)
                    ContractSource {
                        name: cd.name.clone(), // checkrs: allow(clone_in_loops)
                        file: file.clone(),    // checkrs: allow(clone_in_loops)
                        bases,
                    },
                );
            }
        }

        // If the artifact JSON defines contract name `N`, but its AST contains
        // `N` and other contracts defined in the same source file, we still
        // want only the entry matching the artifact's contract (plus its
        // inheritance chain). To resolve parents, we need to index ALL
        // artifacts so we can find parent artifact files.
        self.index_all_parents(&mut sources, contract_name)?;

        Ok(sources)
    }

    /// Index all artifact files to find contract source definitions for
    /// parent contracts that may be defined in different source files.
    fn index_all_parents(
        &self,
        sources: &mut HashMap<String, ContractSource>,
        _root_name: &str,
    ) -> Result<()> {
        let index = ArtifactIndex::build(&self.out_dir);

        // Collect all contract names we still need to resolve.
        let mut to_resolve: Vec<String> = sources
            .values()
            .flat_map(|s| s.bases.iter().cloned())
            .filter(|name| !sources.contains_key(name))
            .collect();
        to_resolve.sort();
        to_resolve.dedup();

        for name in to_resolve {
            let Some(paths) = index.get(&name) else {
                debug!(name, "no artifact candidates found for parent");
                continue;
            };

            let mut selected: Option<HashMap<String, ContractSource>> = None;
            for path in paths {
                let more = self.parse_contract_sources(path)?;
                let declares_parent = more.contains_key(&name);
                debug!(
                    name,
                    path = %path.display(),
                    declares_parent,
                    "examined parent artifact candidate"
                );
                if declares_parent {
                    selected = Some(more);
                    break;
                }
            }

            if let Some(more) = selected {
                debug!(name, "selected parent artifact that declares the parent");
                for (key, value) in more {
                    sources.entry(key).or_insert(value);
                }
            }
        }

        Ok(())
    }

    /// Return the path relative to `project_root` as a string, falling back
    /// to the absolute path.
    fn rel_path_str(&self, file: &Path) -> String {
        file.strip_prefix(&self.project_root)
            .unwrap_or(file)
            .display()
            .to_string()
    }
}

/// Minimal artifact wrapper for extracting the AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

/// A contract extracted from an artifact AST: its name, source file, and base
/// contract names.
#[derive(Debug)]
struct ContractSource {
    name: String,
    file: String,
    bases: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritances")
    }

    fn ambiguous_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritances-ambiguous")
    }

    #[test]
    fn inspect_shows_inheritance_for_contract_with_no_parents() {
        let inspector = InheritanceGraphInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Base");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Inheritance graph:\n\nBase\n\nSources:\n\n1. Base (file: src/Base.sol)\n"
        );
    }

    #[test]
    fn inspect_shows_inheritance_chain() {
        let inspector = InheritanceGraphInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Child");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Inheritance graph:\n\nChild\n\u{2514}\u{2500}\u{2500} Middle\n    \u{2514}\u{2500}\u{2500} Base\n\nSources:\n\n1. Base (file: src/Base.sol)\n2. Child (file: src/Child.sol)\n3. Middle (file: src/Middle.sol)\n"
        );
    }

    #[test]
    fn inspect_shows_multiple_inheritance() {
        let inspector = InheritanceGraphInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("MultiChild");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Inheritance graph:\n\nMultiChild\n\u{251c}\u{2500}\u{2500} MultiBase\n\u{2514}\u{2500}\u{2500} AnotherBase\n\nSources:\n\n1. AnotherBase (file: src/AnotherBase.sol)\n2. MultiBase (file: src/MultiBase.sol)\n3. MultiChild (file: src/MultiChild.sol)\n"
        );
    }

    #[test]
    fn inspect_resolves_parent_when_first_artifact_has_no_declaration() {
        let inspector = InheritanceGraphInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("WrapperChild");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Inheritance graph:\n\nWrapperChild\n\u{2514}\u{2500}\u{2500} IParent\n\nSources:\n\n1. IParent (file: src/IParent.sol)\n2. WrapperChild (file: src/WrapperChild.sol)\n"
        );
    }

    #[test]
    fn inspect_allows_shared_ancestors_in_sibling_branches() {
        let inspector = InheritanceGraphInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("DiamondChild");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            "Inheritance graph:\n\nDiamondChild\n\u{251c}\u{2500}\u{2500} LeftBase\n\u{2502}   \u{2514}\u{2500}\u{2500} SharedBase\n\u{2514}\u{2500}\u{2500} RightBase\n    \u{2514}\u{2500}\u{2500} SharedBase\n\nSources:\n\n1. DiamondChild (file: src/DiamondChild.sol)\n2. LeftBase (file: src/LeftBase.sol)\n3. RightBase (file: src/RightBase.sol)\n4. SharedBase (file: src/SharedBase.sol)\n5. SharedBase (file: src/SharedBase.sol)\n"
        );
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let inspector = InheritanceGraphInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Nonexistent");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(err, "\"Nonexistent\" not found.");
    }

    #[test]
    fn inspect_errors_for_ambiguous_contract() {
        let inspector = InheritanceGraphInspector::new(Project::open(ambiguous_fixture_path()));
        let id = ArtifactId::new("Dupe");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            "found 2 \"Dupe\"\n\nSelect one of the following:\n\nhawk inspect inheritance-graph Dupe.sol:Dupe\nhawk inspect inheritance-graph lib/Dupe.sol:Dupe\n"
        );
    }

    #[test]
    fn inspect_resolves_path_name_format_for_first_dupe() {
        let inspector = InheritanceGraphInspector::new(Project::open(ambiguous_fixture_path()));
        let id = ArtifactId::new("Dupe.sol:Dupe");
        let output = inspector.inspect(&id).unwrap();
        assert!(output.to_string().contains("Inheritance graph:"));
        assert!(output.to_string().contains("Dupe"));
        assert!(output.to_string().contains("Dupe (file: src/Dupe.sol)"));
    }

    #[test]
    fn inspect_resolves_path_name_format_for_second_dupe() {
        let inspector = InheritanceGraphInspector::new(Project::open(ambiguous_fixture_path()));
        let id = ArtifactId::new("lib/Dupe.sol:Dupe");
        let output = inspector.inspect(&id).unwrap();
        assert!(output.to_string().contains("Inheritance graph:"));
        assert!(output.to_string().contains("Dupe"));
        assert!(output.to_string().contains("Dupe (file: src/lib/Dupe.sol)"));
    }

    #[test]
    fn inspect_errors_with_file_specified_but_not_found() {
        let inspector = InheritanceGraphInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Nonexistent.sol:Missing");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert!(err.contains("not found"));
    }
}
