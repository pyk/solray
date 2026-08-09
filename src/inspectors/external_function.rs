//! External function inspection for Foundry projects.
//!
//! [`ExternalFunctionInspector`] reads artifact files and produces structured
//! output with source locations, visibility, mutability, and modifiers for
//! every externally callable function.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use solc::abi::{Abi, AbiItem, Function as AbiFunction, StateMutability};
use solc::ast::{
    ContractDefinition, ContractDefinitionNode, ContractKind, FunctionDefinition, FunctionKind,
    SourceLocation, SourceUnit, SourceUnitNode, VariableDeclaration, Visibility,
};
use tracing::debug;

use crate::artifact_index::ArtifactIndex;
use crate::inspectors::artifact_id::ArtifactId;
use crate::project::Project;

/// Resolved source location of a function declaration.
///
/// Combines the source file path with the byte-offset location from the
/// Solidity AST and a pre-computed line number for display.
#[derive(Debug, Clone)]
pub struct SourceInfo {
    /// File path relative to the project root, e.g. `src/AccountV4.sol`.
    pub file: String,
    /// Byte-offset source location from the Solidity AST.
    pub location: SourceLocation,
    /// Pre-computed 1-based line number.
    pub line: usize,
}

/// Metadata about a single externally callable function.
#[derive(Debug, Clone)]
pub struct ExternalFunctionInfo {
    /// Display signature, e.g. `deposit(address[],uint256[],uint256[],uint256[])`.
    pub signature: String,
    /// Resolved source location, if known.
    pub source: Option<SourceInfo>,
    /// Solidity visibility.
    pub visibility: Visibility,
    /// State mutability.
    pub mutability: StateMutability,
    /// Modifier names (e.g. `["onlyOwner", "nonReentrant"]`).
    pub modifiers: Vec<String>,
}

/// The category of an externally callable function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionCategory {
    StateChanging,
    Callback,
    Special,
    ReadOnly,
}

/// The output of an [`ExternalFunctionInspector`] inspection.
#[derive(Debug)]
pub struct ExternalFunctionInspectorOutput {
    pub contract_name: String,
    /// The source file of the queried contract.
    pub source_file: Option<String>,
    pub state_changing: Vec<ExternalFunctionInfo>,
    pub callback: Vec<ExternalFunctionInfo>,
    pub special: Vec<ExternalFunctionInfo>,
    pub read_only: Vec<ExternalFunctionInfo>,
}

impl ExternalFunctionInspectorOutput {
    pub fn new(
        contract_name: &str,
        source_file: Option<String>,
        state_changing: Vec<ExternalFunctionInfo>,
        callback: Vec<ExternalFunctionInfo>,
        special: Vec<ExternalFunctionInfo>,
        read_only: Vec<ExternalFunctionInfo>,
    ) -> Self {
        Self {
            contract_name: contract_name.to_string(),
            source_file,
            state_changing,
            callback,
            special,
            read_only,
        }
    }
}

impl std::fmt::Display for ExternalFunctionInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.state_changing.len()
            + self.callback.len()
            + self.special.len()
            + self.read_only.len();

        writeln!(f, "Contract: {}", self.contract_name)?;
        if let Some(ref file) = self.source_file {
            writeln!(f, "File: {file}")?;
        }
        writeln!(f)?;
        writeln!(f, "Summary:")?;
        writeln!(f, "- {total} externally callable functions")?;
        writeln!(f, "- {} mutable functions", self.state_changing.len())?;
        writeln!(f, "- {} view functions", self.read_only.len())?;
        writeln!(f, "- {} callback functions", self.callback.len())?;
        writeln!(f, "- {} special functions", self.special.len())?;
        writeln!(f)?;

        write_section(f, "MUTABLE FUNCTIONS", &self.state_changing)?;
        write_section(f, "VIEW FUNCTIONS", &self.read_only)?;
        write_section(f, "CALLBACK FUNCTIONS", &self.callback)?;
        write_section(f, "SPECIAL FUNCTIONS", &self.special)?;

        Ok(())
    }
}

