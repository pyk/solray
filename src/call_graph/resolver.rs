//! Call graph resolution for Solidity functions.
//!
//! [`CallGraphResolver`] resolves call graphs using a lightweight
//! [`FunctionIndex`] for O(1) lookups from AST node IDs to artifact
//! paths. Full AST parsing happens only for artifacts that are actually
//! reached during call graph traversal.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use solc::ast::{
    ContractDefinitionNode, Expression, FunctionCallExpression, FunctionDefinition, SourceUnitNode,
    TypeName, VariableDeclaration, Visibility,
};

use crate::artifact_index::ArtifactIndex;
use crate::call_graph::{FunctionID, node::CallGraphNode};
use crate::function_index::FunctionIndex;
use crate::project::Project;

/// Function information extracted from an artifact AST for call graph resolution.
#[derive(Debug, Clone)]
struct FunctionInfo {
    id: i64,
    name: String,
    contract_name: String,
    file: PathBuf,
    parameters: Vec<VariableDeclaration>,
    visibility: Visibility,
    definition: FunctionDefinition,
}

/// Minimal artifact wrapper for extracting the AST on demand.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<solc::ast::SourceUnit>,
}

/// Resolves call graphs for Solidity functions in a Foundry project.
///
/// A lightweight [`FunctionIndex`] is built at construction time (scanning
/// only function definition IDs, not full ASTs). During call graph traversal,
/// artifact files are parsed on demand using the index for O(1) lookup from
/// callee ID to artifact path.
pub struct CallGraphResolver {
    project: Project,
    /// Lightweight index: contract name -> one or more artifact paths
    /// (kept for ambiguity detection).
    artifact_index: ArtifactIndex,
    /// Lightweight index: Solc AST node ID -> artifact path + source file.
    function_index: FunctionIndex,
    /// Cache of already-parsed artifacts: source file -> Vec<FunctionInfo>.
    /// Populated on demand during traversal.
    artifact_cache: RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
}

impl CallGraphResolver {
    /// Build a [`CallGraphResolver`] that owns a [`Project`].
    ///
    /// Builds a lightweight [`FunctionIndex`] by scanning all artifacts
    /// for function definition IDs (no full AST parsing). Full parsing
    /// happens lazily during call graph traversal.
    pub fn new(project: Project) -> Self {
        let artifact_index = ArtifactIndex::build(project.out_dir());
        let function_index = FunctionIndex::build(&artifact_index);
        Self {
            project,
            artifact_index,
            function_index,
            artifact_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Resolve a `Contract::function` ID and return its call graph.
    ///
    /// Parses only the artifacts needed to traverse the call graph,
    /// using the [`FunctionIndex`] for O(1) lookups from callee IDs
    /// to artifact paths.
    #[tracing::instrument(skip(self))]
    pub fn resolve(&self, function_id: &str) -> Result<CallGraphNode> {
        let fid = FunctionID::try_from(function_id)?;
        tracing::trace!(?function_id);

        // Detect contract-level ambiguity from the artifact index.
        if let Ok(artifact_paths) = self.artifact_index.try_get(fid.contract_name())
            && artifact_paths.len() > 1
        {
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                artifact_paths.len(),
                fid.contract_name()
            );
            for artifact_path in &artifact_paths {
                let rp = artifact_path
                    .strip_prefix(self.project.path())
                    .unwrap_or(artifact_path)
                    .to_string_lossy();
                msg.push_str(&format!("\nhawk inspect calls {}:{}", rp, fid));
            }
            msg.push('\n');
            bail!(msg);
        }

        // Parse the target contract's artifacts to get initial functions.
        let artifact_paths = self.artifact_index.try_get(fid.contract_name())?;
        let mut functions: HashMap<i64, FunctionInfo> = HashMap::new();
        for artifact_path in &artifact_paths {
            self.load_artifact(artifact_path, &mut functions)?;
        }

        // Find the target function, deduplicating across source files.
        let mut seen = HashSet::new();
        let target: Vec<&FunctionInfo> = functions
            .values()
            .filter(|fi| {
                fi.contract_name == fid.contract_name()
                    && fi.name == fid.function_name()
                    && seen.insert((fi.name.clone(), fi.file.clone()))
            })
            .collect();

        ensure!(
            !target.is_empty(),
            "\"{}\" not found in \"{}\".",
            fid.function_name(),
            fid.contract_name()
        );

        if target.len() > 1 {
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                target.len(),
                fid
            );
            for fi in &target {
                let sig = format!(
                    "{}::{}({})",
                    fi.contract_name,
                    fi.name,
                    format_params(&fi.parameters)
                );
                msg.push_str(&format!("\nhawk inspect calls \"{}\"", sig));
            }
            msg.push('\n');
            bail!(msg);
        }

        let target_id = target[0].id;

        let mut visited: HashSet<i64> = HashSet::new();
        self.build_call_node(target_id, &mut functions, &mut visited)
    }

