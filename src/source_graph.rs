//! Complete source code resolution for Solidity functions.
//!
//! [`SourceResolver`] resolves a function by its `Contract::function` ID and emits
//! the full source code of the function together with every declaration it references,
//! recursively. Natspec comments are preserved for each declaration.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use solc::ast::{
    ContractDefinitionNode, Expression, FunctionCallExpression, SourceUnit, SourceUnitNode,
    TypeName,
};

use crate::artifact_index::ArtifactIndex;
use crate::build_info::BuildInfo;
use crate::call_graph::FunctionID;
use crate::project::Project;
use crate::symbol_index::SymbolIndex;

/// A resolved declaration with its source code and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ResolvedSymbol {
    /// Human-readable signature, e.g. `execute(uint256)` or `Data`
    symbol: String,
    /// The source file path
    file: PathBuf,
    /// Byte offset in source
    offset: usize,
    /// Byte length of the definition
    length: usize,
}

/// Minimal artifact wrapper for extracting the full AST.
#[derive(Deserialize)]
struct Artifact {
    ast: Option<SourceUnit>,
}

/// Context passed through reference-collection to resolve IDs against the AST.
struct RefCtx<'a> {
    ast: &'a SourceUnit,
    source_file: &'a Path,
    current_fn_id: Option<i64>,
    symbol_index: &'a SymbolIndex,
    build_info_id: &'a str,
}

/// Resolves the complete source code for a function and all symbols it references.
pub struct SourceResolver {
    project: Project,
    artifact_index: ArtifactIndex,
    symbol_index: SymbolIndex,
}

impl SourceResolver {
    /// Build a [`SourceResolver`] for the given project.
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

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Resolve a `Contract::function` ID and return the formatted source output.
    pub fn resolve(&self, function_id: &str) -> Result<String> {
        let fid = FunctionID::try_from(function_id)?;

        // Detect contract-level ambiguity.
        let artifact_paths = self.artifact_index.try_get(fid.contract_name())?;

        if artifact_paths.len() > 1 {
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
                msg.push_str(&format!("\nhawk inspect sources {}:{}", rp, fid));
            }
            msg.push('\n');
            bail!(msg);
        }

        // Find the target function and its source location.
        let root_symbol = self.find_function(&fid, &artifact_paths)?;

        // Recursively resolve all referenced declarations.
        let resolved = self.resolve_recursive(root_symbol)?;

