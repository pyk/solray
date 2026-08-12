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
    ContractDefinition, ContractDefinitionNode, ContractKind, Expression, FunctionCallExpression,
    FunctionKind, SourceUnit, SourceUnitNode, TypeName, Visibility,
};
use tracing::debug;

use crate::artifact_index::ArtifactIndex;
use crate::build_info::BuildInfo;
use crate::inspectors::artifact_id::ArtifactId;
use crate::inspectors::function_source::symbol_index::SymbolIndex;
use crate::project::Project;

pub mod symbol_index;

/// A resolved declaration with its source code and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedSymbol {
    /// Human-readable signature, e.g. `Main.execute(uint256)` or `Data`
    symbol: String,
    /// The source file path
    file: PathBuf,
    /// Byte offset in source
    offset: usize,
    /// Byte length of the definition
    length: usize,
    /// Solc AST node type (e.g. "FunctionDefinition", "EventDefinition")
    node_type: String,
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

/// Map a node type string to its human-readable section heading prefix.
fn node_type_to_heading(node_type: &str) -> &str {
    match node_type {
        "FunctionDefinition" => "Function",
        "VariableDeclaration" => "Variable",
        "StructDefinition" => "Struct",
        "EnumDefinition" => "Enum",
        "ErrorDefinition" => "Error",
        "EventDefinition" => "Event",
        "ContractDefinition" => "Contract",
        "AbstractContractDefinition" => "Abstract Contract",
        "InterfaceDefinition" => "Interface",
        "LibraryDefinition" => "Library",
        "ModifierDefinition" => "Modifier",
        "UserDefinedValueTypeDefinition" => "User Defined Value Type",
        _ => "Declaration",
    }
}

/// Return the refined AST node type for a contract definition, based on its
/// Solidity kind and abstract flag.
fn contract_node_type(cd: &ContractDefinition) -> &'static str {
    match cd.contract_kind {
        ContractKind::Interface => "InterfaceDefinition",
        ContractKind::Library => "LibraryDefinition",
        ContractKind::Contract if cd.r#abstract => "AbstractContractDefinition",
        ContractKind::Contract => "ContractDefinition",
    }
}

/// Whether a node type represents any kind of contract definition.
fn is_contract_node_type(node_type: &str) -> bool {
    matches!(
        node_type,
        "ContractDefinition"
            | "AbstractContractDefinition"
            | "InterfaceDefinition"
            | "LibraryDefinition"
    )
}

/// Extract a display name from a symbol string.
///
/// For functions: `Contract.funcName(params)` -> `funcName`
/// For variables: `Contract.varName` -> `varName`
/// For other declarations: return the symbol as-is.
fn symbol_display_name(symbol: &str, node_type: &str) -> String {
    if node_type == "FunctionDefinition" {
        // Format: ContractName.funcName(params)
        if let Some(dot_pos) = symbol.find('.') {
            let after_dot = &symbol[dot_pos + 1..];
            if let Some(paren_pos) = after_dot.find('(') {
                return after_dot[..paren_pos].to_string();
            }
            return after_dot.to_string();
        }
    }
    if node_type == "VariableDeclaration" {
        // Format: ContractName.varName
        if let Some(dot_pos) = symbol.find('.') {
            return symbol[dot_pos + 1..].to_string();
        }
    }
    symbol.to_string()
}

/// Compute the 1-indexed line number for a byte offset in source content.
fn byte_offset_to_line(content: &str, offset: usize) -> usize {
    let offset = offset.min(content.len());
    content[..offset].matches('\n').count() + 1
}

/// Extract the contract name from the first symbol (root function).
/// Symbol format: `ContractName.functionName(params)`.
fn extract_contract_name(symbol: &str) -> &str {
    symbol.split('.').next().unwrap_or("?")
}

/// Rank a symbol by how close its declaring contract is to the queried
/// contract in the inheritance chain. Lower is more-derived.
fn inheritance_rank(symbol: &ResolvedSymbol, inheritance_order: &[String]) -> (usize, usize) {
    let contract = extract_contract_name(&symbol.symbol);
    let position = inheritance_order
        .iter()
        .position(|name| name == contract)
        .unwrap_or(usize::MAX);
    (position, symbol.offset)
}

