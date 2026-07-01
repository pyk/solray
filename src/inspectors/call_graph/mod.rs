//! Call graph inspection for Foundry projects.
//!
//! [`CallGraphInspector`] reads a contract artifact, finds the specified
//! function, and produces a tree showing every function it calls recursively.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Deserialize;
use solc::ast::{
    ContractDefinitionNode, Expression, FunctionCallExpression, FunctionDefinition, SourceUnit,
    SourceUnitNode, TypeName, VariableDeclaration, Visibility,
};

use crate::artifact_index::ArtifactIndex;
use crate::inspectors::artifact_id::ArtifactId;
use crate::inspectors::call_graph::node::CallGraphNode;
use crate::inspectors::call_graph::source_renderer::offset_to_line_range;
use crate::project::Project;

mod node;
mod source_renderer;

/// Identifies a function within a contract artifact.
///
/// Constructed from an `ArtifactId` (contract) and a function name.
pub struct FunctionId {
    artifact_id: ArtifactId,
    function: String,
}

impl FunctionId {
    /// Create a new [`FunctionId`] from an [`ArtifactId`] and function name.
    pub fn new(artifact_id: ArtifactId, function: &str) -> Self {
        Self {
            artifact_id,
            function: function.to_string(),
        }
    }

    /// The artifact ID identifying the contract.
    pub fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// The function name.
    pub fn function_name(&self) -> &str {
        &self.function
    }
}

/// The output of a [`CallGraphInspector`] inspection.
#[derive(Debug)]
pub struct CallGraphInspectorOutput {
    root: CallGraphNode,
    project_root: PathBuf,
}

impl CallGraphInspectorOutput {
    /// Create a new [`CallGraphInspectorOutput`] from a root call graph node.
    pub fn new(root: CallGraphNode, project_root: PathBuf) -> Self {
        Self { root, project_root }
    }
}

impl std::fmt::Display for CallGraphInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sources = self.root.flatten_sources();
        let cwd = std::env::current_dir().unwrap_or_else(|_| self.project_root.clone());
        let project_abs =
            std::path::absolute(&self.project_root).unwrap_or_else(|_| self.project_root.clone());

        let mut line_maps: HashMap<PathBuf, Vec<usize>> = HashMap::new();

        writeln!(f, "Call graph:\n")?;
        write!(f, "{}", self.root)?;
        writeln!(f, "\nResolved from {} sources:\n", sources.len())?;

        for (i, (file, src)) in sources.iter().enumerate() {
            let full_path = project_abs.join(file);
            let rel_path = full_path.strip_prefix(&cwd).unwrap_or(&full_path);
            let line_range = offset_to_line_range(&full_path, src, &mut line_maps);
            writeln!(f, "{}. {}#{}", i + 1, rel_path.display(), line_range)?;
        }

        Ok(())
    }
}

/// Result of resolving an artifact ID to a specific artifact path.
enum ResolvedPath {
    /// A single artifact path was found.
    Single(PathBuf),
    /// The contract was found in multiple artifact files.
    Ambiguous(Vec<PathBuf>),
    /// The contract was not found.
    NotFound,
}

/// Inspect a Foundry project for the call graph of a single function.
pub struct CallGraphInspector {
    project: Project,
}

