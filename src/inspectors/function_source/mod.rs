//! Function source inspection for Foundry projects.
//!
//! [`FunctionSourceInspector`] resolves the complete source code for a
//! function and all symbols it references, recursively.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use solc::ast::{
    ContractDefinitionNode, Expression, FunctionCallExpression, FunctionKind, SourceUnit,
    SourceUnitNode, TypeName,
};

use crate::artifact_index::ArtifactIndex;
use crate::build_info::BuildInfo;
use crate::inspectors::artifact_id::ArtifactId;
use crate::inspectors::function_source::symbol_index::SymbolIndex;
use crate::project::Project;

pub mod symbol_index;

/// A resolved declaration with its source code and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedSymbol {
    /// Human-readable signature, e.g. `execute(uint256)` or `Data`
    symbol: String,
    /// The source file path
    file: PathBuf,
    /// Byte offset in source
    offset: usize,
    /// Byte length of the definition
    length: usize,
}

/// Context passed through reference-collection to resolve IDs against the AST.
struct RefCtx<'a> {
    ast: &'a SourceUnit,
    source_file: &'a Path,
    current_fn_id: Option<i64>,
    symbol_index: &'a SymbolIndex,
    build_info_id: &'a str,
}

/// The output of a [`FunctionSourceInspector`] inspection.
#[derive(Debug)]
pub struct FunctionSourceInspectorOutput {
    symbols: Vec<ResolvedSymbol>,
    project_path: PathBuf,
    artifact_index: ArtifactIndex,
}

impl FunctionSourceInspectorOutput {
    /// Create a new [`FunctionSourceInspectorOutput`] from resolved symbols.
    pub fn new(
        symbols: Vec<ResolvedSymbol>,
        project_path: impl AsRef<Path>,
        artifact_index: ArtifactIndex,
    ) -> Self {
        Self {
            symbols,
            project_path: project_path.as_ref().to_path_buf(),
            artifact_index,
        }
    }
}

impl std::fmt::Display for FunctionSourceInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_abs =
            std::path::absolute(&self.project_path).unwrap_or(self.project_path.clone());
        let mut file_contents: HashMap<PathBuf, String> = HashMap::new();

        for symbol in &self.symbols {
            let full_path = project_abs.join(&symbol.file);
            let rel_path = full_path.strip_prefix(&cwd).unwrap_or(&full_path);

            let content = if let Some(c) = file_contents.get(&symbol.file) {
                c.clone() // checkrs: allow(clone_in_loops)
            } else {
                let Ok(c) = fs::read_to_string(&full_path) else {
                    writeln!(
                        f,
                        "// {} | {}:? (unable to read)\n",
                        symbol.symbol,
                        rel_path.display()
                    )?;
                    continue;
                };
                file_contents.insert(symbol.file.clone(), c.clone()); // checkrs: allow(clone_in_loops)
                c
            };

            let line_offsets = build_line_offsets(&content);
            let start_line = byte_offset_to_line(symbol.offset, &line_offsets);

            let mut natspec = extract_natspec(&content, symbol.offset);
            let source_text = &content[symbol.offset..symbol.offset + symbol.length];

            let base = base_indent(&content, symbol.offset);
            natspec = resolve_inheritdoc_natspec(
                &natspec,
                &symbol.symbol,
                &self.artifact_index,
                &self.project_path,
            );
            let natspec = dedent(&natspec, base);
            let source_text = dedent(source_text, base);

            write!(
                f,
                "// {} | {}:{}\n\n",
                symbol.symbol,
                rel_path.display(),
                start_line,
            )?;
            if !natspec.is_empty() {
                write!(f, "{}", natspec)?;
            }
            write!(f, "{}\n\n", source_text)?;
        }

        Ok(())
    }
}

/// Inspect the complete source code of a Solidity function.
pub struct FunctionSourceInspector {
    project: Project,
    artifact_index: ArtifactIndex,
    symbol_index: SymbolIndex,
}

impl FunctionSourceInspector {
    /// Build a [`FunctionSourceInspector`] for the given project.
    pub fn inspect_project(project: Project) -> Self {
        let artifact_index = ArtifactIndex::build(project.out_dir());
        let build_infos = BuildInfo::load_all(project.out_dir());
        let symbol_index = SymbolIndex::build(&artifact_index, &build_infos);
        Self {
            project,
            artifact_index,
            symbol_index,
        }
    }

