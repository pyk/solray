//! Asset transfer scanner.
//!
//! [`AssetTransferScanner`] inspects a Foundry project's AST and reports all
//! call sites where assets are transferred, including ERC20 transfers, ETH
//! transfers, and low-level calls with value.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use solc::ast::{
    Block, ContractDefinitionNode, Expression, FunctionCall, FunctionCallExpression, FunctionKind,
    SourceUnit, SourceUnitNode, StateMutability, Statement, Visibility,
};
use walkdir::WalkDir;

use crate::build_info::BuildInfo;
use crate::project::Project;

/// The kind of asset transfer detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetTransferKind {
    /// ERC20 `transfer`
    Erc20Transfer,
    /// ERC20 `safeTransfer`
    Erc20SafeTransfer,
    /// ERC20 `transferFrom`
    Erc20TransferFrom,
    /// ERC20 `safeTransferFrom`
    Erc20SafeTransferFrom,
    /// ETH `send`
    EthSend,
    /// ETH low-level `call{value}`
    EthCall,
    /// ETH `transfer`
    EthTransfer,
    /// ETH `receive` function
    EthReceive,
}

impl std::fmt::Display for AssetTransferKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetTransferKind::Erc20Transfer => write!(f, "Transfer ERC20"),
            AssetTransferKind::Erc20SafeTransfer => write!(f, "Safe Transfer ERC20"),
            AssetTransferKind::Erc20TransferFrom => write!(f, "Transfer From ERC20"),
            AssetTransferKind::Erc20SafeTransferFrom => write!(f, "Safe Transfer From ERC20"),
            AssetTransferKind::EthSend => write!(f, "Send ETH"),
            AssetTransferKind::EthCall => write!(f, "Call ETH"),
            AssetTransferKind::EthTransfer => write!(f, "Transfer ETH"),
            AssetTransferKind::EthReceive => write!(f, "Receive ETH"),
        }
    }
}

/// A single asset transfer call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetTransfer {
    /// The contract or library name containing the function that makes the
    /// transfer.
    pub contract_name: String,
    /// The function name containing the transfer call.
    pub function_name: String,
    /// The visibility of the function.
    pub visibility: String,
    /// The source expression (function signature or call expression).
    pub expression: String,
    /// The kind of asset transfer.
    pub kind: AssetTransferKind,
    /// The source file path (relative to the project root).
    pub file: PathBuf,
    /// The 1-based line number in the source file.
    pub line: usize,
}

/// The output of an [`AssetTransferScanner`] scan.
pub struct AssetTransferScannerOutput {
    transfers: Vec<AssetTransfer>,
    _project_root: PathBuf,
}

impl AssetTransferScannerOutput {
    pub fn new(transfers: Vec<AssetTransfer>, project_root: PathBuf) -> Self {
        Self {
            transfers,
            _project_root: project_root,
        }
    }
}

impl std::fmt::Display for AssetTransferScannerOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sorted = self.transfers.clone();
        sorted.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.contract_name.cmp(&b.contract_name))
        });

        let summary = if sorted.len() == 1 {
            "1 asset transfer found.".to_string()
        } else {
            format!("{} asset transfers found.", sorted.len())
        };

        writeln!(f, "## Asset Transfers")?;
        writeln!(f)?;
        writeln!(f, "{summary}")?;
        writeln!(f)?;

        if sorted.is_empty() {
            return Ok(());
        }

        writeln!(
            f,
            "| # | Contract | Function | Visibility | Expression | Kind | Source Location |"
        )?;
        writeln!(
            f,
            "| - | -------- | -------- | ---------- | ---------- | ---- | --------------- |"
        )?;

        for (i, transfer) in sorted.iter().enumerate() {
            let location = format!("{}:{}", transfer.file.display(), transfer.line);
            let expr = transfer.expression.replace('|', "\\|");
            writeln!(
                f,
                "| {} | `{}` | `{}` | {} | `{}` | {} | `{}` |",
                i + 1,
                transfer.contract_name,
                transfer.function_name,
                transfer.visibility,
                expr,
                transfer.kind,
                location,
            )?;
        }

        Ok(())
    }
}

/// Scan a Foundry project for asset transfer calls.
pub struct AssetTransferScanner {
    project: Project,
}

