//! Foundry project inspection.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rayon::prelude::*;
use serde::Deserialize;
use solc::ast::{
    ContractDefinition, ContractDefinitionNode, ContractKind, Expression, FunctionCallExpression,
    SourceUnit, SourceUnitNode, TypeName, Visibility,
};
use solc::ast::{FunctionDefinition, StateMutability, VariableDeclaration};
use walkdir::WalkDir;

use crate::call_graph::CallGraphNode;
use crate::inheritance::InheritanceNode;

/// A single Solidity source-level declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub kind: DeclarationKind,
    pub file: PathBuf,
}

/// The kind of a top-level Solidity declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Contract,
    AbstractContract,
    Interface,
    Library,
}

/// Internal: contract info extracted from an artifact AST for inheritance resolution.
#[derive(Debug, Clone)]
struct ContractInfo {
    name: String,
    file: PathBuf,
    base_contracts: Vec<String>,
}

/// Internal: function info extracted from an artifact AST for call graph resolution.
#[derive(Debug, Clone)]
#[expect(dead_code)]
struct FunctionInfo {
    id: i64,
    name: String,
    contract_name: String,
    contract_kind: ContractKind,
    file: PathBuf,
    parameters: Vec<VariableDeclaration>,
    visibility: Visibility,
    state_mutability: StateMutability,
    definition: FunctionDefinition,
}

/// A Foundry project opened for inspection.
#[derive(Debug)]
pub struct Project {
    path: PathBuf,
    out: PathBuf,
}

/// Minimal artifact wrapper for extracting the AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