fn write_section(
    f: &mut std::fmt::Formatter<'_>,
    title: &str,
    funcs: &[ExternalFunctionInfo],
) -> std::fmt::Result {
    if funcs.is_empty() {
        return Ok(());
    }
    writeln!(f, "{title}")?;
    writeln!(f)?;
    for (i, info) in funcs.iter().enumerate() {
        writeln!(f, "{}. {}", i + 1, info.signature)?;
        if let Some(ref src) = info.source {
            writeln!(f, "   source: {}:{}", src.file, src.line)?;
        }
        writeln!(
            f,
            "   visibility: {}",
            format!("{:?}", info.visibility).to_lowercase()
        )?;
        writeln!(
            f,
            "   mutability: {}",
            format!("{:?}", info.mutability).to_lowercase()
        )?;
        if info.modifiers.is_empty() {
            writeln!(f, "   modifiers: none")?;
        } else {
            writeln!(f, "   modifiers: {}", info.modifiers.join(", "))?;
        }
        writeln!(f)?;
    }
    Ok(())
}

// Inspector

/// Inspect a Foundry project for a single contract's external functions.
pub struct ExternalFunctionInspector {
    project: Project,
}

impl ExternalFunctionInspector {
    /// Build an [`ExternalFunctionInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Inspect the external functions for the given [`ArtifactId`].
    pub fn inspect(&self, id: &ArtifactId) -> Result<ExternalFunctionInspectorOutput> {
        let artifact_path = self.resolve_artifact_path(id)?;
        let contract_name = id.name.clone();

        let artifact = FullArtifact::parse(&artifact_path)?;
        let abi = artifact.abi.clone().with_context(|| {
            format!("artifact `{}` is missing the ABI", artifact_path.display())
        })?;

        let proj_path = self.project.path().to_path_buf();
        let contract_source_file = artifact.source_file(&proj_path).and_then(|p| {
            p.strip_prefix(&proj_path)
                .ok()
                .map(|r| r.to_string_lossy().to_string())
        });

        let index = build_function_index(&self.project, &artifact)?;

        let mut state_changing = Vec::new();
        let mut callback = Vec::new();
        let mut special = Vec::new();
        let mut read_only = Vec::new();

        for item in &abi.items {
            match item {
                AbiItem::Function(function) => {
                    let signature = external_function_signature(function);
                    debug!(
                        contract = %contract_name,
                        function = %function.name,
                        param_count = function.inputs.len(),
                        "resolving external function source"
                    );
                    let info = index.resolve_function(
                        &contract_name,
                        &function.name,
                        &abi_param_signature(function),
                    );

                    let fi = info.cloned();
                    let (source, visibility, modifiers) = match fi {
                        Some(f) => {
                            let mods = f.modifier_strings();
                            let file = f.file.unwrap_or_default();
                            let line = f.line.unwrap_or(0);
                            let src_info = SourceInfo {
                                file,
                                location: f.location,
                                line,
                            };
                            (Some(src_info), f.visibility, mods)
                        }
                        None => (None, Visibility::External, vec![]),
                    };

                    let mutability = function.state_mutability.clone(); // checkrs: allow(clone_in_loops)
                    let func_info = ExternalFunctionInfo {
                        signature,
                        source,
                        visibility,
                        mutability,
                        modifiers,
                    };

                    if is_callback_function(&function.name) {
                        callback.push(func_info);
                    } else if function.state_mutability == StateMutability::View
                        || function.state_mutability == StateMutability::Pure
                    {
                        read_only.push(func_info);
                    } else {
                        state_changing.push(func_info);
                    }
                }
                AbiItem::Receive(_) => {
                    let info = resolve_special(
                        "receive",
                        &index,
                        &contract_name,
                        &FunctionKind::Receive,
                        &contract_source_file,
                        StateMutability::Payable,
                    );
                    special.push(info);
                }
                AbiItem::Fallback(_) => {
                    let info = resolve_special(
                        "fallback",
                        &index,
                        &contract_name,
                        &FunctionKind::Fallback,
                        &contract_source_file,
                        StateMutability::Nonpayable,
                    );
                    special.push(info);
                }
                _ => {}
            }
        }

        Ok(ExternalFunctionInspectorOutput::new(
            &contract_name,
            contract_source_file,
            state_changing,
            callback,
            special,
            read_only,
        ))
    }