impl AssetTransferScanner {
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Scan the project and return all asset transfer calls.
    pub fn scan(&self) -> Result<AssetTransferScannerOutput> {
        self.project.validate()?;
        let project_root = std::path::absolute(self.project.path())?;
        let directories = self.project.directories()?;
        let src_dir = directories.src;
        let artifact_paths = self.artifact_paths();
        let build_infos = BuildInfo::load_all(self.project.out_dir());

        let mut transfers = Vec::new();
        let mut visited_src: HashSet<(usize, usize)> = HashSet::new();
        for path in &artifact_paths {
            if let Some(mut found) =
                scan_artifact(path, &project_root, &build_infos, &mut visited_src)?
            {
                transfers.append(&mut found);
            }
        }

        transfers.retain(|t| t.file.starts_with(&src_dir));

        Ok(AssetTransferScannerOutput::new(transfers, project_root))
    }

    fn artifact_paths(&self) -> Vec<PathBuf> {
        WalkDir::new(self.project.out_dir())
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
}

/// Scan a single artifact for asset transfer calls.
fn scan_artifact(
    artifact_path: impl AsRef<Path>,
    project_root: &Path,
    build_infos: &[BuildInfo],
    visited_src: &mut HashSet<(usize, usize)>,
) -> Result<Option<Vec<AssetTransfer>>> {
    let artifact_path = artifact_path.as_ref();
    let content = fs::read_to_string(artifact_path)?;
    let artifact: Artifact = serde_json::from_str(&content)?;

    let ast = match artifact.ast {
        None => return Ok(None),
        Some(ast) => ast,
    };

    // Resolve source file via build-info, falling back to ast.absolutePath.
    let source_file = resolve_source_file(project_root, &ast, build_infos);
    let line_cache = LineCache::new();
    let mut transfers = Vec::new();

    for node in &ast.nodes {
        let cd = match node {
            SourceUnitNode::ContractDefinition(cd) => cd,
            _ => continue,
        };

        for member in &cd.nodes {
            let fn_def = match member {
                ContractDefinitionNode::FunctionDefinition(fd) => fd,
                _ => continue,
            };

            // Any external or public payable function can receive ETH.
            let is_payable_entry = fn_def.state_mutability == StateMutability::Payable
                && (fn_def.visibility == Visibility::External
                    || fn_def.visibility == Visibility::Public);
            if is_payable_entry
                && visited_src.insert((fn_def.src.offset, fn_def.src.length))
                && let Some(transfer) =
                    build_receive_sink(fn_def, &cd.name, &source_file, project_root, &line_cache)
            {
                transfers.push(transfer);
            }

            let fn_name = if fn_def.name.is_empty() {
                match fn_def.kind {
                    FunctionKind::Receive => "receive",
                    FunctionKind::Fallback => "fallback",
                    _ => &fn_def.name,
                }
            } else {
                &fn_def.name
            };

            let vis = format_visibility(&fn_def.visibility);

            if let Some(ref body) = fn_def.body {
                let found = find_transfer_calls(
                    &body.statements,
                    &cd.name,
                    fn_name,
                    &vis,
                    &source_file,
                    project_root,
                    &line_cache,
                    visited_src,
                );
                transfers.extend(found);
            }
        }
    }

    if transfers.is_empty() {
        return Ok(None);
    }

    Ok(Some(transfers))
}

/// Resolve the correct source file for an AST using build-info, falling back
/// to `ast.absolutePath`.
fn resolve_source_file(
    project_root: &Path,
    ast: &SourceUnit,
    build_infos: &[BuildInfo],
) -> PathBuf {
    // Try build-info resolution first using the source index from any node's
    // `src` field. We use the first node's source index as a proxy.
    if let Some(first_src_index) = ast.nodes.first().and_then(|node| {
        let src = match node {
            SourceUnitNode::ContractDefinition(cd) => &cd.src,
            SourceUnitNode::ImportDirective(id) => &id.src,
            _ => return None,
        };
        Some(src.source_index.to_string())
    }) && let Some(resolved) = BuildInfo::resolve_source_id(build_infos, &first_src_index)
    {
        return project_root.join(resolved);
    }

    project_root.join(&ast.absolute_path)
}

/// Shared context passed through the AST traversal.
struct ScanContext<'a> {
    contract_name: &'a str,
    function_name: &'a str,
    visibility: &'a str,
    source_file: &'a Path,
    project_root: &'a Path,
    line_cache: &'a LineCache,
    visited_src: &'a mut HashSet<(usize, usize)>,
    transfers: &'a mut Vec<AssetTransfer>,
}