impl CallGraphInspector {
    /// Build a [`CallGraphInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Inspect the call graph for the given [`FunctionId`].
    pub fn inspect(&self, id: &FunctionId) -> Result<CallGraphInspectorOutput> {
        let resolved = self.resolve_artifact_path(id.artifact_id())?;

        let (artifact_path, ambiguity_candidates) = match resolved {
            ResolvedPath::Single(path) => (path, None),
            ResolvedPath::Ambiguous(candidates) => {
                // Load the first candidate to look for the target function.
                // If not found, emit an ambiguity error showing all candidates.
                let first = candidates[0].clone();
                (first, Some(candidates))
            }
            ResolvedPath::NotFound => {
                bail!("\"{}\" not found.", id.artifact_id().name);
            }
        };

        let artifact_index = ArtifactIndex::build(self.project.out_dir());
        let cache: RefCell<HashMap<PathBuf, Vec<FunctionInfo>>> = RefCell::new(HashMap::new());

        let mut functions: HashMap<i64, FunctionInfo> = HashMap::new();
        load_artifact_functions(&artifact_path, &mut functions, &cache)?;

        let project_root = self.project.path().to_path_buf();
        let target_name = id.function_name();

        let target_ids: Vec<i64> = functions
            .values()
            .filter(|fi| fi.name == target_name)
            .map(|fi| fi.id)
            .collect();

        if target_ids.is_empty() {
            // If there was contract-level ambiguity, emit an ambiguity error
            // with all candidates so the user can disambiguate.
            if let Some(candidates) = ambiguity_candidates {
                self.emit_contract_ambiguity_error(
                    &candidates,
                    id.artifact_id().name.as_str(),
                    target_name,
                )?;
            }
            let contract_name = find_contract_name(&functions, id.artifact_id().name.as_str());
            bail!("\"{target_name}\" not found in \"{contract_name}\".");
        }

        let target_id = target_ids[0];
        let mut visited: HashSet<i64> = HashSet::new();
        let root = self.build_call_node(
            target_id,
            &artifact_index,
            &cache,
            &mut functions,
            &mut visited,
        )?;

        Ok(CallGraphInspectorOutput::new(root, project_root))
    }

    /// Emit an ambiguity error showing all candidate artifact paths.
    fn emit_contract_ambiguity_error(
        &self,
        candidates: &[PathBuf],
        contract_name: &str,
        function_name: &str,
    ) -> Result<()> {
        let mut sorted = candidates.to_vec();
        sorted.sort();

        let mut msg = format!(
            "found {} \"{}\"\n\nSelect one of the following:\n",
            sorted.len(),
            contract_name
        );
        for candidate in &sorted {
            let rel = candidate
                .strip_prefix(self.project.out_dir())
                .unwrap_or(candidate);
            let parent = rel.parent().and_then(|p| p.to_str()).unwrap_or("");
            msg.push_str(&format!(
                "\nhawk inspect call-graph {}:{} {function_name}",
                parent, contract_name
            ));
        }
        msg.push('\n');
        bail!(msg);
    }

    /// Resolve the artifact file path for the given [`ArtifactId`].
    fn resolve_artifact_path(&self, id: &ArtifactId) -> Result<ResolvedPath> {
        match &id.file {
            Some(file) => {
                let path = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                if path.exists() {
                    Ok(ResolvedPath::Single(path))
                } else {
                    Ok(ResolvedPath::NotFound)
                }
            }
            None => {
                let index = ArtifactIndex::build(self.project.out_dir());
                let candidates = index.get(&id.name).cloned().unwrap_or_default();

                match candidates.len() {
                    0 => Ok(ResolvedPath::NotFound),
                    1 => Ok(ResolvedPath::Single(candidates[0].clone())),
                    _ => Ok(ResolvedPath::Ambiguous(candidates)),
                }
            }
        }
    }