impl Project {
    /// Create a [`Project`] handle for the Foundry project at `path`.
    ///
    /// This simply records the project path and the expected `out/` directory.
    /// Call [`validate`](Self::validate) to check that the project is properly
    /// configured (e.g. `foundry.toml` exists and `ast = true` is set).
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let out = path.join("out");
        Project { path, out }
    }

    /// Validate that the project at [`self.path`](Self::path) is a properly
    /// configured Foundry project.
    ///
    /// Checks that `foundry.toml` exists and that `ast = true` is set in the
    /// default profile. This ensures artifacts can be inspected.
    pub fn validate(&self) -> Result<()> {
        let foundry_toml = self.path.join("foundry.toml");

        ensure!(
            foundry_toml.exists(),
            "not a Foundry project: {} not found",
            foundry_toml.display()
        );

        let config: toml::Value = toml::from_str(&fs::read_to_string(&foundry_toml)?)?;

        let ast = config
            .get("profile")
            .and_then(|p| p.get("default"))
            .and_then(|d| d.get("ast"))
            .and_then(|a| a.as_bool());

        ensure!(
            ast == Some(true),
            "`ast = true` must be set in the [profile.default] section of {}",
            foundry_toml.display()
        );

        Ok(())
    }

    /// Return the project root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the output directory path.
    pub fn out_dir(&self) -> &Path {
        &self.out
    }

    /// Collect all JSON artifact paths from the output directory,
    /// excluding `build-info` files.
    fn artifact_paths(&self) -> Vec<PathBuf> {
        WalkDir::new(&self.out)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let p = e.path();
                p.extension().and_then(|s| s.to_str()) == Some("json")
                    && !p.to_string_lossy().contains("build-info")
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    }

    /// Return all declarations found across all artifacts.
    pub fn declarations(&self) -> Result<Vec<Declaration>> {
        if !self.out.exists() {
            return Ok(Vec::new());
        }

        let paths = self.artifact_paths();
        let mut results: Vec<Declaration> = paths
            .into_par_iter()
            .filter_map(|path| process_artifact(path).ok().flatten())
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(results)
    }

    /// Return only deployable (non-abstract, non-interface, non-library) contracts.
    pub fn deployable_contracts(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::Contract)
                .collect()
        })
    }

    /// Return only abstract contracts.
    pub fn abstract_contracts(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::AbstractContract)
                .collect()
        })
    }

    /// Return only libraries.
    pub fn libraries(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::Library)
                .collect()
        })
    }

    /// Return only interfaces.
    pub fn interfaces(&self) -> Result<Vec<Declaration>> {
        self.declarations().map(|decls| {
            decls
                .into_iter()
                .filter(|d| d.kind == DeclarationKind::Interface)
                .collect()
        })
    }

    /// Find a declaration by exact name match (case-sensitive).
    pub fn find_declaration(&self, name: &str) -> Result<Option<Declaration>> {
        let decls = self.declarations()?;
        Ok(decls.into_iter().find(|d| d.name == name))
    }

    /// Find all declarations matching `name` exactly (case-sensitive).
    ///
    /// This is useful for detecting name collisions where the same declaration
    /// name appears in multiple files (e.g., a dependency and the project itself).
    pub fn find_declarations_by_name(&self, name: &str) -> Result<Vec<Declaration>> {
        let decls = self.declarations()?;
        Ok(decls.into_iter().filter(|d| d.name == name).collect())
    }

    /// Build the inheritance tree for a contract identified by name.
    ///
    /// Returns an [`InheritanceNode`] where `name` and `file` represent the
    /// root contract and `parents` contains the resolved base contracts
    /// recursively.
    pub fn inheritance_tree(&self, name: &str) -> Result<InheritanceNode> {
        let infos = self.load_contract_infos()?;
        let by_name: HashMap<&str, &ContractInfo> =
            infos.iter().map(|ci| (ci.name.as_str(), ci)).collect();

        let mut visited: HashSet<&str> = HashSet::new();
        build_tree(name, &by_name, &mut visited)
    }

    /// Build the inheritance tree for a contract identified by name and file path.
    ///
    /// Unlike [`inheritance_tree`], this disambiguates which contract to use
    /// as the root when multiple contracts share the same name. Base contracts
    /// are still resolved by name alone.
    pub fn inheritance_tree_by_path(
        &self,
        name: &str,
        file_path: impl AsRef<Path>,
    ) -> Result<InheritanceNode> {
        let file_path = file_path.as_ref();
        let infos = self.load_contract_infos()?;
        let by_name: HashMap<&str, &ContractInfo> =
            infos.iter().map(|ci| (ci.name.as_str(), ci)).collect();

        let root_info = infos
            .iter()
            .find(|ci| ci.name == name && ci.file == file_path)
            .with_context(|| {
                format!("contract `{}` not found in `{}`", name, file_path.display())
            })?;

        let mut visited: HashSet<&str> = HashSet::new();
        visited.insert(&root_info.name);

        let parents: Vec<InheritanceNode> = root_info
            .base_contracts
            .iter()
            .map(|base_name| build_tree(base_name, &by_name, &mut visited))
            .collect::<Result<Vec<InheritanceNode>>>()?;

        Ok(InheritanceNode {
            name: root_info.name.clone(),
            file: root_info.file.clone(),
            parents,
        })
    }

    /// Load contract info (name, file, base_contracts) from all artifacts.
    fn load_contract_infos(&self) -> Result<Vec<ContractInfo>> {
        if !self.out.exists() {
            return Ok(Vec::new());
        }
        let paths = self.artifact_paths();
        let results: Vec<ContractInfo> = paths
            .into_par_iter()
            .filter_map(|path| process_artifact_for_inheritance(path).ok().flatten())
            .collect();
        Ok(results)
    }

    /// Build the call graph for a function identified by `Contract::function`.
    ///
    /// Resolves the contract and function, then recursively traverses the AST
    /// to find all function calls.
    pub fn call_graph(&self, function_id: &str) -> Result<CallGraphNode> {
        let (contract_name, function_name) = parse_function_id(function_id)?;

        let decls = self.find_declarations_by_name(contract_name)?;

        ensure!(!decls.is_empty(), "\"{}\" not found.", contract_name);

        // Build the global function info map.
        let func_infos = self.load_function_infos()?;
        let by_id: HashMap<i64, &FunctionInfo> = func_infos.iter().map(|fi| (fi.id, fi)).collect();

        // Find matching functions in the target contract.
        // Deduplicate: a function may appear in multiple artifacts (e.g. Main.json
        // and Base.json both contain the full AST for src/Main.sol). We keep one
        // per unique (name, file) pair.
        let mut seen = HashSet::new();
        let matched: Vec<&FunctionInfo> = func_infos
            .iter()
            .filter(|fi| fi.contract_name == contract_name && fi.name == function_name)
            .filter(|fi| seen.insert((fi.name.clone(), fi.file.clone())))
            .collect();

        // Handle ambiguity: multiple contracts with same name.
        if decls.len() > 1 {
            let project_root = &self.path;
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                decls.len(),
                contract_name
            );
            for d in &decls {
                let rp = d
                    .file
                    .strip_prefix(project_root)
                    .unwrap_or(&d.file)
                    .to_string_lossy();
                msg.push_str(&format!("\nhawk inspect calls {}:{}", rp, function_id));
            }
            msg.push('\n');
            bail!(msg);
        }

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

        let target = matched.first().with_context(|| {
            format!("\"{}\" not found in \"{}\".", function_name, contract_name)
        })?;

        let mut visited: HashSet<i64> = HashSet::new();
        let node = self.build_call_node(target, &by_id, &mut visited)?;
        Ok(node)
    }

    /// Build a CallGraphNode for a given function info, recursively.
    fn build_call_node(
        &self,
        info: &FunctionInfo,
        by_id: &HashMap<i64, &FunctionInfo>,
        visited: &mut HashSet<i64>,
    ) -> Result<CallGraphNode> {
        if !visited.insert(info.id) {
            // Recursive call detected; return a stub to avoid infinite recursion.
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
                // Resolve the called function from the expression.
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
                // Also check arguments for nested function calls.
                for arg in &fc.arguments {
                    self.collect_calls_from_expression(arg, by_id, visited, nodes)?;
                }
                // Also check options if present.
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

    /// Load all function definitions from all artifacts into a map keyed by AST node ID.
    fn load_function_infos(&self) -> Result<Vec<FunctionInfo>> {
        if !self.out.exists() {
            return Ok(Vec::new());
        }
        let paths = self.artifact_paths();
        let results: Vec<FunctionInfo> = paths
            .into_par_iter()
            .filter_map(|path| process_artifact_for_functions(path).ok().flatten())
            .flatten()
            .collect();
        Ok(results)
    }
}

fn classify_contract(cd: &ContractDefinition) -> DeclarationKind {
    match cd.contract_kind {
        ContractKind::Contract if cd.r#abstract => DeclarationKind::AbstractContract,
        ContractKind::Contract => DeclarationKind::Contract,
        ContractKind::Interface => DeclarationKind::Interface,
        ContractKind::Library => DeclarationKind::Library,
    }
}

/// Extract the base contract names from a list of inheritance specifiers.
fn base_contract_names(base_contracts: Vec<solc::ast::InheritanceSpecifier>) -> Vec<String> {
    base_contracts
        .into_iter()
        .map(|bc| bc.base_name.name)
        .collect()
}

/// Process a single artifact JSON file, returning a [`ContractInfo`] if a
/// contract definition is found.
fn process_artifact_for_inheritance(path: impl AsRef<Path>) -> Result<Option<ContractInfo>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
    };

    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)?;

    let ast = match artifact.ast {
        None => bail!(
            "artifact `{}` is missing the AST; rebuild with `ast = true` in foundry.toml",
            path.display()
        ),
        Some(ast) => ast,
    };

    let source_file = ast.absolute_path;
    if let Some(cd) = ast.nodes.into_iter().find_map(|node| {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.name == contract_name
        {
            return Some(cd);
        }
        None
    }) {
        let bases = base_contract_names(cd.base_contracts);
        Ok(Some(ContractInfo {
            name: cd.name,
            file: source_file,
            base_contracts: bases,
        }))
    } else {
        Ok(None)
    }
}