    /// Inspect the source code for the given artifact ID and function name.
    pub fn inspect(
        &self,
        id: &ArtifactId,
        function_name: &str,
    ) -> Result<FunctionSourceInspectorOutput> {
        let artifact_paths = match &id.file {
            Some(file) => {
                let artifact_path = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                ensure!(artifact_path.exists(), "\"{}\" not found.", id.name);
                vec![artifact_path]
            }
            None => {
                let candidates = self
                    .artifact_index
                    .get(&id.name)
                    .cloned()
                    .unwrap_or_default();
                match candidates.len() {
                    0 => {
                        bail!("\"{}\" not found.", id.name);
                    }
                    n if n > 1 => {
                        let mut sorted = candidates;
                        sorted.sort();
                        let mut msg = format!(
                            "found {} \"{}\"\n\nSelect one of the following:\n",
                            n, id.name
                        );
                        for candidate in &sorted {
                            let parent = candidate
                                .parent()
                                .and_then(|p| p.file_name())
                                .and_then(|n| n.to_str())
                                .unwrap_or("");
                            msg.push_str(&format!(
                                "\nhawk inspect function-source {parent}:{id_name} {function_name}",
                                id_name = id.name
                            ));
                        }
                        msg.push('\n');
                        bail!(msg);
                    }
                    _ => candidates,
                }
            }
        };

        let root_symbol = self.find_function(&id.name, function_name, &artifact_paths)?;
        let resolved = self.resolve_recursive(root_symbol)?;

        Ok(FunctionSourceInspectorOutput::new(
            resolved,
            self.project.path(),
            self.artifact_index.clone(),
        ))
    }
}

impl FunctionSourceInspector {
    /// Find a function across artifacts and return its ResolvedSymbol.
    fn find_function(
        &self,
        contract_name: &str,
        function_name: &str,
        artifact_paths: &[PathBuf],
    ) -> Result<ResolvedSymbol> {
        let (base_name, is_exact) = if let Some(pos) = function_name.find('(') {
            (&function_name[..pos], true)
        } else {
            (function_name, false)
        };

        let mut functions: HashMap<String, ResolvedSymbol> = HashMap::new();
        for artifact_path in artifact_paths {
            let parsed = parse_artifact(artifact_path)?;
            if let Some(ast) = parsed {
                extract_function_symbols(&ast, contract_name, base_name, &mut functions);
            }
        }

        if functions.is_empty() {
            let mut all_fns: Vec<String> = Vec::new();
            for artifact_path in artifact_paths {
                let parsed = parse_artifact(artifact_path)?;
                if let Some(ast) = parsed {
                    collect_contract_functions(&ast, contract_name, &mut all_fns);
                }
            }
            all_fns.sort();
            all_fns.dedup();
            bail!(
                "\"{}\" not found in \"{}\".\n\nAvailable functions in \"{}\": {}",
                function_name,
                contract_name,
                contract_name,
                all_fns.join(", ")
            );
        }

        if is_exact {
            let target_sig = format!("{}.{}", contract_name, function_name);
            let matched: Vec<&ResolvedSymbol> = functions
                .values()
                .filter(|s| s.symbol == target_sig)
                .collect();
            if matched.is_empty() {
                let mut msg = format!(
                    "\"{}\" not found in \"{}\".\n\nSelect one of the following:\n",
                    function_name, contract_name
                );
                let mut sorted: Vec<&String> = functions.values().map(|s| &s.symbol).collect();
                sorted.sort();
                for sym in sorted {
                    msg.push_str(&format!(
                        "\nhawk inspect function-source {} {}",
                        contract_name, sym
                    ));
                }
                msg.push('\n');
                bail!(msg);
            }
            return Ok(matched[0].clone());
        }

        if functions.len() > 1 {
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                functions.len(),
                function_name
            );
            let mut sorted: Vec<&String> = functions.values().map(|s| &s.symbol).collect();
            sorted.sort();
            for sym in sorted {
                msg.push_str(&format!(
                    "\nhawk inspect function-source {} {}",
                    contract_name, sym
                ));
            }
            msg.push('\n');
            bail!(msg);
        }

