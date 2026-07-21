//! Shared call graph engine used by both [`CallGraphInspector`] and
//! [`CallPathInspector`] to build call trees from project artifacts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use solc::ast::{
    ContractDefinition, ContractDefinitionNode, Expression, FunctionCallExpression,
    FunctionDefinition, FunctionKind, SourceUnit, SourceUnitNode, TypeName, VariableDeclaration,
    Visibility,
};

use crate::artifact_index::ArtifactIndex;
use crate::build_info::BuildInfo;
use crate::inspectors::artifact_id::ArtifactId;
use crate::inspectors::function_source::symbol_index::SymbolIndex;
use crate::project::Project;

/// A node in a call graph.
///
/// Each node represents a function call that may contain child calls.
#[derive(Debug, Clone)]
pub struct CallGraphNode {
    /// Human-readable signature, e.g. `Main::execute(uint256)`.
    pub signature: String,
    /// The contract name that defines this function.
    pub contract_name: String,
    /// The source file path.
    pub file: PathBuf,
    /// Visibility: `external`, `public`, `internal`, `private`.
    pub visibility: String,
    /// Source location range for the function (offset:length).
    pub src: String,
    /// Calls made within this function.
    pub children: Vec<CallGraphNode>,
}

impl CallGraphNode {
    /// Create a new call graph node.
    pub fn new(
        signature: &str,
        contract_name: &str,
        file: PathBuf,
        visibility: &str,
        src: &str,
        children: Vec<CallGraphNode>,
    ) -> Self {
        Self {
            signature: signature.to_string(),
            contract_name: contract_name.to_string(),
            file,
            visibility: visibility.to_string(),
            src: src.to_string(),
            children,
        }
    }

    /// Flatten the call graph into a depth-first list of `(file, src)` pairs.
    pub fn flatten_sources(&self) -> Vec<(PathBuf, String)> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        self.flatten_sources_recursive(&mut result, &mut seen);
        result
    }

    fn flatten_sources_recursive(
        &self,
        out: &mut Vec<(PathBuf, String)>,
        seen: &mut HashSet<(PathBuf, String)>,
    ) {
        if !self.file.as_os_str().is_empty() {
            let key = (self.file.clone(), self.src.clone());
            if seen.insert(key) {
                out.push((self.file.clone(), self.src.clone()));
            }
        }
        for child in &self.children {
            child.flatten_sources_recursive(out, seen);
        }
    }

    /// Check if this node or any of its descendants matches the target function signature.
    pub fn reaches_target(&self, target: &str) -> bool {
        if self.matches_target(target) {
            return true;
        }
        for child in &self.children {
            if child.reaches_target(target) {
                return true;
            }
        }
        false
    }

    /// Extract the function name from this node's signature.
    ///
    /// For example, `"Main::execute(uint256)"` returns `"execute"`.
    pub fn func_name(&self) -> &str {
        self.signature
            .split("::")
            .nth(1)
            .and_then(|part| part.split('(').next())
            .and_then(|part| part.split('<').next())
            .unwrap_or("")
    }

    /// Check if this single node matches the given target.
    ///
    /// The target can be:
    /// - A bare function name (e.g., `"removeLiquidity"`) to match any contract.
    /// - A scoped name (e.g., `"LiquidityLib::removeLiquidity"`) to match a
    ///   specific contract and function.
    pub fn matches_target(&self, target: &str) -> bool {
        if let Some((contract_part, func_part)) = target.split_once("::") {
            self.contract_name == contract_part && self.func_name() == func_part
        } else {
            self.func_name() == target
        }
    }

    /// Find the first descendant node matching the target function.
    pub fn find_matching(&self, target: &str) -> Option<&Self> {
        if self.matches_target(target) {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find_matching(target) {
                return Some(found);
            }
        }
        None
    }
}

impl fmt::Display for CallGraphNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} ({})",
            self.signature.replacen("::", ".", 1),
            self.visibility
        )?;
        fmt_children(&self.children, f, "")
    }
}