    /// Resolve the artifact path for an `ArtifactId`.
    fn resolve_artifact_path(&self, id: &ArtifactId) -> Result<PathBuf> {
        match &id.file {
            Some(file) => {
                let direct = self
                    .project
                    .out_dir()
                    .join(file)
                    .join(format!("{}.json", id.name));
                if direct.exists() {
                    return Ok(direct);
                }
                let index = ArtifactIndex::build(self.project.out_dir());
                match index.find_by_source_path(file, &id.name) {
                    Some(path) => Ok(path),
                    None => Ok(direct),
                }
            }
            None => {
                let index = ArtifactIndex::build(self.project.out_dir());
                let candidates = index.get(&id.name).cloned().unwrap_or_default();
                debug!(name = %id.name, candidates = ?candidates, "looked up root artifact candidates");
                match candidates.len() {
                    0 => bail!("\"{}\" not found.", id.name),
                    1 => {
                        let path = candidates
                            .into_iter()
                            .next()
                            .context("expected one candidate but got none")?;
                        Ok(path)
                    }
                    n => {
                        let mut sorted = candidates;
                        sorted.sort();
                        let mut msg = format!(
                            "found {n} \"{}\"\n\nSelect one of the following:\n",
                            id.name
                        );
                        for candidate in &sorted {
                            let qualified = ArtifactIndex::qualified_name(candidate, &id.name);
                            msg.push_str(&format!(
                                "\nsolray inspect external-functions {qualified}"
                            ));
                        }
                        msg.push('\n');
                        bail!(msg);
                    }
                }
            }
        }
    }
}

// Function source index

/// Resolved source information for a single function or getter.
#[derive(Debug, Clone)]
struct FuncInfo {
    file: Option<String>,
    line: Option<usize>,
    location: SourceLocation,
    visibility: Visibility,
    modifiers: Vec<String>,
    signature: String,
    is_interface: bool,
}

impl FuncInfo {
    fn from_ast(
        fn_def: &FunctionDefinition,
        file: Option<String>,
        line: Option<usize>,
        is_interface: bool,
    ) -> Self {
        Self {
            location: fn_def.src.clone(),
            file,
            line,
            visibility: fn_def.visibility.clone(),
            modifiers: fn_def
                .modifiers
                .iter()
                .map(|m| m.modifier_name.name.to_string())
                .collect(),
            signature: ast_param_signature(&fn_def.parameters.parameters),
            is_interface,
        }
    }

    fn from_variable(
        var: &VariableDeclaration,
        file: Option<String>,
        line: Option<usize>,
        is_interface: bool,
    ) -> Self {
        Self {
            location: var.src.clone(),
            file,
            line,
            visibility: var.visibility.clone(),
            modifiers: vec![],
            signature: String::new(),
            is_interface,
        }
    }

    fn modifier_strings(&self) -> Vec<String> {
        self.modifiers.clone()
    }
}

/// Index of function definitions and public variable getters across artifacts.
struct FunctionIndex {
    /// Key: (contract_name, function_name) -> Vec of FuncInfo (for overloads).
    by_name: HashMap<(String, String), Vec<FuncInfo>>,
    /// Key: (contract_name, kind_name) -> FuncInfo for special functions (fallback, receive).
    by_kind: HashMap<(String, String), FuncInfo>,
    /// Contract names whose Solidity kind is `interface`.
    interface_contracts: HashSet<String>,
    /// File cache for computing line numbers.
    line_cache: HashMap<PathBuf, Vec<usize>>,
}

impl FunctionIndex {
    fn new() -> Self {
        Self {
            by_name: HashMap::new(),
            by_kind: HashMap::new(),
            interface_contracts: HashSet::new(),
            line_cache: HashMap::new(),
        }
    }

    fn register(
        &mut self,
        contract_name: &str,
        fn_def: &FunctionDefinition,
        source_file: Option<PathBuf>,
        project_root: &Path,
        is_interface: bool,
    ) {
        let display_file = source_file.as_ref().and_then(|p| {
            p.strip_prefix(project_root)
                .ok()
                .map(|r| r.to_string_lossy().to_string())
        });
        let line = source_file
            .as_ref()
            .and_then(|f| self.byte_offset_to_line(f, fn_def.src.offset));
        let info = FuncInfo::from_ast(fn_def, display_file, line, is_interface);
        match fn_def.kind {
            FunctionKind::Receive | FunctionKind::Fallback => {
                let kind_name = format!("{:?}", fn_def.kind);
                self.by_kind
                    .insert((contract_name.to_string(), kind_name), info);
            }
            _ => {
                self.by_name
                    .entry((contract_name.to_string(), fn_def.name.clone()))
                    .or_default()
                    .push(info);
            }
        }
    }