        functions
            .into_values()
            .next()
            .context("internal error: function list is empty")
    }

    /// Recursively resolve all referenced declarations.
    fn resolve_recursive(&self, root: ResolvedSymbol) -> Result<Vec<ResolvedSymbol>> {
        let mut resolved: Vec<ResolvedSymbol> = Vec::new();
        let mut seen: HashSet<(PathBuf, usize)> = HashSet::new();
        let mut queue: Vec<ResolvedSymbol> = vec![root];
        let mut artifact_cache: HashMap<PathBuf, SourceUnit> = HashMap::new();

        while let Some(symbol) = queue.pop() {
            let file_key = (symbol.file.clone(), symbol.offset); // checkrs: allow(clone_in_loops)
            if !seen.insert(file_key) {
                continue;
            }

            let artifact_path =
                find_artifact_for_source(&symbol.file, &self.artifact_index, &self.symbol_index);

            if let Some(ref a_path) = artifact_path
                && !artifact_cache.contains_key(a_path)
                && let Some(ast) = parse_artifact(a_path)?
            {
                artifact_cache.insert(a_path.clone(), ast); // checkrs: allow(clone_in_loops)
            }
            if let Some(ref a_path) = artifact_path
                && let Some(ast) = artifact_cache.get(a_path)
            {
                let build_info_id = self.symbol_index.build_info_for(&symbol.file).unwrap_or("");
                let refs = collect_referenced_declarations(
                    ast,
                    symbol.offset,
                    symbol.length,
                    &symbol.file,
                    &self.symbol_index,
                    build_info_id,
                );
                for rs in refs {
                    let key = (rs.file.clone(), rs.offset); // checkrs: allow(clone_in_loops)
                    let new_symbol = !seen.contains(&key);
                    if new_symbol {
                        queue.push(rs);
                    }
                }
            }

            resolved.push(symbol);
        }

        Ok(resolved)
    }
}

// ===== Free functions for artifact parsing and source resolution =====

/// Parse an artifact JSON file and return its AST.
fn parse_artifact(path: impl AsRef<Path>) -> Result<Option<SourceUnit>> {
    let content = fs::read_to_string(path.as_ref())?;
    let artifact: Artifact = serde_json::from_str(&content)?;
    Ok(artifact.ast)
}

/// Minimal artifact wrapper for extracting the full AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