/// Recursively build an [`InheritanceNode`] tree from a contract name.
fn build_tree<'a>(
    name: &'a str,
    by_name: &HashMap<&str, &'a ContractInfo>,
    visited: &mut HashSet<&'a str>,
) -> Result<InheritanceNode> {
    ensure!(
        visited.insert(name),
        "circular inheritance detected for `{}`",
        name
    );

    let info = by_name
        .get(name)
        .with_context(|| format!("contract `{}` not found in artifacts", name))?;

    let parents: Vec<InheritanceNode> = info
        .base_contracts
        .iter()
        .map(|base_name| build_tree(base_name, by_name, visited))
        .collect::<Result<Vec<InheritanceNode>>>()?;

    Ok(InheritanceNode {
        name: info.name.clone(),
        file: info.file.clone(),
        parents,
    })
}

/// Process a single artifact JSON file, extracting its [`Declaration`] if one
/// is found.
fn process_artifact(path: impl AsRef<Path>) -> Result<Option<Declaration>> {
    let path = path.as_ref();
    let contract_name = match path.file_stem().and_then(|s| s.to_str()) {
        Some(name) => name,
        None => return Ok(None),
    };

    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)?;

    let ast = match artifact.ast {
        None => bail!(
            "artifact `{}` is missing the AST; rebuild with `ast = true` in foundry.toml",
            path.display()
        ),
        Some(ast) => ast,
    };

    let source_file = ast.absolute_path;
    if let Some(cd) = ast.nodes.into_iter().find_map(|node| {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.name == contract_name
        {
            return Some(cd);
        }
        None
    }) {
        let kind = classify_contract(&cd);
        Ok(Some(Declaration {
            name: cd.name,
            kind,
            file: source_file,
        }))
    } else {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Call graph helpers
// ---------------------------------------------------------------------------

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
            // Use the path_node name or fall back to type identifier
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

/// Process a single artifact JSON file, returning all [`FunctionInfo`] entries found
/// across all contracts in the AST.
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
fn extract_contract_functions(cd: ContractDefinition, source_file: &Path) -> Vec<FunctionInfo> {
    let contract_name = cd.name;
    let contract_kind = cd.contract_kind;
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
                contract_kind: contract_kind.clone(),
                file: file.clone(),
                parameters: fd.parameters.parameters.clone(),
                visibility: fd.visibility.clone(),
                state_mutability: fd.state_mutability.clone(),
                definition: fd,
            })
        })
        .collect()
}