    fn register_variable(
        &mut self,
        contract_name: &str,
        var: &VariableDeclaration,
        source_file: Option<PathBuf>,
        project_root: &Path,
        is_interface: bool,
    ) {
        if var.visibility != Visibility::Public {
            return;
        }
        let display_file = source_file.as_ref().and_then(|p| {
            p.strip_prefix(project_root)
                .ok()
                .map(|r| r.to_string_lossy().to_string())
        });
        let line = source_file
            .as_ref()
            .and_then(|f| self.byte_offset_to_line(f, var.src.offset));
        let info = FuncInfo::from_variable(var, display_file, line, is_interface);
        self.by_name
            .entry((contract_name.to_string(), var.name.clone()))
            .or_default()
            .push(info);
    }

    fn resolve_function(
        &self,
        contract_name: &str,
        name: &str,
        signature: &str,
    ) -> Option<&FuncInfo> {
        // Prefer functions defined in the target contract.
        if let Some(infos) = self
            .by_name
            .get(&(contract_name.to_string(), name.to_string()))
        {
            if let Some(info) = infos.iter().find(|info| info.signature == signature) {
                return Some(info);
            }
            if let Some(info) = infos.first() {
                return Some(info);
            }
        }
        // Fall back to any registered contract (inherited functions).
        let candidates: Vec<&FuncInfo> = self
            .by_name
            .iter()
            .filter(|((_, n), _)| n == name)
            .flat_map(|(_, infos)| infos)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        // Flattened projects put interfaces and implementations in the same
        // file, so prefer the declaration kind that matches the queried
        // contract: concrete implementations for contracts, interface
        // declarations for interfaces.
        let prefer_interface = self.interface_contracts.contains(contract_name);
        let mut preferred: Vec<&FuncInfo> = candidates
            .iter()
            .copied()
            .filter(|info| info.is_interface == prefer_interface)
            .collect();
        preferred.sort_by_key(|info| (info.line.unwrap_or(0), info.location.offset));
        if let Some(info) = preferred
            .iter()
            .copied()
            .find(|info| info.signature == signature)
        {
            return Some(info);
        }
        // Public state-variable getters have an empty AST signature even
        // though their ABI signature includes the mapping key, so prefer any
        // matching-kind declaration before falling back to the other kind.
        if let Some(info) = preferred.first() {
            return Some(info);
        }
        let mut fallback: Vec<&FuncInfo> = candidates
            .iter()
            .copied()
            .filter(|info| info.is_interface != prefer_interface)
            .collect();
        fallback.sort_by_key(|info| (info.line.unwrap_or(0), info.location.offset));
        if let Some(info) = fallback
            .iter()
            .copied()
            .find(|info| info.signature == signature)
        {
            return Some(info);
        }
        fallback.first().copied()
    }

    fn resolve_by_kind(&self, contract_name: &str, kind: &FunctionKind) -> Option<&FuncInfo> {
        let kind_name = format!("{kind:?}");
        self.by_kind.get(&(contract_name.to_string(), kind_name))
    }

    fn byte_offset_to_line(&mut self, file: &Path, offset: usize) -> Option<usize> {
        if let Some(lines) = self.line_cache.get(file) {
            return Some(Self::offset_to_line(lines, offset));
        }
        let content = fs::read_to_string(file).ok()?;
        let lines: Vec<usize> = content
            .replace('\r', "")
            .as_bytes()
            .iter()
            .enumerate()
            .filter(|&(_, b)| *b == b'\n')
            .map(|(i, _)| i)
            .collect();
        let result = Self::offset_to_line(&lines, offset);
        self.line_cache.insert(file.to_path_buf(), lines);
        Some(result)
    }

    fn offset_to_line(newline_positions: &[usize], offset: usize) -> usize {
        newline_positions.partition_point(|&pos| pos < offset) + 1
    }
}

// Full artifact parsing

/// A Foundry artifact with ABI, AST, and raw metadata.
#[derive(Deserialize)]
struct FullArtifact {
    abi: Option<Abi>,
    ast: Option<SourceUnit>,
    #[serde(rename = "rawMetadata")]
    raw_metadata: Option<String>,
}