/// Recursively search a list of statements for asset transfer calls.
#[allow(clippy::too_many_arguments)]
fn find_transfer_calls(
    statements: &[Statement],
    contract_name: &str,
    function_name: &str,
    visibility: &str,
    source_file: &Path,
    project_root: &Path,
    line_cache: &LineCache,
    visited_src: &mut HashSet<(usize, usize)>,
) -> Vec<AssetTransfer> {
    let mut transfers = Vec::new();
    let mut ctx = ScanContext {
        contract_name,
        function_name,
        visibility,
        source_file,
        project_root,
        line_cache,
        visited_src,
        transfers: &mut transfers,
    };
    for stmt in statements {
        collect_from_statement(stmt, &mut ctx);
    }
    transfers
}

/// Process a body statement by recursing into it.
fn process_body(body: &Statement, ctx: &mut ScanContext) {
    if let Statement::Block(block) = body {
        for s in &block.statements {
            collect_from_statement(s, ctx);
        }
    }
}

/// Traverse a single statement looking for asset transfer calls, recursing
/// into nested blocks and control flow.
fn collect_from_statement(stmt: &Statement, ctx: &mut ScanContext) {
    match stmt {
        Statement::Block(Block { statements, .. }) => {
            for s in statements {
                collect_from_statement(s, ctx);
            }
        }
        solc::ast::Statement::ExpressionStatement(es) => {
            collect_from_expression(&es.expression, ctx);
        }
        solc::ast::Statement::IfStatement(if_stmt) => {
            collect_from_expression(&if_stmt.condition, ctx);
            process_body(&if_stmt.true_body, ctx);
            if let Some(ref false_body) = if_stmt.false_body {
                process_body(false_body, ctx);
            }
        }
        solc::ast::Statement::WhileStatement(ws) => {
            collect_from_expression(&ws.condition, ctx);
            process_body(&ws.body, ctx);
        }
        solc::ast::Statement::DoWhileStatement(dws) => {
            collect_from_expression(&dws.condition, ctx);
            process_body(&dws.body, ctx);
        }
        solc::ast::Statement::ForStatement(fs) => {
            if let Some(ref init) = fs.initialization_expression {
                collect_from_expression(init, ctx);
            }
            collect_from_expression(&fs.condition, ctx);
            if let Some(ref loop_expr) = fs.loop_expression {
                collect_from_expression(loop_expr, ctx);
            }
            process_body(&fs.body, ctx);
        }
        solc::ast::Statement::Return(ret) => {
            if let Some(ref expr) = ret.expression {
                collect_from_expression(expr, ctx);
            }
        }
        solc::ast::Statement::VariableDeclarationStatement(vds) => {
            if let Some(ref init) = vds.initial_value {
                collect_from_expression(init, ctx);
            }
        }
        solc::ast::Statement::RevertStatement(rs) => {
            for arg in &rs.error_call.arguments {
                collect_from_expression(arg, ctx);
            }
        }
        solc::ast::Statement::TryStatement(ts) => {
            collect_from_expression(&ts.external_call, ctx);
            for clause in &ts.clauses {
                for s in &clause.block.statements {
                    collect_from_statement(s, ctx);
                }
            }
        }
        solc::ast::Statement::UncheckedBlock(ub) => {
            for s in &ub.statements {
                collect_from_statement(s, ctx);
            }
        }
        solc::ast::Statement::EmitStatement(_)
        | solc::ast::Statement::InlineAssembly(_)
        | solc::ast::Statement::PlaceholderStatement(_)
        | solc::ast::Statement::Break(_)
        | solc::ast::Statement::Continue(_) => {}
    }
}

