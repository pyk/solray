//! Entry point resolution for Solidity contracts.
//!
//! [`EntrypointsResolver`] resolves a deployable contract and emits the
//! list of writable functions exposed by its ABI.

use std::borrow::Cow;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use solc::abi::{Abi, AbiItem, Function, Param, StateMutability};
use solc::ast::SourceUnit;

use crate::artifact_index::ArtifactIndex;
use crate::project::{Declaration, Project};

/// Resolves the entrypoints exposed by a deployable contract.
pub struct EntrypointsResolver {
    project: Project,
    artifact_index: ArtifactIndex,
}

impl EntrypointsResolver {
    /// Build an [`EntrypointsResolver`] for the given project.
    pub fn new(project: Project) -> Self {
        let artifact_index = ArtifactIndex::build(project.out_dir());
        Self {
            project,
            artifact_index,
        }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Resolve a deployable contract and return the formatted entrypoint list.
    pub fn resolve(&self, deployable: &str) -> Result<String> {
        let project_root_abs = std::path::absolute(self.project.path())?;
        let (name, maybe_file) = parse_target(deployable);
        let deployables = self.project.deployable_contracts()?;
        let found: Vec<Declaration> = deployables.into_iter().filter(|d| d.name == name).collect();

        let declaration = match (maybe_file, found.len()) {
            (Some(file_path), _) => found
                .iter()
                .find(|d| rel_path_str(&d.file, &project_root_abs) == file_path)
                .with_context(|| {
                    format!(
                        "\"{}\" not found.\n\nAvailable matches for \"{}\":",
                        deployable, name
                    )
                })?,
            (None, 0) => {
                let names = self.available_contract_names()?;
                bail!(
                    "\"{}\" not found.\n\nAvailable contracts: {}",
                    deployable,
                    names.join(", ")
                );
            }
            (None, 1) => &found[0],
            (None, n) => {
                let mut found = found;
                found.sort_by(|a, b| a.file.cmp(&b.file));

                let mut msg = format!("found {} \"{}\"\n\nSelect one of the following:\n", n, name);
                for d in &found {
                    let rp = rel_path_str(&d.file, &project_root_abs);
                    msg.push_str(&format!("\nhawk inspect entrypoints {}:{}", rp, d.name));
                }
                msg.push('\n');
                bail!(msg);
            }
        };

        let artifact = self.load_contract_artifact(declaration)?;
        let entrypoints: Vec<String> = artifact
            .abi
            .items
            .iter()
            .filter_map(write_entrypoint_signature)
            .collect();

        Ok(format_output(&entrypoints))
    }

    fn available_contract_names(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .project
            .deployable_contracts()?
            .into_iter()
            .map(|d| d.name)
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    fn load_contract_artifact(&self, declaration: &Declaration) -> Result<ContractArtifact> {
        let candidates = self.artifact_index.try_get(&declaration.name)?;

        if candidates.len() == 1 {
            let artifact = load_artifact(&candidates[0])?;
            let abi = artifact.abi.with_context(|| {
                format!("artifact `{}` is missing the ABI", candidates[0].display())
            })?;
            return Ok(ContractArtifact { abi });
        }

        for candidate in candidates {
            let artifact = load_artifact(&candidate)?;
            if let Some(ast) = &artifact.ast
                && ast.absolute_path == declaration.file
            {
                let abi = artifact.abi.with_context(|| {
                    format!("artifact `{}` is missing the ABI", candidate.display())
                })?;
                return Ok(ContractArtifact { abi });
            }
        }

        bail!(
            "unable to resolve artifact for `{}` in `{}`",
            declaration.name,
            declaration.file.display()
        )
    }
}

#[derive(Deserialize)]
struct Artifact {
    abi: Option<Abi>,
    ast: Option<SourceUnit>,
}

struct ContractArtifact {
    abi: Abi,
}

fn load_artifact(path: impl AsRef<Path>) -> Result<Artifact> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn write_entrypoint_signature(item: &AbiItem) -> Option<String> {
    let AbiItem::Function(function) = item else {
        return None;
    };

    if !is_write_function(function) {
        return None;
    }

    Some(format!(
        "{}({})",
        function.name,
        format_params(&function.inputs)
    ))
}

fn is_write_function(function: &Function) -> bool {
    matches!(
        function.state_mutability,
        StateMutability::Nonpayable | StateMutability::Payable
    )
}

fn format_params(params: &[Param]) -> String {
    params
        .iter()
        .map(|p| p.r#type.as_str())
        .collect::<Vec<&str>>()
        .join(",")
}

fn format_output(entrypoints: &[String]) -> String {
    let mut output = String::new();
    output.push_str(&format!("Found {} entrypoints\n\n", entrypoints.len()));
    for (i, entrypoint) in entrypoints.iter().enumerate() {
        output.push_str(&format!("{}. {}\n", i + 1, entrypoint));
    }
    output
}

/// Split `deployable` into (name, optional_file_path).
fn parse_target(deployable: &str) -> (&str, Option<&str>) {
    match deployable.rsplit_once(':') {
        Some((path, name)) if !path.is_empty() && !name.is_empty() => (name, Some(path)),
        _ => (deployable, None),
    }
}

fn rel_path_str<'a>(file: &'a Path, project_root: &Path) -> Cow<'a, str> {
    file.strip_prefix(project_root)
        .unwrap_or(file)
        .to_string_lossy()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/entrypoints")
    }

    #[test]
    fn resolve_shows_entrypoints_for_a_unique_contract() {
        let project = Project::open(fixture_path());
        let resolver = EntrypointsResolver::new(project);
        let result = resolver.resolve("ContractB").unwrap();
        assert_eq!(
            result,
            "\
Found 2 entrypoints\n\n1. charge()\n2. update(address)\n"
        );
    }

    #[test]
    fn resolve_shows_entrypoints_for_path_qualified_contract() {
        let project = Project::open(fixture_path());
        let resolver = EntrypointsResolver::new(project);
        let result = resolver.resolve("src/Foo.sol:ContractA").unwrap();
        assert_eq!(
            result,
            "\
Found 2 entrypoints\n\n1. entrypointOne(string)\n2. payMe()\n"
        );
    }

    #[test]
    fn resolve_errors_for_unknown_contract() {
        let project = Project::open(fixture_path());
        let resolver = EntrypointsResolver::new(project);
        let result = resolver.resolve("Missing");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\
\"Missing\" not found.\n\nAvailable contracts: ContractA, ContractB"
        );
    }

    #[test]
    fn resolve_errors_for_ambiguous_contract() {
        let project = Project::open(fixture_path());
        let resolver = EntrypointsResolver::new(project);
        let result = resolver.resolve("ContractA");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\
found 2 \"ContractA\"\n\nSelect one of the following:\n\nhawk inspect entrypoints src/Bar.sol:ContractA\nhawk inspect entrypoints src/Foo.sol:ContractA\n"
        );
    }
}