impl FullArtifact {
    fn parse(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse artifact `{}`", path.display()))
    }

    fn source_file(&self, project_root: &Path) -> Option<PathBuf> {
        let raw = self.raw_metadata.as_ref()?;
        let md: serde_json::Value = serde_json::from_str(raw).ok()?;
        let target = md.get("settings")?.get("compilationTarget")?.as_object()?;
        let (file, _) = target.iter().next()?;
        Some(project_root.join(file))
    }
}

// Building the function index

/// Build a [`FunctionIndex`] by scanning the target artifact and all
/// artifacts that could declare functions the contract inherits.
fn build_function_index(project: &Project, artifact: &FullArtifact) -> Result<FunctionIndex> {
    let mut index = FunctionIndex::new();

    if let Some(ref ast) = artifact.ast {
        let source_file = artifact.source_file(project.path());
        for node in &ast.nodes {
            let cd = match node {
                SourceUnitNode::ContractDefinition(cd) => cd,
                _ => continue,
            };
            index_contract(&mut index, cd, &source_file, project.path());
        }
    }

    let artifact_index = ArtifactIndex::build(project.out_dir());
    let target_path = artifact_path_name(artifact);
    for entry in artifact_index.all_entries() {
        if let Some(ref tp) = target_path
            && entry.ends_with(tp)
        {
            continue;
        }
        let other = match FullArtifact::parse(entry) {
            Ok(a) => a,
            _ => continue,
        };
        let source_file = other.source_file(project.path());
        let ast = match other.ast {
            Some(ref ast) => ast,
            None => continue,
        };
        for node in &ast.nodes {
            let cd = match node {
                SourceUnitNode::ContractDefinition(cd) => cd,
                _ => continue,
            };
            index_contract(&mut index, cd, &source_file, project.path());
        }
    }

    Ok(index)
}

/// Index all function definitions and public variables in a contract.
fn index_contract(
    index: &mut FunctionIndex,
    cd: &ContractDefinition,
    source_file: &Option<PathBuf>,
    project_root: &Path,
) {
    if cd.contract_kind == ContractKind::Interface {
        index.interface_contracts.insert(cd.name.clone());
    }
    for node in &cd.nodes {
        match node {
            ContractDefinitionNode::FunctionDefinition(fd)
                if fd.visibility == Visibility::External || fd.visibility == Visibility::Public =>
            {
                index.register(
                    &cd.name,
                    fd,
                    source_file.clone(),
                    project_root,
                    cd.contract_kind == ContractKind::Interface,
                );
            }
            ContractDefinitionNode::VariableDeclaration(var) => {
                index.register_variable(
                    &cd.name,
                    var,
                    source_file.clone(),
                    project_root,
                    cd.contract_kind == ContractKind::Interface,
                );
            }
            _ => {}
        }
    }
}

/// Derive the artifact's JSON filename (e.g. `Foo.sol/ContractA.json`).
fn artifact_path_name(artifact: &FullArtifact) -> Option<PathBuf> {
    let raw = artifact.raw_metadata.as_ref()?;
    let md: serde_json::Value = serde_json::from_str(raw).ok()?;
    let target = md.get("settings")?.get("compilationTarget")?.as_object()?;
    let (file, contract) = target.iter().next()?;
    let contract_name = contract.as_str()?;
    Some(PathBuf::from(file).join(format!("{contract_name}.json")))
}

// Helpers

fn external_function_signature(function: &AbiFunction) -> String {
    format!(
        "{}({})",
        function.name,
        function
            .inputs
            .iter()
            .map(|p| match &p.internal_type {
                Some(t) if t.starts_with("struct ") =>
                    t.rsplit('.').next().unwrap_or(t).to_string(),
                _ => p.r#type.clone(),
            })
            .collect::<Vec<String>>()
            .join(",")
    )
}