impl std::fmt::Display for FunctionSourceInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let project_abs =
            std::path::absolute(&self.project_path).unwrap_or(self.project_path.clone());
        let mut file_contents: HashMap<PathBuf, String> = HashMap::new();

        if let Some(root) = self.symbols.first() {
            let root_name = symbol_display_name(&root.symbol, &root.node_type);
            let contract_name = extract_contract_name(&root.symbol);

            let full_path = project_abs.join(&root.file);

            if let Ok(content) = read_source_normalized(&full_path) {
                file_contents.insert(root.file.clone(), content);
            }

            writeln!(f, "# {} - {} Source Code", contract_name, root_name)?;
            writeln!(f)?;

            // checkrs: allow(long_if_let_blocks, nested_if_let)
            if let Some(content) = file_contents.get(&root.file) {
                let line = byte_offset_to_line(content, root.offset);
                writeln!(f, "Source path: `{}:{line}`", root.file.display())?;
                writeln!(f)?;

                let base = base_indent(content, root.offset);
                let mut natspec = extract_natspec(content, root.offset);
                natspec = resolve_inheritdoc_natspec(
                    &natspec,
                    &root.symbol,
                    &self.artifact_index,
                    &self.project_path,
                );
                let natspec = dedent(&natspec, base);
                let source_text = dedent(&content[root.offset..root.offset + root.length], base);

                writeln!(f, "```solidity")?;
                if !natspec.is_empty() {
                    writeln!(f, "{}", natspec.trim_end())?;
                }
                writeln!(f, "{}", source_text.trim_end())?;
                writeln!(f, "```")?;
            } else {
                writeln!(f, "Source path: `{}`", root.file.display())?;
                writeln!(f)?;
                writeln!(f, "```solidity")?;
                writeln!(f, "// unable to read source")?;
                writeln!(f, "```")?;
            }

            let mut symbols: Vec<&ResolvedSymbol> = self.symbols.iter().skip(1).collect();
            symbols.sort_by(|a, b| {
                let a_heading = node_type_to_heading(&a.node_type);
                let b_heading = node_type_to_heading(&b.node_type);
                let a_name = symbol_display_name(&a.symbol, &a.node_type).to_lowercase();
                let b_name = symbol_display_name(&b.symbol, &b.node_type).to_lowercase();
                a_heading.cmp(b_heading).then(a_name.cmp(&b_name))
            });

            for symbol in &symbols {
                writeln!(f)?;
                writeln!(f, "---")?;

                let display_name = symbol_display_name(&symbol.symbol, &symbol.node_type);
                let heading = node_type_to_heading(&symbol.node_type);

                let full_path = project_abs.join(&symbol.file);

                let content = if let Some(c) = file_contents.get(&symbol.file) {
                    c.clone() // checkrs: allow(clone_in_loops)
                } else {
                    let Ok(c) = read_source_normalized(&full_path) else {
                        writeln!(f)?;
                        writeln!(f, "## {}: `{}`", heading, display_name)?;
                        writeln!(f)?;
                        writeln!(f, "Source path: `{}`", symbol.file.display())?;
                        writeln!(f)?;
                        writeln!(f, "```solidity")?;
                        writeln!(f, "// unable to read")?;
                        writeln!(f, "```")?;
                        continue;
                    };
                    file_contents.insert(symbol.file.clone(), c.clone()); // checkrs: allow(clone_in_loops)
                    c
                };

                let line = byte_offset_to_line(&content, symbol.offset);
                let base = base_indent(&content, symbol.offset);
                let mut natspec = extract_natspec(&content, symbol.offset);
                natspec = resolve_inheritdoc_natspec(
                    &natspec,
                    &symbol.symbol,
                    &self.artifact_index,
                    &self.project_path,
                );
                let natspec = dedent(&natspec, base);
                let source_text = if is_contract_node_type(&symbol.node_type) {
                    dedent(contract_header(&content, symbol.offset), base)
                } else {
                    dedent(&content[symbol.offset..symbol.offset + symbol.length], base)
                };

                writeln!(f)?;
                writeln!(f, "## {}: `{}`", heading, display_name)?;
                writeln!(f)?;
                writeln!(f, "Source path: `{}:{line}`", symbol.file.display())?;
                writeln!(f)?;
                writeln!(f, "```solidity")?;
                if !natspec.is_empty() {
                    writeln!(f, "{}", natspec.trim_end())?;
                }
                writeln!(f, "{}", source_text.trim_end())?;
                writeln!(f, "```")?;
            }
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
                let direct = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                let artifact_path = if direct.exists() {
                    direct
                } else {
                    self.artifact_index
                        .find_by_source_path(file, &id.name)
                        .unwrap_or(direct)
                };
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
                            let qualified = ArtifactIndex::qualified_name(candidate, &id.name);
                            msg.push_str(&format!(
                                "\nsolray inspect function-source {qualified} \"{function_name}\""
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
        let mut inheritance_order: Vec<String> = vec![contract_name.to_string()];
        for artifact_path in artifact_paths {
            let Some(ast) = parse_artifact(artifact_path)? else {
                continue;
            };
            extract_function_symbols(
                &ast,
                contract_name,
                base_name,
                &mut functions,
                SpecialFunctionFilter::All,
                true,
            );
            let inherited = self.inherited_contracts(artifact_path, contract_name)?;
            for (base_contract, base_path) in &inherited {
                let Some(base_ast) = parse_artifact(base_path)? else {
                    continue;
                };
                let is_new_base = !inheritance_order.contains(base_contract);
                if is_new_base {
                    inheritance_order.push(base_contract.clone()); // checkrs: allow(clone_in_loops)
                }
                extract_function_symbols(
                    &base_ast,
                    base_contract,
                    base_name,
                    &mut functions,
                    SpecialFunctionFilter::FallbackReceive,
                    false,
                );
            }
        }

        if functions.is_empty() {
            let mut all_fns: Vec<String> = Vec::new();
            for artifact_path in artifact_paths {
                let Some(ast) = parse_artifact(artifact_path)? else {
                    continue;
                };
                collect_contract_functions(
                    &ast,
                    contract_name,
                    &mut all_fns,
                    SpecialFunctionFilter::All,
                    true,
                );
                let inherited = self.inherited_contracts(artifact_path, contract_name)?;
                for (base_contract, base_path) in inherited {
                    let Some(base_ast) = parse_artifact(base_path)? else {
                        continue;
                    };
                    collect_contract_functions(
                        &base_ast,
                        &base_contract,
                        &mut all_fns,
                        SpecialFunctionFilter::FallbackReceive,
                        false,
                    );
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
            let suffix = format!(".{}", function_name);
            let matched: Vec<&ResolvedSymbol> = functions
                .values()
                .filter(|s| s.symbol == target_sig || s.symbol.ends_with(&suffix))
                .collect();
            if matched.is_empty() {
                let mut msg = format!(
                    "\"{}\" not found in \"{}\".\n\nSelect one of the following:\n",
                    function_name, contract_name
                );
                let mut sorted: Vec<&String> = functions.values().map(|s| &s.symbol).collect();
                sorted.sort();
                for sym in sorted {
                    let fn_name = sym.split_once('.').map(|(_, sig)| sig).unwrap_or(sym);
                    msg.push_str(&format!(
                        "\nsolray inspect function-source {} \"{}\"",
                        contract_name, fn_name
                    ));
                }
                msg.push('\n');
                bail!(msg);
            }
            if matched.len() > 1 {
                // Prefer the most-derived declaration when the same signature
                // is declared by inherited overrides. Keep the ambiguity error
                // when the closest contract still has multiple candidates.
                let best = matched
                    .iter()
                    .min_by_key(|symbol| inheritance_rank(symbol, &inheritance_order));
                if let Some(best) = best
                    && matched
                        .iter()
                        .filter(|symbol| {
                            inheritance_rank(symbol, &inheritance_order)
                                == inheritance_rank(best, &inheritance_order)
                        })
                        .count()
                        == 1
                {
                    return Ok((*best).clone());
                }
                let mut msg = format!(
                    "found {} \"{}\"\n\nSelect one of the following:\n",
                    matched.len(),
                    function_name
                );
                let mut sorted: Vec<&String> = matched.iter().map(|s| &s.symbol).collect();
                sorted.sort();
                for sym in sorted {
                    let fn_name = sym.split_once('.').map(|(_, sig)| sig).unwrap_or(sym);
                    msg.push_str(&format!(
                        "\nsolray inspect function-source {} \"{}\"",
                        contract_name, fn_name
                    ));
                }
                msg.push('\n');
                bail!(msg);
            }
            return Ok(matched[0].clone());
        }

        if functions.len() > 1 {
            // When every candidate has the same signature, this is an
            // inherited override rather than a real overload: prefer the
            // most-derived declaration.
            let signatures: HashSet<&str> = functions
                .values()
                .map(|s| {
                    s.symbol
                        .split_once('.')
                        .map_or(s.symbol.as_str(), |(_, sig)| sig)
                })
                .collect();
            if signatures.len() == 1 {
                let best = functions
                    .values()
                    .min_by_key(|symbol| inheritance_rank(symbol, &inheritance_order));
                if let Some(best) = best
                    && functions
                        .values()
                        .filter(|symbol| {
                            inheritance_rank(symbol, &inheritance_order)
                                == inheritance_rank(best, &inheritance_order)
                        })
                        .count()
                        == 1
                {
                    return Ok(best.clone());
                }
            }
            let mut msg = format!(
                "found {} \"{}\"\n\nSelect one of the following:\n",
                functions.len(),
                function_name
            );
            let mut sorted: Vec<&String> = functions.values().map(|s| &s.symbol).collect();
            sorted.sort();
            for sym in sorted {
                let fn_name = sym.split_once('.').map(|(_, sig)| sig).unwrap_or(sym);
                msg.push_str(&format!(
                    "\nsolray inspect function-source {} \"{}\"",
                    contract_name, fn_name
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

    /// Collect the transitive base contracts of a contract, as
    /// `(contract_name, artifact_path)` pairs.
    fn inherited_contracts(
        &self,
        artifact_path: impl AsRef<Path>,
        contract_name: &str,
    ) -> Result<Vec<(String, PathBuf)>> {
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        self.collect_inherited_contracts(artifact_path, contract_name, &mut visited, &mut out)?;
        Ok(out)
    }

    fn collect_inherited_contracts(
        &self,
        artifact_path: impl AsRef<Path>,
        contract_name: &str,
        visited: &mut HashSet<String>,
        out: &mut Vec<(String, PathBuf)>,
    ) -> Result<()> {
        if !visited.insert(contract_name.to_string()) {
            return Ok(());
        }
        let parsed = parse_artifact(artifact_path)?;
        let Some(ast) = parsed else {
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
                    {
                        let info = self.symbol_index.artifact_info(entry.artifact_id);
                        if info.build_info_id == build_info_id {
                            out.push((
                                entry.name.clone(),         // checkrs: allow(clone_in_loops)
                                info.artifact_path.clone(), // checkrs: allow(clone_in_loops)
                            ));
                            self.collect_inherited_contracts(
                                &info.artifact_path,
                                &entry.name,
                                visited,
                                out,
                            )?;
                        }
                    }
                }
                break;
            }
        }
        Ok(())
    }

    /// Resolve the refined contract node type for a contract AST node ID.
    fn contract_node_type_for_id(
        &self,
        artifact_path: impl AsRef<Path>,
        contract_id: i64,
    ) -> Result<String> {
        let Some(ast) = parse_artifact(artifact_path)? else {
            return Ok("ContractDefinition".to_string());
        };
        for node in &ast.nodes {
            if let SourceUnitNode::ContractDefinition(cd) = node
                && cd.id == contract_id
            {
                return Ok(contract_node_type(cd).to_string());
            }
        }
        Ok("ContractDefinition".to_string())
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

            debug!(
                "[resolve_recursive] processing symbol: {:?} type={} file={:?} offset={}",
                symbol.symbol, symbol.node_type, symbol.file, symbol.offset
            );

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
                // Contract-kind symbols (contracts, abstracts, interfaces, and
                // libraries) span the whole declaration, so walking their range
                // would collect references from every member body, not just the
                // referenced declaration. Only member symbols that are actually
                // referenced are resolved, so container symbols contribute just
                // their own header (and their base contracts below).
                if !is_contract_node_type(&symbol.node_type) {
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

                // For ContractDefinition symbols, also resolve base contracts
                // (inheritance) so parent interfaces are recursively resolved.
                if is_contract_node_type(&symbol.node_type) {
                    for node in &ast.nodes {
                        if let SourceUnitNode::ContractDefinition(cd) = node
                            && cd.src.offset == symbol.offset
                            && cd.src.length == symbol.length
                        {
                            for base in &cd.base_contracts {
                                if let Some(id) = base.base_name.referenced_declaration
                                    && let Some(entry) = self.symbol_index.get(build_info_id, id)
                                {
                                    let info = self.symbol_index.artifact_info(entry.artifact_id);
                                    if info.build_info_id == build_info_id {
                                        let node_type = self
                                            .contract_node_type_for_id(&info.artifact_path, id)?;
                                        let rs = ResolvedSymbol {
                                            symbol: entry.name.clone(),     // checkrs: allow(clone_in_loops)
                                            file: info.source_file.clone(), // checkrs: allow(clone_in_loops)
                                            offset: entry.offset,
                                            length: entry.length,
                                            node_type,
                                        };
                                        let key = (rs.file.clone(), rs.offset); // checkrs: allow(clone_in_loops)
                                        let new_symbol = !seen.contains(&key);
                                        if new_symbol {
                                            queue.push(rs);
                                        }
                                    }
                                }
                            }
                            break;
                        }
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
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let artifact: Artifact = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse artifact `{}`", path.display()))?;
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

/// Which special function kinds to include when collecting symbols.
#[derive(Clone, Copy)]
enum SpecialFunctionFilter {
    /// Include inherited `receive` and `fallback` functions, but not constructors.
    FallbackReceive,
    /// Include constructors, `receive`, and `fallback`.
    All,
}

/// Extract function symbols from an AST for a given contract/function name.
fn extract_function_symbols(
    ast: &SourceUnit,
    contract_name: &str,
    function_name: &str,
    out: &mut HashMap<String, ResolvedSymbol>,
    special: SpecialFunctionFilter,
    include_interface_declarations: bool,
) {
    let source_file = &ast.absolute_path;
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.name == contract_name
        {
            for inner in &cd.nodes {
                if let ContractDefinitionNode::FunctionDefinition(fd) = inner
                    && (fd.implemented
                        || (cd.name == contract_name
                            && include_interface_declarations
                            && cd.contract_kind == ContractKind::Interface))
                    && function_name_for_display(&fd.kind, &fd.name) == function_name
                {
                    let is_special = matches!(
                        fd.kind,
                        FunctionKind::Constructor | FunctionKind::Receive | FunctionKind::Fallback
                    );
                    let include_special = match special {
                        SpecialFunctionFilter::All => true,
                        SpecialFunctionFilter::FallbackReceive => {
                            matches!(fd.kind, FunctionKind::Receive | FunctionKind::Fallback)
                        }
                    };
                    if is_special && !include_special {
                        continue;
                    }
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
                        file: source_file.clone(),
                        offset: fd.src.offset,
                        length: fd.src.length,
                        node_type: "FunctionDefinition".into(),
                    });
                }
                if let ContractDefinitionNode::VariableDeclaration(vd) = inner
                    && vd.visibility == Visibility::Public
                    && vd.name == function_name
                {
                    let sig = format!("{}.{}", contract_name, vd.name);
                    let sig_key = sig.clone(); // checkrs: allow(clone_in_loops)
                    out.entry(sig_key).or_insert(ResolvedSymbol {
                        symbol: sig,
                        file: source_file.clone(),
                        offset: vd.src.offset,
                        length: vd.src.length,
                        node_type: "VariableDeclaration".into(),
                    });
                }
            }
        }
    }
}

/// Collect available function names in a contract.
fn collect_contract_functions(
    ast: &SourceUnit,
    contract_name: &str,
    out: &mut Vec<String>,
    special: SpecialFunctionFilter,
    include_interface_declarations: bool,
) {
    for node in &ast.nodes {
        if let SourceUnitNode::ContractDefinition(cd) = node
            && cd.name == contract_name
        {
            for inner in &cd.nodes {
                if let ContractDefinitionNode::FunctionDefinition(fd) = inner
                    && (fd.implemented
                        || (cd.name == contract_name
                            && include_interface_declarations
                            && cd.contract_kind == ContractKind::Interface))
                {
                    let is_special = matches!(
                        fd.kind,
                        FunctionKind::Constructor | FunctionKind::Receive | FunctionKind::Fallback
                    );
                    let include_special = match special {
                        SpecialFunctionFilter::All => true,
                        SpecialFunctionFilter::FallbackReceive => {
                            matches!(fd.kind, FunctionKind::Receive | FunctionKind::Fallback)
                        }
                    };
                    if is_special && !include_special {
                        continue;
                    }
                    out.push(function_name_for_display(&fd.kind, &fd.name).to_string());
                }
                if let ContractDefinitionNode::VariableDeclaration(vd) = inner
                    && vd.visibility == Visibility::Public
                {
                    out.push(vd.name.clone()); // checkrs: allow(clone_in_loops)
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
        ContractDefinitionNode::VariableDeclaration(vd) => {
            let vd_start = vd.src.offset;
            let vd_end = vd_start + vd.src.length;
            if vd_start < range_end && vd_end > range_start {
                collect_from_type_name(&vd.type_name, seen_ids, results, ctx);
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
        solc::ast::Statement::RevertStatement(rs) => {
            collect_from_function_call(&rs.error_call, seen_ids, results, ctx);
        }
        solc::ast::Statement::EmitStatement(es) => {
            collect_from_function_call(&es.event_call, seen_ids, results, ctx);
        }
        solc::ast::Statement::TryStatement(ts) => {
            collect_from_expression(&ts.external_call, seen_ids, results, ctx);
            for clause in &ts.clauses {
                collect_from_statements(&clause.block.statements, seen_ids, results, ctx);
            }
        }
        solc::ast::Statement::UncheckedBlock(ub) => {
            collect_from_statements(&ub.statements, seen_ids, results, ctx);
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
            collect_from_function_call(fc, seen_ids, results, ctx);
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
        debug!(
            "[resolve_and_add_symbol] resolved id={} ({:?}) in current AST",
            id, rs.node_type
        );
        results.push(rs);
        return;
    }
    let Some(entry) = ctx.symbol_index.get(ctx.build_info_id, id) else {
        debug!(
            "[resolve_and_add_symbol] id={} NOT FOUND in symbol_index (bid={})",
            id, ctx.build_info_id
        );
        return;
    };
    let info = ctx.symbol_index.artifact_info(entry.artifact_id);
    debug!(
        "[resolve_and_add_symbol] id={} found in symbol_index: name={}, build_info={}, source={:?}, ctx_build_info={}, ctx_source={:?}",
        id, entry.name, info.build_info_id, info.source_file, ctx.build_info_id, ctx.source_file
    );
    if info.build_info_id == ctx.build_info_id && info.source_file != *ctx.source_file {
        let symbol = match entry.node_type.as_str() {
            "FunctionDefinition" | "VariableDeclaration" => {
                format!("{}.{}", entry.contract_name, entry.name)
            }
            _ => entry.name.clone(),
        };
        debug!(
            "[resolve_and_add_symbol] adding {} (id={}) from {:?}",
            symbol, id, info.source_file
        );
        let mut node_type = entry.node_type.clone();
        if node_type == "ContractDefinition"
            && let Ok(Some(ast)) = parse_artifact(&info.artifact_path)
        {
            for node in &ast.nodes {
                if let SourceUnitNode::ContractDefinition(cd) = node
                    && cd.id == id
                {
                    node_type = contract_node_type(cd).to_string();
                    break;
                }
            }
        }
        results.push(ResolvedSymbol {
            symbol,
            file: info.source_file.clone(),
            offset: entry.offset,
            length: entry.length,
            node_type,
        });
    } else {
        debug!(
            "[resolve_and_add_symbol] SKIPPING id={}: build_info mismatch or same source (info.bid={}, ctx.bid={}, info.src={:?}, ctx.src={:?})",
            id, info.build_info_id, ctx.build_info_id, info.source_file, ctx.source_file
        );
    }
}

fn resolve_id_in_ast(id: i64, ast: &SourceUnit, source_file: &Path) -> Option<ResolvedSymbol> {
    for node in &ast.nodes {
        match node {
            SourceUnitNode::ContractDefinition(cd) => {
                if cd.id == id {
                    return Some(ResolvedSymbol {
                        symbol: cd.name.clone(), // checkrs: allow(clone_in_loops)
                        file: source_file.to_path_buf(),
                        offset: cd.src.offset,
                        length: cd.src.length,
                        node_type: contract_node_type(cd).into(),
                    });
                }
                for inner in &cd.nodes {
                    if let Some(rs) = node_to_symbol(inner, id, &cd.name, source_file) {
                        return Some(rs);
                    }
                }
            }
            SourceUnitNode::ErrorDefinition(ed) if ed.id == id => {
                return Some(ResolvedSymbol {
                    symbol: ed.name.clone(), // checkrs: allow(clone_in_loops)
                    file: source_file.to_path_buf(),
                    offset: ed.src.offset,
                    length: ed.src.length,
                    node_type: "ErrorDefinition".into(),
                });
            }
            SourceUnitNode::EventDefinition(ev) if ev.id == id => {
                return Some(ResolvedSymbol {
                    symbol: ev.name.clone(), // checkrs: allow(clone_in_loops)
                    file: source_file.to_path_buf(),
                    offset: ev.src.offset,
                    length: ev.src.length,
                    node_type: "EventDefinition".into(),
                });
            }
            SourceUnitNode::StructDefinition(sd) if sd.id == id => {
                return Some(ResolvedSymbol {
                    symbol: sd.name.clone(), // checkrs: allow(clone_in_loops)
                    file: source_file.to_path_buf(),
                    offset: sd.src.offset,
                    length: sd.src.length,
                    node_type: "StructDefinition".into(),
                });
            }
            SourceUnitNode::EnumDefinition(ed) if ed.id == id => {
                return Some(ResolvedSymbol {
                    symbol: ed.name.clone(), // checkrs: allow(clone_in_loops)
                    file: source_file.to_path_buf(),
                    offset: ed.src.offset,
                    length: ed.src.length,
                    node_type: "EnumDefinition".into(),
                });
            }
            SourceUnitNode::FunctionDefinition(fd) if fd.id == id => {
                let sig = format!("{}({})", fd.name, format_params(&fd.parameters.parameters));
                return Some(ResolvedSymbol {
                    symbol: sig,
                    file: source_file.to_path_buf(),
                    offset: fd.src.offset,
                    length: fd.src.length,
                    node_type: "FunctionDefinition".into(),
                });
            }
            SourceUnitNode::VariableDeclaration(vd) if vd.id == id => {
                return Some(ResolvedSymbol {
                    symbol: vd.name.clone(), // checkrs: allow(clone_in_loops)
                    file: source_file.to_path_buf(),
                    offset: vd.src.offset,
                    length: vd.src.length,
                    node_type: "VariableDeclaration".into(),
                });
            }
            SourceUnitNode::UserDefinedValueTypeDefinition(udvtd) if udvtd.id == id => {
                return Some(ResolvedSymbol {
                    symbol: udvtd.name.clone(), // checkrs: allow(clone_in_loops)
                    file: source_file.to_path_buf(),
                    offset: udvtd.src.offset,
                    length: udvtd.src.length,
                    node_type: "UserDefinedValueTypeDefinition".into(),
                });
            }
            _ => {}
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
                node_type: "FunctionDefinition".into(),
            })
        }
        ContractDefinitionNode::VariableDeclaration(vd) if vd.id == target_id => {
            Some(ResolvedSymbol {
                symbol: format!("{}.{}", contract_name, vd.name),
                file: source_file.to_path_buf(),
                offset: vd.src.offset,
                length: vd.src.length,
                node_type: "VariableDeclaration".into(),
            })
        }
        ContractDefinitionNode::StructDefinition(sd) if sd.id == target_id => {
            Some(ResolvedSymbol {
                symbol: sd.name.clone(),
                file: source_file.to_path_buf(),
                offset: sd.src.offset,
                length: sd.src.length,
                node_type: "StructDefinition".into(),
            })
        }
        ContractDefinitionNode::EnumDefinition(ed) if ed.id == target_id => Some(ResolvedSymbol {
            symbol: ed.name.clone(),
            file: source_file.to_path_buf(),
            offset: ed.src.offset,
            length: ed.src.length,
            node_type: "EnumDefinition".into(),
        }),
        ContractDefinitionNode::ErrorDefinition(ed) if ed.id == target_id => Some(ResolvedSymbol {
            symbol: ed.name.clone(),
            file: source_file.to_path_buf(),
            offset: ed.src.offset,
            length: ed.src.length,
            node_type: "ErrorDefinition".into(),
        }),
        ContractDefinitionNode::EventDefinition(ed) if ed.id == target_id => Some(ResolvedSymbol {
            symbol: ed.name.clone(),
            file: source_file.to_path_buf(),
            offset: ed.src.offset,
            length: ed.src.length,
            node_type: "EventDefinition".into(),
        }),
        ContractDefinitionNode::ModifierDefinition(md) if md.id == target_id => {
            Some(ResolvedSymbol {
                symbol: md.name.clone(),
                file: source_file.to_path_buf(),
                offset: md.src.offset,
                length: md.src.length,
                node_type: "ModifierDefinition".into(),
            })
        }
        ContractDefinitionNode::UserDefinedValueTypeDefinition(udvtd) if udvtd.id == target_id => {
            Some(ResolvedSymbol {
                symbol: udvtd.name.clone(),
                file: source_file.to_path_buf(),
                offset: udvtd.src.offset,
                length: udvtd.src.length,
                node_type: "UserDefinedValueTypeDefinition".into(),
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

/// Return the header of a contract or interface definition (up to, but not
/// including, the opening brace). Trims trailing whitespace.
fn contract_header(content: &str, offset: usize) -> &str {
    let remaining = &content[offset..];
    if let Some(brace_pos) = remaining.find('{') {
        remaining[..brace_pos].trim_end()
    } else {
        remaining.trim_end()
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
                let Ok(content) = read_source_normalized(&full_path) else {
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

/// Read a source file with LF-normalized line endings.
///
/// Solc reports AST `src` offsets against LF-normalized source text, so
/// slicing raw CRLF bytes would shift every offset after the first line break.
fn read_source_normalized(path: impl AsRef<Path>) -> std::io::Result<String> {
    let content = fs::read_to_string(path)?;
    Ok(content.replace('\r', ""))
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

/// Collect symbols referenced inside a [`FunctionCall`], including the called
/// declaration and all argument expressions.
fn collect_from_function_call(
    fc: &solc::ast::FunctionCall,
    seen_ids: &mut HashSet<i64>,
    results: &mut Vec<ResolvedSymbol>,
    ctx: &RefCtx,
) {
    // Extract the called function ID from the expression and descend into inner
    // expressions to handle chained calls (e.g. a().b().c()).
    match &*fc.expression {
        FunctionCallExpression::MemberAccess(ma) => {
            if let Some(id) = ma.referenced_declaration {
                resolve_and_add_symbol(id, seen_ids, results, ctx);
            }
            collect_from_expression(&ma.expression, seen_ids, results, ctx);
        }
        FunctionCallExpression::Identifier(id) => {
            if let Some(ref_id) = id.referenced_declaration {
                resolve_and_add_symbol(ref_id, seen_ids, results, ctx);
            }
        }
        FunctionCallExpression::FunctionCallOptions(fco) => {
            if let Some(id) = resolve_called_id_from_expr(&fco.expression) {
                resolve_and_add_symbol(id, seen_ids, results, ctx);
            }
            collect_from_expression(&fco.expression, seen_ids, results, ctx);
            for opt in &fco.options {
                collect_from_expression(opt, seen_ids, results, ctx);
            }
        }
        _ => {}
    }
    for arg in &fc.arguments {
        collect_from_expression(arg, seen_ids, results, ctx);
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-function-source")
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
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_execute.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_crlf_file() {
        let output = inspect("Crlf", "run").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_crlf.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_inherited_function() {
        let output = inspect("Inherited", "owner").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/inspect_resolves_inherited_function.txt"
            )
        );
    }

    #[test]
    fn inspect_labels_abstract_contract() {
        let output = inspect("AbstractHeading", "run").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_labels_abstract_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_constructor() {
        let output = inspect("SpecialFunctions", "constructor").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_constructor.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_receive() {
        let output = inspect("SpecialFunctions", "receive").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_receive.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_fallback() {
        let output = inspect("SpecialFunctions", "fallback").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_fallback.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_inherited_fallback() {
        let output = inspect("InheritedSpecialChild", "fallback").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_inherited_fallback.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_inherited_receive() {
        let output = inspect("InheritedSpecialChild", "receive").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_inherited_receive.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_inherited_override() {
        let output = inspect("InheritedOverrideChild", "_beforeFallback").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_inherited_override.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_interface_function_root() {
        let output = inspect("ITypeConversion", "doOther").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_interface_function_root.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_with_recursive_refs() {
        let output = inspect("Main", "_processData").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_with_recursive_refs.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_overloaded_with_params() {
        let output = inspect("Overloaded", "beforeTokenTransfer(address,address,uint256)").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_overloaded_with_params.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let err = inspect("Unknown", "function").unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_errors_for_unknown_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_unknown_function() {
        let err = inspect("Main", "unknownFunction").unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_errors_for_unknown_function.txt"
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
                "../../../fixtures/inspect-function-source/expected/run_errors_for_overloaded_function.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_natspec_block_comment() {
        let output = inspect("NatspecBlock", "compute").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_natspec_block_comment.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_user_defined_types_in_variable_declarations() {
        let output = inspect("TypeRefs", "passThrough").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_user_defined_types_in_variable_declarations.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_cross_file_type_references() {
        let output = inspect("CrossFileConsumer", "translate").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_cross_file_type_references.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_index_access_expressions() {
        let output = inspect("IndexAccessTest", "getItem").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_index_access_expressions.txt"
            )
        );
    }

    #[test]
    fn incremental_build_does_not_leak_symbols() {
        let output = inspect("Main", "execute").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/incremental_build_does_not_leak_symbols.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_function_return_types() {
        let output = inspect("ReturnTypeRef", "makeWidget").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_function_return_types.txt"
            )
        );
    }

    #[test]
    fn inspect_extracts_regular_block_comments() {
        let output = inspect("BlockComment", "getItem").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_extracts_regular_block_comments.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_path_qualified_contract() {
        let output = inspect("Main.sol:Main", "execute").unwrap();
        assert!(output.to_string().contains("# Main - execute Source Code"));
    }

    #[test]
    fn inspect_resolves_modifiers() {
        let output = inspect("ModifierRef", "increment").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_modifiers.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_cross_file_modifier() {
        let output = inspect("CrossFileModifierUser", "setValue").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_cross_file_modifier.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_inheritdoc() {
        let output = inspect("InheritdocUser", "doSomething").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_inheritdoc.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_cross_file_function_references() {
        let output = inspect("CrossFileFnUser", "process").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_cross_file_function_references.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_chained_function_calls() {
        let output = inspect("ChainedCall", "run").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_chained_function_calls.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_interface_inheritance() {
        let output = inspect("InheritanceUser", "useChild").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_interface_inheritance.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_interface_types_through_variable_declaration() {
        let output = inspect("InterfaceConsumer", "useTarget").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_interface_types_through_variable_declaration.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_interface_types_through_type_conversion() {
        let output = inspect("InterfaceConsumer", "useConversion").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_resolves_interface_types_through_type_conversion.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_public_getter() {
        let output = inspect("Getter", "project").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_shows_source_for_public_getter.txt"
            )
        );
    }

    #[test]
    fn inspect_does_not_include_unreferenced_library_symbols() {
        let output = inspect("LibraryScopeUser", "run").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/inspect-function-source/expected/run_does_not_include_unreferenced_library_symbols.txt"
            )
        );
    }
}