fn fmt_children(
    children: &[CallGraphNode],
    f: &mut fmt::Formatter<'_>,
    prefix: &str,
) -> fmt::Result {
    let len = children.len();
    for (i, child) in children.iter().enumerate() {
        let is_last = i == len - 1;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let continuation = if is_last { "    " } else { "\u{2502}   " };

        writeln!(
            f,
            "{}{}{} ({})",
            prefix,
            connector,
            child.signature.replacen("::", ".", 1),
            child.visibility
        )?;
        let child_prefix = format!("{}{}", prefix, continuation);
        fmt_children(&child.children, f, &child_prefix)?;
    }
    Ok(())
}

/// Identifies a function within a contract artifact.
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

/// Result of finding external entry points that can reach a target function.
pub struct CallPaths {
    /// Root nodes (external/public functions) that reach the target.
    pub roots: Vec<CallGraphNode>,
    /// Source file of the target function.
    pub target_file: PathBuf,
    /// Source location (offset:length) of the target function.
    pub target_src: String,
    /// Scoped target string ("Contract::function") used for matching.
    pub scoped_target: String,
}

// Internal types

/// Result of resolving an artifact ID to a specific artifact path.
enum ResolvedPath {
    Single(PathBuf),
    Ambiguous(Vec<PathBuf>),
    NotFound,
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

// Call graph engine

/// The core call-graph engine shared by inspectors.
///
/// Provides high-level methods to build call trees and find call paths
/// from a project, encapsulating all internal bookkeeping.
pub struct CallGraph {
    project: Project,
    artifact_index: ArtifactIndex,
    symbol_index: SymbolIndex,
}

impl CallGraph {
    /// Build a [`CallGraph`] for the given project.
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

    /// The project's root path.
    pub fn project_root(&self) -> &Path {
        self.project.path()
    }

    /// The project's output directory.
    pub fn out_dir(&self) -> &Path {
        self.project.out_dir()
    }

    /// Build a call tree for a single function (forward call graph).
    ///
    /// `ambiguity_candidates` is an optional list of alternative artifact paths
    /// to show when the target function is not found.
    pub fn build_call_tree(
        &self,
        id: &FunctionId,
        ambiguity_candidates: Option<&[PathBuf]>,
    ) -> Result<CallGraphNode> {
        let artifact_path = self.resolve_artifact(id.artifact_id())?;

        let cache: RefCell<HashMap<PathBuf, Vec<FunctionInfo>>> = RefCell::new(HashMap::new());
        let mut functions: HashMap<i64, FunctionInfo> = HashMap::new();
        load_artifact_functions(&artifact_path, &mut functions, &cache)?;

        let target_name = id.function_name();

        let target_ids: Vec<i64> = functions
            .values()
            .filter(|fi| fi.name == target_name)
            .map(|fi| fi.id)
            .collect();

        if target_ids.is_empty() {
            // If there was contract-level ambiguity, emit a suggestion error.
            if let Some(candidates) = ambiguity_candidates {
                bail!(format_ambiguity_error(
                    candidates,
                    self.project.out_dir(),
                    id.artifact_id().name.as_str(),
                    target_name,
                ));
            }
            let contract_name = find_contract_name(&functions, id.artifact_id().name.as_str());
            bail!("\"{target_name}\" not found in \"{contract_name}\".");
        }

        let target_id = target_ids[0];
        let mut visited: HashSet<i64> = HashSet::new();
        self.build_call_node(target_id, &cache, &mut functions, &mut visited)
    }