/// Find the artifact path that corresponds to a source file.
fn find_artifact_for_source(
    source_file: &Path,
    artifact_index: &ArtifactIndex,
    symbol_index: &SymbolIndex,
) -> Option<PathBuf> {
    for entry in symbol_index.values() {
        let info = symbol_index.artifact_info(entry.artifact_id);
        if info.source_file == source_file {
            return Some(info.artifact_path.clone()); // checkrs: allow(clone_in_loops)
        }
    }
    for artifact_paths in artifact_index.values() {
        for artifact_path in artifact_paths {
            if let Some(parent) = artifact_path.parent().and_then(|p| p.file_stem())
                && let Some(stem) = source_file.file_stem()
                && parent == stem
            {
                return Some(artifact_path.clone()); // checkrs: allow(clone_in_loops)
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

/// Extract function symbols from an AST for a given contract/function name.
fn extract_function_symbols(
    ast: &SourceUnit,
    contract_name: &str,
    function_name: &str,
    out: &mut HashMap<String, ResolvedSymbol>,
) {
    let source_file = &ast.absolute_path;
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.name == contract_name
        {
            for inner in &cd.nodes {
                if let ContractDefinitionNode::FunctionDefinition(fd) = inner
                    && fd.implemented
                    && function_name_for_display(&fd.kind, &fd.name) == function_name
                {
                    let display_name = function_name_for_display(&fd.kind, &fd.name);
                    let sig = format!(
                        "{}.{}({})",
                        contract_name,
                        display_name,
                        format_params(&fd.parameters.parameters)
                    );
                    let sig_key = sig.clone(); // checkrs: allow(clone_in_loops)
                    out.entry(sig_key).or_insert(ResolvedSymbol {
                        symbol: sig,
                        file: source_file.clone(), // checkrs: allow(clone_in_loops)
                        offset: fd.src.offset,
                        length: fd.src.length,
                    });
                }
            }
        }
    }
}

/// Collect available function names in a contract.
fn collect_contract_functions(ast: &SourceUnit, contract_name: &str, out: &mut Vec<String>) {
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.name == contract_name
        {
            for inner in &cd.nodes {
                if let ContractDefinitionNode::FunctionDefinition(fd) = inner
                    && fd.implemented
                {
                    out.push(function_name_for_display(&fd.kind, &fd.name).to_string());
                }
            }
        }
    }
}

/// Collect all referenced declarations within a source range of the AST.
fn collect_referenced_declarations(
    ast: &SourceUnit,
    target_offset: usize,
    target_length: usize,
    source_file: &Path,
    symbol_index: &SymbolIndex,
    build_info_id: &str,
) -> Vec<ResolvedSymbol> {
    let end = target_offset + target_length;
    let mut seen_ids: HashSet<i64> = HashSet::new();
    let mut results: Vec<ResolvedSymbol> = Vec::new();

    let ctx = RefCtx {
        ast,
        source_file,
        current_fn_id: None,
        symbol_index,
        build_info_id,
    };

    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node {
            for inner in &cd.nodes {
                collect_from_contract_node(
                    inner,
                    target_offset,
                    end,
                    &mut seen_ids,
                    &mut results,
                    &ctx,
                );
            }
        }
    }

    results
}

fn collect_from_contract_node(
    node: &ContractDefinitionNode,
    range_start: usize,
    range_end: usize,
    seen_ids: &mut HashSet<i64>,
    results: &mut Vec<ResolvedSymbol>,
    ctx: &RefCtx,
) {
    match node {
        ContractDefinitionNode::FunctionDefinition(fd) => {
            let Some(ref body) = fd.body else {
                return;
            };
            let body_start = body.src.offset;
            let body_end = body_start + body.src.length;
            if body_start < range_end && body_end > range_start {
                let fn_ctx = RefCtx {
                    current_fn_id: Some(fd.id),
                    ..*ctx
                };
                for param in &fd.parameters.parameters {
                    collect_from_type_name(&param.type_name, seen_ids, results, &fn_ctx);
                }
                for param in &fd.return_parameters.parameters {
                    collect_from_type_name(&param.type_name, seen_ids, results, &fn_ctx);
                }
                for modifier in &fd.modifiers {
                    if let Some(id) = modifier.modifier_name.referenced_declaration {
                        resolve_and_add_symbol(id, seen_ids, results, &fn_ctx);
                    }
                    if let Some(ref args) = modifier.arguments {
                        for arg in args {
                            collect_from_expression(arg, seen_ids, results, &fn_ctx);
                        }
                    }
                }
                collect_from_statements(&body.statements, seen_ids, results, &fn_ctx);
            }
        }
        ContractDefinitionNode::ModifierDefinition(md) => {
            let body_start = md.body.src.offset;
            let body_end = body_start + md.body.src.length;
            if body_start < range_end && body_end > range_start {
                let md_ctx = RefCtx {
                    current_fn_id: Some(md.id),
                    ..*ctx
                };
                for param in &md.parameters.parameters {
                    collect_from_type_name(&param.type_name, seen_ids, results, &md_ctx);
                }
                collect_from_statements(&md.body.statements, seen_ids, results, &md_ctx);
            }
        }
        _ => {}
    }
}

fn collect_from_statements(
    stmts: &[solc::ast::Statement],
    seen_ids: &mut HashSet<i64>,
    results: &mut Vec<ResolvedSymbol>,
    ctx: &RefCtx,
) {
    for stmt in stmts {
        collect_from_statement(stmt, seen_ids, results, ctx);
    }
}

fn collect_from_statement(
    stmt: &solc::ast::Statement,
    seen_ids: &mut HashSet<i64>,
    results: &mut Vec<ResolvedSymbol>,
    ctx: &RefCtx,
) {
    match stmt {
        solc::ast::Statement::ExpressionStatement(es) => {
            collect_from_expression(&es.expression, seen_ids, results, ctx);
        }
        solc::ast::Statement::Block(block) => {
            collect_from_statements(&block.statements, seen_ids, results, ctx);
        }
        solc::ast::Statement::IfStatement(ifs) => {
            collect_from_expression(&ifs.condition, seen_ids, results, ctx);
            collect_from_statement(&ifs.true_body, seen_ids, results, ctx);
            if let Some(ref false_body) = ifs.false_body {
                collect_from_statement(false_body, seen_ids, results, ctx);
            }
        }
        solc::ast::Statement::ForStatement(fors) => {
            if let Some(ref init) = fors.initialization_expression {
                collect_from_expression(init, seen_ids, results, ctx);
            }
            collect_from_expression(&fors.condition, seen_ids, results, ctx);
            if let Some(ref loop_expr) = fors.loop_expression {
                collect_from_expression(loop_expr, seen_ids, results, ctx);
            }
            collect_from_statement(&fors.body, seen_ids, results, ctx);
        }
        solc::ast::Statement::WhileStatement(whiles) => {
            collect_from_expression(&whiles.condition, seen_ids, results, ctx);
            collect_from_statement(&whiles.body, seen_ids, results, ctx);
        }
        solc::ast::Statement::DoWhileStatement(dw) => {
            collect_from_statement(&dw.body, seen_ids, results, ctx);
            collect_from_expression(&dw.condition, seen_ids, results, ctx);
        }
        solc::ast::Statement::Return(ret) => {
            if let Some(ref expr) = ret.expression {
                collect_from_expression(expr, seen_ids, results, ctx);
            }
        }
        solc::ast::Statement::VariableDeclarationStatement(vds) => {
            if let Some(ref expr) = vds.initial_value {
                collect_from_expression(expr, seen_ids, results, ctx);
            }
            for decl in vds.declarations.iter().flatten() {
                collect_from_type_name(&decl.type_name, seen_ids, results, ctx);
            }
        }
        _ => {}
    }
}

fn collect_from_expression(
    expr: &Expression,
    seen_ids: &mut HashSet<i64>,
    results: &mut Vec<ResolvedSymbol>,
    ctx: &RefCtx,
) {
    match expr {
        Expression::FunctionCall(fc) => {
            let called_id = match &*fc.expression {
                FunctionCallExpression::MemberAccess(ma) => ma.referenced_declaration,
                FunctionCallExpression::Identifier(id) => id.referenced_declaration,
                FunctionCallExpression::FunctionCallOptions(fco) => {
                    resolve_called_id_from_expr(&fco.expression)
                }
                _ => None,
            };
            if let Some(id) = called_id {
                resolve_and_add_symbol(id, seen_ids, results, ctx);
            }
            for arg in &fc.arguments {
                collect_from_expression(arg, seen_ids, results, ctx);
            }
            if let FunctionCallExpression::FunctionCallOptions(fco) = &*fc.expression {
                for opt in &fco.options {
                    collect_from_expression(opt, seen_ids, results, ctx);
                }
            }
        }
        Expression::Assignment(assign) => {
            collect_from_expression(&assign.right_hand_side, seen_ids, results, ctx);
            collect_from_expression(&assign.left_hand_side, seen_ids, results, ctx);
        }
        Expression::MemberAccess(ma) => {
            if let Some(id) = ma.referenced_declaration {
                resolve_and_add_symbol(id, seen_ids, results, ctx);
            }
            collect_from_expression(&ma.expression, seen_ids, results, ctx);
        }
        Expression::Identifier(id) => {
            if let Some(ref_id) = id.referenced_declaration {
                resolve_and_add_symbol(ref_id, seen_ids, results, ctx);
            }
        }
        Expression::BinaryOperation(binop) => {
            collect_from_expression(&binop.left_expression, seen_ids, results, ctx);
            collect_from_expression(&binop.right_expression, seen_ids, results, ctx);
        }
        Expression::UnaryOperation(unop) => {
            collect_from_expression(&unop.sub_expression, seen_ids, results, ctx);
        }
        Expression::Conditional(cond) => {
            collect_from_expression(&cond.condition, seen_ids, results, ctx);
            collect_from_expression(&cond.true_expression, seen_ids, results, ctx);
            collect_from_expression(&cond.false_expression, seen_ids, results, ctx);
        }
        Expression::TupleExpression(tuple) => {
            for comp in tuple.components.iter().flatten() {
                collect_from_expression(comp, seen_ids, results, ctx);
            }
        }
        Expression::IndexAccess(ia) => {
            collect_from_expression(&ia.base_expression, seen_ids, results, ctx);
            if let Some(ref idx) = ia.index_expression {
                collect_from_expression(idx, seen_ids, results, ctx);
            }
        }
        Expression::IndexRangeAccess(ira) => {
            collect_from_expression(&ira.base_expression, seen_ids, results, ctx);
            if let Some(ref start) = ira.start_expression {
                collect_from_expression(start, seen_ids, results, ctx);
            }
        }
        _ => {}
    }
}

fn collect_from_type_name(
    type_name: &TypeName,
    seen_ids: &mut HashSet<i64>,
    results: &mut Vec<ResolvedSymbol>,
    ctx: &RefCtx,
) {
    match type_name {
        TypeName::UserDefinedTypeName(udtn) => {
            if let Some(id) = udtn.referenced_declaration {
                resolve_and_add_symbol(id, seen_ids, results, ctx);
            }
        }
        TypeName::ArrayTypeName(atn) => {
            collect_from_type_name(&atn.base_type, seen_ids, results, ctx);
        }
        TypeName::Mapping(m) => {
            collect_from_type_name(&m.key_type, seen_ids, results, ctx);
            collect_from_type_name(&m.value_type, seen_ids, results, ctx);
        }
        _ => {}
    }
}

fn resolve_and_add_symbol(
    id: i64,
    seen_ids: &mut HashSet<i64>,
    results: &mut Vec<ResolvedSymbol>,
    ctx: &RefCtx,
) {
    if ctx.current_fn_id == Some(id) {
        return;
    }
    if !seen_ids.insert(id) {
        return;
    }
    if let Some(rs) = resolve_id_in_ast(id, ctx.ast, ctx.source_file) {
        results.push(rs);
        return;
    }
    let Some(entry) = ctx.symbol_index.get(id) else {
        return;
    };
    let info = ctx.symbol_index.artifact_info(entry.artifact_id);
    if info.build_info_id == ctx.build_info_id && info.source_file != *ctx.source_file {
        let symbol = match entry.node_type.as_str() {
            "FunctionDefinition" | "VariableDeclaration" => {
                format!("{}.{}", entry.contract_name, entry.name)
            }
            _ => entry.name.clone(),
        };
        results.push(ResolvedSymbol {
            symbol,
            file: info.source_file.clone(),
            offset: entry.offset,
            length: entry.length,
        });
    }
}

fn resolve_id_in_ast(id: i64, ast: &SourceUnit, source_file: &Path) -> Option<ResolvedSymbol> {
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node {
            for inner in &cd.nodes {
                if let Some(rs) = node_to_symbol(inner, id, &cd.name, source_file) {
                    return Some(rs);
                }
            }
        }
    }
    None
}

