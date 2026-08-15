//! Shared call graph engine used by both [`CallGraphInspector`] and
//! [`CallPathInspector`] to build call trees from project artifacts.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use solc::ast::{
    Block, ContractDefinition, ContractDefinitionNode, ContractKind, Expression,
    FunctionCallExpression, FunctionKind, ModifierInvocation, ModifierInvocationKind, SourceUnit,
    SourceUnitNode, TypeName, UserDefinedTypeName, VariableDeclaration, Visibility,
};
use tracing::debug;

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
    /// - A full signature (e.g., `"removeLiquidity(address,address)"`) to match
    ///   a specific overload.
    pub fn matches_target(&self, target: &str) -> bool {
        if let Some((contract_part, func_part)) = target.split_once("::") {
            self.contract_name == contract_part && self.matches_func_target(func_part)
        } else {
            self.matches_func_target(target)
        }
    }

    fn matches_func_target(&self, func_target: &str) -> bool {
        let target = parse_function_target(func_target);
        if self.func_name() != target.name {
            return false;
        }
        match target.params {
            Some(params) => self.signature_params() == params,
            None => true,
        }
    }

    fn signature_params(&self) -> &str {
        self.signature
            .split_once('(')
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .unwrap_or("")
    }

    /// Identity of the linear path from this node to `target`.
    ///
    /// Used to drop duplicate roots that come from expanding the same function
    /// once per inheriting contract.
    fn path_key_to_target(&self, target: &str) -> Vec<(String, String, String)> {
        let mut key = vec![(
            self.contract_name.clone(),
            self.signature.clone(),
            self.src.clone(),
        )];
        if self.matches_target(target) {
            return key;
        }
        for child in &self.children {
            if child.reaches_target(target) {
                key.extend(child.path_key_to_target(target));
                break;
            }
        }
        key
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
    is_interface: bool,
    parameters: Vec<VariableDeclaration>,
    visibility: Visibility,
    kind: FunctionKind,
    src_offset: usize,
    src_length: usize,
    body: Option<Block>,
    modifiers: Vec<ModifierInvocation>,
}

/// A parsed function target, either a bare name or a `name(params)` signature.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FunctionTarget {
    name: String,
    params: Option<String>,
}

fn parse_function_target(target: &str) -> FunctionTarget {
    let Some(open) = target.find('(') else {
        return FunctionTarget {
            name: target.trim().to_string(),
            params: None,
        };
    };
    let name = target[..open].trim().to_string();
    let params = target
        .get(open + 1..)
        .and_then(|rest| rest.strip_suffix(')'))
        .map(|params| {
            params
                .split(',')
                .map(str::trim)
                .collect::<Vec<&str>>()
                .join(",")
        });
    FunctionTarget { name, params }
}

fn function_matches_target(fi: &FunctionInfo, target: &FunctionTarget) -> bool {
    fi.name == target.name
        && match &target.params {
            Some(params) => format_params(&fi.parameters) == *params,
            None => true,
        }
}

/// Minimal artifact wrapper for extracting the AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

// Call graph engine

/// The core call graph engine shared by inspectors.
///
/// Provides high-level methods to build call trees and find call paths
/// from a project, encapsulating all internal bookkeeping.
pub struct CallGraph {
    project: Project,
    artifact_index: ArtifactIndex,
    symbol_index: SymbolIndex,
    modifier_bodies: RefCell<HashMap<i64, Option<Block>>>,
    inheritance_order: RefCell<Vec<String>>,
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
            modifier_bodies: RefCell::new(HashMap::new()),
            inheritance_order: RefCell::new(Vec::new()),
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
        load_contract_functions(
            &artifact_path,
            &id.artifact_id().name,
            &mut functions,
            &cache,
        )?;
        self.load_inherited_functions(
            &artifact_path,
            id.artifact_id().name.as_str(),
            &mut functions,
            &cache,
        )?;

        let raw_target = id.function_name();
        let target = parse_function_target(raw_target);

        let target_functions: Vec<&FunctionInfo> = functions
            .values()
            .filter(|fi| function_matches_target(fi, &target))
            .collect();

        if target_functions.is_empty() {
            // If there was contract-level ambiguity, emit a suggestion error.
            if let Some(candidates) = ambiguity_candidates {
                bail!(format_ambiguity_error(
                    candidates,
                    id.artifact_id().name.as_str(),
                    raw_target,
                ));
            }
            let contract_name = find_contract_name(&functions, id.artifact_id().name.as_str());
            bail!("\"{raw_target}\" not found in \"{contract_name}\".");
        }

