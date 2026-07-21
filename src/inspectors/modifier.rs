//! Modifier inspection for Foundry projects.
//!
//! [`ModifierInspector`] reads a single artifact file and resolves
//! the full set of modifiers available in a contract, including those
//! inherited from parent contracts.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use solc::ast::{ContractDefinitionNode, ContractKind, SourceUnit, SourceUnitNode};
use tracing::debug;

use crate::artifact_index::ArtifactIndex;
use crate::inspectors::artifact_id::ArtifactId;
use crate::project::Project;

/// A single modifier definition found in a contract or one of its ancestors.
#[derive(Debug, Clone)]
pub struct ModifierInfo {
    /// The modifier name.
    pub name: String,
    /// The source file path (relative to the project root).
    pub source_file: String,
    /// The line number where the modifier is defined.
    pub line: usize,
}

/// The output of a [`ModifierInspector`] inspection.
#[derive(Debug)]
pub struct ModifierInspectorOutput {
    contract_name: String,
    source_file: String,
    modifiers: Vec<ModifierInfo>,
}

impl ModifierInspectorOutput {
    /// Create a new [`ModifierInspectorOutput`].
    pub fn new(contract_name: &str, source_file: &str, modifiers: Vec<ModifierInfo>) -> Self {
        Self {
            contract_name: contract_name.to_string(),
            source_file: source_file.to_string(),
            modifiers,
        }
    }
}

impl std::fmt::Display for ModifierInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Contract: {}", self.contract_name)?;
        writeln!(f, "Source: {}", self.source_file)?;
        writeln!(f)?;
        writeln!(f, "Found {} modifiers.", self.modifiers.len())?;
        writeln!(f)?;
        for (i, modifier) in self.modifiers.iter().enumerate() {
            writeln!(
                f,
                "{}. {} (source: {}:{})",
                i + 1,
                modifier.name,
                modifier.source_file,
                modifier.line
            )?;
        }
        Ok(())
    }
}

/// Inspect a Foundry project for a single contract's modifiers.
pub struct ModifierInspector {
    project: Project,
}

impl ModifierInspector {
    /// Build a [`ModifierInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the modifiers for the given [`ArtifactId`].
    ///
    /// Returns all modifiers defined in the contract itself and any of its
    /// ancestors in the inheritance chain.
    pub fn inspect(&self, id: &ArtifactId) -> Result<ModifierInspectorOutput> {
        debug!(
            contract = %id.name,
            project = %self.project.path().display(),
            "starting modifier inspection"
        );
        let artifact_path = self.resolve_artifact_path(id)?;
        debug!(path = %artifact_path.display(), "resolved root artifact");
        let contract_name = id.name.clone();
        let project_root = self.project.path().to_path_buf();

        let ctx = ResolutionContext {
            project_root,
            out_dir: self.project.out_dir().to_path_buf(),
        };

        let contract_defs = ctx.build_contract_sources(&artifact_path, &contract_name)?;
        debug!(
            contract = %contract_name,
            definitions = ?contract_defs.keys().collect::<Vec<&String>>(),
            "resolved contract definitions"
        );

        let source_file = contract_defs
            .get(&contract_name)
            .map(|d| d.file.clone()) // checkrs: allow(clone_in_iterator)
            .unwrap_or_default();

        let mut modifiers = Vec::new();
        let mut visited = HashSet::new();
        ctx.collect_modifiers(&contract_name, &contract_defs, &mut modifiers, &mut visited)?;
        debug!(
            contract = %contract_name,
            modifiers = ?modifiers.iter().map(|modifier| &modifier.name).collect::<Vec<&String>>(),
            "collected modifiers before deduplication"
        );

        // Deduplicate by name, keeping the first (most derived) occurrence.
        let mut seen = HashSet::new();
        modifiers.retain(|m| seen.insert(m.name.clone()));

        Ok(ModifierInspectorOutput::new(
            &contract_name,
            &source_file,
            modifiers,
        ))
    }

    /// Resolve the artifact path for an [`ArtifactId`].
    fn resolve_artifact_path(&self, id: &ArtifactId) -> Result<PathBuf> {
        match &id.file {
            Some(file) => {
                let path = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                ensure!(path.exists(), "artifact `{}` not found", path.display());
                Ok(path)
            }
            None => {
                let index = ArtifactIndex::build(self.project.out_dir());
                let candidates = index.get(&id.name).cloned().unwrap_or_default();
                debug!(name = %id.name, candidates = ?candidates, "looked up root artifact candidates");
                match candidates.len() {
                    0 => bail!("\"{}\" not found.", id.name),
                    1 => {
                        let path = candidates
                            .into_iter()
                            .next()
                            .context("expected one candidate but got none")?;
                        Ok(path)
                    }
                    n => {
                        let mut sorted = candidates;
                        sorted.sort();
                        let mut msg = format!(
                            "found {n} \"{}\"\n\nSelect one of the following:\n",
                            id.name
                        );
                        for candidate in &sorted {
                            let parent = candidate
                                .parent()
                                .and_then(|p| p.file_name())
                                .and_then(|n| n.to_str())
                                .unwrap_or("");
                            msg.push_str(&format!("\nhawk inspect modifiers {parent}:{}", id.name));
                        }
                        msg.push('\n');
                        bail!(msg);
                    }
                }
            }
        }
    }
}