/// Convert a contract member node to a ResolvedSymbol if its ID matches.
fn node_to_symbol(
    node: &ContractDefinitionNode,
    target_id: i64,
    contract_name: &str,
    source_file: &Path,
) -> Option<ResolvedSymbol> {
    match node {
        ContractDefinitionNode::FunctionDefinition(fd) if fd.id == target_id => {
            let sig = format!(
                "{}.{}({})",
                contract_name,
                fd.name,
                format_params(&fd.parameters.parameters)
            );
            Some(ResolvedSymbol {
                symbol: sig,
                file: source_file.to_path_buf(),
                offset: fd.src.offset,
                length: fd.src.length,
            })
        }
        ContractDefinitionNode::VariableDeclaration(vd) if vd.id == target_id => {
            Some(ResolvedSymbol {
                symbol: format!("{}.{}", contract_name, vd.name),
                file: source_file.to_path_buf(),
                offset: vd.src.offset,
                length: vd.src.length,
            })
        }
        ContractDefinitionNode::StructDefinition(sd) if sd.id == target_id => {
            Some(ResolvedSymbol {
                symbol: sd.name.clone(),
                file: source_file.to_path_buf(),
                offset: sd.src.offset,
                length: sd.src.length,
            })
        }
        ContractDefinitionNode::EnumDefinition(ed) if ed.id == target_id => Some(ResolvedSymbol {
            symbol: ed.name.clone(),
            file: source_file.to_path_buf(),
            offset: ed.src.offset,
            length: ed.src.length,
        }),
        ContractDefinitionNode::ErrorDefinition(ed) if ed.id == target_id => Some(ResolvedSymbol {
            symbol: ed.name.clone(),
            file: source_file.to_path_buf(),
            offset: ed.src.offset,
            length: ed.src.length,
        }),
        ContractDefinitionNode::EventDefinition(ed) if ed.id == target_id => Some(ResolvedSymbol {
            symbol: ed.name.clone(),
            file: source_file.to_path_buf(),
            offset: ed.src.offset,
            length: ed.src.length,
        }),
        ContractDefinitionNode::ModifierDefinition(md) if md.id == target_id => {
            Some(ResolvedSymbol {
                symbol: md.name.clone(),
                file: source_file.to_path_buf(),
                offset: md.src.offset,
                length: md.src.length,
            })
        }
        ContractDefinitionNode::UserDefinedValueTypeDefinition(udvtd) if udvtd.id == target_id => {
            Some(ResolvedSymbol {
                symbol: udvtd.name.clone(),
                file: source_file.to_path_buf(),
                offset: udvtd.src.offset,
                length: udvtd.src.length,
            })
        }
        _ => None,
    }
}