    /// Parse a single artifact and insert its functions into the map.
    #[tracing::instrument(skip(self, path, functions))]
    fn load_artifact(
        &self,
        path: impl AsRef<Path>,
        functions: &mut HashMap<i64, FunctionInfo>,
    ) -> Result<()> {
        let path = path.as_ref();
        tracing::trace!(?path);
        // Check the cache first.
        {
            let cache = self.artifact_cache.borrow();
            if let Some(cached) = cache.get(path) {
                for fi in cached {
                    // checkrs: allow(clone_in_loops)
                    functions.insert(fi.id, fi.clone());
                }
                return Ok(());
            }
        }

        if let Some(funcs) = process_artifact_for_functions(path)? {
            self.artifact_cache
                .borrow_mut()
                .insert(path.to_path_buf(), funcs.clone());
            for fi in funcs {
                functions.insert(fi.id, fi);
            }
        }
        Ok(())
    }

    /// Ensure a function ID is loaded, using the [`FunctionIndex`] for
    /// O(1) lookup to find which artifact contains it.
    #[tracing::instrument(skip(self, functions))]
    fn ensure_function_loaded(
        &self,
        id: i64,
        functions: &mut HashMap<i64, FunctionInfo>,
    ) -> Result<()> {
        if functions.contains_key(&id) {
            return Ok(());
        }

        tracing::trace!(id, "cache miss, looking up artifact");

        // O(1) lookup: find the artifact containing this function ID.
        if let Some(entry) = self.function_index.get(id) {
            self.load_artifact(&entry.artifact_path, functions)?;
        }

        // ID not found in the index -- the call target is something we
        // cannot resolve (built-in, library, or interface function).
        Ok(())
    }

