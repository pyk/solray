//! Show the inheritance graph of a Solidity declaration.
//!
//! This module is the CLI-facing layer for the `hawk inspect inheritances` command.
//! The core logic lives in [`crate::project::Project`].

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::inheritance::InheritanceNode;
use crate::project::Project;

/// Run the inheritance inspection for the given declaration.
///
/// `decl` can be either:
/// - A plain name (e.g. `Address`)
/// - A `path:name` pair (e.g. `lib/mc/src/devkit/Flattened.sol:Address`)
///
/// Returns the formatted output on success. Returns an error with a
/// user-friendly message when the declaration is not found, or when
/// multiple declarations share the same name.
pub fn run(decl: &str, path: impl AsRef<Path>) -> Result<String> {
    let project = Project::open(path)?;
    let project_root = project.path().to_path_buf();

    let (name, maybe_file) = parse_decl(decl);

    let found = project.find_declarations_by_name(name)?;

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
            let tree = project.inheritance_tree_by_path(name, &matched.file)?;
            format_output(tree)
        }
        // name-only, not found
        (None, 0) => {
            let decls = project.declarations()?;
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
            let tree = project.inheritance_tree(name)?;
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
    fn run_errors_for_ambiguous_declaration() {
        let result = run("Dupe", ambiguous_fixture_path());
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\
found 2 \"Dupe\"\n\
\n\
Select one of the following:\n\
\n\
hawk inspect inheritances src/Dupe.sol:Dupe\n\
hawk inspect inheritances src/lib/Dupe.sol:Dupe\n\
",
        );
    }

    #[test]
    fn run_resolves_path_name_format_for_first_dupe() {
        let result = run("src/Dupe.sol:Dupe", ambiguous_fixture_path()).unwrap();
        assert!(result.contains("Inheritance graph:"));
        assert!(result.contains("Dupe"));
        assert!(result.contains("src/Dupe.sol:Dupe"));
    }

    #[test]
    fn run_resolves_path_name_format_for_second_dupe() {
        let result = run("src/lib/Dupe.sol:Dupe", ambiguous_fixture_path()).unwrap();
        assert!(result.contains("Inheritance graph:"));
        assert!(result.contains("Dupe"));
        assert!(result.contains("src/lib/Dupe.sol:Dupe"));
    }

    #[test]
    fn run_errors_for_unknown_declaration() {
        let result = run("Nonexistent", fixture_path());
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\
\"Nonexistent\" not found.

Available declarations: AnotherBase, Base, Child, Middle, MultiBase, MultiChild",
        );
    }

    #[test]
    fn run_shows_inheritance_for_contract_with_no_parents() {
        let result = run("Base", fixture_path()).unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:

Base

Sources:

1. src/Base.sol:Base
",
        );
    }

    #[test]
    fn run_shows_inheritance_chain() {
        let result = run("Child", fixture_path()).unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:

Child
\u{2514}\u{2500}\u{2500} Middle
    \u{2514}\u{2500}\u{2500} Base

Sources:

1. src/Child.sol:Child
2. src/Middle.sol:Middle
3. src/Base.sol:Base
",
        );
    }

    #[test]
    fn run_shows_multiple_inheritance() {
        let result = run("MultiChild", fixture_path()).unwrap();
        assert_eq!(
            result,
            "\
Inheritance graph:

MultiChild
\u{251c}\u{2500}\u{2500} MultiBase
\u{2514}\u{2500}\u{2500} AnotherBase

Sources:

1. src/MultiChild.sol:MultiChild
2. src/MultiBase.sol:MultiBase
3. src/AnotherBase.sol:AnotherBase
",
        );
    }
}