        let target_info = {
            let inheritance_order = self.inheritance_order.borrow();
            select_target_function(&target_functions, &inheritance_order)
                .context("target function list is non-empty")?
        };
        debug!(
            contract = %target_info.contract_name,
            function = %target.name,
            "selected target function declaration"
        );
        let target_id = target_info.id;
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
        load_contract_functions(
            &artifact_path,
            &id.artifact_id().name,
            &mut functions,
            &cache,
        )?;
        self.load_inherited_functions(
            &artifact_path,
            id.artifact_id().name.as_str(),
            &mut functions,
            &cache,
        )?;

        let target = parse_function_target(target_function);
        let target_funcs: Vec<&FunctionInfo> = functions
            .values()
            .filter(|fi| function_matches_target(fi, &target))
            .collect();

        ensure!(
            !target_funcs.is_empty(),
            "\"{}\" not found in \"{}\".",
            target_function,
            id.artifact_id().name
        );

        // Interface declarations with the same name are not overloads of the
        // inherited implementation, so select from concrete declarations first.
        let concrete_funcs: Vec<&FunctionInfo> = target_funcs
            .iter()
            .copied()
            .filter(|fi| !fi.is_interface)
            .collect();
        let own_funcs: Vec<&FunctionInfo> = concrete_funcs
            .iter()
            .copied()
            .filter(|fi| fi.contract_name == id.artifact_id().name)
            .collect();
        let target_func = match own_funcs.len() {
            1 => own_funcs[0],
            0 if concrete_funcs.len() == 1 => concrete_funcs[0],
            0 if concrete_funcs.is_empty() && target_funcs.len() == 1 => target_funcs[0],
            0 if concrete_funcs.len() > 1 => {
                // Multiple concrete declarations with the same name (for
                // example an abstract base declaration and its override) are
                // not overloads: prefer the most-derived declaration in the
                // queried contract's inheritance chain, matching
                // `inspect call-graph` and `inspect function-source`.
                let inheritance_order = self.inheritance_order.borrow();
                match select_target_function(&concrete_funcs, &inheritance_order) {
                    Some(fi) => fi,
                    None => {
                        bail!(
                            "\"{}\" has multiple overloads in \"{}\"; use the full signature.",
                            target_function,
                            id.artifact_id().name
                        );
                    }
                }
            }
            _ => {
                bail!(
                    "\"{}\" has multiple overloads in \"{}\"; use the full signature.",
                    target_function,
                    id.artifact_id().name
                );
            }
        };

        // 3. Build the scoped target ("Contract::function") so that matching
        //    only finds nodes from the specific contract that defines the
        //    function (which may be a parent of the user-specified artifact).
        let defining_contract = &target_func.contract_name;
        let scoped_target = match &target.params {
            Some(params) => format!("{}::{}({})", defining_contract, target.name, params),
            None => format!("{}::{}", defining_contract, target.name),
        };
        debug!(
            contract = %defining_contract,
            function = %target.name,
            "selected call-path target"
        );

        // 4. Determine the project's src directory to filter which artifacts
        //    to search. We only look for call paths from source files within
        //    the configured src directory (e.g., "src" or "contracts"), not
        //    from test or library files.
        let project_root = self.project.path();
        let test_dir = self
            .project
            .directories()
            .ok()
            .map(|d| d.test)
            .unwrap_or_else(|| PathBuf::from("test"));

        // 5. Collect source files, one per source file to avoid
        //    ID collisions across compilation units. Every compiled file is
        //    a candidate root except test files: project contracts in
        //    library or other directories (listed by `inspect contracts`
        //    and `inspect abstracts`) are valid path roots even when they
        //    live outside the configured `src` directory.
        let mut source_files: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut seen_sources: HashSet<PathBuf> = HashSet::new();
        for path in self.artifact_index.all_entries() {
            let Some(source) = extract_artifact_source(path) else {
                continue;
            };
            if !seen_sources.insert(source.clone()) {
                continue;
            }
            if is_under_dir(&source, project_root, &test_dir) {
                continue;
            }
            source_files.push((path.clone(), source)); // checkrs: allow(clone_in_loops)
        }

        let mut matching_roots = Vec::new();
        let mut seen_paths: HashSet<Vec<(String, String, String)>> = HashSet::new();
        let mut target_file = target_func.file.clone();
        let mut target_src = format!("{}:{}", target_func.src_offset, target_func.src_length);

