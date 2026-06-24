//! Show the complete resolved source code of a Solidity function.
//!
//! This module is the CLI-facing layer for the `hawk inspect sources` command.
//! The core logic lives in [`crate::source_graph::SourceResolver`].

use std::path::Path;

use anyhow::Result;

use crate::project::Project;
use crate::source_graph::SourceResolver;

/// Run the source inspection for the given function ID.
///
/// `function_id` should be in the format `Contract::function`.
pub fn run(project_path: impl AsRef<Path>, function_id: &str) -> Result<String> {
    let project = Project::open(project_path.as_ref());
    project.validate()?;
    let resolver = SourceResolver::new(project);
    resolver.resolve(function_id)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/sources")
    }

    #[test]
    fn run_shows_source_for_execute() {
        let result = run(fixture_path(), "Main::execute").unwrap();
        assert_eq!(
            result,
            include_str!("../../fixtures/sources/expected/execute.txt")
        );
    }

    #[test]
    fn run_shows_source_with_recursive_refs() {
        let result = run(fixture_path(), "Main::_processData").unwrap();
        assert_eq!(
            result,
            include_str!("../../fixtures/sources/expected/process_data.txt")
        );
    }

    #[test]
    fn run_shows_source_for_overloaded_with_params() {
        let result = run(
            fixture_path(),
            "Overloaded::beforeTokenTransfer(address,address,uint256)",
        )
        .unwrap();
        assert_eq!(
            result,
            include_str!("../../fixtures/sources/expected/overloaded_exact.txt")
        );
    }

    #[test]
    fn run_errors_for_unknown_contract() {
        let result = run(fixture_path(), "Unknown::function");
        let err = result.unwrap_err().to_string();
        assert_eq!(err, "\"Unknown\" not found.");
    }

    #[test]
    fn run_errors_for_unknown_function() {
        let result = run(fixture_path(), "Main::unknownFunction");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\"unknownFunction\" not found in \"Main\".\n\nAvailable functions in \"Main\": _compute, _processData, execute",
        );
    }

    #[test]
    fn run_errors_for_overloaded_function() {
        let result = run(fixture_path(), "Overloaded::beforeTokenTransfer");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "found 2 \"Overloaded::beforeTokenTransfer\"\n\nSelect one of the following:\n\nhawk inspect sources \"Overloaded::beforeTokenTransfer(address,address)\"\nhawk inspect sources \"Overloaded::beforeTokenTransfer(address,address,uint256)\"\n",
        );
    }

    // Regression test: block-comment natspec (/** ... */) must be
    // resolved and included in the output. Previously, only the closing
    // `*/` line was captured because the backward scan broke on `*/`
    // instead of continuing up to the opening `/**`.
    #[test]
    fn run_shows_natspec_block_comment() {
        let result = run(fixture_path(), "NatspecBlock::compute").unwrap();
        assert_eq!(
            result,
            include_str!("../../fixtures/sources/expected/natspec_block.txt")
        );
    }
}
