//! Abstract contract inspection for Foundry projects.
//!
//! [`AbstractInspector`] scans the artifact directory and produces structured
//! output for each abstract contract found.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::project::Project;

/// A single abstract contract.
pub struct Abstract {
    /// The contract name.
    pub name: String,
    /// The source file path (relative to the project root).
    pub path: PathBuf,
}

impl Abstract {
    /// Create a new [`Abstract`] from a name and path.
    pub fn new(name: impl Into<String>, path: impl AsRef<Path>) -> Self {
        Self {
            name: name.into(),
            path: path.as_ref().to_path_buf(),
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
}

impl AbstractInspectorOutput {
    /// Create a new [`AbstractInspectorOutput`] from a list of abstracts.
    pub fn new(abstracts: Vec<Abstract>) -> Self {
        Self { abstracts }
    }
}

impl std::fmt::Display for AbstractInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "found {} abstracts\n", self.abstracts.len())?;
        for (i, abstract_) in self.abstracts.iter().enumerate() {
            writeln!(f, "{}. {}", i + 1, abstract_)?;
        }
        Ok(())
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
    pub fn project_path(&self) -> &std::path::Path {
        self.project.path()
    }

    /// Inspect the project and return all abstract contracts.
    pub fn inspect(&self) -> Result<AbstractInspectorOutput> {
        self.project.validate()?;

        let project_root_abs = std::path::absolute(self.project.path())?;
        let declarations = self.project.abstract_contracts()?;
        let abstracts: Vec<Abstract> = declarations
            .into_iter()
            .map(|d| {
                let rel = d.file.strip_prefix(&project_root_abs).unwrap_or(&d.file);
                Abstract::new(d.name, rel)
            })
            .collect();
        Ok(AbstractInspectorOutput::new(abstracts))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_project() -> Project {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-contracts");
        Project::open(path)
    }

    #[test]
    fn inspect_returns_abstract_contracts_only() {
        let inspector = AbstractInspector::new(fixture_project());
        let output = inspector.inspect().unwrap();

        assert_eq!(
            output
                .abstracts
                .iter()
                .map(|a: &Abstract| a.to_string())
                .collect::<Vec<String>>(),
            vec!["src/AbstractBase.sol:AbstractBase"]
        );
    }

    #[test]
    fn abstract_display_formats_as_path_colon_name() {
        let abstract_ = Abstract::new("AbstractBase", "src/AbstractBase.sol");
        assert_eq!(abstract_.to_string(), "src/AbstractBase.sol:AbstractBase");
    }

    #[test]
    fn output_display_numbers_each_abstract() {
        let abstract_ = Abstract::new("AbstractBase", "src/AbstractBase.sol");
        let output = AbstractInspectorOutput::new(vec![abstract_]);
        assert_eq!(
            output.to_string(),
            "found 1 abstracts\n\n1. src/AbstractBase.sol:AbstractBase\n"
        );
    }
}