/// Canonical ABI parameter signature used to disambiguate overloads.
///
/// Uses the ABI `internalType` when available so overloads that differ only by
/// user-defined types (for example `skip(Alpha)` vs `skip(Beta)`) stay
/// distinguishable even though their canonical ABI type is `tuple`.
fn abi_param_signature(function: &AbiFunction) -> String {
    function
        .inputs
        .iter()
        .map(|p| normalize_type_key(p.internal_type.as_deref().unwrap_or(&p.r#type)))
        .collect::<Vec<String>>()
        .join(",")
}

/// AST parameter signature for a function declaration.
fn ast_param_signature(params: &[VariableDeclaration]) -> String {
    params
        .iter()
        .map(|p| normalize_type_key(p.type_descriptions.type_string.as_deref().unwrap_or("")))
        .collect::<Vec<String>>()
        .join(",")
}

/// Normalize solc/ABI type strings so both sides use the same key.
fn normalize_type_key(s: &str) -> String {
    s.replace(" memory", "")
        .replace(" calldata", "")
        .replace(" storage", "")
        .replace(" payable", "")
}

/// Returns `true` if the function name corresponds to a well-known ERC callback.
fn is_callback_function(name: &str) -> bool {
    matches!(
        name,
        "onERC721Received" | "onERC1155Received" | "onERC1155BatchReceived"
    )
}

fn resolve_special(
    name: &str,
    index: &FunctionIndex,
    contract_name: &str,
    kind: &FunctionKind,
    source_file: &Option<String>,
    mutability: StateMutability,
) -> ExternalFunctionInfo {
    let source = match index.resolve_by_kind(contract_name, kind) {
        Some(info) => {
            let file = info.file.clone().unwrap_or_default();
            let line = info.line.unwrap_or(0);
            Some(SourceInfo {
                file,
                location: info.location.clone(),
                line,
            })
        }
        None => source_file.clone().map(|file| SourceInfo {
            file,
            location: SourceLocation::default(),
            line: 0,
        }),
    };
    ExternalFunctionInfo {
        signature: format!("{name}()"),
        source,
        visibility: Visibility::External,
        mutability,
        modifiers: vec![],
    }
}

// Tests

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inspect-external-functions")
    }

    #[test]
    fn inspect_shows_external_functions_for_a_unique_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ContractB");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_shows_external_functions_for_a_unique_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_external_functions_for_path_qualified_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Foo.sol:ContractA");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_shows_external_functions_for_path_qualified_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Missing");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_errors_for_unknown_contract.txt"
            )
            .trim_end()
        );
    }

    #[test]
    fn inspect_lists_direct_receive_and_fallback() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("DirectFallback");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_lists_direct_receive_and_fallback.txt"
            )
        );
    }

    #[test]
    fn inspect_lists_inherited_receive_and_fallback() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ChildIsFallback");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_lists_inherited_receive_and_fallback.txt"
            )
        );
    }

    #[test]
    fn inspect_classifies_callbacks_with_source() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("CallbackReceiver");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_classifies_callbacks_with_source.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_inherited_functions() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ViewsChild");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_shows_source_for_inherited_functions.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_flattened_inherited_functions() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Child");
        // The fallback selection used HashMap iteration order before the fix,
        // so run repeatedly to exercise both orderings.
        for _ in 0..8 {
            let output = inspector.inspect(&id).unwrap().to_string();
            assert_eq!(
                output,
                include_str!(
                    "../../fixtures/inspect-external-functions/expected/inspect_shows_source_for_flattened_inherited_functions.txt"
                )
            );
        }
    }

    #[test]
    fn inspect_shows_source_for_inherited_interface_functions() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("IChildRouter");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_shows_source_for_inherited_interface_functions.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_overloaded_functions() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Overloaded");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_shows_source_for_overloaded_functions.txt"
            )
        );
    }

    #[test]
    fn inspect_shows_source_for_crlf_file() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Crlf");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_shows_source_for_crlf_file.txt"
            )
        );
    }

    #[test]
    fn inspect_errors_for_ambiguous_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ContractA");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_errors_for_ambiguous_contract.txt"
            )
        );
    }

    #[test]
    fn inspect_resolves_plain_name_ierc20() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("IERC20");
        let output = inspector.inspect(&id).unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../fixtures/inspect-external-functions/expected/inspect_resolves_plain_name_ierc20.txt"
            )
        );
    }

    #[test]
    fn artifact_id_parses_name_only() {
        let id = ArtifactId::new("MyContract");
        assert_eq!(id.name, "MyContract");
        assert_eq!(id.file, None);
    }

    #[test]
    fn artifact_id_parses_file_and_name() {
        let id = ArtifactId::new("Foo.sol:MyContract");
        assert_eq!(id.name, "MyContract");
        assert_eq!(id.file, Some("Foo.sol".to_string()));
    }
}