    /// Find all external/public functions that can reach the target.
    ///
    /// Returns [`CallPaths`] containing the matching root nodes and the
    /// target function's source location.
    pub fn find_call_paths(&self, id: &FunctionId, target_function: &str) -> Result<CallPaths> {
        // 1. Validate the contract artifact exists.
        let artifact_path = self.resolve_artifact(id.artifact_id())?;

        // 2. Load functions from the target artifact to validate the function.
        let cache: RefCell<HashMap<PathBuf, Vec<FunctionInfo>>> = RefCell::new(HashMap::new());
        let mut functions: HashMap<i64, FunctionInfo> = HashMap::new();
        load_artifact_functions(&artifact_path, &mut functions, &cache)?;

        let target_funcs: Vec<&FunctionInfo> = functions
            .values()
            .filter(|fi| fi.name == target_function)
            .collect();

        ensure!(
            !target_funcs.is_empty(),
            "\"{}\" not found in \"{}\".",
            target_function,
            id.artifact_id().name
        );

        ensure!(
            target_funcs.len() <= 1,
            "\"{}\" has multiple overloads in \"{}\"; use the full signature.",
            target_function,
            id.artifact_id().name
        );

        // 3. Build the scoped target ("Contract::function") so that matching
        //    only finds nodes from the specific contract that defines the
        //    function (which may be a parent of the user-specified artifact).
        let defining_contract = &target_funcs[0].contract_name;
        let scoped_target = format!("{}::{}", defining_contract, target_function);

        // 4. Determine the project's src directory to filter which artifacts
        //    to search. We only look for call paths from source files within
        //    the configured src directory (e.g., "src" or "contracts"), not
        //    from test or library files.
        let project_root = self.project.path();
        let src_dir = self
            .project
            .directories()
            .ok()
            .map(|d| d.src)
            .unwrap_or_else(|| PathBuf::from("src"));

        // 5. Collect src-filtered source files, one per source file to avoid
        //    ID collisions across compilation units.
        let mut source_files: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut seen_sources: HashSet<PathBuf> = HashSet::new();
        for path in self.artifact_index.all_entries() {
            let Some(source) = extract_artifact_source(path) else {
                continue;
            };
            if !seen_sources.insert(source.clone()) {
                continue;
            }
            if !is_in_src_dir(&source, project_root, &src_dir) {
                continue;
            }
            source_files.push((path.clone(), source)); // checkrs: allow(clone_in_loops)
        }

        let mut matching_roots = Vec::new();
        let mut target_file = PathBuf::new();
        let mut target_src = String::new();
        let mut target_found = false;

        for (artifact_path, _source) in &source_files {
            let cache: RefCell<HashMap<PathBuf, Vec<FunctionInfo>>> = RefCell::new(HashMap::new());
            let mut functions: HashMap<i64, FunctionInfo> = HashMap::new();
            load_artifact_functions(artifact_path, &mut functions, &cache)?;

            // Collect external/public function IDs from this source file.
            let external_ids: Vec<i64> = functions
                .iter()
                .filter(|(_, fi)| {
                    matches!(fi.visibility, Visibility::External | Visibility::Public)
                })
                .map(|(id, _)| *id)
                .collect();

            for &func_id in &external_ids {
                let mut visited: HashSet<i64> = HashSet::new();
                let root = self.build_call_node(func_id, &cache, &mut functions, &mut visited)?;
                // Use the scoped target (Contract::function) so we only match
                // nodes from the specific target contract.
                if root.reaches_target(&scoped_target) {
                    matching_roots.push(root);
                }
            }

            // Find target source location from this source file's functions.
            if !target_found
                && let Some(fi) = functions.values().find(|fi| fi.name == target_function)
            {
                target_file = fi.file.clone(); // checkrs: allow(clone_in_loops)
                target_src = format!("{}:{}", fi.definition.src.offset, fi.definition.src.length);
                target_found = true;
            }
        }

        matching_roots.sort_by(|a, b| a.signature.cmp(&b.signature));

        // Prefer target source from matching roots when available.
        if let Some((f, s)) = find_target_source_in_roots(&matching_roots, &scoped_target) {
            target_file = f.clone();
            target_src = s.to_string();
        }

        Ok(CallPaths {
            roots: matching_roots,
            target_file,
            target_src,
            scoped_target,
        })
    }
    /// Resolve an artifact ID, returning the path and any ambiguity candidates.
    pub fn resolve_artifact_with_candidates(
        &self,
        id: &ArtifactId,
    ) -> Result<(PathBuf, Option<Vec<PathBuf>>)> {
        match self.resolve_artifact_path(id) {
            ResolvedPath::Single(path) => Ok((path, None)),
            ResolvedPath::Ambiguous(candidates) => {
                let first = candidates[0].clone();
                Ok((first, Some(candidates)))
            }
            ResolvedPath::NotFound => {
                bail!("\"{}\" not found.", id.name);
            }
        }
    }

    /// Resolve an artifact ID to a single artifact path.
    pub fn resolve_artifact(&self, id: &ArtifactId) -> Result<PathBuf> {
        match self.resolve_artifact_path(id) {
            ResolvedPath::Single(path) => Ok(path),
            ResolvedPath::Ambiguous(candidates) => Ok(candidates[0].clone()),
            ResolvedPath::NotFound => {
                bail!("\"{}\" not found.", id.name);
            }
        }
    }

