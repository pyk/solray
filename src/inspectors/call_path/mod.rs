//! Call path inspection for Foundry projects.
//!
//! [`CallPathInspector`] finds all external/public functions that can reach
//! a given target function, showing only the linear path to the target.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use solc::ast::{
    ContractDefinition, Expression, FunctionCallExpression, SourceUnitNode, Visibility,
};

use crate::artifact_index::ArtifactIndex;
use crate::build_info::BuildInfo;
use crate::inspectors::artifact_id::ArtifactId;
use crate::inspectors::call_graph;
use crate::inspectors::call_graph::FunctionId;
use crate::inspectors::call_graph::node::CallGraphNode;
use crate::inspectors::call_graph::source_renderer::offset_to_line_range;
use crate::inspectors::function_source::symbol_index::SymbolIndex;
use crate::project::Project;

/// The output of a [`CallPathInspector`] inspection.
///
/// Shows compact call paths from external functions to a target function.
#[derive(Debug)]
pub struct CallPathInspectorOutput {
    roots: Vec<CallGraphNode>,
    project_root: PathBuf,
    target_function: String,
    target_file: PathBuf,
    target_line: String,
}

impl CallPathInspectorOutput {
    /// Create a new [`CallPathInspectorOutput`].
    pub fn new(
        roots: Vec<CallGraphNode>,
        project_root: PathBuf,
        target_function: &str,
        target_file: PathBuf,
        target_line: &str,
    ) -> Self {
        Self {
            roots,
            project_root,
            target_function: target_function.to_string(),
            target_file,
            target_line: target_line.to_string(),
        }
    }
}

impl std::fmt::Display for CallPathInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "## Call Paths\n")?;

        // Format the target function name without visibility or params.
        let target_display = format_target_display(&self.target_function);

        writeln!(f, "Target: `{}`", target_display)?;
        if self.target_file.as_os_str().is_empty() {
            writeln!(f, "Source: Not found")?;
        } else {
            writeln!(
                f,
                "Source: `{}:{}`",
                self.target_file.display(),
                self.target_line
            )?;
        }

        if self.roots.is_empty() {
            writeln!(
                f,
                "\n0 paths found.\n\nNo external functions reach \"{}\".",
                target_display
            )?;
            return Ok(());
        }

        writeln!(f, "\n{} paths found.\n", self.roots.len())?;

        let project_abs =
            std::path::absolute(&self.project_root).unwrap_or_else(|_| self.project_root.clone());

        let mut line_maps: HashMap<PathBuf, Vec<usize>> = HashMap::new();

        for (i, root) in self.roots.iter().enumerate() {
            writeln!(f, "### Path {}\n", i + 1)?;

            // Extract the linear path from root to target.
            let path = extract_path_to_target(root, &self.target_function);

            writeln!(f, "```")?;
            for (j, node) in path.iter().enumerate() {
                let node_name = format_call_path_node(node);
                let full_path = project_abs.join(&node.file);
                let rel_path = full_path.strip_prefix(&project_abs).unwrap_or(&full_path);
                let line = if !node.src.is_empty() {
                    let range = offset_to_line_range(&full_path, &node.src, &mut line_maps);
                    range
                        .strip_prefix('L')
                        .and_then(|r| r.split('-').next())
                        .unwrap_or(&range)
                        .to_string()
                } else {
                    String::new()
                };
                let file_display = if line.is_empty() {
                    rel_path.display().to_string()
                } else {
                    format!("{}:{}", rel_path.display(), line)
                };

                if j == 0 {
                    // First node - no connector
                    writeln!(f, "{}", node_name)?;
                    writeln!(f, "{}", file_display)?;
                } else {
                    let connector_indent = " ".repeat((j - 1) * 4);
                    let file_indent = " ".repeat(j * 4);
                    writeln!(
                        f,
                        "{}\u{2514}\u{2500}\u{2500} {}",
                        connector_indent, node_name
                    )?;
                    writeln!(f, "{}{}", file_indent, file_display)?;
                }
            }
            writeln!(f, "```")?;
            writeln!(f)?;
        }

        Ok(())
    }
}

/// Format a target function string for display (strip visibility, params).
fn format_target_display(target: &str) -> String {
    // If target contains "::", use it as-is but strip params if present.
    if target.contains("::") {
        // Try to strip params from the function part
        if let Some((contract, rest)) = target.split_once("::") {
            let func = rest.split('(').next().unwrap_or(rest);
            return format!("{}::{}", contract, func);
        }
    }
    target.to_string()
}