/// Traverse an expression looking for asset transfer calls, recursing into
/// sub-expressions.
fn collect_from_expression(expr: &Expression, ctx: &mut ScanContext) {
    match expr {
        Expression::FunctionCall(fc) => {
            // Check if this is a regular member-access transfer (transfer,
            // safeTransfer, transferFrom, safeTransferFrom, send).
            if let FunctionCallExpression::MemberAccess(ma) = &*fc.expression
                && is_transfer_method(&ma.member_name)
                && ctx.visited_src.insert((fc.src.offset, fc.src.length))
                && let Some(transfer) = build_transfer_sink(fc, &ma.member_name, ctx)
            {
                ctx.transfers.push(transfer);
            }

            // Check if this is a call{value: ...} pattern.
            if let FunctionCallExpression::FunctionCallOptions(fco) = &*fc.expression
                && fco.names.iter().any(|n| n == "value")
                && let Expression::MemberAccess(ma) = &*fco.expression
                && ma.member_name == "call"
                && ctx.visited_src.insert((fc.src.offset, fc.src.length))
                && let Some(transfer) = build_call_with_value(fc, ctx)
            {
                ctx.transfers.push(transfer);
            }

            // Recurse into arguments.
            for arg in &fc.arguments {
                collect_from_expression(arg, ctx);
            }
        }
        Expression::Assignment(assign) => {
            collect_from_expression(&assign.right_hand_side, ctx);
        }
        Expression::BinaryOperation(binop) => {
            collect_from_expression(&binop.left_expression, ctx);
            collect_from_expression(&binop.right_expression, ctx);
        }
        Expression::UnaryOperation(unop) => {
            collect_from_expression(&unop.sub_expression, ctx);
        }
        Expression::Conditional(cond) => {
            collect_from_expression(&cond.condition, ctx);
            collect_from_expression(&cond.true_expression, ctx);
            collect_from_expression(&cond.false_expression, ctx);
        }
        Expression::MemberAccess(ma) => {
            collect_from_expression(&ma.expression, ctx);
        }
        Expression::TupleExpression(tuple) => {
            for comp in tuple.components.iter().flatten() {
                collect_from_expression(comp, ctx);
            }
        }
        Expression::IndexAccess(ia) => {
            collect_from_expression(&ia.base_expression, ctx);
            if let Some(ref index) = ia.index_expression {
                collect_from_expression(index, ctx);
            }
        }
        Expression::IndexRangeAccess(ira) => {
            collect_from_expression(&ira.base_expression, ctx);
            if let Some(ref start) = ira.start_expression {
                collect_from_expression(start, ctx);
            }
        }
        Expression::NewExpression(_)
        | Expression::Identifier(_)
        | Expression::Literal(_)
        | Expression::ElementaryTypeNameExpression(_)
        | Expression::VariableDeclarationStatement(_)
        | Expression::ExpressionStatement(_) => {}
    }
}

/// Check if a method name is a known asset transfer method.
fn is_transfer_method(member_name: &str) -> bool {
    matches!(
        member_name,
        "transfer" | "safeTransfer" | "transferFrom" | "safeTransferFrom" | "send"
    )
}

/// Convert a method name to an [`AssetTransferKind`].
/// `fc` is used to disambiguate ERC20 `transfer` from ETH `transfer`.
fn method_to_kind(member_name: &str, fc: &FunctionCall) -> Option<AssetTransferKind> {
    match member_name {
        "transfer" => {
            // ETH `transfer` takes 1 argument, ERC20 `transfer` takes 2.
            if fc.arguments.len() == 1 {
                Some(AssetTransferKind::EthTransfer)
            } else {
                Some(AssetTransferKind::Erc20Transfer)
            }
        }
        "safeTransfer" => Some(AssetTransferKind::Erc20SafeTransfer),
        "transferFrom" => Some(AssetTransferKind::Erc20TransferFrom),
        "safeTransferFrom" => Some(AssetTransferKind::Erc20SafeTransferFrom),
        "send" => Some(AssetTransferKind::EthSend),
        _ => None,
    }
}

/// Collapse all whitespace into a single space, trimming the result.
fn normalize_expression(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(ch);
            prev_was_space = false;
        }
    }
    let trimmed = result.trim_end();
    trimmed.to_string()
}

/// Build an asset transfer from a function call that matches a transfer method.
fn build_transfer_sink(
    fc: &FunctionCall,
    member_name: &str,
    ctx: &ScanContext,
) -> Option<AssetTransfer> {
    let raw = source_text_for_location(ctx.source_file, fc.src.offset, fc.src.length)?;
    let expression = normalize_expression(&raw);
    let line = ctx
        .line_cache
        .offset_to_line(ctx.source_file, fc.src.offset)?;
    let rel_file = ctx
        .source_file
        .strip_prefix(ctx.project_root)
        .unwrap_or(ctx.source_file)
        .to_path_buf();
    let kind = method_to_kind(member_name, fc)?;

    Some(AssetTransfer {
        contract_name: ctx.contract_name.to_string(),
        function_name: ctx.function_name.to_string(),
        visibility: ctx.visibility.to_string(),
        expression,
        kind,
        file: rel_file,
        line,
    })
}

/// Build an asset transfer for a `call{value: ...}` pattern.
fn build_call_with_value(fc: &FunctionCall, ctx: &ScanContext) -> Option<AssetTransfer> {
    let raw = source_text_for_location(ctx.source_file, fc.src.offset, fc.src.length)?;
    let expression = normalize_expression(&raw);
    let line = ctx
        .line_cache
        .offset_to_line(ctx.source_file, fc.src.offset)?;
    let rel_file = ctx
        .source_file
        .strip_prefix(ctx.project_root)
        .unwrap_or(ctx.source_file)
        .to_path_buf();

    Some(AssetTransfer {
        contract_name: ctx.contract_name.to_string(),
        function_name: ctx.function_name.to_string(),
        kind: AssetTransferKind::EthCall,
        visibility: ctx.visibility.to_string(),
        expression,
        file: rel_file,
        line,
    })
}