    fn resolve_artifact_path(&self, id: &ArtifactId) -> ResolvedPath {
        match &id.file {
            Some(file) => {
                let path = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                if path.exists() {
                    ResolvedPath::Single(path)
                } else {
                    ResolvedPath::NotFound
                }
            }
            None => {
                let candidates = self
                    .artifact_index
                    .get(&id.name)
                    .cloned()
                    .unwrap_or_default();

                match candidates.len() {
                    0 => ResolvedPath::NotFound,
                    1 => ResolvedPath::Single(candidates[0].clone()),
                    _ => ResolvedPath::Ambiguous(candidates),
                }
            }
        }
    }

    fn build_call_node(
        &self,
        func_id: i64,
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
            self.collect_calls(stmts, cache, functions, visited)?
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

    fn collect_calls(
        &self,
        statements: Vec<solc::ast::Statement>,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<Vec<CallGraphNode>> {
        let mut nodes = Vec::new();
        for stmt in &statements {
            self.collect_calls_from_statement(stmt, cache, functions, visited, &mut nodes)?;
        }
        Ok(nodes)
    }

    fn collect_calls_from_statement(
        &self,
        stmt: &solc::ast::Statement,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
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
            solc::ast::Statement::UncheckedBlock(ub) => {
                for s in &ub.statements {
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

    fn collect_calls_from_expression(
        &self,
        expr: &Expression,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
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
                            self.push_loaded_function(id, cache, functions, visited, nodes)
                        })?;
                    }
                    FunctionCallExpression::FunctionCallOptions(fco) => {
                        resolve_called_function_id_from_fc_expression(&fco.expression)
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

    fn push_loaded_function(
        &self,
        id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
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

    fn ensure_function_loaded(
        &self,
        id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
    ) -> Result<()> {
        let Some(entry) = self.symbol_index.get(id) else {
            return Ok(());
        };
        let artifact_path = &self
            .symbol_index
            .artifact_info(entry.artifact_id)
            .artifact_path;
        // Always load from the correct artifact, even if the ID is already
        // present in `functions`. The same AST node ID can refer to different
        // functions across compilation units (different .sol files).
        load_artifact_functions(artifact_path, functions, cache)?;
        Ok(())
    }
}

/// Get the file and src for a target function by searching the roots.
fn find_target_source_in_roots<'a>(
    roots: &'a [CallGraphNode],
    target: &str,
) -> Option<(&'a PathBuf, &'a str)> {
    for root in roots {
        if let Some(node) = root.find_matching(target)
            && !node.file.as_os_str().is_empty()
        {
            return Some((&node.file, &node.src));
        }
    }
    None
}

// Private helpers

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

fn load_artifact_functions(
    path: impl AsRef<Path>,
    functions: &mut HashMap<i64, FunctionInfo>,
    cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
) -> Result<()> {
    let path = path.as_ref();
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

/// Extract the source file path from an artifact JSON file.
fn extract_artifact_source(path: impl AsRef<Path>) -> Option<PathBuf> {
    let content = fs::read_to_string(path.as_ref()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("ast")?
        .get("absolutePath")?
        .as_str()
        .map(PathBuf::from)
}

/// Check if a source file path is under the project's configured `src` directory.
fn is_in_src_dir(
    source: impl AsRef<Path>,
    project_root: impl AsRef<Path>,
    src_dir: impl AsRef<Path>,
) -> bool {
    // The source path from the artifact is an absolute path. Compute the
    // relative path from the project root and check if it starts with the
    // configured src directory (e.g., "src" or "contracts").
    let relative = source
        .as_ref()
        .strip_prefix(project_root.as_ref())
        .unwrap_or(source.as_ref());
    relative.starts_with(src_dir.as_ref())
}

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

fn extract_contract_functions(cd: ContractDefinition, source_file: &Path) -> Vec<FunctionInfo> {
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

fn build_signature(info: &FunctionInfo) -> String {
    let name = match info.definition.kind {
        FunctionKind::Receive => "receive",
        FunctionKind::Fallback => "fallback",
        _ => &info.name,
    };
    format!(
        "{}::{}({})",
        info.contract_name,
        name,
        format_params(&info.parameters)
    )
}

fn visibility_str(vis: &Visibility) -> String {
    match vis {
        Visibility::External => "external".into(),
        Visibility::Public => "public".into(),
        Visibility::Internal => "internal".into(),
        Visibility::Private => "private".into(),
    }
}

fn format_params(params: &[VariableDeclaration]) -> String {
    params
        .iter()
        .map(|p| format_type_name(&p.type_name))
        .collect::<Vec<String>>()
        .join(",")
}

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

fn resolve_called_function_id_from_fc_expression(expr: &Expression) -> Option<i64> {
    match expr {
        Expression::MemberAccess(ma) => ma.referenced_declaration,
        Expression::Identifier(id) => id.referenced_declaration,
        _ => None,
    }
}

fn is_low_level_call(member_name: &str) -> bool {
    matches!(
        member_name,
        "call" | "delegatecall" | "staticcall" | "callcode"
    )
}

/// Format an ambiguity error message for call-graph resolution.
fn format_ambiguity_error(
    candidates: &[PathBuf],
    out_dir: impl AsRef<Path>,
    contract_name: &str,
    function_name: &str,
) -> String {
    let out_dir = out_dir.as_ref();
    let mut sorted = candidates.to_vec();
    sorted.sort();

    let mut msg = format!(
        "found {} \"{}\"\n\nSelect one of the following:\n",
        sorted.len(),
        contract_name
    );
    for candidate in &sorted {
        let rel = candidate.strip_prefix(out_dir).unwrap_or(candidate);
        let parent = rel.parent().and_then(|p| p.to_str()).unwrap_or("");
        msg.push_str(&format!(
            "\nhawk inspect call-graph {}:{} {}",
            parent, contract_name, function_name
        ));
    }
    msg.push('\n');
    msg
}

// Tests

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn display_simple_function() {
        let node = CallGraphNode::new(
            "Main::execute(uint256)",
            "Main",
            PathBuf::from("src/Main.sol"),
            "public",
            "276:110",
            vec![],
        );
        assert_eq!(node.to_string(), "Main.execute(uint256) (public)\n");
    }

    #[test]
    fn display_nested_function_calls() {
        let node = CallGraphNode::new(
            "Main::execute(uint256)",
            "Main",
            PathBuf::from("src/Main.sol"),
            "public",
            "276:110",
            vec![
                CallGraphNode::new(
                    "Helper::assist(uint256)",
                    "Helper",
                    PathBuf::from("src/Helper.sol"),
                    "public",
                    "109:72",
                    vec![],
                ),
                CallGraphNode::new(
                    "Main::internalWork()",
                    "Main",
                    PathBuf::from("src/Main.sol"),
                    "internal",
                    "392:79",
                    vec![CallGraphNode::new(
                        "Base::baseWork()",
                        "Base",
                        PathBuf::from("src/Main.sol"),
                        "internal",
                        "226:42",
                        vec![],
                    )],
                ),
            ],
        );
        let expected = concat!(
            "Main.execute(uint256) (public)\n",
            "\u{251c}\u{2500}\u{2500} Helper.assist(uint256) (public)\n",
            "\u{2514}\u{2500}\u{2500} Main.internalWork() (internal)\n",
            "    \u{2514}\u{2500}\u{2500} Base.baseWork() (internal)\n",
        );
        assert_eq!(node.to_string(), expected);
    }

    #[test]
    fn flatten_sources_collects_depth_first() {
        let node = CallGraphNode::new(
            "Main::execute(uint256)",
            "Main",
            PathBuf::from("src/Main.sol"),
            "public",
            "276:110",
            vec![CallGraphNode::new(
                "Helper::assist(uint256)",
                "Helper",
                PathBuf::from("src/Helper.sol"),
                "public",
                "109:72",
                vec![],
            )],
        );
        let sources = node.flatten_sources();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].0, PathBuf::from("src/Main.sol"));
        assert_eq!(sources[0].1, "276:110");
        assert_eq!(sources[1].0, PathBuf::from("src/Helper.sol"));
        assert_eq!(sources[1].1, "109:72");
    }
}