        for (root_artifact_path, _source) in &source_files {
            let Some(ast) = parse_artifact_ast(root_artifact_path)? else {
                continue;
            };
            // Expand each concrete/abstract contract with its own inheritance
            // chain. Virtual hooks such as `_afterTokenTransfer` must dispatch
            // to that contract's override, not the queried target's chain.
            // Querying a library (`Checkpoints.push`) would otherwise miss
            // every path that only exists through a token override.
            for contract_name in dispatch_contract_names(&ast) {
                let cache: RefCell<HashMap<PathBuf, Vec<FunctionInfo>>> =
                    RefCell::new(HashMap::new());
                let mut functions: HashMap<i64, FunctionInfo> = HashMap::new();
                load_contract_functions(
                    root_artifact_path,
                    &contract_name,
                    &mut functions,
                    &cache,
                )?;
                self.load_inherited_functions(
                    root_artifact_path,
                    &contract_name,
                    &mut functions,
                    &cache,
                )?;

                let external_ids: Vec<i64> = functions
                    .iter()
                    .filter(|(_, fi)| {
                        matches!(fi.visibility, Visibility::External | Visibility::Public)
                    })
                    .map(|(id, _)| *id)
                    .collect();

                debug!(
                    file = ?root_artifact_path,
                    contract = %contract_name,
                    roots = ?external_ids,
                    "call-path: expanding path roots"
                );

                for &func_id in &external_ids {
                    let mut visited: HashSet<i64> = HashSet::new();
                    let root =
                        self.build_call_node(func_id, &cache, &mut functions, &mut visited)?;
                    if root.reaches_target(&scoped_target) {
                        let path_key = root.path_key_to_target(&scoped_target);
                        if seen_paths.insert(path_key) {
                            matching_roots.push(root);
                        }
                    }
                }
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

    /// Load functions from all transitive base contracts of `contract_name`.
    ///
    /// The queried contract's artifact AST only contains its own declarations,
    /// so inherited functions must be loaded from the base contracts' artifacts.
    fn load_inherited_functions(
        &self,
        artifact_path: impl AsRef<Path>,
        contract_name: &str,
        functions: &mut HashMap<i64, FunctionInfo>,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
    ) -> Result<()> {
        let mut visited = HashSet::new();
        let mut inheritance_order = vec![contract_name.to_string()];
        self.load_inherited_functions_recursive(
            artifact_path,
            contract_name,
            &mut visited,
            functions,
            cache,
            &mut inheritance_order,
        )
        .map(|()| {
            *self.inheritance_order.borrow_mut() = inheritance_order;
        })
    }

    fn load_inherited_functions_recursive(
        &self,
        artifact_path: impl AsRef<Path>,
        contract_name: &str,
        visited: &mut HashSet<String>,
        functions: &mut HashMap<i64, FunctionInfo>,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        inheritance_order: &mut Vec<String>,
    ) -> Result<()> {
        if !visited.insert(contract_name.to_string()) {
            return Ok(());
        }
        let Some(ast) = parse_artifact_ast(artifact_path)? else {
            return Ok(());
        };
        let build_info_id = self
            .symbol_index
            .build_info_for(&ast.absolute_path)
            .unwrap_or("");
        for node in &ast.nodes {
            if let SourceUnitNode::ContractDefinition(cd) = node
                && cd.name == contract_name
            {
                for base in &cd.base_contracts {
                    if let Some(id) = base.base_name.referenced_declaration
                        && let Some(entry) = self.symbol_index.get(build_info_id, id)
                        && entry.node_type == "ContractDefinition"
                    {
                        let info = self.symbol_index.artifact_info(entry.artifact_id);
                        if info.build_info_id == build_info_id {
                            inheritance_order.push(entry.name.clone()); // checkrs: allow(clone_in_loops)
                            load_contract_functions(
                                &info.artifact_path,
                                &entry.name,
                                functions,
                                cache,
                            )?;
                            self.load_inherited_functions_recursive(
                                &info.artifact_path,
                                &entry.name,
                                visited,
                                functions,
                                cache,
                                inheritance_order,
                            )?;
                        }
                    }
                }
                break;
            }
        }
        Ok(())
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
                let direct = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                if direct.exists() {
                    ResolvedPath::Single(direct)
                } else if let Some(path) = self.artifact_index.find_by_source_path(file, &id.name) {
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
            let src = format!("{}:{}", info.src_offset, info.src_length);
            return Ok(CallGraphNode::new(
                &sig,
                &info.contract_name,
                info.file.clone(),
                &vis,
                &src,
                vec![],
            ));
        }

        let children = self.collect_function_calls(func_id, cache, functions, visited)?;

        let info = &functions[&func_id];
        let sig = build_signature(info);
        let vis = visibility_str(&info.visibility);
        let src = format!("{}:{}", info.src_offset, info.src_length);

        Ok(CallGraphNode::new(
            &sig,
            &info.contract_name,
            info.file.clone(),
            &vis,
            &src,
            children,
        ))
    }

    fn collect_function_calls(
        &self,
        func_id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<Vec<CallGraphNode>> {
        let mut nodes = Vec::new();
        let (modifiers, body_stmts) = match functions.get(&func_id) {
            Some(info) => (
                info.modifiers.clone(),
                info.body.as_ref().map(|b| b.statements.clone()), // checkrs: allow(clone_in_iterator)
            ),
            None => (Vec::new(), None),
        };
        for modifier in &modifiers {
            self.collect_calls_from_modifier_invocation(
                modifier, cache, functions, visited, &mut nodes,
            )?;
        }
        if let Some(stmts) = body_stmts {
            nodes.extend(self.collect_calls(stmts, cache, functions, visited)?);
        }
        Ok(nodes)
    }

    fn collect_calls_from_modifier_invocation(
        &self,
        modifier: &ModifierInvocation,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
    ) -> Result<()> {
        if self.is_base_constructor_specifier(modifier) {
            debug!(
                name = %modifier.modifier_name.name,
                kind = ?modifier.kind,
                "call-graph: following base constructor specifier"
            );
            if let Some(constructor_id) = self.load_base_constructor_id(modifier, functions)? {
                let node = self.build_call_node(constructor_id, cache, functions, visited)?;
                nodes.push(node);
            }
            return Ok(());
        }

        if let Some(body) = self.load_modifier_body(modifier)? {
            nodes.extend(self.collect_calls(body.statements.clone(), cache, functions, visited)?);
        }
        Ok(())
    }

    /// solc >= 0.8 sets `kind`. Older compilers omit it, so a specifier is
    /// inferred when the name points at a `ContractDefinition`.
    fn is_base_constructor_specifier(&self, modifier: &ModifierInvocation) -> bool {
        match modifier.kind {
            Some(ModifierInvocationKind::BaseConstructorSpecifier) => true,
            Some(_) => false,
            None => self.modifier_name_refers_to_contract(modifier),
        }
    }

    fn modifier_name_refers_to_contract(&self, modifier: &ModifierInvocation) -> bool {
        let Some(declaration_id) = modifier.modifier_name.referenced_declaration else {
            return false;
        };
        let Some(entry) = self.symbol_index.get_unscoped(declaration_id) else {
            return false;
        };
        entry.node_type == "ContractDefinition"
    }

    fn load_base_constructor_id(
        &self,
        modifier: &ModifierInvocation,
        functions: &mut HashMap<i64, FunctionInfo>,
    ) -> Result<Option<i64>> {
        if let Some(id) = functions.values().find(|fi| {
            fi.kind == FunctionKind::Constructor && fi.contract_name == modifier.modifier_name.name
        }) {
            return Ok(Some(id.id));
        }

        let Some(base_id) = modifier.modifier_name.referenced_declaration else {
            return Ok(None);
        };
        let Some(entry) = self.symbol_index.get_unscoped(base_id) else {
            return Ok(None);
        };
        let artifact_path = &self
            .symbol_index
            .artifact_info(entry.artifact_id)
            .artifact_path;
        let cache = RefCell::new(HashMap::new());
        load_artifact_functions(artifact_path, functions, &cache)?;
        Ok(functions
            .values()
            .find(|fi| {
                fi.kind == FunctionKind::Constructor
                    && fi.contract_name == modifier.modifier_name.name
            })
            .map(|fi| fi.id))
    }

    /// Resolve the constructor invoked by a `new <Contract>(...)` expression.
    fn load_new_contract_constructor(
        &self,
        type_name: &UserDefinedTypeName,
        functions: &mut HashMap<i64, FunctionInfo>,
    ) -> Result<Option<i64>> {
        let Some(contract_id) = type_name.referenced_declaration else {
            return Ok(None);
        };
        let contract_name: Option<&str> = type_name
            .path_node
            .as_ref()
            .map(|p| p.name.as_str())
            .or(type_name.name.as_deref());
        if let Some(name) = contract_name
            && let Some(id) = functions
                .values()
                .find(|fi| fi.kind == FunctionKind::Constructor && fi.contract_name == name)
        {
            return Ok(Some(id.id));
        }
        let Some(entry) = self.symbol_index.get_unscoped(contract_id) else {
            return Ok(None);
        };
        let artifact_path = &self
            .symbol_index
            .artifact_info(entry.artifact_id)
            .artifact_path;
        let cache = RefCell::new(HashMap::new());
        load_artifact_functions(artifact_path, functions, &cache)?;
        Ok(functions
            .values()
            .find(|fi| fi.kind == FunctionKind::Constructor && fi.contract_name == entry.name)
            .map(|fi| fi.id))
    }

    fn load_modifier_body(&self, modifier: &ModifierInvocation) -> Result<Option<Block>> {
        let Some(modifier_id) = modifier.modifier_name.referenced_declaration else {
            return Ok(None);
        };
        if let Some(cached) = self.modifier_bodies.borrow().get(&modifier_id) {
            return Ok(cached.clone());
        }

        let body = self.resolve_modifier_body(modifier_id)?;
        self.modifier_bodies
            .borrow_mut()
            .insert(modifier_id, body.clone());
        Ok(body)
    }

    fn resolve_modifier_body(&self, modifier_id: i64) -> Result<Option<Block>> {
        let Some(entry) = self.symbol_index.get_unscoped(modifier_id) else {
            return Ok(None);
        };
        let artifact_path = &self
            .symbol_index
            .artifact_info(entry.artifact_id)
            .artifact_path;
        let Some(ast) = parse_artifact_ast(artifact_path)? else {
            return Ok(None);
        };
        Ok(find_modifier_body(&ast, modifier_id))
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
            solc::ast::Statement::EmitStatement(es) => {
                debug!(
                    id = es.id,
                    args = es.event_call.arguments.len(),
                    "call-graph: walking EmitStatement"
                );
                for arg in &es.event_call.arguments {
                    self.collect_calls_from_expression(arg, cache, functions, visited, nodes)?;
                }
            }
            solc::ast::Statement::TryStatement(ts) => {
                debug!(
                    id = ts.id,
                    clauses = ts.clauses.len(),
                    "call-graph: walking TryStatement"
                );
                self.collect_calls_from_expression(
                    &ts.external_call,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                for clause in &ts.clauses {
                    for stmt in &clause.block.statements {
                        self.collect_calls_from_statement(stmt, cache, functions, visited, nodes)?;
                    }
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
                    FunctionCallExpression::MemberAccess(ma) => {
                        match ma.referenced_declaration {
                            Some(id) => {
                                let redirect = !self.is_direct_base_member_access(ma);
                                self.push_loaded_function(
                                    id, cache, functions, visited, nodes, redirect,
                                )?;
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
                        }
                        self.collect_calls_from_expression(
                            &ma.expression,
                            cache,
                            functions,
                            visited,
                            nodes,
                        )?;
                    }
                    FunctionCallExpression::Identifier(id) => {
                        id.referenced_declaration.map_or(Ok(()), |id| {
                            self.push_loaded_function(id, cache, functions, visited, nodes, true)
                        })?;
                    }
                    FunctionCallExpression::FunctionCallOptions(fco) => {
                        if let Expression::MemberAccess(ma) = &*fco.expression
                            && ma.referenced_declaration.is_none()
                            && is_low_level_call(&ma.member_name)
                        {
                            let sig = format!("(address).{}()", ma.member_name);
                            nodes.push(CallGraphNode::new(
                                &sig,
                                "",
                                PathBuf::new(),
                                "low-level",
                                "",
                                vec![],
                            ));
                        } else {
                            let redirect = match &*fco.expression {
                                Expression::MemberAccess(ma) => {
                                    !self.is_direct_base_member_access(ma)
                                }
                                _ => true,
                            };
                            resolve_called_function_id_from_fc_expression(&fco.expression).map_or(
                                Ok(()),
                                |id| {
                                    self.push_loaded_function(
                                        id, cache, functions, visited, nodes, redirect,
                                    )
                                },
                            )?;
                        }
                        self.collect_calls_from_expression(
                            &fco.expression,
                            cache,
                            functions,
                            visited,
                            nodes,
                        )?;
                    }
                    FunctionCallExpression::NewExpression(ne) => {
                        if let TypeName::UserDefinedTypeName(udtn) = &ne.type_name
                            && let Some(id) = self.load_new_contract_constructor(udtn, functions)?
                        {
                            let node = self.build_call_node(id, cache, functions, visited)?;
                            nodes.push(node);
                        }
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
            Expression::IndexAccess(index) => {
                self.collect_calls_from_expression(
                    &index.base_expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                if let Some(ref idx) = index.index_expression {
                    self.collect_calls_from_expression(idx, cache, functions, visited, nodes)?;
                }
            }
            Expression::IndexRangeAccess(index) => {
                self.collect_calls_from_expression(
                    &index.base_expression,
                    cache,
                    functions,
                    visited,
                    nodes,
                )?;
                if let Some(ref start) = index.start_expression {
                    self.collect_calls_from_expression(start, cache, functions, visited, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether a member access is a statically-dispatched call on a base
    /// contract: `super.member(...)` or `BaseContract.member(...)`. Such calls
    /// must keep the referenced declaration and must not be redirected to a
    /// derived override.
    fn is_direct_base_member_access(&self, ma: &solc::ast::MemberAccess) -> bool {
        let Expression::Identifier(id) = &*ma.expression else {
            return false;
        };
        if id.name == "super" {
            return true;
        }
        let Some(ref_id) = id.referenced_declaration else {
            return false;
        };
        matches!(
            self.symbol_index.get_unscoped(ref_id),
            Some(entry) if entry.node_type == "ContractDefinition"
        )
    }

    fn push_loaded_function(
        &self,
        id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
        visited: &mut HashSet<i64>,
        nodes: &mut Vec<CallGraphNode>,
        redirect_override: bool,
    ) -> Result<()> {
        if !functions.contains_key(&id) {
            self.ensure_function_loaded(id, cache, functions)?;
        }

        if functions.contains_key(&id) {
            let effective_id = if redirect_override {
                self.resolve_effective_function_id(id, functions)?
            } else {
                id
            };
            let node = self.build_call_node(effective_id, cache, functions, visited)?;
            nodes.push(node);
        }
        Ok(())
    }

    /// Resolve a virtual call to the most-derived override in the queried
    /// contract's inheritance chain. Non-virtual calls keep their original id.
    fn resolve_effective_function_id(
        &self,
        id: i64,
        functions: &HashMap<i64, FunctionInfo>,
    ) -> Result<i64> {
        let Some(info) = functions.get(&id) else {
            return Ok(id);
        };
        if info.kind != FunctionKind::Function {
            return Ok(id);
        }
        let inheritance_order = self.inheritance_order.borrow();
        if inheritance_order.is_empty() {
            return Ok(id);
        }
        // Only redirect calls to functions declared by the queried contract's
        // inheritance chain. A same-named function from an unrelated contract
        // (for example an internal `helper()` in another source file) is not
        // virtual and must not be redirected to the chain's declaration.
        if !inheritance_order
            .iter()
            .any(|name| name == &info.contract_name)
        {
            return Ok(id);
        }
        let signature = format_params(&info.parameters);
        let rank = |candidate: &FunctionInfo| {
            let position = inheritance_order
                .iter()
                .position(|name| name == &candidate.contract_name)
                .unwrap_or(usize::MAX);
            (
                // Implemented declarations win over abstract interface
                // declarations, matching the compiler's own resolution (solc
                // resolves an inherited call to the most-derived declaration
                // with a body, even when an interface base is listed earlier
                // in the inheritance order).
                !candidate.body.is_some(),
                position,
                candidate.is_interface,
                candidate.src_offset,
                candidate.id,
            )
        };
        let best = functions
            .values()
            .filter(|candidate| {
                candidate.kind == FunctionKind::Function
                    && candidate.name == info.name
                    && format_params(&candidate.parameters) == signature
                    && inheritance_order
                        .iter()
                        .any(|name| name == &candidate.contract_name)
            })
            .min_by_key(|candidate| rank(candidate));
        if let Some(best) = &best {
            debug!(
                id,
                name = %info.name,
                redirected = best.id,
                "call-path: resolved virtual call to most-derived override"
            );
        }
        Ok(best.map_or(id, |best| best.id))
    }

    fn ensure_function_loaded(
        &self,
        id: i64,
        cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
        functions: &mut HashMap<i64, FunctionInfo>,
    ) -> Result<()> {
        let Some(entry) = self.symbol_index.get_unscoped(id) else {
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

/// Select the function declaration that should represent a requested name.
///
/// The queried contract's own declaration takes precedence over inherited
/// declarations with the same name.
fn select_target_function<'a>(
    candidates: &[&'a FunctionInfo],
    inheritance_order: &[String],
) -> Option<&'a FunctionInfo> {
    candidates.iter().copied().min_by_key(|fi| {
        let position = inheritance_order
            .iter()
            .position(|name| name == &fi.contract_name)
            .unwrap_or(usize::MAX);
        // Implemented declarations win over abstract interface declarations,
        // matching the compiler's own resolution (solc resolves an inherited
        // call to the most-derived declaration with a body, even when an
        // interface base is listed earlier in the inheritance order).
        (
            !fi.body.is_some(),
            position,
            fi.is_interface,
            fi.src_offset,
            fi.id,
        )
    })
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

/// Load only the named contract's own functions from an artifact.
///
/// Flattened artifacts contain multiple contracts in one file; loading all of
/// them makes unrelated interface declarations appear as target candidates.
fn load_contract_functions(
    path: impl AsRef<Path>,
    contract_name: &str,
    functions: &mut HashMap<i64, FunctionInfo>,
    cache: &RefCell<HashMap<PathBuf, Vec<FunctionInfo>>>,
) -> Result<()> {
    let path = path.as_ref();
    {
        let cache_ref = cache.borrow();
        if let Some(cached) = cache_ref.get(path) {
            for fi in cached {
                if fi.contract_name == contract_name {
                    functions.insert(fi.id, fi.clone()); // checkrs: allow(clone_in_loops)
                }
            }
            return Ok(());
        }
    }

    let funcs = parse_artifact_functions(path)?;

    if !funcs.is_empty() {
        cache.borrow_mut().insert(path.to_path_buf(), funcs.clone());
        for fi in funcs {
            if fi.contract_name == contract_name {
                functions.insert(fi.id, fi);
            }
        }
    }
    Ok(())
}

/// Contract names in `ast` that can be a runtime dispatch type.
///
/// Interfaces and libraries are skipped: they are not inherited as token-style
/// override contexts. Abstract contracts are included because they still
/// participate in virtual dispatch.
fn dispatch_contract_names(ast: &SourceUnit) -> Vec<String> {
    let mut names = Vec::new();
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.contract_kind == ContractKind::Contract
        {
            names.push(cd.name.clone()); // checkrs: allow(clone_in_loops)
        }
    }
    names
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

/// Check if a source file path is under a given project directory.
fn is_under_dir(
    source: impl AsRef<Path>,
    project_root: impl AsRef<Path>,
    dir: impl AsRef<Path>,
) -> bool {
    // The source path from the artifact is an absolute path. Compute the
    // relative path from the project root and check if it starts with the
    // given directory (e.g., "src", "contracts", or "test").
    let relative = source
        .as_ref()
        .strip_prefix(project_root.as_ref())
        .unwrap_or(source.as_ref());
    relative.starts_with(dir.as_ref())
}

fn parse_artifact_functions(path: impl AsRef<Path>) -> Result<Vec<FunctionInfo>> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse artifact `{}`", path.display()))?;

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

fn parse_artifact_ast(path: impl AsRef<Path>) -> Result<Option<SourceUnit>> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse artifact `{}`", path.display()))?;
    Ok(artifact.ast)
}

fn extract_contract_functions(cd: ContractDefinition, source_file: &Path) -> Vec<FunctionInfo> {
    let contract_name = cd.name;
    let file = source_file.to_path_buf();
    let is_interface = cd.contract_kind == ContractKind::Interface;
    cd.nodes
        .into_iter()
        .filter_map(|inner| match inner {
            ContractDefinitionNode::FunctionDefinition(fd) => Some(FunctionInfo {
                id: fd.id,
                name: function_name_for_display(&fd.kind, &fd.name).to_string(),
                contract_name: contract_name.clone(),
                file: file.clone(),
                is_interface,
                parameters: fd.parameters.parameters.clone(),
                visibility: fd.visibility.clone(),
                kind: fd.kind.clone(),
                src_offset: fd.src.offset,
                src_length: fd.src.length,
                body: fd.body,
                modifiers: fd.modifiers,
            }),
            ContractDefinitionNode::VariableDeclaration(vd)
                if vd.visibility == Visibility::Public =>
            {
                Some(FunctionInfo {
                    id: vd.id,
                    name: vd.name.clone(),
                    contract_name: contract_name.clone(),
                    file: file.clone(),
                    is_interface,
                    parameters: Vec::new(),
                    visibility: Visibility::Public,
                    kind: FunctionKind::Function,
                    src_offset: vd.src.offset,
                    src_length: vd.src.length,
                    body: None,
                    modifiers: Vec::new(),
                })
            }
            _ => None,
        })
        .collect()
}

fn find_modifier_body(ast: &SourceUnit, modifier_id: i64) -> Option<Block> {
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node {
            for inner in &cd.nodes {
                if let ContractDefinitionNode::ModifierDefinition(md) = inner
                    && md.id == modifier_id
                {
                    return Some(md.body.clone()); // checkrs: allow(clone_in_loops)
                }
            }
        }
    }
    None
}

fn function_name_for_display<'a>(kind: &FunctionKind, name: &'a str) -> &'a str {
    match kind {
        FunctionKind::Constructor => "constructor",
        FunctionKind::Receive => "receive",
        FunctionKind::Fallback => "fallback",
        _ => name,
    }
}

fn build_signature(info: &FunctionInfo) -> String {
    let name = function_name_for_display(&info.kind, &info.name);
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
            let base = format_type_name(&arr.base_type);
            match array_length(arr) {
                Some(length) => format!("{base}[{length}]"),
                None => format!("{base}[]"),
            }
        }
        TypeName::UserDefinedTypeName(udtn) => format_user_defined_type(udtn),
        TypeName::Mapping(_) => "mapping".into(),
        TypeName::FunctionTypeName(ftn) => {
            let params = format_params(&ftn.parameter_types.parameters);
            debug!(params = %params, "formatted FunctionTypeName parameter");
            format!("function({params})")
        }
    }
}

fn format_user_defined_type(udtn: &UserDefinedTypeName) -> String {
    if let Some(ref path) = udtn.path_node {
        return path.name.clone();
    }
    debug!(
        type_string = ?udtn.type_descriptions.type_string,
        "user-defined type missing pathNode; falling back to typeString"
    );
    short_user_defined_name(udtn.type_descriptions.type_string.as_deref())
        .unwrap_or_else(|| "unknown".into())
}

fn short_user_defined_name(type_string: Option<&str>) -> Option<String> {
    let raw = type_string?.trim();
    let stripped = raw
        .strip_prefix("struct ")
        .or_else(|| raw.strip_prefix("enum "))
        .or_else(|| raw.strip_prefix("contract "))
        .or_else(|| raw.strip_prefix("interface "))
        .or_else(|| raw.strip_prefix("library "))
        .unwrap_or(raw);
    let token = stripped.split_whitespace().next().unwrap_or(stripped);
    let token = token.split('[').next().unwrap_or(token);
    Some(token.rsplit('.').next().unwrap_or(token).to_string())
}

fn array_length(arr: &solc::ast::ArrayTypeName) -> Option<String> {
    let expr = arr.length.as_ref()?;
    match expr.as_ref() {
        Expression::Literal(lit) => lit.value.clone(),
        _ => None,
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
        "call" | "delegatecall" | "staticcall" | "callcode" | "transfer" | "send"
    )
}

/// Format an ambiguity error message for call-graph resolution.
fn format_ambiguity_error(
    candidates: &[PathBuf],
    contract_name: &str,
    function_name: &str,
) -> String {
    let mut sorted = candidates.to_vec();
    sorted.sort();

    let mut msg = format!(
        "found {} \"{}\"\n\nSelect one of the following:\n",
        sorted.len(),
        contract_name
    );
    for candidate in &sorted {
        let qualified = ArtifactIndex::qualified_name(candidate, contract_name);
        msg.push_str(&format!(
            "\nsolray inspect call-graph {qualified} {}",
            function_name
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

    #[test]
    fn select_target_function_prefers_own_contract() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/inspect-call-graph/out/Override.sol");
        let iface = contract_functions(&root.join("IOverride.json"), "IOverride");
        let own = contract_functions(&root.join("Override.json"), "Override");

        let selected =
            select_target_function(&[&iface[0], &own[0]], &["Override".to_string()]).unwrap();
        assert_eq!(selected.contract_name, "Override");
    }

    fn contract_functions(path: impl AsRef<Path>, contract_name: &str) -> Vec<FunctionInfo> {
        let ast = parse_artifact_ast(path).unwrap().unwrap();
        let source_file = PathBuf::from("src/Override.sol");
        let mut out = Vec::new();
        for node in ast.nodes {
            if let SourceUnitNode::ContractDefinition(cd) = node {
                if cd.name == contract_name {
                    out.extend(extract_contract_functions(cd, &source_file));
                }
            }
        }
        out
    }
}