/// Format a [`CallGraphNode`] for call-path display (no visibility, no params).
fn format_call_path_node(node: &CallGraphNode) -> String {
    let func_name = call_graph::extract_func_name_from_sig(&node.signature);
    format!("{}::{}", node.contract_name, func_name)
}

/// Extract the linear path from root to the target node.
///
/// Returns a list of nodes from root to target (inclusive), following only
/// the branch that leads to the target.
fn extract_path_to_target<'a>(root: &'a CallGraphNode, target: &str) -> Vec<&'a CallGraphNode> {
    if call_graph::function_node_matches_target(root, target) {
        return vec![root];
    }
    for child in &root.children {
        if call_graph::call_graph_reaches_target(child, target) {
            let mut path = extract_path_to_target(child, target);
            path.insert(0, root);
            return path;
        }
    }
    vec![root]
}

/// Get the file and line for a target function by searching the roots.
fn find_target_source<'a>(
    roots: &'a [CallGraphNode],
    target: &str,
) -> Option<(&'a PathBuf, &'a str)> {
    for root in roots {
        if let Some(node) = find_matching_node(root, target)
            && !node.file.as_os_str().is_empty()
        {
            return Some((&node.file, &node.src));
        }
    }
    None
}

/// Find the first node matching the target function in a tree.
fn find_matching_node<'a>(node: &'a CallGraphNode, target: &str) -> Option<&'a CallGraphNode> {
    if call_graph::function_node_matches_target(node, target) {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_matching_node(child, target) {
            return Some(found);
        }
    }
    None
}

/// Convert an `offset:length` src string to a starting line number.
fn src_to_start_line(
    src: &str,
    project_abs: &Path,
    file: &Path,
    line_maps: &mut HashMap<PathBuf, Vec<usize>>,
) -> String {
    let full_path = project_abs.join(file);
    let range = offset_to_line_range(&full_path, src, line_maps);
    // Range is like "L36" or "L36-L41" - extract just the starting line number.
    range
        .strip_prefix('L')
        .and_then(|r| r.split('-').next())
        .unwrap_or(&range)
        .to_string()
}

/// Inspect a Foundry project for call paths to a target function.
pub struct CallPathInspector {
    project: Project,
    artifact_index: ArtifactIndex,
    symbol_index: SymbolIndex,
}

