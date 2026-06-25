//! Inheritance resolution for Solidity declarations.
//!
//! [`InheritanceResolver`] resolves a declaration and emits the formatted
//! inheritance graph and source list.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::inheritance::InheritanceNode;
use crate::project::Project;

/// Resolves inheritance graphs for declarations in a Foundry project.
pub struct InheritanceResolver {
    project: Project,
}

impl InheritanceResolver {
    /// Build an [`InheritanceResolver`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Resolve a declaration and return the formatted inheritance graph.
    pub fn resolve(&self, decl: &str) -> Result<String> {
        self.project.validate()?;
        let project_root = self.project.path().to_path_buf();
        let (name, maybe_file) = parse_decl(decl);

        let found = self.project.find_declarations_by_name(name)?;

        match (maybe_file, found.len()) {
            // path:name format -- resolve to exactly one declaration
            (Some(file_path), _) => {
                let matched = found
                    .iter()
                    .find(|d| rel_path_str(&d.file, &project_root) == file_path)
                    .with_context(|| {
                        format!(
                            "\"{}\" not found.\n\nAvailable matches for \"{}\":",
                            decl, name
                        )
                    })?;
                let tree = self.project.inheritance_tree_by_path(name, &matched.file)?;
                format_output(tree)
            }
            // name-only, not found
            (None, 0) => {
                let decls = self.project.declarations()?;
                let mut names: Vec<String> = decls.into_iter().map(|d| d.name).collect();
                names.dedup();
                bail!(
                    "\"{}\" not found.\n\nAvailable declarations: {}",
                    decl,
                    names.join(", ")
                );
            }
            // name-only, exactly one match
            (None, 1) => {
                let tree = self.project.inheritance_tree(name)?;
                format_output(tree)
            }
            // name-only, multiple matches -- ambiguity
            (None, n) => {
                let mut found = found;
                found.sort_by(|a, b| a.file.cmp(&b.file));

                let mut msg = format!("found {} \"{}\"\n\nSelect one of the following:\n", n, name);
                for d in &found {
                    let rp = rel_path_str(&d.file, &project_root);
                    msg.push_str(&format!("\nhawk inspect inheritances {}:{}", rp, d.name));
                }
                msg.push('\n');
                bail!(msg);
            }
        }
    }

    /// Return the formatted output for a resolved inheritance tree.
    pub fn format(&self, tree: InheritanceNode) -> Result<String> {
        format_output(tree)
    }
}

/// Split `decl` into (name, optional_file_path).
///
/// A `path:name` input like `src/Base.sol:Base` yields `("Base", Some("src/Base.sol"))`.
/// A plain `Base` yields `("Base", None)`.
fn parse_decl(decl: &str) -> (&str, Option<&str>) {
    match decl.rsplit_once(':') {
        Some((path, name)) if !path.is_empty() && !name.is_empty() => (name, Some(path)),
        _ => (decl, None),
    }
}

/// Return `file` relative to `project_root` as a string, falling back to the
/// original path.
fn rel_path_str<'a>(file: &'a Path, project_root: &Path) -> std::borrow::Cow<'a, str> {
    file.strip_prefix(project_root)
        .unwrap_or(file)
        .to_string_lossy()
}

fn format_output(tree: InheritanceNode) -> Result<String> {
    let sources = tree.flatten_sources();
    let mut output = String::new();

    output.push_str("Inheritance graph:\n\n");
    output.push_str(&tree.to_string());

    output.push_str("\nSources:\n\n");
    for (i, (file, name)) in sources.iter().enumerate() {
        output.push_str(&format!("{}. {}:{}\n", i + 1, file, name));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritances")
    }

    fn ambiguous_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritances-ambiguous")
    }

    #[test]
    fn resolve_errors_for_ambiguous_declaration() {
        let project = Project::open(ambiguous_fixture_path());
        let resolver = InheritanceResolver::new(project);
        let err = resolver.resolve("Dupe").unwrap_err().to_string();
        assert_eq!(
            err,
            "\
found 2 \"Dupe\"\n\nSelect one of the following:\n\nhawk inspect inheritances src/Dupe.sol:Dupe\nhawk inspect inheritances src/lib/Dupe.sol:Dupe\n"
        );
    }

    #[test]
    fn resolve_resolves_path_name_format_for_first_dupe() {
        let project = Project::open(ambiguous_fixture_path());
        let resolver = InheritanceResolver::new(project);
        let result = resolver.resolve("src/Dupe.sol:Dupe").unwrap();
        assert!(result.contains("Inheritance graph:"));
        assert!(result.contains("Dupe"));
        assert!(result.contains("src/Dupe.sol:Dupe"));
    }

    #[test]
    fn resolve_resolves_path_name_format_for_second_dupe() {
        let project = Project::open(ambiguous_fixture_path());
        let resolver = InheritanceResolver::new(project);
        let result = resolver.resolve("src/lib/Dupe.sol:Dupe").unwrap();
        assert!(result.contains("Inheritance graph:"));
        assert!(result.contains("Dupe"));
        assert!(result.contains("src/lib/Dupe.sol:Dupe"));
    }

    #[test]
    fn resolve_errors_for_unknown_declaration() {
        let project = Project::open(fixture_path());
        let resolver = InheritanceResolver::new(project);
        let err = resolver.resolve("Nonexistent").unwrap_err().to_string();
        assert_eq!(
            err,
            "\
\"Nonexistent\" not found.\n\nAvailable declarations: AnotherBase, Base, Child, Middle, MultiBase, MultiChild"
        );
    }

    #[test]
    fn resolve_shows_inheritance_for_contract_with_no_parents() {
        let project = Project::open(fixture_path());
        let resolver = InheritanceResolver::new(project);
        let result = resolver.resolve("Base").unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:\n\nBase\n\nSources:\n\n1. src/Base.sol:Base\n"
        );
    }

    #[test]
    fn resolve_shows_inheritance_chain() {
        let project = Project::open(fixture_path());
        let resolver = InheritanceResolver::new(project);
        let result = resolver.resolve("Child").unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:\n\nChild\n\u{2514}\u{2500}\u{2500} Middle\n    \u{2514}\u{2500}\u{2500} Base\n\nSources:\n\n1. src/Child.sol:Child\n2. src/Middle.sol:Middle\n3. src/Base.sol:Base\n"
        );
    }

    #[test]
    fn resolve_shows_multiple_inheritance() {
        let project = Project::open(fixture_path());
        let resolver = InheritanceResolver::new(project);
        let result = resolver.resolve("MultiChild").unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:\n\nMultiChild\n\u{251c}\u{2500}\u{2500} MultiBase\n\u{2514}\u{2500}\u{2500} AnotherBase\n\nSources:\n\n1. src/MultiChild.sol:MultiChild\n2. src/MultiBase.sol:MultiBase\n3. src/AnotherBase.sol:AnotherBase\n"
        );
    }
}