    /// Build a `CallGraphNode` for a function by ID, recursively.
    fn build_call_node(
        &self,
        func_id: i64,
        artifact_index: &ArtifactIndex,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
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

        let body_stmts = functions
            .get(&func_id)
            .and_then(|fi| fi.definition.body.as_ref().map(|b| b.statements.clone())); // checkrs: allow(clone_in_iterator)

        let children = if let Some(stmts) = body_stmts {
            self.collect_calls(stmts, artifact_index, cache, functions, visited)?
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
        statements: Vec<solc::ast::Statement>,
        artifact_index: &ArtifactIndex,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<Vec<CallGraphNode>> {
        let mut nodes = Vec::new();
        for stmt in &statements {
            self.collect_calls_from_statement(
                stmt,
                artifact_index,
                cache,
                functions,
                visited,
                &mut nodes,
            )?;
        }
        Ok(nodes)
    }

    /// Collect function calls from a single statement.
    fn collect_calls_from_statement(
        &self,
        stmt: &solc::ast::Statement,
        artifact_index: &ArtifactIndex,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match stmt {
            solc::ast::Statement::ExpressionStatement(es) => {
                self.collect_calls_from_expression(
                    &es.expression,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            solc::ast::Statement::Block(block) => {
                for s in &block.statements {
                    self.collect_calls_from_statement(
                        s,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
                }
            }
            solc::ast::Statement::IfStatement(ifs) => {
                self.collect_calls_from_expression(
                    &ifs.condition,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_statement(
                    &ifs.true_body,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                if let Some(ref false_body) = ifs.false_body {
                    self.collect_calls_from_statement(
                        false_body,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
                }
            }
            solc::ast::Statement::ForStatement(fors) => {
                if let Some(ref init) = fors.initialization_expression {
                    self.collect_calls_from_expression(
                        init,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
                }
                self.collect_calls_from_expression(
                    &fors.condition,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                if let Some(ref loop_expr) = fors.loop_expression {
                    self.collect_calls_from_expression(
                        loop_expr,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
                }
                self.collect_calls_from_statement(
                    &fors.body,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            solc::ast::Statement::WhileStatement(whiles) => {
                self.collect_calls_from_expression(
                    &whiles.condition,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_statement(
                    &whiles.body,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            solc::ast::Statement::DoWhileStatement(dw) => {
                self.collect_calls_from_statement(
                    &dw.body,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &dw.condition,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            solc::ast::Statement::Return(ret) => {
                if let Some(ref expr) = ret.expression {
                    self.collect_calls_from_expression(
                        expr,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
                }
            }
            solc::ast::Statement::VariableDeclarationStatement(vds) => {
                if let Some(ref expr) = vds.initial_value {
                    self.collect_calls_from_expression(
                        expr,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
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
        artifact_index: &ArtifactIndex,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        match expr {
            Expression::FunctionCall(fc) => {
                match &*fc.expression {
                    FunctionCallExpression::MemberAccess(ma) => match ma.referenced_declaration {
                        Some(id) => self.push_loaded_function(
                            id,
                            artifact_index,
                            cache,
                            functions,
                            visited,
                            nodes,
                        )?,
                        None if is_low_level_call(&ma.member_name) => {
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
                            self.push_loaded_function(
                                id,
                                artifact_index,
                                cache,
                                functions,
                                visited,
                                nodes,
                            )
                        })?;
                    }
                    FunctionCallExpression::FunctionCallOptions(fco) => {
                        resolve_called_function_id_from_fc_expression(&fco.expression).map_or(
                            Ok(()),
                            |id| {
                                self.push_loaded_function(
                                    id,
                                    artifact_index,
                                    cache,
                                    functions,
                                    visited,
                                    nodes,
                                )
                            },
                        )?;
                    }
                    _ => {}
                }
                for arg in &fc.arguments {
                    self.collect_calls_from_expression(
                        arg,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
                }
                if let FunctionCallExpression::FunctionCallOptions(fco) = &*fc.expression {
                    for opt in &fco.options {
                        self.collect_calls_from_expression(
                            opt,
                            artifact_index,
                            cache,
                            functions,
                            visited,
                            nodes,
                        )?;
                    }
                }
            }
            Expression::Assignment(assign) => {
                self.collect_calls_from_expression(
                    &assign.right_hand_side,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &assign.left_hand_side,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::MemberAccess(ma) => {
                self.collect_calls_from_expression(
                    &ma.expression,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::BinaryOperation(binop) => {
                self.collect_calls_from_expression(
                    &binop.left_expression,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &binop.right_expression,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::UnaryOperation(unop) => {
                self.collect_calls_from_expression(
                    &unop.sub_expression,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::Conditional(cond) => {
                self.collect_calls_from_expression(
                    &cond.condition,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &cond.true_expression,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                self.collect_calls_from_expression(
                    &cond.false_expression,
                    artifact_index,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
            }
            Expression::TupleExpression(tuple) => {
                for expr in tuple.components.iter().flatten() {
                    self.collect_calls_from_expression(
                        expr,
                        artifact_index,
                        cache,
                        functions,
                        visited,
                        nodes,
                    )?;
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
        artifact_index: &ArtifactIndex,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        if !functions.contains_key(&id) {
            self.ensure_function_loaded(id, artifact_index, cache, functions)?;
        }

        if functions.contains_key(&id) {
            let node = self.build_call_node(id, artifact_index, cache, functions, visited)?;
            nodes.push(node);
        }
        Ok(())
    }

    /// Ensure a function ID is loaded by scanning artifacts from the index.
    fn ensure_function_loaded(
        &self,
        id: i64,
        artifact_index: &ArtifactIndex,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
    ) -> Result<()> {
        for artifact_path in artifact_index.all_entries() {
            if cache.borrow().contains_key(artifact_path) {
                continue;
            }
            load_artifact_functions(artifact_path, functions, cache)?;
            if functions.contains_key(&id) {
                return Ok(());
            }
        }
        Ok(())
    }
}

// -- Internal helpers --

/// Find the contract name in the loaded functions that matches a given
/// contract name from the artifact ID.
fn find_contract_name(functions: &HashMap<i64, FunctionInfo>, target_name: &str) -> String {
    if functions.values().any(|fi| fi.contract_name == target_name) {
        return target_name.to_string();
    }
    functions
        .values()
        .next()
        .map(|fi| fi.contract_name.clone()) // checkrs: allow(clone_in_iterator)
        .unwrap_or_default()
}

/// Function information extracted from an artifact AST.
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

/// Minimal artifact wrapper for extracting the AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

/// Parse a single artifact JSON file and insert its functions into the map.
fn load_artifact_functions(
    path: impl AsRef<Path>,
    functions: &mut HashMap<i64, FunctionInfo>,
    cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
) -> Result<()> {
    let path = path.as_ref();

    // Check the cache first.
    {
        let cache_ref = cache.borrow();
        if let Some(cached) = cache_ref.get(path) {
            for fi in cached {
                functions.insert(fi.id, fi.clone()); // checkrs: allow(clone_in_loops)
            }
            return Ok(());
        }
    }

    let funcs = parse_artifact_functions(path)?;

    if !funcs.is_empty() {
        cache.borrow_mut().insert(path.to_path_buf(), funcs.clone());
        for fi in funcs {
            functions.insert(fi.id, fi);
        }
    }
    Ok(())
}

/// Parse a single artifact JSON file, returning all [`FunctionInfo`] entries.
fn parse_artifact_functions(path: impl AsRef<Path>) -> Result<Vec<FunctionInfo>> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)?;

    let ast = match artifact.ast {
        None => return Ok(Vec::new()),
        Some(ast) => ast,
    };

    let source_file = ast.absolute_path;
    let mut functions = Vec::new();

    for node in ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node {
            functions.extend(extract_contract_functions(cd, &source_file));
        }
    }

    Ok(functions)
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

/// Build a human-readable function signature.
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

/// Format parameter declarations into a comma-separated type list.
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

    fn make_id(artifact_id: &str, function: &str) -> FunctionId {
        let aid = ArtifactId::new(artifact_id);
        FunctionId::new(aid, function)
    }

    #[test]
    fn call_graph_for_readonly() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "readOnly");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_for_readonly.txt")
        );
    }

    #[test]
    fn call_graph_for_execute() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "execute");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_for_execute.txt")
        );
    }

    #[test]
    fn call_graph_errors_for_unknown_contract() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Unknown", "function");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/calls/expected/call_graph_errors_for_unknown_contract.txt"
            )
            .trim_end()
        );
    }

    #[test]
    fn call_graph_errors_for_unknown_function() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "unknownFunction");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/calls/expected/call_graph_errors_for_unknown_function.txt"
            )
            .trim_end()
        );
    }

    #[test]
    fn ambiguity_shows_suggestions() {
        let inspector = CallGraphInspector::new(fixture_ambiguous_project());
        let id = make_id("Dupe", "someFunction");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!("../../../fixtures/calls/expected/ambiguity_shows_suggestions.txt")
        );
    }

    #[test]
    fn call_graph_for_interface_call() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "callViaInterface");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_for_interface_call.txt")
        );
    }

    #[test]
    fn call_graph_includes_low_level_call() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("LowLevelCaller", "callWithPayload");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_includes_low_level_call.txt")
        );
    }
}