impl CallPathInspector {
    /// Build a [`CallPathInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        let artifact_index = ArtifactIndex::build(project.out_dir());
        let build_infos = BuildInfo::load_all(project.out_dir());
        let symbol_index = SymbolIndex::build(&artifact_index, &build_infos);
        Self {
            project,
            artifact_index,
            symbol_index,
        }
    }

    /// Find all external/public functions that can reach the target function.
    ///
    /// The `target_function` may be a simple name (e.g., `"internalWork"`)
    /// or a library-specific path (e.g., `"Lib::libWork"`).
    pub fn inspect(
        &self,
        id: &FunctionId,
        target_function: &str,
    ) -> Result<CallPathInspectorOutput> {
        let resolved = self.resolve_artifact_path(id.artifact_id())?;

        let artifact_path = match resolved {
            call_graph::ResolvedPath::Single(path) => path,
            call_graph::ResolvedPath::Ambiguous(candidates) => candidates[0].clone(), // checkrs: allow(clone_in_loops)
            call_graph::ResolvedPath::NotFound => {
                bail!("\"{}\" not found.", id.artifact_id().name);
            }
        };

        let cache: RefCell<HashMap<PathBuf, Vec<call_graph::FunctionInfo>>> =
            RefCell::new(HashMap::new());
        let mut functions: HashMap<i64, call_graph::FunctionInfo> = HashMap::new();
        call_graph::load_artifact_functions(&artifact_path, &mut functions, &cache)?;

        let project_root = self.project.path().to_path_buf();
        let target_name = id.artifact_id().name.as_str();

        // Build the inheritance chain: collect all contract names reachable
        // from the target contract.
        let contract_names = self.build_inheritance_chain(target_name, &artifact_path)?;

        // Collect all external/public function IDs from the inheritance chain.
        let external_ids: Vec<i64> = functions
            .iter()
            .filter(|(_, fi)| {
                contract_names.contains(&fi.contract_name)
                    && matches!(fi.visibility, Visibility::External | Visibility::Public)
            })
            .map(|(id, _)| *id) // checkrs: allow(clone_in_iterator)
            .collect();

        // Build call graphs for each external function and check reachability.
        let mut matching_roots = Vec::new();
        for &func_id in &external_ids {
            let mut visited: HashSet<i64> = HashSet::new();
            let root = self.build_call_node(func_id, &cache, &mut functions, &mut visited)?;
            if call_graph::call_graph_reaches_target(&root, target_function) {
                matching_roots.push(root);
            }
        }

        // Sort roots by their signature for deterministic output.
        matching_roots.sort_by(|a, b| a.signature.cmp(&b.signature));

        // Find the target function's source location.
        let project_abs =
            std::path::absolute(&project_root).unwrap_or_else(|_| project_root.clone());

        let (target_file, target_src) = find_target_source(&matching_roots, target_function)
            .map(|(file, src)| (file.clone(), src.to_string())) // checkrs: allow(clone_in_iterator)
            .unwrap_or_else(|| {
                // Fallback: try to get target info from function map.
                let func_name = if target_function.contains("::") {
                    target_function
                        .split("::")
                        .nth(1)
                        .unwrap_or(target_function)
                } else {
                    target_function
                };
                let found: Option<(PathBuf, String)> = functions
                    .values()
                    .find(|fi| fi.name == func_name)
                    // checkrs: allow(clone_in_iterator)
                    .map(|fi| {
                        (
                            fi.file.clone(),
                            format!("{}:{}", fi.definition.src.offset, fi.definition.src.length),
                        )
                    });
                found.unwrap_or_default()
            });

        let mut line_maps: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        let target_line =
            src_to_start_line(&target_src, &project_abs, &target_file, &mut line_maps);

        Ok(CallPathInspectorOutput::new(
            matching_roots,
            project_root,
            target_function,
            target_file,
            &target_line,
        ))
    }

    /// Walk the inheritance chain of the given contract, returning the set
    /// of all contract names in the chain.
    fn build_inheritance_chain(
        &self,
        contract_name: &str,
        artifact_path: impl AsRef<Path>,
    ) -> Result<HashSet<String>> {
        let content = fs::read_to_string(artifact_path.as_ref())?;
        let artifact: call_graph::Artifact = serde_json::from_str(&content)?;

        let ast = match artifact.ast {
            None => return Ok(HashSet::from([contract_name.to_string()])),
            Some(ast) => ast,
        };

        // Find all contract definitions in this artifact.
        let contracts: Vec<ContractDefinition> = ast
            .nodes
            .into_iter()
            .filter_map(|node| match node {
                SourceUnitNode::ContractDefinition(cd) => Some(cd),
                _ => None,
            })
            .collect();

        // Build a name -> contract map for easy lookup.
        let contract_map: HashMap<String, ContractDefinition> = contracts
            .into_iter()
            .map(|cd| (cd.name.clone(), cd)) // checkrs: allow(clone_in_iterator)
            .collect();

        let mut chain = HashSet::new();
        let mut to_visit = vec![contract_name.to_string()];

        while let Some(name) = to_visit.pop() {
            // checkrs: allow(clone_in_loops)
            if !chain.insert(name.clone()) {
                continue;
            }
            if let Some(cd) = contract_map.get(&name) {
                for base in &cd.base_contracts {
                    let base_name = &base.base_name.name;
                    to_visit.push(base_name.clone()); // checkrs: allow(clone_in_loops)
                }
            }
        }

        Ok(chain)
    }

    /// Resolve the artifact file path for the given [`ArtifactId`].
    fn resolve_artifact_path(&self, id: &ArtifactId) -> Result<call_graph::ResolvedPath> {
        match &id.file {
            Some(file) => {
                let path = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                if path.exists() {
                    Ok(call_graph::ResolvedPath::Single(path))
                } else {
                    Ok(call_graph::ResolvedPath::NotFound)
                }
            }
            None => {
                let candidates = self
                    .artifact_index
                    .get(&id.name)
                    .cloned()
                    .unwrap_or_default();

                match candidates.len() {
                    0 => Ok(call_graph::ResolvedPath::NotFound),
                    1 => Ok(call_graph::ResolvedPath::Single(candidates[0].clone())),
                    _ => Ok(call_graph::ResolvedPath::Ambiguous(candidates)),
                }
            }
        }
    }

    /// Build a `CallGraphNode` for a function by ID, recursively.
    fn build_call_node(
        &self,
        func_id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<call_graph::FunctionInfo>>>,
        functions: &mut HashMap<i64, call_graph::FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<CallGraphNode> {
        if !visited.insert(func_id) {
            let info = &functions[&func_id];
            let sig = call_graph::build_signature(info);
            let vis = call_graph::visibility_str(&info.visibility);
            let src = format!(
                "{}:{}",
                info.definition.src.offset, info.definition.src.length
            );
            return Ok(CallGraphNode::new(
                &sig,
                &info.contract_name,
                info.file.clone(), // checkrs: allow(clone_in_loops)
                &vis,
                &src,
                vec![],
            ));
        }

        let body_stmts = functions
            .get(&func_id)
            .and_then(|fi| fi.definition.body.as_ref().map(|b| b.statements.clone())); // checkrs: allow(clone_in_iterator)

        let children = if let Some(stmts) = body_stmts {
            self.collect_calls(stmts, cache, functions, visited)?
        } else {
            Vec::new()
        };

        let info = &functions[&func_id];
        let sig = call_graph::build_signature(info);
        let vis = call_graph::visibility_str(&info.visibility);
        let src = format!(
            "{}:{}",
            info.definition.src.offset, info.definition.src.length
        );

        Ok(CallGraphNode::new(
            &sig,
            &info.contract_name,
            info.file.clone(), // checkrs: allow(clone_in_loops)
            &vis,
            &src,
            children,
        ))
    }

    /// Collect all function calls from a list of statements.
    fn collect_calls(
        &self,
        statements: Vec<solc::ast::Statement>,
        cache: &RefCell<HashMap<PathBuf, Vec<call_graph::FunctionInfo>>>,
        functions: &mut HashMap<i64, call_graph::FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<Vec<CallGraphNode>> {
        let mut nodes = Vec::new();
        for stmt in &statements {
            self.collect_calls_from_statement(stmt, cache, functions, visited, &mut nodes)?;
        }
        Ok(nodes)
    }

    /// Collect function calls from a single statement.
    fn collect_calls_from_statement(
        &self,
        stmt: &solc::ast::Statement,
        cache: &RefCell<HashMap<PathBuf, Vec<call_graph::FunctionInfo>>>,
        functions: &mut HashMap<i64, call_graph::FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match stmt {
            solc::ast::Statement::ExpressionStatement(es) => {
                self.collect_calls_from_expression(
                    &es.expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            solc::ast::Statement::Block(block) => {
                for s in &block.statements {
                    self.collect_calls_from_statement(s, cache, functions, visited, nodes)?;
                }
            }
            solc::ast::Statement::IfStatement(ifs) => {
                self.collect_calls_from_expression(
                    &ifs.condition,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_statement(
                    &ifs.true_body,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                if let Some(ref false_body) = ifs.false_body {
                    self.collect_calls_from_statement(
                        false_body, cache, functions, visited, nodes,
                    )?;
                }
            }
            solc::ast::Statement::ForStatement(fors) => {
                if let Some(ref init) = fors.initialization_expression {
                    self.collect_calls_from_expression(init, cache, functions, visited, nodes)?;
                }
                self.collect_calls_from_expression(
                    &fors.condition,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                if let Some(ref loop_expr) = fors.loop_expression {
                    self.collect_calls_from_expression(
                        loop_expr, cache, functions, visited, nodes,
                    )?;
                }
                self.collect_calls_from_statement(&fors.body, cache, functions, visited, nodes)?;
            }
            solc::ast::Statement::WhileStatement(whiles) => {
                self.collect_calls_from_expression(
                    &whiles.condition,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_statement(&whiles.body, cache, functions, visited, nodes)?;
            }
            solc::ast::Statement::DoWhileStatement(dw) => {
                self.collect_calls_from_statement(&dw.body, cache, functions, visited, nodes)?;
                self.collect_calls_from_expression(
                    &dw.condition,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            solc::ast::Statement::Return(ret) => {
                if let Some(ref expr) = ret.expression {
                    self.collect_calls_from_expression(expr, cache, functions, visited, nodes)?;
                }
            }
            solc::ast::Statement::VariableDeclarationStatement(vds) => {
                if let Some(ref expr) = vds.initial_value {
                    self.collect_calls_from_expression(expr, cache, functions, visited, nodes)?;
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
        cache: &RefCell<HashMap<PathBuf, Vec<call_graph::FunctionInfo>>>,
        functions: &mut HashMap<i64, call_graph::FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match expr {
            Expression::FunctionCall(fc) => {
                match &*fc.expression {
                    FunctionCallExpression::MemberAccess(ma) => match ma.referenced_declaration {
                        Some(id) => {
                            self.push_loaded_function(id, cache, functions, visited, nodes)?;
                        }
                        None if call_graph::is_low_level_call(&ma.member_name) => {
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
                        None => {}
                    },
                    FunctionCallExpression::Identifier(id) => {
                        id.referenced_declaration.map_or(Ok(()), |id| {
                            self.push_loaded_function(id, cache, functions, visited, nodes)
                        })?;
                    }
                    FunctionCallExpression::FunctionCallOptions(fco) => {
                        call_graph::resolve_called_function_id_from_fc_expression(&fco.expression)
                            .map_or(Ok(()), |id| {
                                self.push_loaded_function(id, cache, functions, visited, nodes)
                            })?;
                    }
                    _ => {}
                }
                for arg in &fc.arguments {
                    self.collect_calls_from_expression(arg, cache, functions, visited, nodes)?;
                }
                if let FunctionCallExpression::FunctionCallOptions(fco) = &*fc.expression {
                    for opt in &fco.options {
                        self.collect_calls_from_expression(opt, cache, functions, visited, nodes)?;
                    }
                }
            }
            Expression::Assignment(assign) => {
                self.collect_calls_from_expression(
                    &assign.right_hand_side,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &assign.left_hand_side,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::MemberAccess(ma) => {
                self.collect_calls_from_expression(
                    &ma.expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::BinaryOperation(binop) => {
                self.collect_calls_from_expression(
                    &binop.left_expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &binop.right_expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::UnaryOperation(unop) => {
                self.collect_calls_from_expression(
                    &unop.sub_expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::Conditional(cond) => {
                self.collect_calls_from_expression(
                    &cond.condition,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &cond.true_expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &cond.false_expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::TupleExpression(tuple) => {
                for expr in tuple.components.iter().flatten() {
                    self.collect_calls_from_expression(expr, cache, functions, visited, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Ensure a function is loaded and build its node if found.
    fn push_loaded_function(
        &self,
        id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<call_graph::FunctionInfo>>>,
        functions: &mut HashMap<i64, call_graph::FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        if !functions.contains_key(&id) {
            self.ensure_function_loaded(id, cache, functions)?;
        }

        if functions.contains_key(&id) {
            let node = self.build_call_node(id, cache, functions, visited)?;
            nodes.push(node);
        }
        Ok(())
    }

    /// Ensure a function ID is loaded by looking up its artifact path in the
    /// pre-built symbol index and loading only that artifact.
    fn ensure_function_loaded(
        &self,
        id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<call_graph::FunctionInfo>>>,
        functions: &mut HashMap<i64, call_graph::FunctionInfo>,
    ) -> Result<()> {
        if functions.contains_key(&id) {
            return Ok(());
        }
        let Some(entry) = self.symbol_index.get(id) else {
            return Ok(());
        };
        let artifact_path = &self
            .symbol_index
            .artifact_info(entry.artifact_id)
            .artifact_path;
        call_graph::load_artifact_functions(artifact_path, functions, cache)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::inspectors::artifact_id::ArtifactId;
    use crate::project::Project;

    fn fixture_call_path_project() -> Project {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/call-path");
        Project::open(root)
    }

    fn make_id(artifact_id: &str, function: &str) -> FunctionId {
        let aid = ArtifactId::new(artifact_id);
        FunctionId::new(aid, function)
    }

    #[test]
    fn call_path_for_target_internal() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "targetInternal");
        let output = inspector.inspect(&id, "targetInternal").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_targetInternal.txt")
        );
    }

    #[test]
    fn call_path_for_parent_work() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "parentWork");
        let output = inspector.inspect(&id, "parentWork").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_parentWork.txt")
        );
    }

    #[test]
    fn call_path_for_grandparent_work() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "grandparentWork");
        let output = inspector.inspect(&id, "grandparentWork").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_grandparentWork.txt")
        );
    }

    #[test]
    fn call_path_for_lib_lib_work() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "Lib::libWork");
        let output = inspector.inspect(&id, "Lib::libWork").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_Lib_libWork.txt")
        );
    }

    #[test]
    fn call_path_returns_empty_when_target_not_found() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "nonExistent");
        let output = inspector.inspect(&id, "nonExistent").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/call-path/expected/call_path_returns_empty_when_target_not_found.txt"
            )
        );
    }
}