    /// Build a `CallGraphNode` for a function by ID, recursively.
    fn build_call_node(
        &self,
        func_id: i64,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<CallGraphNode> {
        if !visited.insert(func_id) {
            let info = &functions[&func_id];
            let sig = build_signature(info);
            let vis = visibility_str(&info.visibility);
            let src = format!(
                "{}:{}",
                info.definition.src.offset, info.definition.src.length
            );
            return Ok(CallGraphNode::new(
                &sig,
                &info.contract_name,
                info.file.clone(),
                &vis,
                &src,
                vec![],
            ));
        }

        // Clone body statements so we don't hold a borrow across the
        // recursive call (which may lazy-load new functions).
        let body_stmts = functions
            .get(&func_id)
            .and_then(|fi| fi.definition.body.as_ref().map(|b| b.statements.clone())); // checkrs: allow(clone_in_iterator)

        let children = if let Some(stmts) = body_stmts {
            self.collect_calls(&stmts, functions, visited)?
        } else {
            Vec::new()
        };

        let info = &functions[&func_id];
        let sig = build_signature(info);
        let vis = visibility_str(&info.visibility);
        let src = format!(
            "{}:{}",
            info.definition.src.offset, info.definition.src.length
        );

        Ok(CallGraphNode::new(
            &sig,
            &info.contract_name,
            info.file.clone(),
            &vis,
            &src,
            children,
        ))
    }

    /// Collect all function calls from a list of statements.
    fn collect_calls(
        &self,
        statements: &[solc::ast::Statement],
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<Vec<CallGraphNode>> {
        let mut nodes = Vec::new();
        for stmt in statements {
            self.collect_calls_from_statement(stmt, functions, visited, &mut nodes)?;
        }
        Ok(nodes)
    }

    /// Collect function calls from a single statement.
    fn collect_calls_from_statement(
        &self,
        stmt: &solc::ast::Statement,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match stmt {
            solc::ast::Statement::ExpressionStatement(es) => {
                self.collect_calls_from_expression(&es.expression, functions, visited, nodes)?;
            }
            solc::ast::Statement::Block(block) => {
                for s in &block.statements {
                    self.collect_calls_from_statement(s, functions, visited, nodes)?;
                }
            }
            solc::ast::Statement::IfStatement(ifs) => {
                self.collect_calls_from_expression(&ifs.condition, functions, visited, nodes)?;
                self.collect_calls_from_statement(&ifs.true_body, functions, visited, nodes)?;
                if let Some(ref false_body) = ifs.false_body {
                    self.collect_calls_from_statement(false_body, functions, visited, nodes)?;
                }
            }
            solc::ast::Statement::ForStatement(fors) => {
                if let Some(ref init) = fors.initialization_expression {
                    self.collect_calls_from_expression(init, functions, visited, nodes)?;
                }
                self.collect_calls_from_expression(&fors.condition, functions, visited, nodes)?;
                if let Some(ref loop_expr) = fors.loop_expression {
                    self.collect_calls_from_expression(loop_expr, functions, visited, nodes)?;
                }
                self.collect_calls_from_statement(&fors.body, functions, visited, nodes)?;
            }
            solc::ast::Statement::WhileStatement(whiles) => {
                self.collect_calls_from_expression(&whiles.condition, functions, visited, nodes)?;
                self.collect_calls_from_statement(&whiles.body, functions, visited, nodes)?;
            }
            solc::ast::Statement::DoWhileStatement(dw) => {
                self.collect_calls_from_statement(&dw.body, functions, visited, nodes)?;
                self.collect_calls_from_expression(&dw.condition, functions, visited, nodes)?;
            }
            solc::ast::Statement::Return(ret) => {
                if let Some(ref expr) = ret.expression {
                    self.collect_calls_from_expression(expr, functions, visited, nodes)?;
                }
            }
            solc::ast::Statement::VariableDeclarationStatement(vds) => {
                if let Some(ref expr) = vds.initial_value {
                    self.collect_calls_from_expression(expr, functions, visited, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Collect function calls from an expression, recursively.
    fn collect_calls_from_expression(
        &self,
        expr: &Expression,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match expr {
            Expression::FunctionCall(fc) => {
                let called_func_id = match &*fc.expression {
                    FunctionCallExpression::MemberAccess(ma) => ma.referenced_declaration,
                    FunctionCallExpression::Identifier(id) => id.referenced_declaration,
                    FunctionCallExpression::FunctionCallOptions(fco) => {
                        resolve_called_function_id_from_fc_expression(&fco.expression)
                    }
                    _ => None,
                };
                // checkrs: allow(nested_if_let)
                if let Some(id) = called_func_id {
                    self.ensure_function_loaded(id, functions)?;
                    if functions.contains_key(&id) {
                        let node = self.build_call_node(id, functions, visited)?;
                        nodes.push(node);
                    }
                } else if let FunctionCallExpression::MemberAccess(ma) = &*fc.expression
                    && is_low_level_call(&ma.member_name)
                {
                    // Low-level calls (.call(), .delegatecall(),
                    // .staticcall()) have no referenced_declaration
                    // because they are built-in methods on the
                    // `address` type, not user-defined functions.
                    // Synthesize a leaf node so they appear in the
                    // call graph.
                    let sig = format!("(address).{}()", ma.member_name);
                    nodes.push(CallGraphNode::new(
                        &sig,
                        "",
                        PathBuf::new(),
                        "low-level",
                        "",
                        vec![],
                    ));
                }
                for arg in &fc.arguments {
                    self.collect_calls_from_expression(arg, functions, visited, nodes)?;
                }
                if let FunctionCallExpression::FunctionCallOptions(fco) = &*fc.expression {
                    for opt in &fco.options {
                        self.collect_calls_from_expression(opt, functions, visited, nodes)?;
                    }
                }
            }
            Expression::Assignment(assign) => {
                self.collect_calls_from_expression(
                    &assign.right_hand_side,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &assign.left_hand_side,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::MemberAccess(ma) => {
                self.collect_calls_from_expression(&ma.expression, functions, visited, nodes)?;
            }
            Expression::BinaryOperation(binop) => {
                self.collect_calls_from_expression(
                    &binop.left_expression,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &binop.right_expression,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::UnaryOperation(unop) => {
                self.collect_calls_from_expression(
                    &unop.sub_expression,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::Conditional(cond) => {
                self.collect_calls_from_expression(&cond.condition, functions, visited, nodes)?;
                self.collect_calls_from_expression(
                    &cond.true_expression,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &cond.false_expression,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::TupleExpression(tuple) => {
                for expr in tuple.components.iter().flatten() {
                    self.collect_calls_from_expression(expr, functions, visited, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Build a human-readable function signature, e.g. `Main::execute(uint256)`.
fn build_signature(info: &FunctionInfo) -> String {
    format!(
        "{}::{}({})",
        info.contract_name,
        info.name,
        format_params(&info.parameters)
    )
}

/// Return a string representation of the visibility.
fn visibility_str(vis: &Visibility) -> String {
    match vis {
        Visibility::External => "external".into(),
        Visibility::Public => "public".into(),
        Visibility::Internal => "internal".into(),
        Visibility::Private => "private".into(),
    }
}

/// Format parameter declarations into a comma-separated type list, e.g. `uint256,address`.
fn format_params(params: &[VariableDeclaration]) -> String {
    params
        .iter()
        .map(|p| format_type_name(&p.type_name))
        .collect::<Vec<String>>()
        .join(",")
}

/// Format a type name to a human-readable string.
fn format_type_name(type_name: &TypeName) -> String {
    match type_name {
        TypeName::ElementaryTypeName(etn) => match etn.name {
            solc::ast::ElementaryType::Uint(bits) => {
                if bits == 256 {
                    "uint256".into()
                } else {
                    format!("uint{}", bits)
                }
            }
            solc::ast::ElementaryType::Int(bits) => {
                if bits == 256 {
                    "int256".into()
                } else {
                    format!("int{}", bits)
                }
            }
            solc::ast::ElementaryType::Address => "address".into(),
            solc::ast::ElementaryType::Payable => "address payable".into(),
            solc::ast::ElementaryType::Bool => "bool".into(),
            solc::ast::ElementaryType::String => "string".into(),
            solc::ast::ElementaryType::Bytes => "bytes".into(),
            solc::ast::ElementaryType::FixedBytes(n) => format!("bytes{}", n),
            solc::ast::ElementaryType::Ufixed(m, n) => format!("ufixed{}x{}", m, n),
            solc::ast::ElementaryType::Fixed(m, n) => format!("fixed{}x{}", m, n),
        },
        TypeName::ArrayTypeName(arr) => {
            format!("{}[]", format_type_name(&arr.base_type))
        }
        TypeName::UserDefinedTypeName(udtn) => {
            if let Some(ref path) = udtn.path_node {
                path.name.clone()
            } else {
                "unknown".into()
            }
        }
        TypeName::Mapping(_) => "mapping".into(),
        TypeName::FunctionTypeName(_) => "function".into(),
    }
}

/// Extract the referenced declaration ID from a function call expression.
fn resolve_called_function_id_from_fc_expression(expr: &Expression) -> Option<i64> {
    match expr {
        Expression::MemberAccess(ma) => ma.referenced_declaration,
        Expression::Identifier(id) => id.referenced_declaration,
        _ => None,
    }
}

/// Check whether a member name refers to a low-level `address` call method.
fn is_low_level_call(member_name: &str) -> bool {
    matches!(
        member_name,
        "call" | "delegatecall" | "staticcall" | "callcode"
    )
}

/// Process a single artifact JSON file, returning all [`FunctionInfo`] entries
/// found across all contracts in the AST.
#[tracing::instrument(skip_all)]
fn process_artifact_for_functions(path: impl AsRef<Path>) -> Result<Option<Vec<FunctionInfo>>> {
    let path = path.as_ref();
    tracing::trace!(?path);
    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)?;

    let ast = match artifact.ast {
        None => return Ok(None),
        Some(ast) => ast,
    };

    let source_file = ast.absolute_path;
    let mut functions = Vec::new();

    for node in ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node {
            functions.extend(extract_contract_functions(cd, &source_file));
        }
    }

    if functions.is_empty() {
        Ok(None)
    } else {
        Ok(Some(functions))
    }
}

/// Extract all implemented functions from a contract definition.
fn extract_contract_functions(
    cd: solc::ast::ContractDefinition,
    source_file: &Path,
) -> Vec<FunctionInfo> {
    let contract_name = cd.name;
    let file = source_file.to_path_buf();
    cd.nodes
        .into_iter()
        .filter_map(|inner| {
            let ContractDefinitionNode::FunctionDefinition(fd) = inner else {
                return None;
            };
            // Include all function definitions, including interface
            // functions (implemented = false). Interface functions have
            // no body, so they become leaf nodes in the call graph,
            // which is correct for external calls through interface
            // casts (e.g. IHelper(address(h)).doWork()).
            Some(FunctionInfo {
                id: fd.id,
                name: fd.name.clone(),
                contract_name: contract_name.clone(),
                file: file.clone(),
                parameters: fd.parameters.parameters.clone(),
                visibility: fd.visibility.clone(),
                definition: fd,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::artifact_index::ArtifactIndex;

    fn fixture_project() -> Project {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/calls");
        Project::open(root)
    }

    fn fixture_ambiguous_project() -> Project {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritances-ambiguous");
        Project::open(root)
    }

    #[test]
    fn index_builds_for_calls_fixture() {
        let project = fixture_project();
        let index = ArtifactIndex::build(project.out_dir());
        assert!(index.contains_key("Main"));
        assert!(index.contains_key("Helper"));
        assert!(index.contains_key("Base"));
    }

    #[test]
    fn call_graph_for_readonly() {
        let resolver = CallGraphResolver::new(fixture_project());
        let node = resolver.resolve("Main::readOnly").unwrap();
        let output = node.to_string();
        assert!(output.contains("Main::readOnly()"));
    }

    #[test]
    fn call_graph_for_execute() {
        let resolver = CallGraphResolver::new(fixture_project());
        let node = resolver.resolve("Main::execute").unwrap();
        let output = node.to_string();
        assert!(output.contains("Main::execute(uint256)"));
    }

    #[test]
    fn call_graph_errors_for_unknown_contract() {
        let resolver = CallGraphResolver::new(fixture_project());
        let err = resolver
            .resolve("Unknown::function")
            .unwrap_err()
            .to_string();
        assert_eq!(err, "\"Unknown\" not found.");
    }

    #[test]
    fn call_graph_errors_for_unknown_function() {
        let resolver = CallGraphResolver::new(fixture_project());
        let err = resolver
            .resolve("Main::unknownFunction")
            .unwrap_err()
            .to_string();
        assert_eq!(err, "\"unknownFunction\" not found in \"Main\".");
    }

    #[test]
    fn ambiguity_shows_suggestions() {
        let resolver = CallGraphResolver::new(fixture_ambiguous_project());
        let err = resolver
            .resolve("Dupe::someFunction")
            .unwrap_err()
            .to_string();
        assert!(err.contains("found 2 \"Dupe\""));
        assert!(err.contains("hawk inspect calls"));
    }

    // Regression test: calls through interface casts (e.g. IHelper(address(h)).doWork())
    // must appear in the resolved call graph. Previously, interface functions were not
    // indexed because they have `implemented = false`, causing the resolver to silently
    // drop them.
    #[test]
    fn call_graph_for_interface_call() {
        let resolver = CallGraphResolver::new(fixture_project());
        let node = resolver.resolve("Main::callViaInterface").unwrap();
        let output = node.to_string();
        let expected = concat!(
            "Main::callViaInterface() (public)\n",
            "\u{2514}\u{2500}\u{2500} IHelper::doWork() (external)\n",
        );
        assert_eq!(output, expected);
    }

    // Regression test: low-level calls (.call(), .delegatecall(),
    // .staticcall()) must appear in the resolved call graph. Previously,
    // they were silently dropped because their `referenced_declaration` is
    // `None` (they are built-in methods on the `address` type, not
    // user-defined functions).
    #[test]
    fn call_graph_includes_low_level_call() {
        let resolver = CallGraphResolver::new(fixture_project());
        let node = resolver.resolve("LowLevelCaller::callWithPayload").unwrap();
        let output = node.to_string();
        let expected = concat!(
            "LowLevelCaller::callWithPayload(address,bytes) (external)\n",
            "\u{2514}\u{2500}\u{2500} (address).call() (low-level)\n",
        );
        assert_eq!(output, expected);
    }
}
