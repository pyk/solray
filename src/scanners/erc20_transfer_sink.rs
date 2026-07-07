//! ERC20 Transfer Sink scanner.
//!
//! [`Erc20TransferSinkScanner`] inspects a Foundry project's AST and reports
//! all call sites where `.transfer()` or `.safeTransfer()` is invoked on an
//! ERC20 token.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use solc::ast::{
    Block, ContractDefinitionNode, Expression, FunctionCallExpression, SourceUnit, SourceUnitNode,
    Statement,
};
use walkdir::WalkDir;

use crate::project::Project;

/// A single ERC20 transfer or safeTransfer call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erc20TransferSink {
    /// The contract or library name containing the function that makes the
    /// transfer.
    pub contract_name: String,
    /// The function name containing the transfer call.
    pub function_name: String,
    /// The source text of the transfer call expression.
    pub transfer_expression: String,
    /// The source file path (relative to the project root).
    pub file: PathBuf,
    /// The 1-based line number in the source file.
    pub line: usize,
}

impl std::fmt::Display for Erc20TransferSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.file.display(),
            self.line,
            self.transfer_expression
        )
    }
}

/// The output of an [`Erc20TransferSinkScanner`] scan.
pub struct Erc20TransferSinkScannerOutput {
    sinks: Vec<Erc20TransferSink>,
    _project_root: PathBuf,
}

impl Erc20TransferSinkScannerOutput {
    pub fn new(sinks: Vec<Erc20TransferSink>, project_root: PathBuf) -> Self {
        Self {
            sinks,
            _project_root: project_root,
        }
    }
}

impl std::fmt::Display for Erc20TransferSinkScannerOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sorted = self.sinks.clone();
        sorted.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.contract_name.cmp(&b.contract_name))
        });

        let summary = if sorted.len() == 1 {
            "1 ERC20 transfer sink found.".to_string()
        } else {
            format!("{} ERC20 transfer sinks found.", sorted.len())
        };

        writeln!(f, "## ERC20 Transfer Sinks")?;
        writeln!(f)?;
        writeln!(f, "{summary}")?;
        writeln!(f)?;

        if sorted.is_empty() {
            return Ok(());
        }

        writeln!(
            f,
            "| # | Contract | Function | Transfer | Source Location |"
        )?;
        writeln!(
            f,
            "| - | -------- | -------- | -------- | --------------- |"
        )?;

        for (i, sink) in sorted.iter().enumerate() {
            let location = format!("{}:{}", sink.file.display(), sink.line);
            let expr = sink.transfer_expression.replace('|', "\\|");
            writeln!(
                f,
                "| {} | `{}` | `{}` | `{}` | `{}` |",
                i + 1,
                sink.contract_name,
                sink.function_name,
                expr,
                location,
            )?;
        }

        Ok(())
    }
}

/// Scan a Foundry project for ERC20 transfer sinks.
pub struct Erc20TransferSinkScanner {
    project: Project,
}

impl Erc20TransferSinkScanner {
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Scan the project and return all ERC20 transfer sinks.
    pub fn scan(&self) -> Result<Erc20TransferSinkScannerOutput> {
        self.project.validate()?;
        let project_root = std::path::absolute(self.project.path())?;
        let artifact_paths = self.artifact_paths();

        let mut sinks = Vec::new();
        // Deduplicate across artifacts (each artifact contains the full AST
        // of the source file, so the same call site appears in every artifact
        // that shares the file).
        let mut visited_src: HashSet<(usize, usize)> = HashSet::new();
        for path in &artifact_paths {
            if let Some(mut found) = scan_artifact(path, &project_root, &mut visited_src)? {
                sinks.append(&mut found);
            }
        }

        Ok(Erc20TransferSinkScannerOutput::new(sinks, project_root))
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

/// Scan a single artifact for ERC20 transfer (safeTransfer) calls.
fn scan_artifact(
    artifact_path: impl AsRef<Path>,
    project_root: &Path,
    visited_src: &mut HashSet<(usize, usize)>,
) -> Result<Option<Vec<Erc20TransferSink>>> {
    let artifact_path = artifact_path.as_ref();
    let content = fs::read_to_string(artifact_path)?;
    let artifact: Artifact = serde_json::from_str(&content)?;

    let ast = match artifact.ast {
        None => return Ok(None),
        Some(ast) => ast,
    };

    let source_file = project_root.join(&ast.absolute_path);
    let line_cache = LineCache::new();
    let mut sinks = Vec::new();

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

            if let Some(ref body) = fn_def.body {
                let found = find_transfer_calls(
                    &body.statements,
                    &cd.name,
                    &fn_def.name,
                    &source_file,
                    project_root,
                    &line_cache,
                    visited_src,
                );
                sinks.extend(found);
            }
        }
    }