/// Build an asset transfer for a `receive()` function -- flagged as ETH
/// receiver.
fn build_receive_sink(
    fd: &solc::ast::FunctionDefinition,
    contract_name: &str,
    source_file: &Path,
    project_root: &Path,
    line_cache: &LineCache,
) -> Option<AssetTransfer> {
    let line = line_cache.offset_to_line(source_file, fd.src.offset)?;
    let rel_file = source_file
        .strip_prefix(project_root)
        .unwrap_or(source_file)
        .to_path_buf();

    // Extract just the function signature (up to the opening brace).
    let expression = source_text_for_location(source_file, fd.src.offset, fd.src.length)
        .map(|text| extract_function_signature(&text));

    let fn_name = if fd.name.is_empty() {
        match fd.kind {
            FunctionKind::Receive => "receive",
            FunctionKind::Fallback => "fallback",
            _ => &fd.name,
        }
    } else {
        &fd.name
    };
    let vis = format_visibility(&fd.visibility);

    Some(AssetTransfer {
        contract_name: contract_name.to_string(),
        function_name: fn_name.to_string(),
        kind: AssetTransferKind::EthReceive,
        visibility: vis,
        expression: expression.unwrap_or_default(),
        file: rel_file,
        line,
    })
}

/// Format a [`Visibility`] enum to a lowercase string.
fn format_visibility(v: &Visibility) -> String {
    match v {
        Visibility::External => "external".to_string(),
        Visibility::Public => "public".to_string(),
        Visibility::Internal => "internal".to_string(),
        Visibility::Private => "private".to_string(),
    }
}

/// Extract the source text for a given byte offset and length from a file.
fn source_text_for_location(file: &Path, offset: usize, length: usize) -> Option<String> {
    let content = fs::read_to_string(file).ok()?;
    if offset + length > content.len() {
        return None;
    }
    Some(content[offset..offset + length].to_string())
}

/// Extract just the function signature from a full function definition source.
/// Truncates at the first `{` character, stripping trailing whitespace.
fn extract_function_signature(source: &str) -> String {
    match source.find('{') {
        Some(pos) => source[..pos].trim_end().to_string(),
        None => source.trim_end().to_string(),
    }
}

/// Cache for line number computations.
struct LineCache {
    cache: std::cell::RefCell<HashMap<PathBuf, Option<Vec<usize>>>>,
}

impl LineCache {
    fn new() -> Self {
        Self {
            cache: std::cell::RefCell::new(HashMap::new()),
        }
    }

    fn offset_to_line(&self, file: &Path, offset: usize) -> Option<usize> {
        let mut cache = self.cache.borrow_mut();
        let entry = cache.entry(file.to_path_buf()).or_insert_with(|| {
            let content = fs::read_to_string(file).ok()?;
            Some(
                content
                    .as_bytes()
                    .iter()
                    .enumerate()
                    .filter(|&(_, b)| *b == b'\n')
                    .map(|(i, _)| i)
                    .collect(),
            )
        });
        let positions = entry.as_ref()?;
        Some(positions.partition_point(|&pos| pos < offset) + 1)
    }
}

/// Lightweight artifact that deserializes only the AST.
#[derive(Deserialize)]
struct Artifact {
    #[serde(default)]
    ast: Option<SourceUnit>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/asset-transfers")
    }

    #[test]
    fn scan_finds_all_asset_transfers() {
        let scanner = AssetTransferScanner::new(Project::open(fixture_path()));
        let output = scanner.scan().unwrap();
        let expected = include_str!("../../fixtures/asset-transfers/expected/output.txt");
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn scan_finds_correct_number_of_transfers() {
        let scanner = AssetTransferScanner::new(Project::open(fixture_path()));
        let output = scanner.scan().unwrap();
        assert_eq!(output.transfers.len(), 19);
    }

    #[test]
    fn display_formats_correctly() {
        let transfer = AssetTransfer {
            contract_name: "AssetTransferTest".to_string(),
            function_name: "erc20Transfer".to_string(),
            visibility: "external".to_string(),
            expression: "token.transfer(to, amount)".to_string(),
            kind: AssetTransferKind::Erc20Transfer,
            file: PathBuf::from("src/AssetTransfers.sol"),
            line: 14,
        };
        assert_eq!(transfer.expression, "token.transfer(to, amount)");
    }
}