/// Compute the leading whitespace count on the line containing `offset`.
fn base_indent(content: &str, offset: usize) -> usize {
    let line_start = content[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    content[line_start..offset]
        .chars()
        .take_while(|c| c.is_whitespace())
        .count()
}

/// Strip up to `base` spaces of leading whitespace from every non-empty line.
fn dedent(text: &str, base: usize) -> String {
    if base == 0 {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed_start = line
            .chars()
            .take_while(|c| c.is_whitespace())
            .count()
            .min(base);
        result.push_str(&line[trimmed_start..]);
        result.push('\n');
    }
    result
}

/// Build a vector where `line_offsets[n]` is the byte offset of the start of line `n`.
fn build_line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0, 0];
    for (i, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Given a byte offset and a line-offsets vector, return the 1-indexed line number.
fn byte_offset_to_line(offset: usize, line_offsets: &[usize]) -> usize {
    match line_offsets.binary_search(&offset) {
        Ok(line) => line.max(1),
        Err(line) => line.saturating_sub(1).max(1),
    }
}

/// Extract natspec comments preceding a given byte offset in source content.
fn extract_natspec(content: &str, offset: usize) -> String {
    let prefix = if offset > content.len() {
        content
    } else {
        &content[..offset]
    };

    let mut lines: Vec<&str> = Vec::new();

    for line in prefix.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("///") {
            lines.push(line);
        } else if trimmed.starts_with("/*") || trimmed.starts_with('*') {
            lines.push(line);
            if trimmed.starts_with("/*") {
                break;
            }
        } else if trimmed.is_empty() {
            lines.push(line);
        } else {
            break;
        }
    }

    if lines.is_empty() {
        return String::new();
    }

    lines.reverse();

    while let Some(last) = lines.last()
        && last.trim().is_empty()
    {
        lines.pop();
    }

    while let Some(first) = lines.first()
        && first.trim().is_empty()
    {
        lines.remove(0);
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    for line in lines {
        result.push_str(line);
        result.push('\n');
    }
    result
}

/// Resolve `@inheritdoc ContractName` by looking up the referenced contract's natspec.
///
/// If the natspec contains an `@inheritdoc` directive, this function tries to
/// find the matching function in the referenced contract (interface) and returns
/// *its* natspec instead. Returns the original natspec if resolution fails.
fn resolve_inheritdoc_natspec(
    natspec: &str,
    symbol: &str,
    artifact_index: &ArtifactIndex,
    project_path: impl AsRef<Path>,
) -> String {
    // Find the @inheritdoc line
    let inheritdoc_line = match natspec
        .lines()
        .find(|l| l.trim().starts_with("/// @inheritdoc"))
    {
        Some(line) => line.trim(),
        None => return natspec.to_string(),
    };

    // Parse the contract name from "/// @inheritdoc ContractName"
    let rest = match inheritdoc_line.strip_prefix("/// @inheritdoc") {
        Some(r) => r.trim(),
        None => return natspec.to_string(),
    };

    // The @inheritdoc may reference a parent path like "IMetricOmmPool.IMetricOmmPoolActions"
    // We only care about the contract name itself (last segment after any dot)
    let interface_name = rest.rsplit('.').next_back().unwrap_or(rest);

    // Extract function name from symbol (e.g., "Main.execute(uint256)" -> "execute")
    let func_name = match symbol.split('.').nth(1) {
        Some(part) => part.split('(').next().unwrap_or(""),
        None => return natspec.to_string(),
    };

    if func_name.is_empty() {
        return natspec.to_string();
    }

    // Look up the interface contract in the artifact index
    let artifact_paths = match artifact_index.get(interface_name) {
        Some(paths) => paths,
        None => return natspec.to_string(),
    };

    // Walk each artifact to find the interface contract and matching function
    for artifact_path in artifact_paths {
        let Some(ast) = (|| -> Option<SourceUnit> {
            let content = fs::read_to_string(artifact_path).ok()?;
            let artifact: Artifact = serde_json::from_str(&content).ok()?;
            artifact.ast
        })() else {
            continue;
        };

        for node in &ast.nodes {
            let SourceUnitNode::ContractDefinition(cd) = node else {
                continue;
            };
            if cd.name != interface_name {
                continue;
            }

            for inner in &cd.nodes {
                let ContractDefinitionNode::FunctionDefinition(fd) = inner else {
                    continue;
                };
                if fd.name != func_name {
                    continue;
                }

                // Found the matching function -- extract natspec from its source
                let source_file = &ast.absolute_path;
                let full_path = project_path.as_ref().join(source_file);
                let Ok(content) = fs::read_to_string(&full_path) else {
                    return natspec.to_string();
                };
                let resolved = extract_natspec(&content, fd.src.offset);
                if resolved.is_empty() {
                    return natspec.to_string();
                }
                let base = base_indent(&content, fd.src.offset);
                return dedent(&resolved, base);
            }
        }
    }

    // Resolution failed -- return the original natspec unchanged
    natspec.to_string()
}

/// Format parameter declarations into a comma-separated type list.
fn format_params(params: &[solc::ast::VariableDeclaration]) -> String {
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

/// Extract the referenced declaration ID from an expression inside a function call.
fn resolve_called_id_from_expr(expr: &Expression) -> Option<i64> {
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

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/function-source")
    }

    fn inspect(contract: &str, function_name: &str) -> Result<FunctionSourceInspectorOutput> {
        let project = Project::open(fixture_path());
        project.validate()?;
        let inspector = FunctionSourceInspector::inspect_project(project);
        let id = ArtifactId::new(contract);
        inspector.inspect(&id, function_name)
    }

    #[test]
    fn inspect_shows_source_for_execute() {
        let output = inspect("Main", "execute").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_shows_source_for_execute.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_constructor() {
        let output = inspect("SpecialFunctions", "constructor").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_shows_source_for_constructor.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_receive() {
        let output = inspect("SpecialFunctions", "receive").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_shows_source_for_receive.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_fallback() {
        let output = inspect("SpecialFunctions", "fallback").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_shows_source_for_fallback.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_with_recursive_refs() {
        let output = inspect("Main", "_processData").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_shows_source_with_recursive_refs.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_overloaded_with_params() {
        let output = inspect("Overloaded", "beforeTokenTransfer(address,address,uint256)").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_shows_source_for_overloaded_with_params.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let err = inspect("Unknown", "function").unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/function-source/expected/run_errors_for_unknown_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_unknown_function() {
        let err = inspect("Main", "unknownFunction").unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/function-source/expected/run_errors_for_unknown_function.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_overloaded_function() {
        let err = inspect("Overloaded", "beforeTokenTransfer")
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/function-source/expected/run_errors_for_overloaded_function.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_natspec_block_comment() {
        let output = inspect("NatspecBlock", "compute").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_shows_natspec_block_comment.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_user_defined_types_in_variable_declarations() {
        let output = inspect("TypeRefs", "passThrough").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_resolves_user_defined_types_in_variable_declarations.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_cross_file_type_references() {
        let output = inspect("CrossFileConsumer", "translate").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_resolves_cross_file_type_references.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_index_access_expressions() {
        let output = inspect("IndexAccessTest", "getItem").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_resolves_index_access_expressions.txt"
            )
        );
    }

    #[test]
    fn incremental_build_does_not_leak_symbols() {
        let output = inspect("Main", "execute").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/incremental_build_does_not_leak_symbols.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_function_return_types() {
        let output = inspect("ReturnTypeRef", "makeWidget").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_resolves_function_return_types.txt"
            )
        );
    }

    #[test]
    fn inspect_extracts_regular_block_comments() {
        let output = inspect("BlockComment", "getItem").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_extracts_regular_block_comments.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_path_qualified_contract() {
        let output = inspect("Main.sol:Main", "execute").unwrap();
        assert!(output.to_string().contains("// Main.execute(uint256) |"));
    }

    #[test]
    fn inspect_resolves_modifiers() {
        let output = inspect("ModifierRef", "increment").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/function-source/expected/run_resolves_modifiers.txt")
        );
    }

    #[test]
    fn inspect_resolves_inheritdoc() {
        let output = inspect("InheritdocUser", "doSomething").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/function-source/expected/run_resolves_inheritdoc.txt")
        );
    }

    #[test]
    fn inspect_resolves_cross_file_function_references() {
        let output = inspect("CrossFileFnUser", "process").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/function-source/expected/run_resolves_cross_file_function_references.txt"
            )
        );
    }
}