        // Format output.
        self.format_output(&resolved)
    }

    /// Find a function across artifacts and return its ResolvedSymbol.
    fn find_function(
        &self,
        fid: &FunctionID,
        artifact_paths: &[PathBuf],
    ) -> Result<ResolvedSymbol> {
        let fn_name = fid.function_name();
        let (base_name, is_exact) = if let Some(pos) = fn_name.find('(') {
            (&fn_name[..pos], true)
        } else {
            (fn_name, false)
        };

        let mut functions: HashMap<String, ResolvedSymbol> = HashMap::new();
        for artifact_path in artifact_paths {
            let parsed = parse_artifact(artifact_path)?;
            if let Some(ast) = parsed {
                extract_function_symbols(&ast, fid.contract_name(), base_name, &mut functions);
            }
        }

        if functions.is_empty() {
            let mut all_fns: Vec<String> = Vec::new();
            for artifact_path in artifact_paths {
                let parsed = parse_artifact(artifact_path)?;
                if let Some(ast) = parsed {
                    collect_contract_functions(&ast, fid.contract_name(), &mut all_fns);
                }
            }
            all_fns.sort();
            all_fns.dedup();
            bail!(
                "\"{}\" not found in \"{}\".\n\nAvailable functions in \"{}\": {}",
                fid.function_name(),
                fid.contract_name(),
                fid.contract_name(),
                all_fns.join(", ")
            );
        }

        if is_exact {
            // Filter to exactly matching signatures
            let target_sig = format!("{}::{}", fid.contract_name(), fn_name);
            let matched: Vec<&ResolvedSymbol> = functions
                .values()
                .filter(|s| s.symbol == target_sig)
                .collect();
            if matched.is_empty() {
                let mut msg = format!(
                    "\"{}\" not found in \"{}\".\n\nSelect one of the following:\n",
                    fid.function_name(),
                    fid.contract_name()
                );
                let mut sorted: Vec<&String> = functions.values().map(|s| &s.symbol).collect();
                sorted.sort();
                for sym in sorted {
                    msg.push_str(&format!("\nhawk inspect sources \"{}\"", sym));
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
                fid
            );
            let mut sorted: Vec<&String> = functions.values().map(|s| &s.symbol).collect();
            sorted.sort();
            for sym in sorted {
                msg.push_str(&format!("\nhawk inspect sources \"{}\"", sym));
            }
            msg.push('\n');
            bail!(msg);
        }

        // We've already checked is_empty() and len() > 1, so exactly one entry remains.
        let result = functions
            .into_values()
            .next()
            .context("internal error: function list is empty")?;
        Ok(result)
    }

    /// Recursively resolve all referenced declarations.
    // checkrs: allow(clone_in_loops)
    fn resolve_recursive(&self, root: ResolvedSymbol) -> Result<Vec<ResolvedSymbol>> {
        let mut resolved: Vec<ResolvedSymbol> = Vec::new();
        let mut seen: HashSet<(PathBuf, usize)> = HashSet::new();
        let mut queue: Vec<ResolvedSymbol> = vec![root];

        // Cache for parsed artifacts: artifact_path -> SourceUnit
        let mut artifact_cache: HashMap<PathBuf, SourceUnit> = HashMap::new();

        while let Some(symbol) = queue.pop() {
            let file_key = (symbol.file.clone(), symbol.offset); // checkrs: allow(clone_in_loops)
            if !seen.insert(file_key) {
                continue;
            }

            // Find the artifact for this source file, parse it, and collect refs.
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

    // checkrs: allow(clone_in_loops)
    fn format_output(&self, symbols: &[ResolvedSymbol]) -> Result<String> {
        let cwd = std::env::current_dir()?;
        let project_abs = std::path::absolute(self.project.path())?;
        let mut output = String::new();
        let mut file_contents: HashMap<PathBuf, String> = HashMap::new();

        for symbol in symbols {
            let full_path = project_abs.join(&symbol.file);
            let rel_path = full_path.strip_prefix(&cwd).unwrap_or(&full_path);

            let content = if let Some(c) = file_contents.get(&symbol.file) {
                c.clone() // checkrs: allow(clone_in_loops)
            } else {
                let Ok(c) = fs::read_to_string(&full_path) else {
                    output.push_str(&format!(
                        "// symbol: {}\n// path: {} (unable to read)\n\n",
                        symbol.symbol,
                        rel_path.display()
                    ));
                    continue;
                };
                file_contents.insert(symbol.file.clone(), c.clone()); // checkrs: allow(clone_in_loops)
                c
            };

            let line_offsets = build_line_offsets(&content);
            let start_line = byte_offset_to_line(symbol.offset, &line_offsets);
            let end_line = byte_offset_to_line(
                symbol
                    .offset
                    .saturating_add(symbol.length)
                    .saturating_sub(1),
                &line_offsets,
            );

            let natspec = extract_natspec(&content, symbol.offset);
            let source_text = &content[symbol.offset..symbol.offset + symbol.length];

            // Compute the base indentation: the whitespace preceding the symbol
            // in the source file. Strip it so the output is flush-left.
            let base = base_indent(&content, symbol.offset);
            let natspec = dedent(&natspec, base);
            let source_text = dedent(source_text, base);

            output.push_str(&format!("// symbol: {}\n", symbol.symbol));
            output.push_str(&format!(
                "// path: {}#L{}-L{}\n\n",
                rel_path.display(),
                start_line,
                end_line
            ));

            if !natspec.is_empty() {
                output.push_str(&natspec);
            }

            output.push_str(&source_text);
            output.push_str("\n\n");
        }

        Ok(output)
    }
}

/// Parse an artifact JSON file and return its AST.
fn parse_artifact(path: impl AsRef<Path>) -> Result<Option<SourceUnit>> {
    let content = fs::read_to_string(path.as_ref())?;
    let artifact: Artifact = serde_json::from_str(&content)?;
    Ok(artifact.ast)
}

/// Find the artifact path that corresponds to a source file.
// checkrs: allow(clone_in_loops)
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

/// Extract function symbols from an AST for a given contract/function name.
// checkrs: allow(clone_in_loops)
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
                    && fd.name == function_name
                    && fd.implemented
                {
                    let sig = format!(
                        "{}::{}({})",
                        contract_name,
                        fd.name,
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
// checkrs: allow(clone_in_loops)
fn collect_contract_functions(ast: &SourceUnit, contract_name: &str, out: &mut Vec<String>) {
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.name == contract_name
        {
            for inner in &cd.nodes {
                if let ContractDefinitionNode::FunctionDefinition(fd) = inner
                    && fd.implemented
                {
                    out.push(fd.name.clone()); // checkrs: allow(clone_in_loops)
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
    if let ContractDefinitionNode::FunctionDefinition(fd) = node
        && let Some(ref body) = fd.body
    {
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
            collect_from_statements(&body.statements, seen_ids, results, &fn_ctx);
        }
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
    // Skip self-reference
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
    // Cross-file reference: look up the declaration in the symbol index,
    // scoped to the same build-info to avoid ID collisions across
    // incremental compilation units.
    let Some(entry) = ctx.symbol_index.get(id) else {
        return;
    };
    let info = ctx.symbol_index.artifact_info(entry.artifact_id);
    if info.build_info_id == ctx.build_info_id && info.source_file != *ctx.source_file {
        results.push(ResolvedSymbol {
            symbol: entry.name.clone(),
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
                "{}::{}({})",
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
                symbol: format!("{}::{}", contract_name, vd.name),
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

/// Build a vector where `line_offsets[n]` is the byte offset of the start of line `n`
/// (1-indexed: `line_offsets[1]` is the offset of line 1).
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

    // Trim trailing blank lines (between natspec and the declaration).
    while let Some(last) = lines.last()
        && last.trim().is_empty()
    {
        lines.pop();
    }

    // Trim leading blank lines (before the first natspec line).
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

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/sources")
    }

    fn run(project_path: impl AsRef<Path>, function_id: &str) -> Result<String> {
        let project = Project::open(project_path);
        project.validate()?;
        let resolver = SourceResolver::new(project);
        resolver.resolve(function_id)
    }

    #[test]
    fn resolves_execute_with_recursive_refs() {
        let result = run(fixture_path(), "Main::execute").unwrap();
        assert_eq!(
            result,
            include_str!("../fixtures/sources/expected/run_shows_source_for_execute.txt")
        );
    }

    #[test]
    fn errors_for_unknown_contract() {
        let result = run(fixture_path(), "Unknown::function");
        let err = result.unwrap_err().to_string();
        assert_eq!(err, "\"Unknown\" not found.");
    }

    #[test]
    fn errors_for_unknown_function() {
        let result = run(fixture_path(), "Main::unknownFunction");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "\"unknownFunction\" not found in \"Main\".\n\nAvailable functions in \"Main\": _compute, _processData, execute"
        );
    }

    #[test]
    fn errors_for_overloaded_function() {
        let result = run(fixture_path(), "Overloaded::beforeTokenTransfer");
        let err = result.unwrap_err().to_string();
        assert_eq!(
            err,
            "found 2 \"Overloaded::beforeTokenTransfer\"\n\nSelect one of the following:\n\nhawk inspect sources \"Overloaded::beforeTokenTransfer(address,address)\"\nhawk inspect sources \"Overloaded::beforeTokenTransfer(address,address,uint256)\"\n"
        );
    }

    #[test]
    fn build_line_offsets_works() {
        let content = "line1\nline2\nline3\n";
        let offsets = build_line_offsets(content);
        assert_eq!(offsets, vec![0, 0, 6, 12, 18]);
    }

    #[test]
    fn byte_offset_to_line_finds_correct_line() {
        let content = "line1\nline2\nline3\n";
        let offsets = build_line_offsets(content);
        assert_eq!(byte_offset_to_line(0, &offsets), 1);
        assert_eq!(byte_offset_to_line(3, &offsets), 1);
        assert_eq!(byte_offset_to_line(6, &offsets), 2);
    }

    #[test]
    fn extract_natspec_gets_doc_comments() {
        let content =
            "/// @notice Does something.\n/// @param x The input.\nfunction foo(uint256 x) {}";
        // offset of 'function' = len of "/// @notice Does something.\n/// @param x The input.\n"
        let fn_offset = 52;
        let natspec = extract_natspec(content, fn_offset);
        assert_eq!(
            natspec,
            "/// @notice Does something.\n/// @param x The input.\n"
        );
    }

    #[test]
    fn base_indent_finds_leading_whitespace() {
        let content = "    function foo() {}\n";
        assert_eq!(base_indent(content, 4), 4);
    }

    #[test]
    fn base_indent_zero_for_no_whitespace() {
        let content = "function foo() {}\n";
        assert_eq!(base_indent(content, 0), 0);
    }

    #[test]
    fn dedent_strips_leading_spaces() {
        let text = "    /// @notice hi\n    /// @param x\n\nfunction foo() {\n    bar();\n}\n";
        let result = dedent(text, 4);
        assert_eq!(
            result,
            "/// @notice hi\n/// @param x\n\nfunction foo() {\nbar();\n}\n"
        );
    }

    #[test]
    fn dedent_preserves_text_with_no_indent() {
        let text = "function foo() {}\n";
        assert_eq!(dedent(text, 0), text);
    }
}