/// Context for resolving modifiers across artifact files.
struct ResolutionContext {
    project_root: PathBuf,
    out_dir: PathBuf,
}

impl ResolutionContext {
    /// Build a map of contract sources starting from the given artifact and
    /// resolving all parent contracts.
    fn build_contract_sources(
        &self,
        artifact_path: impl AsRef<Path>,
        root_name: &str,
    ) -> Result<HashMap<String, ContractDef>> {
        let artifact_path = artifact_path.as_ref();
        debug!(path = %artifact_path.display(), root = %root_name, "building contract source map");
        let index = ArtifactIndex::build(&self.out_dir);
        let mut sources = HashMap::new();
        self.parse_and_resolve(artifact_path, &index, &mut sources)?;

        ensure!(
            sources.contains_key(root_name),
            "contract `{root_name}` not found in `{}`",
            artifact_path.display()
        );

        Ok(sources)
    }

    /// Parse a single artifact and merge its contract definitions into
    /// `sources`, then recursively resolve any parent contracts not yet
    /// indexed.
    fn parse_and_resolve(
        &self,
        artifact_path: impl AsRef<Path>,
        index: &ArtifactIndex,
        sources: &mut HashMap<String, ContractDef>,
    ) -> Result<()> {
        let artifact_path = artifact_path.as_ref();
        debug!(
            path = %artifact_path.display(),
            known_definitions = ?sources.keys().collect::<Vec<&String>>(),
            "parsing artifact for contract definitions"
        );
        let new_sources = self.parse_artifact(artifact_path)?;

        // Merge newly parsed contracts, then recursively resolve their
        // parents.
        for (name, def) in new_sources {
            if sources.contains_key(&name) {
                debug!(name = %name, "skipping already known contract definition");
                continue;
            }
            debug!(name = %name, bases = ?def.bases, "adding contract definition");
            sources.insert(name, def);
        }

        self.resolve_parents(index, sources)?;

        Ok(())
    }

