//! Call graph resolution for Solidity functions using a pre-built
//! [`FunctionIndex`] for O(1) AST-node-ID lookups.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Result, bail, ensure};
use solc::ast::{Expression, FunctionCallExpression, TypeName, VariableDeclaration, Visibility};

use crate::artifact_index::ArtifactIndex;
use crate::call_graph::{FunctionID, node::CallGraphNode};
use crate::function_index::{FunctionIndex, FunctionInfo};
use crate::project::Project;

/// Resolves call graphs for Solidity functions in a Foundry project.
///
/// All functions from all artifacts are pre-loaded into a [`FunctionIndex`]
/// at construction time, so lookups during traversal are O(1).
pub struct CallGraphResolver {
    project: Project,
    /// Lightweight index: contract name → one or more artifact paths
    /// (kept for ambiguity detection).
    artifact_index: ArtifactIndex,
    /// Pre-built index: Solc AST node ID → FunctionInfo.
    function_index: FunctionIndex,
}

impl CallGraphResolver {
    /// Build a [`CallGraphResolver`] that owns a [`Project`].
    ///
    /// Pre-builds a [`FunctionIndex`] from all artifacts on disk so that
    /// call-graph traversal never needs to parse JSON files lazily.
    pub fn new(project: Project) -> Self {
        let artifact_index = ArtifactIndex::build(project.out_dir());
        let function_index = FunctionIndex::build(&artifact_index);
        Self {
            project,
            artifact_index,
            function_index,
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
        let fid = FunctionID::try_from(function_id)?;

        // Detect contract-level ambiguity from the artifact index before
        // looking at functions, so we can show helpful suggestions even
        // when the contract has no implemented functions.
        if let Ok(entries) = self.artifact_index.try_get(fid.contract_name())
            && entries.len() > 1
        {
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                entries.len(),
                fid.contract_name()
            );
            for entry in &entries {
                let rp = entry
                    .path
                    .strip_prefix(self.project.path())
                    .unwrap_or(&entry.path)
                    .to_string_lossy();
                msg.push_str(&format!("\nhawk inspect calls {}:{}", rp, fid));
            }
            msg.push('\n');
            bail!(msg);
        }

        // Find the target function in the pre-built index.
        let mut seen = HashSet::new();
        let target: Vec<&FunctionInfo> = self
            .function_index
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
        self.build_call_node(target_id, &mut visited)
    }

    /// Build a `CallGraphNode` for a function by ID, recursively.
    fn build_call_node(&self, func_id: i64, visited: &mut HashSet<i64>) -> Result<CallGraphNode> {
        if !visited.insert(func_id) {
            let info = &self.function_index[&func_id];
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

        // Clone body statements so we don't hold a borrow.
        let body_stmts = self
            .function_index
            .get(func_id)
            .and_then(|fi| fi.definition.body.as_ref().map(|b| b.statements.clone())); // checkrs: allow(clone_in_iterator)

        let children = if let Some(stmts) = body_stmts {
            self.collect_calls(&stmts, visited)?
        } else {
            Vec::new()
        };

        let info = &self.function_index[&func_id];
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
        visited: &mut HashSet<i64>,
    ) -> Result<Vec<CallGraphNode>> {
        let mut nodes = Vec::new();
        for stmt in statements {
            self.collect_calls_from_statement(stmt, visited, &mut nodes)?;
        }
        Ok(nodes)
    }

    /// Collect function calls from a single statement.
    fn collect_calls_from_statement(
        &self,
        stmt: &solc::ast::Statement,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match stmt {
            solc::ast::Statement::ExpressionStatement(es) => {
                self.collect_calls_from_expression(&es.expression, visited, nodes)?;
            }
            solc::ast::Statement::Block(block) => {
                for s in &block.statements {
                    self.collect_calls_from_statement(s, visited, nodes)?;
                }
            }
            solc::ast::Statement::IfStatement(ifs) => {
                self.collect_calls_from_expression(&ifs.condition, visited, nodes)?;
                self.collect_calls_from_statement(&ifs.true_body, visited, nodes)?;
                if let Some(ref false_body) = ifs.false_body {
                    self.collect_calls_from_statement(false_body, visited, nodes)?;
                }
            }
            solc::ast::Statement::ForStatement(fors) => {
                if let Some(ref init) = fors.initialization_expression {
                    self.collect_calls_from_expression(init, visited, nodes)?;
                }
                self.collect_calls_from_expression(&fors.condition, visited, nodes)?;
                if let Some(ref loop_expr) = fors.loop_expression {
                    self.collect_calls_from_expression(loop_expr, visited, nodes)?;
                }
                self.collect_calls_from_statement(&fors.body, visited, nodes)?;
            }
            solc::ast::Statement::WhileStatement(whiles) => {
                self.collect_calls_from_expression(&whiles.condition, visited, nodes)?;
                self.collect_calls_from_statement(&whiles.body, visited, nodes)?;
            }
            solc::ast::Statement::DoWhileStatement(dw) => {
                self.collect_calls_from_statement(&dw.body, visited, nodes)?;
                self.collect_calls_from_expression(&dw.condition, visited, nodes)?;
            }
            solc::ast::Statement::Return(ret) => {
                if let Some(ref expr) = ret.expression {
                    self.collect_calls_from_expression(expr, visited, nodes)?;
                }
            }
            solc::ast::Statement::VariableDeclarationStatement(vds) => {
                if let Some(ref expr) = vds.initial_value {
                    self.collect_calls_from_expression(expr, visited, nodes)?;
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
                if let Some(id) = called_func_id
                    && self.function_index.contains(id)
                {
                    let node = self.build_call_node(id, visited)?;
                    nodes.push(node);
                }
                for arg in &fc.arguments {
                    self.collect_calls_from_expression(arg, visited, nodes)?;
                }
                if let FunctionCallExpression::FunctionCallOptions(fco) = &*fc.expression {
                    for opt in &fco.options {
                        self.collect_calls_from_expression(opt, visited, nodes)?;
                    }
                }
            }
            Expression::Assignment(assign) => {
                self.collect_calls_from_expression(&assign.right_hand_side, visited, nodes)?;
                self.collect_calls_from_expression(&assign.left_hand_side, visited, nodes)?;
            }
            Expression::MemberAccess(ma) => {
                self.collect_calls_from_expression(&ma.expression, visited, nodes)?;
            }
            Expression::BinaryOperation(binop) => {
                self.collect_calls_from_expression(&binop.left_expression, visited, nodes)?;
                self.collect_calls_from_expression(&binop.right_expression, visited, nodes)?;
            }
            Expression::UnaryOperation(unop) => {
                self.collect_calls_from_expression(&unop.sub_expression, visited, nodes)?;
            }
            Expression::Conditional(cond) => {
                self.collect_calls_from_expression(&cond.condition, visited, nodes)?;
                self.collect_calls_from_expression(&cond.true_expression, visited, nodes)?;
                self.collect_calls_from_expression(&cond.false_expression, visited, nodes)?;
            }
            Expression::TupleExpression(tuple) => {
                for expr in tuple.components.iter().flatten() {
                    self.collect_calls_from_expression(expr, visited, nodes)?;
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
        assert!(err.contains("\"function\" not found in \"Unknown\""));
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