    if sinks.is_empty() {
        return Ok(None);
    }

    Ok(Some(sinks))
}

/// Shared context passed through the AST traversal.
struct ScanContext<'a> {
    contract_name: &'a str,
    function_name: &'a str,
    source_file: &'a Path,
    project_root: &'a Path,
    line_cache: &'a LineCache,
    visited_src: &'a mut HashSet<(usize, usize)>,
    sinks: &'a mut Vec<Erc20TransferSink>,
}

/// Recursively search a list of statements for token transfer calls.
fn find_transfer_calls(
    statements: &[Statement],
    contract_name: &str,
    function_name: &str,
    source_file: &Path,
    project_root: &Path,
    line_cache: &LineCache,
    visited_src: &mut HashSet<(usize, usize)>,
) -> Vec<Erc20TransferSink> {
    let mut sinks = Vec::new();
    let mut ctx = ScanContext {
        contract_name,
        function_name,
        source_file,
        project_root,
        line_cache,
        visited_src,
        sinks: &mut sinks,
    };
    for stmt in statements {
        collect_from_statement(stmt, &mut ctx);
    }
    sinks
}

/// Process a body statement by recursing into it.
fn process_body(body: &Statement, ctx: &mut ScanContext) {
    if let Statement::Block(block) = body {
        for s in &block.statements {
            collect_from_statement(s, ctx);
        }
    }
}

/// Traverse a single statement looking for transfer calls, recursing into
/// nested blocks and control flow.
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
            // error_call is a FunctionCall, not Expression - handle transfer
            // calls within the error's arguments by wrapping as Expression
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

/// Traverse an expression looking for transfer calls, recursing into
/// sub-expressions.
fn collect_from_expression(expr: &Expression, ctx: &mut ScanContext) {
    match expr {
        Expression::FunctionCall(fc) => {
            if let FunctionCallExpression::MemberAccess(ma) = &*fc.expression
                && is_transfer_method(&ma.member_name)
                && ctx.visited_src.insert((fc.src.offset, fc.src.length))
                && let Some(sink) = build_transfer_sink(fc, ctx)
            {
                ctx.sinks.push(sink);
            }
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

/// Check if a method name is an ERC20 transfer method.
fn is_transfer_method(member_name: &str) -> bool {
    member_name == "transfer" || member_name == "safeTransfer"
}

/// Build a transfer sink from a function call that matches a transfer method.
fn build_transfer_sink(
    fc: &solc::ast::FunctionCall,
    ctx: &ScanContext,
) -> Option<Erc20TransferSink> {
    let source_text = source_text_for_location(ctx.source_file, fc.src.offset, fc.src.length)?;
    let line = ctx
        .line_cache
        .offset_to_line(ctx.source_file, fc.src.offset)?;
    let rel_file = ctx
        .source_file
        .strip_prefix(ctx.project_root)
        .unwrap_or(ctx.source_file)
        .to_path_buf();

    Some(Erc20TransferSink {
        contract_name: ctx.contract_name.to_string(),
        function_name: ctx.function_name.to_string(),
        transfer_expression: source_text,
        file: rel_file,
        line,
    })
}

/// Extract the source text for a given byte offset and length from a file.
fn source_text_for_location(file: &Path, offset: usize, length: usize) -> Option<String> {
    let content = fs::read_to_string(file).ok()?;
    if offset + length > content.len() {
        return None;
    }
    Some(content[offset..offset + length].to_string())
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/erc20-transfer-sinks")
    }

    #[test]
    fn scan_finds_transfer_sinks() {
        let scanner = Erc20TransferSinkScanner::new(Project::open(fixture_path()));
        let output = scanner.scan().unwrap();
        let expected = include_str!("../../fixtures/erc20-transfer-sinks/expected/output.txt");
        assert_eq!(output.to_string(), expected);
    }

    #[test]
    fn sink_display_formats_as_file_line_expression() {
        let sink = Erc20TransferSink {
            contract_name: "Token".to_string(),
            function_name: "transferToken".to_string(),
            transfer_expression: "IERC20(token).safeTransfer(to, amount)".to_string(),
            file: PathBuf::from("src/Token.sol"),
            line: 42,
        };
        assert_eq!(
            sink.to_string(),
            "src/Token.sol:42:IERC20(token).safeTransfer(to, amount)"
        );
    }

    #[test]
    fn scan_project_without_transfers_returns_empty_output() {
        // The contracts fixture has no transfer calls.
        let contracts_fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/contracts");
        let scanner = Erc20TransferSinkScanner::new(Project::open(contracts_fixture));
        let output = scanner.scan().unwrap();
        assert_eq!(
            output.to_string(),
            "## ERC20 Transfer Sinks\n\n0 ERC20 transfer sinks found.\n\n"
        );
    }
}