    /// Parse a single artifact and return its contract definitions.
    fn parse_artifact(
        &self,
        artifact_path: impl AsRef<Path>,
    ) -> Result<HashMap<String, ContractDef>> {
        let path = artifact_path.as_ref();
        debug!(path = %path.display(), "reading artifact");
        let content = fs::read_to_string(path)?;
        let artifact: Artifact = serde_json::from_str(&content)?;

        let ast = artifact.ast.with_context(|| {
            format!(
                "artifact `{}` is missing the AST; rebuild with `ast = true` in foundry.toml",
                path.display()
            )
        })?;

        let file = self.rel_path_str(&ast.absolute_path);
        debug!(path = %path.display(), source_file = %file, node_count = ast.nodes.len(), "parsing artifact AST");
        let mut result = HashMap::new();

        for node in &ast.nodes {
            if let SourceUnitNode::ContractDefinition(cd) = node
                && (cd.contract_kind == ContractKind::Contract
                    || cd.contract_kind == ContractKind::Interface
                    || cd.contract_kind == ContractKind::Library)
            {
                let bases: Vec<String> = cd
                    .base_contracts
                    .iter()
                    .map(|bc| bc.base_name.name.clone()) // checkrs: allow(clone_in_iterator, clone_in_loops)
                    .collect();

                let modifiers: Vec<ModifierEntry> = cd
                    .nodes
                    .iter()
                    .filter_map(|n| {
                        if let ContractDefinitionNode::ModifierDefinition(md) = n {
                            Some(ModifierEntry {
                                name: md.name.clone(), // checkrs: allow(clone_in_loops)
                                src_offset: md.src.offset,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                result.insert(
                    cd.name.clone(), // checkrs: allow(clone_in_loops)
                    ContractDef {
                        file: file.clone(), // checkrs: allow(clone_in_loops)
                        bases,
                        modifiers,
                    },
                );
            }
        }

        Ok(result)
    }

    /// Find and recursively resolve parent contracts not yet in `sources`.
    fn resolve_parents(
        &self,
        index: &ArtifactIndex,
        sources: &mut HashMap<String, ContractDef>,
    ) -> Result<()> {
        loop {
            let mut to_resolve: Vec<String> = sources
                .values()
                .flat_map(|s| s.bases.iter().cloned())
                .filter(|name| !sources.contains_key(name))
                .collect();
            to_resolve.sort();
            to_resolve.dedup();

            debug!(
                parents = ?to_resolve,
                known_definitions = ?sources.keys().collect::<Vec<&String>>(),
                "resolving parent contracts"
            );
            if to_resolve.is_empty() {
                return Ok(());
            }

            let mut added_definition = false;
            for name in to_resolve {
                let Some(paths) = index.get(&name) else {
                    debug!(name = %name, "no artifact candidates found for parent");
                    continue;
                };

                for path in paths {
                    let candidate = self.parse_artifact(path)?;
                    let declares_parent = candidate.contains_key(&name);
                    debug!(
                        name = %name,
                        path = %path.display(),
                        declares_parent,
                        "examined parent artifact candidate"
                    );
                    if declares_parent {
                        debug!(name = %name, "selected parent artifact that declares the parent");
                        for (candidate_name, definition) in candidate {
                            if sources.insert(candidate_name, definition).is_none() {
                                added_definition = true;
                            }
                        }
                        break;
                    }
                }
            }

            if !added_definition {
                return Ok(());
            }
        }
    }

    /// Recursively collect modifiers from a contract and its ancestors.
    fn collect_modifiers(
        &self,
        name: &str,
        sources: &HashMap<String, ContractDef>,
        out: &mut Vec<ModifierInfo>,
        visited: &mut HashSet<String>,
    ) -> Result<()> {
        if !visited.insert(name.to_string()) {
            debug!(name = %name, "skipping already visited contract");
            return Ok(());
        }

        debug!(name = %name, "collecting modifiers");
        let def = sources
            .get(name)
            .with_context(|| format!("contract `{name}` not found in artifacts"))?;

        for modifier in &def.modifiers {
            debug!(contract = %name, modifier = %modifier.name, source_file = %def.file, offset = modifier.src_offset, "found modifier definition");
            let line = self
                .byte_offset_to_line(modifier.src_offset, &def.file)
                .unwrap_or(0);
            out.push(ModifierInfo {
                name: modifier.name.clone(),   // checkrs: allow(clone_in_loops)
                source_file: def.file.clone(), // checkrs: allow(clone_in_loops)
                line,
            });
        }

        for base in &def.bases {
            self.collect_modifiers(base, sources, out, visited)?;
        }

        Ok(())
    }

    /// Return the path relative to `project_root` as a string.
    fn rel_path_str(&self, file: &Path) -> String {
        file.strip_prefix(&self.project_root)
            .unwrap_or(file)
            .display()
            .to_string()
    }

    /// Convert a byte offset in a source file to a 1-indexed line number.
    fn byte_offset_to_line(&self, offset: usize, relative_path: impl AsRef<Path>) -> Result<usize> {
        let full_path = self.project_root.join(relative_path);
        let content = fs::read(&full_path)
            .with_context(|| format!("failed to read source file `{}`", full_path.display()))?;
        let line = content[..offset.min(content.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        Ok(line + 1)
    }
}

/// Minimal artifact wrapper for extracting the AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

/// A single modifier entry stored during AST parsing.
#[derive(Debug, Clone)]
struct ModifierEntry {
    name: String,
    src_offset: usize,
}

/// A contract extracted from an artifact AST: its source file, base
/// contract names, and modifier entries.
#[derive(Debug)]
struct ContractDef {
    file: String,
    bases: Vec<String>,
    modifiers: Vec<ModifierEntry>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/modifiers")
    }

    #[test]
    fn inspect_modifiers_child() {
        let inspector = ModifierInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ModifiersChild");
        let output = inspector.inspect(&id).unwrap();
        let text = output.to_string();
        let expected = include_str!("../../fixtures/modifiers/expected/ModifiersChild.txt");
        assert_eq!(text, expected);
    }

    #[test]
    fn inspect_modifiers_middle() {
        let inspector = ModifierInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ModifiersMiddle");
        let output = inspector.inspect(&id).unwrap();
        let text = output.to_string();
        let expected = include_str!("../../fixtures/modifiers/expected/ModifiersMiddle.txt");
        assert_eq!(text, expected);
    }

    #[test]
    fn inspect_modifiers_base() {
        let inspector = ModifierInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ModifiersBase");
        let output = inspector.inspect(&id).unwrap();
        let text = output.to_string();
        let expected = include_str!("../../fixtures/modifiers/expected/ModifiersBase.txt");
        assert_eq!(text, expected);
    }

    #[test]
    fn inspect_modifiers_with_interface_base() {
        let inspector = ModifierInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ModifiersInterfaceChild");
        let output = inspector.inspect(&id).unwrap();
        let text = output.to_string();
        let expected =
            include_str!("../../fixtures/modifiers/expected/ModifiersInterfaceChild.txt");
        assert_eq!(text, expected);
    }

    #[test]
    fn inspect_modifiers_skips_artifact_without_parent_definition() {
        let inspector = ModifierInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ModifierRoot");
        let output = inspector.inspect(&id).unwrap();
        let text = output.to_string();
        let expected = include_str!("../../fixtures/modifiers/expected/ModifierRoot.txt");
        assert_eq!(text, expected);
    }
}
