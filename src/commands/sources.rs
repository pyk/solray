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
            include_str!("../../fixtures/sources/expected/run_shows_source_for_execute.txt")
        );
    }

    #[test]
    fn run_shows_source_with_recursive_refs() {
        let result = run(fixture_path(), "Main::_processData").unwrap();
        assert_eq!(
            result,
            include_str!(
                "../../fixtures/sources/expected/run_shows_source_with_recursive_refs.txt"
            )
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
            include_str!(
                "../../fixtures/sources/expected/run_shows_source_for_overloaded_with_params.txt"
            )
        );
    }

    #[test]
    fn run_errors_for_unknown_contract() {
        let result = run(fixture_path(), "Unknown::function");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!("../../fixtures/sources/expected/run_errors_for_unknown_contract.txt")
        );
    }

    #[test]
    fn run_errors_for_unknown_function() {
        let result = run(fixture_path(), "Main::unknownFunction");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!("../../fixtures/sources/expected/run_errors_for_unknown_function.txt")
        );
    }

    #[test]
    fn run_errors_for_overloaded_function() {
        let result = run(fixture_path(), "Overloaded::beforeTokenTransfer");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!("../../fixtures/sources/expected/run_errors_for_overloaded_function.txt")
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
            include_str!("../../fixtures/sources/expected/run_shows_natspec_block_comment.txt")
        );
    }

    // Regression test: user-defined types used in local variable
    // declarations must be resolved. Previously, type annotations in
    // VariableDeclarationStatement were never inspected, so structs and
    // enums referenced only as type names were silently dropped.
    #[test]
    fn run_resolves_user_defined_types_in_variable_declarations() {
        let result = run(fixture_path(), "TypeRefs::passThrough").unwrap();
        assert_eq!(
            result,
            include_str!(
                "../../fixtures/sources/expected/run_resolves_user_defined_types_in_variable_declarations.txt"
            )
        );
    }

    // Regression test: cross-file type references must be resolved.
    // Previously, resolve_id_in_ast only searched the current AST,
    // so structs defined in other files were silently dropped even
    // when the symbol index contained their location.
    #[test]
    fn run_resolves_cross_file_type_references() {
        let result = run(fixture_path(), "CrossFileConsumer::translate").unwrap();
        assert_eq!(
            result,
            include_str!(
                "../../fixtures/sources/expected/run_resolves_cross_file_type_references.txt"
            )
        );
    }

    // Regression test: IndexAccess expressions (e.g. `arr[i]`) must be
    // traversed. Previously, Expression::IndexAccess was not handled in
    // collect_from_expression, so entire expression subtrees were silently
    // dropped (including nested MemberAccess and FunctionCall nodes).
    #[test]
    fn run_resolves_index_access_expressions() {
        let result = run(fixture_path(), "IndexAccessTest::getItem").unwrap();
        assert_eq!(
            result,
            include_str!(
                "../../fixtures/sources/expected/run_resolves_index_access_expressions.txt"
            )
        );
    }

    // Integration test: incremental builds must not leak symbols across
    // build-info boundaries. The fixtures contain two build-info files
    // (Incremental.sol was compiled separately). Resolving Main::execute
    // must not include Item from the unrelated Incremental contract.
    #[test]
    fn incremental_build_does_not_leak_symbols() {
        let result = run(fixture_path(), "Main::execute").unwrap();
        assert_eq!(
            result,
            include_str!(
                "../../fixtures/sources/expected/incremental_build_does_not_leak_symbols.txt"
            )
        );
    }

    // Regression test: types used only in function return types
    // (and parameter types) must be resolved. Previously,
    // collect_from_contract_node only traversed the function body
    // statements, so types referenced solely in the signature
    // were silently dropped.
    #[test]
    fn run_resolves_function_return_types() {
        let result = run(fixture_path(), "ReturnTypeRef::makeWidget").unwrap();
        assert_eq!(
            result,
            include_str!("../../fixtures/sources/expected/run_resolves_function_return_types.txt")
        );
    }
}
