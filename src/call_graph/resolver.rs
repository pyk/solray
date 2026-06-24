//! Call graph resolution for Solidity functions.
//!
//! [`CallGraphResolver`] resolves call graphs from a pre-built contract index.
//! Artifact JSON files are parsed lazily only when a specific function's call
//! graph is requested, keeping the initial lookup fast.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail, ensure};
use rayon::prelude::*;
use serde::Deserialize;
use solc::ast::{
    ContractDefinitionNode, Expression, FunctionCallExpression, FunctionDefinition, SourceUnitNode,
    TypeName, VariableDeclaration, Visibility,
};

use crate::artifact_index::{ArtifactEntry, ArtifactIndex};
use crate::call_graph::node::CallGraphNode;
use crate::project::Project;

/// Internal: function info extracted from an artifact AST for call graph resolution.
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
/// Maintains a lightweight index of contract names to artifact paths and
/// caches parsed function definitions.
pub struct CallGraphResolver {
    project: Project,
    /// Lightweight index: contract name → one or more artifact paths.
    artifact_index: ArtifactIndex,
    /// Lazily populated: global function info map keyed by AST node ID.
    function_infos: std::sync::OnceLock<Vec<FunctionInfo>>,
}

impl CallGraphResolver {
    /// Build a [`CallGraphResolver`] that owns a [`Project`].
    pub fn new(project: Project) -> Self {
        let artifact_index = ArtifactIndex::build(project.out_dir());
        Self {
            project,
            artifact_index,
            function_infos: std::sync::OnceLock::new(),
        }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Resolve a `Contract::function` ID and return its call graph.
    ///
    /// Handles:
    /// - Contract not found in artifacts
    /// - Multiple contracts sharing the same name (ambiguity)
    /// - Overloaded functions within the same contract
    pub fn resolve(&self, function_id: &str) -> Result<CallGraphNode> {
        let (contract_name, function_name) = parse_function_id(function_id)?;

        let entries = self.artifact_index.get(contract_name);

        let entries = match entries {
            Some(e) if e.is_empty() => bail!("\"{}\" not found.", contract_name),
            Some(e) => e,
            None => bail!("\"{}\" not found.", contract_name),
        };

        // Handle ambiguity: multiple contracts with the same name.
        if entries.len() > 1 {
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                entries.len(),
                contract_name
            );
            for entry in entries {
                let rp = entry
                    .path
                    .strip_prefix(self.project.path())
                    .unwrap_or(&entry.path)
                    .to_string_lossy();
                msg.push_str(&format!("\nhawk inspect calls {}:{}", rp, function_id));
            }
            msg.push('\n');
            bail!(msg);
        }

        let _entry = &entries[0];

        // Ensure function infos are loaded (lazy, cached).
        let func_infos = self.ensure_function_infos();
        let by_id: HashMap<i64, &FunctionInfo> = func_infos.iter().map(|fi| (fi.id, fi)).collect();

        // Find matching functions in the target contract.
        // Deduplicate: a function may appear in multiple artifacts when
        // those artifacts share the same source unit AST.
        let mut seen = HashSet::new();
        let matched: Vec<&FunctionInfo> = func_infos
            .iter()
            .filter(|fi| {
                fi.contract_name == contract_name
                    && fi.name == function_name
                    && seen.insert((fi.name.clone(), fi.file.clone()))
            })
            .collect();

        // Use the artifact entry's path to further filter if needed.
        // In practice, a single artifact covers a source unit, so the entry
        // filter is enough for ambiguity. For overloaded functions we need
        // parameter disambiguation.
        ensure!(
            !matched.is_empty(),
            "\"{}\" not found in \"{}\".",
            function_name,
            contract_name
        );

        // Handle overloaded functions.
        if matched.len() > 1 {
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                matched.len(),
                function_id
            );
            for fi in &matched {
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

        let target = &matched[0];

        let mut visited: HashSet<i64> = HashSet::new();
        self.build_call_node(target, &by_id, &mut visited)
    }

    /// Build a `CallGraphNode` for a given function info, recursively.
    fn build_call_node(
        &self,
        info: &FunctionInfo,
        by_id: &HashMap<i64, &FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<CallGraphNode> {
        if !visited.insert(info.id) {
            // Recursive call detected; return a stub.
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

        let children = if let Some(ref body) = info.definition.body {
            self.collect_calls(&body.statements, by_id, visited)?
        } else {
            Vec::new()
        };

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
        by_id: &HashMap<i64, &FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<Vec<CallGraphNode>> {
        let mut nodes = Vec::new();
        for stmt in statements {
            self.collect_calls_from_statement(stmt, by_id, visited, &mut nodes)?;
        }
        Ok(nodes)
    }

    /// Collect function calls from a single statement.
    fn collect_calls_from_statement(
        &self,
        stmt: &solc::ast::Statement,
        by_id: &HashMap<i64, &FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match stmt {
            solc::ast::Statement::ExpressionStatement(es) => {
                self.collect_calls_from_expression(&es.expression, by_id, visited, nodes)?;
            }
            solc::ast::Statement::Block(block) => {
                for s in &block.statements {
                    self.collect_calls_from_statement(s, by_id, visited, nodes)?;
                }
            }
            solc::ast::Statement::IfStatement(ifs) => {
                self.collect_calls_from_expression(&ifs.condition, by_id, visited, nodes)?;
                self.collect_calls_from_statement(&ifs.true_body, by_id, visited, nodes)?;
                if let Some(ref false_body) = ifs.false_body {
                    self.collect_calls_from_statement(false_body, by_id, visited, nodes)?;
                }
            }
            solc::ast::Statement::ForStatement(fors) => {
                if let Some(ref init) = fors.initialization_expression {
                    self.collect_calls_from_expression(init, by_id, visited, nodes)?;
                }
                self.collect_calls_from_expression(&fors.condition, by_id, visited, nodes)?;
                if let Some(ref loop_expr) = fors.loop_expression {
                    self.collect_calls_from_expression(loop_expr, by_id, visited, nodes)?;
                }
                self.collect_calls_from_statement(&fors.body, by_id, visited, nodes)?;
            }
            solc::ast::Statement::WhileStatement(whiles) => {
                self.collect_calls_from_expression(&whiles.condition, by_id, visited, nodes)?;
                self.collect_calls_from_statement(&whiles.body, by_id, visited, nodes)?;
            }
            solc::ast::Statement::DoWhileStatement(dw) => {
                self.collect_calls_from_statement(&dw.body, by_id, visited, nodes)?;
                self.collect_calls_from_expression(&dw.condition, by_id, visited, nodes)?;
            }
            solc::ast::Statement::Return(ret) => {
                if let Some(ref expr) = ret.expression {
                    self.collect_calls_from_expression(expr, by_id, visited, nodes)?;
                }
            }
            solc::ast::Statement::VariableDeclarationStatement(vds) => {
                if let Some(ref expr) = vds.initial_value {
                    self.collect_calls_from_expression(expr, by_id, visited, nodes)?;
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
        by_id: &HashMap<i64, &FunctionInfo>,
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
                if let Some(fi) = called_func_id.and_then(|id| by_id.get(&id)) {
                    let node = self.build_call_node(fi, by_id, visited)?;
                    nodes.push(node);
                }
                for arg in &fc.arguments {
                    self.collect_calls_from_expression(arg, by_id, visited, nodes)?;
                }
                if let FunctionCallExpression::FunctionCallOptions(fco) = &*fc.expression {
                    for opt in &fco.options {
                        self.collect_calls_from_expression(opt, by_id, visited, nodes)?;
                    }
                }
            }
            Expression::Assignment(assign) => {
                self.collect_calls_from_expression(&assign.right_hand_side, by_id, visited, nodes)?;
                self.collect_calls_from_expression(&assign.left_hand_side, by_id, visited, nodes)?;
            }
            Expression::MemberAccess(ma) => {
                self.collect_calls_from_expression(&ma.expression, by_id, visited, nodes)?;
            }
            Expression::BinaryOperation(binop) => {
                self.collect_calls_from_expression(&binop.left_expression, by_id, visited, nodes)?;
                self.collect_calls_from_expression(&binop.right_expression, by_id, visited, nodes)?;
            }
            Expression::UnaryOperation(unop) => {
                self.collect_calls_from_expression(&unop.sub_expression, by_id, visited, nodes)?;
            }
            Expression::Conditional(cond) => {
                self.collect_calls_from_expression(&cond.condition, by_id, visited, nodes)?;
                self.collect_calls_from_expression(&cond.true_expression, by_id, visited, nodes)?;
                self.collect_calls_from_expression(&cond.false_expression, by_id, visited, nodes)?;
            }
            Expression::TupleExpression(tuple) => {
                for expr in tuple.components.iter().flatten() {
                    self.collect_calls_from_expression(expr, by_id, visited, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Load all function definitions from all artifacts into a vector.
    /// Results are cached in `function_infos`. Parse errors for individual
    /// artifacts are silently skipped.
    fn ensure_function_infos(&self) -> &Vec<FunctionInfo> {
        self.function_infos.get_or_init(|| {
            let entries: Vec<&ArtifactEntry> = self.artifact_index.values().flatten().collect();
            entries
                .par_iter()
                .filter_map(|e| process_artifact_for_functions(&e.path).ok().flatten())
                .flatten()
                .collect()
        })
    }
}

/// Parse a function ID like `Contract::function` into `(contract_name, function_name)`.
fn parse_function_id(function_id: &str) -> Result<(&str, &str)> {
    match function_id.split_once("::") {
        Some((contract, function)) if !contract.is_empty() && !function.is_empty() => {
            Ok((contract, function))
        }
        _ => bail!(
            "invalid function ID \"{}\". Expected format: Contract::function",
            function_id
        ),
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

/// Process a single artifact JSON file, returning all `FunctionInfo` entries
/// found across all contracts in the AST.
fn process_artifact_for_functions(path: impl AsRef<Path>) -> Result<Option<Vec<FunctionInfo>>> {
    let path = path.as_ref();

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
            if !fd.implemented {
                return None;
            }
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
        assert!(err.contains("\"Unknown\" not found"));
    }

    #[test]
    fn call_graph_errors_for_unknown_function() {
        let resolver = CallGraphResolver::new(fixture_project());
        let err = resolver
            .resolve("Main::unknownFunction")
            .unwrap_err()
            .to_string();
        assert!(err.contains("\"unknownFunction\" not found in \"Main\""));
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
}
