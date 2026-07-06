//! External function inspection for Foundry projects.
//!
//! [`ExternalFunctionInspector`] reads artifact files and produces structured
//! output with source locations, visibility, mutability, and modifiers for
//! every externally callable function.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use solc::abi::{Abi, AbiItem, Function as AbiFunction, StateMutability};
use solc::ast::{
    ContractDefinition, ContractDefinitionNode, FunctionDefinition, SourceLocation, SourceUnit,
    SourceUnitNode, VariableDeclaration, Visibility,
};

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
    pub include_read_only: bool,
    pub state_changing: Vec<ExternalFunctionInfo>,
    pub callback: Vec<ExternalFunctionInfo>,
    pub special: Vec<ExternalFunctionInfo>,
    pub read_only: Vec<ExternalFunctionInfo>,
}

impl ExternalFunctionInspectorOutput {
    pub fn new(
        contract_name: &str,
        source_file: Option<String>,
        include_read_only: bool,
        state_changing: Vec<ExternalFunctionInfo>,
        callback: Vec<ExternalFunctionInfo>,
        special: Vec<ExternalFunctionInfo>,
        read_only: Vec<ExternalFunctionInfo>,
    ) -> Self {
        Self {
            contract_name: contract_name.to_string(),
            source_file,
            include_read_only,
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
        writeln!(
            f,
            "- {} state-changing functions",
            self.state_changing.len()
        )?;
        writeln!(f, "- {} callback functions", self.callback.len())?;
        writeln!(f, "- {} special functions", self.special.len())?;
        writeln!(f, "- {} read-only functions", self.read_only.len())?;
        writeln!(f)?;

        write_section(f, "STATE-CHANGING FUNCTIONS", &self.state_changing)?;
        write_section(f, "CALLBACK FUNCTIONS", &self.callback)?;
        write_section(f, "SPECIAL FUNCTIONS", &self.special)?;

        if self.include_read_only {
            write_section(f, "READ-ONLY FUNCTIONS", &self.read_only)?;
        } else if !self.read_only.is_empty() {
            writeln!(f, "READ-ONLY FUNCTIONS")?;
            writeln!(f)?;
            writeln!(f, "{} functions hidden.", self.read_only.len())?;
            writeln!(f)?;
            writeln!(f, "Run with --include-read-only to show them:")?;
            writeln!(f)?;
            writeln!(
                f,
                "    hawk inspect external-functions {} --include-read-only",
                self.contract_name
            )?;
            writeln!(f)?;
        }

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
    ///
    /// When `include_read_only` is `false`, read-only functions are hidden
    /// behind a summary message.
    pub fn inspect(
        &self,
        id: &ArtifactId,
        include_read_only: bool,
    ) -> Result<ExternalFunctionInspectorOutput> {
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

        let index = build_function_index(&self.project, &artifact, &id.name)?;

        let mut state_changing = Vec::new();
        let callback = Vec::new();
        let mut special = Vec::new();
        let mut read_only = Vec::new();

        for item in &abi.items {
            match item {
                AbiItem::Function(function) => {
                    let signature = external_function_signature(function);
                    let info = index
                        .resolve_function(&function.name, function.inputs.len())
                        .or_else(|| index.resolve_function_by_abi(function));

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

                    if function.state_mutability == StateMutability::View
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
                        &contract_source_file,
                        StateMutability::Payable,
                    );
                    special.push(info);
                }
                AbiItem::Fallback(_) => {
                    let info = resolve_special(
                        "fallback",
                        &index,
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
            include_read_only,
            state_changing,
            callback,
            special,
            read_only,
        ))
    }

    /// Resolve the artifact path for an `ArtifactId`.
    fn resolve_artifact_path(&self, id: &ArtifactId) -> Result<PathBuf> {
        match &id.file {
            Some(file) => Ok(self
                .project
                .out_dir()
                .join(file)
                .join(format!("{}.json", id.name))),
            None => {
                let index = ArtifactIndex::build(self.project.out_dir());
                let candidates = index.get(&id.name).cloned().unwrap_or_default();
                match candidates.len() {
                    0 => bail!("\"{}\" not found.", id.name),
                    1 => {
                        // checkrs: allow(unwrap_usage)
                        let path = candidates.into_iter().next().unwrap();
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
                            let parent = candidate
                                .parent()
                                .and_then(|p| p.file_name())
                                .and_then(|n| n.to_str())
                                .unwrap_or("");
                            msg.push_str(&format!(
                                "\nhawk inspect external-functions {parent}:{}",
                                id.name
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
}

impl FuncInfo {
    fn from_ast(fn_def: &FunctionDefinition, file: Option<String>, line: Option<usize>) -> Self {
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
        }
    }

    fn from_variable(var: &VariableDeclaration, file: Option<String>, line: Option<usize>) -> Self {
        Self {
            location: var.src.clone(),
            file,
            line,
            visibility: var.visibility.clone(),
            modifiers: vec![],
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
    /// File cache for computing line numbers.
    line_cache: HashMap<PathBuf, Vec<usize>>,
}

impl FunctionIndex {
    fn new() -> Self {
        Self {
            by_name: HashMap::new(),
            line_cache: HashMap::new(),
        }
    }

    fn register(
        &mut self,
        contract_name: &str,
        fn_def: &FunctionDefinition,
        source_file: Option<PathBuf>,
        project_root: &Path,
    ) {
        let display_file = source_file.as_ref().and_then(|p| {
            p.strip_prefix(project_root)
                .ok()
                .map(|r| r.to_string_lossy().to_string())
        });
        let line = source_file
            .as_ref()
            .and_then(|f| self.byte_offset_to_line(f, fn_def.src.offset));
        let info = FuncInfo::from_ast(fn_def, display_file, line);
        self.by_name
            .entry((contract_name.to_string(), fn_def.name.clone()))
            .or_default()
            .push(info);
    }

    fn register_variable(
        &mut self,
        contract_name: &str,
        var: &VariableDeclaration,
        source_file: Option<PathBuf>,
        project_root: &Path,
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
        let info = FuncInfo::from_variable(var, display_file, line);
        self.by_name
            .entry((contract_name.to_string(), var.name.clone()))
            .or_default()
            .push(info);
    }

    fn resolve_function(&self, name: &str, _param_count: usize) -> Option<&FuncInfo> {
        let mut candidates: Vec<&FuncInfo> = self
            .by_name
            .iter()
            .filter(|((_, n), _)| n == name)
            .flat_map(|(_, infos)| infos)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        Some(candidates.remove(0))
    }

    fn resolve_function_by_abi(&self, function: &AbiFunction) -> Option<&FuncInfo> {
        self.resolve_function(&function.name, function.inputs.len())
    }

    fn byte_offset_to_line(&mut self, file: &Path, offset: usize) -> Option<usize> {
        if let Some(lines) = self.line_cache.get(file) {
            return Some(Self::offset_to_line(lines, offset));
        }
        let content = fs::read_to_string(file).ok()?;
        let lines: Vec<usize> = content
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
        let content = fs::read_to_string(path.as_ref())?;
        Ok(serde_json::from_str(&content)?)
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
fn build_function_index(
    project: &Project,
    artifact: &FullArtifact,
    target_name: &str,
) -> Result<FunctionIndex> {
    let mut index = FunctionIndex::new();

    let mut relevant_contracts: Vec<String> = Vec::new();
    if let Some(ref ast) = artifact.ast {
        let source_file = artifact.source_file(project.path());
        for node in &ast.nodes {
            let cd = match node {
                SourceUnitNode::ContractDefinition(cd) => cd,
                _ => continue,
            };
            relevant_contracts.push(cd.name.clone()); // checkrs: allow(clone_in_loops)
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
            if relevant_contracts.contains(&cd.name) || target_name == cd.name {
                index_contract(&mut index, cd, &source_file, project.path());
            }
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
    for node in &cd.nodes {
        match node {
            ContractDefinitionNode::FunctionDefinition(fd)
                if fd.visibility == Visibility::External || fd.visibility == Visibility::Public =>
            {
                index.register(&cd.name, fd, source_file.clone(), project_root);
            }
            ContractDefinitionNode::VariableDeclaration(var) => {
                index.register_variable(&cd.name, var, source_file.clone(), project_root);
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
            .map(|p| p.r#type.as_str())
            .collect::<Vec<&str>>()
            .join(",")
    )
}

fn resolve_special(
    name: &str,
    _index: &FunctionIndex,
    source_file: &Option<String>,
    mutability: StateMutability,
) -> ExternalFunctionInfo {
    let source = source_file.clone().map(|file| SourceInfo {
        file,
        location: SourceLocation::default(),
        line: 0,
    });
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/external-functions")
    }

    #[test]
    fn inspect_shows_external_functions_for_a_unique_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ContractB");
        let output = inspector.inspect(&id, true).unwrap();
        let text = output.to_string();
        assert!(text.contains("Contract: ContractB"));
        assert!(text.contains("3 externally callable functions"));
        assert!(text.contains("charge()"));
        assert!(text.contains("count()"));
        assert!(text.contains("update(address)"));
    }

    #[test]
    fn inspect_shows_external_functions_for_path_qualified_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Foo.sol:ContractA");
        let output = inspector.inspect(&id, true).unwrap();
        let text = output.to_string();
        assert!(text.contains("Contract: ContractA"));
        assert!(text.contains("3 externally callable functions"));
        assert!(text.contains("entrypointOne(string)"));
        assert!(text.contains("payMe()"));
        assert!(text.contains("readOnly()"));
    }

    #[test]
    fn inspect_errors_for_unknown_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Missing");
        let err = inspector.inspect(&id, true).unwrap_err().to_string();
        assert_eq!(err, "\"Missing\" not found.");
    }

    #[test]
    fn inspect_lists_direct_receive_and_fallback() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("DirectFallback");
        let output = inspector.inspect(&id, true).unwrap();
        let text = output.to_string();
        assert!(text.contains("fallback()"));
        assert!(text.contains("receive()"));
        assert!(text.contains("doSomething()"));
    }

    #[test]
    fn inspect_lists_inherited_receive_and_fallback() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ChildIsFallback");
        let output = inspector.inspect(&id, true).unwrap();
        let text = output.to_string();
        assert!(text.contains("fallback()"));
        assert!(text.contains("receive()"));
        assert!(text.contains("childFunc()"));
        assert!(text.contains("parentFunc()"));
    }

    #[test]
    fn inspect_errors_for_ambiguous_contract() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ContractA");
        let err = inspector.inspect(&id, true).unwrap_err().to_string();
        assert_eq!(
            err,
            "found 2 \"ContractA\"\n\nSelect one of the following:\n\nhawk inspect external-functions Bar.sol:ContractA\nhawk inspect external-functions Foo.sol:ContractA\n"
        );
    }

    #[test]
    fn inspect_hides_read_only_by_default() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("ContractB");
        let output = inspector.inspect(&id, false).unwrap();
        let text = output.to_string();
        assert!(text.contains("1 functions hidden."));
        assert!(text.contains("--include-read-only"));
    }

    #[test]
    fn classification_with_include_read_only() {
        let inspector = ExternalFunctionInspector::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Foo.sol:ContractA");
        let output = inspector.inspect(&id, true).unwrap();
        assert_eq!(output.state_changing.len(), 2);
        assert_eq!(output.special.len(), 0);
        assert_eq!(output.read_only.len(), 1);
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
