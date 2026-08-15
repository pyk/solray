//! Solidity interface generation for Foundry projects.
//!
//! [`InterfaceGenerator`] reads a contract's ABI from its artifact and emits
//! a Solidity interface with every externally callable function, plus the
//! structs, enums, and user-defined value types those functions reference.
//! The referenced types are declared inline so the interface is self-contained.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use solc::abi::{Abi, Component, Function as AbiFunction, Item, Param, StateMutability};
use solc::ast::{ContractDefinitionNode, SourceUnit, SourceUnitNode};

use crate::artifact_index::ArtifactIndex;
use crate::inspectors::artifact_id::ArtifactId;
use crate::project::Project;

/// The output of an [`InterfaceGenerator`] run.
#[derive(Debug)]
pub struct InterfaceGeneratorOutput {
    contract_name: String,
    interface_name: String,
    source: String,
}

impl InterfaceGeneratorOutput {
    /// Create a new [`InterfaceGeneratorOutput`].
    fn new(contract_name: &str, interface_name: &str, source: impl Into<String>) -> Self {
        Self {
            contract_name: contract_name.to_string(),
            interface_name: interface_name.to_string(),
            source: source.into(),
        }
    }

    /// The contract the interface was generated from, e.g. `Pool`.
    pub fn contract_name(&self) -> &str {
        &self.contract_name
    }

    /// The generated interface name, e.g. `IPool`.
    pub fn interface_name(&self) -> &str {
        &self.interface_name
    }
}

impl fmt::Display for InterfaceGeneratorOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

/// Generate a Solidity interface for a single contract.
pub struct InterfaceGenerator {
    project: Project,
}

impl InterfaceGenerator {
    /// Build an [`InterfaceGenerator`] for the given project.
    pub fn new(project: Project) -> Self {
        Self { project }
    }

    /// Return the project root path.
    pub fn project_path(&self) -> &Path {
        self.project.path()
    }

    /// Generate the Solidity interface for the given [`ArtifactId`].
    ///
    /// The interface is derived from the artifact's ABI, so inherited
    /// functions and public variable getters are included automatically.
    pub fn generate(&self, id: &ArtifactId) -> Result<InterfaceGeneratorOutput> {
        let artifact_path = self.resolve_artifact_path(id)?;
        let artifact = Artifact::parse(&artifact_path)?;
        let abi = artifact.abi.with_context(|| {
            format!("artifact `{}` is missing the ABI", artifact_path.display())
        })?;
        let enums = enum_members(artifact.ast);

        let interface_name = format!("I{}", id.name);
        let source = render_interface(&interface_name, &abi, &enums, &self.project);
        Ok(InterfaceGeneratorOutput::new(
            &id.name,
            &interface_name,
            source,
        ))
    }

    /// Resolve the artifact path for an [`ArtifactId`].
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
                if let Some(path) = index.find_by_source_path(file, &id.name) {
                    return Ok(path);
                }
                ensure!(direct.exists(), "\"{}\" not found.", id.name);
                Ok(direct)
            }
            None => {
                let index = ArtifactIndex::build(self.project.out_dir());
                let candidates = index.get(&id.name).cloned().unwrap_or_default();
                match candidates.len() {
                    0 => bail!("\"{}\" not found.", id.name),
                    1 => {
                        let path = candidates
                            .into_iter()
                            .next()
                            .context("expected one candidate but got none")?;
                        Ok(path)
                    }
                    _ => bail!("{}", format_ambiguity_error(&candidates, &id.name)),
                }
            }
        }
    }
}

/// Collects user-defined types referenced by ABI parameters while rendering
/// their Solidity type names.
struct TypeResolver<'a> {
    /// Enum definitions referenced by ABI parameters, keyed by enum name.
    enums: BTreeMap<String, EnumType>,
    /// User-defined value type names mapped to their underlying type.
    udts: BTreeMap<String, String>,
    /// Struct definitions, keyed by struct name.
    structs: BTreeMap<String, StructType>,
    /// Enum member lists from the artifact AST, keyed by enum name.
    enum_defs: BTreeMap<String, Vec<String>>,
    /// Mapping from a qualified type path (e.g. `AgentInfo.Info`) to the
    /// unique local name used in the generated interface.
    type_names: BTreeMap<String, String>,
    /// Local type names already assigned in the generated interface.
    used_names: BTreeSet<String>,
    /// Project used to look up enum members in the declaring artifact.
    project: &'a Project,
    /// Cached enum member lookups keyed by qualified enum path.
    enum_lookup_cache: BTreeMap<String, Option<Vec<String>>>,
}

impl<'a> TypeResolver<'a> {
    fn new(enum_defs: BTreeMap<String, Vec<String>>, project: &'a Project) -> Self {
        Self {
            enums: BTreeMap::new(),
            udts: BTreeMap::new(),
            structs: BTreeMap::new(),
            enum_defs,
            type_names: BTreeMap::new(),
            used_names: BTreeSet::new(),
            project,
            enum_lookup_cache: BTreeMap::new(),
        }
    }

    /// Register any user-defined types in a function parameter and return its
    /// rendered type name.
    fn register_param(&mut self, param: &Param) -> String {
        self.register_type(
            &param.r#type,
            param.internal_type.as_deref(),
            param.components.as_deref(),
        )
    }

    /// Register any user-defined types in a struct component and return its
    /// rendered type name.
    fn register_component(&mut self, component: &Component) -> String {
        self.register_type(
            &component.r#type,
            component.internal_type.as_deref(),
            component.components.as_deref(),
        )
    }

    /// Resolve an ABI type into a Solidity type name, registering any
    /// user-defined type it references.
    fn register_type(
        &mut self,
        canonical: &str,
        internal: Option<&str>,
        components: Option<&[Component]>,
    ) -> String {
        let Some(internal) = internal else {
            return canonical.to_string();
        };

        if let Some(struct_path) = internal.strip_prefix("struct ") {
            let (name, suffix) = split_array_suffix(struct_path);
            let local_name = self.local_type_name(name);
            let members = components
                .map(|components| {
                    components
                        .iter()
                        .cloned()
                        .map(|component| {
                            let r#type = self.register_component(&component);
                            StructMember {
                                name: component.name,
                                r#type,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.structs
                .entry(local_name.clone())
                .or_insert_with(|| StructType {
                    name: local_name.clone(),
                    members,
                });
            return format!("{local_name}{suffix}");
        }

        if let Some(enum_path) = internal.strip_prefix("enum ") {
            let (name, suffix) = split_array_suffix(enum_path);
            let local_name = self.local_type_name(name);
            let members = self.enum_members_for(name);
            self.enums
                .entry(local_name.clone())
                .or_insert_with(|| EnumType {
                    name: local_name.clone(),
                    members,
                    canonical: base_canonical_type(canonical).to_string(),
                });
            return format!("{local_name}{suffix}");
        }

        if is_builtin_type(internal) {
            return internal.to_string();
        }

        if internal.starts_with("contract ")
            || internal.starts_with("interface ")
            || internal.starts_with("library ")
        {
            // Contract types are not declared in interfaces; the ABI encodes
            // them as `address`.
            return canonical.to_string();
        }

        if !internal.contains(' ') && is_value_type(canonical) {
            let (name, suffix) = split_array_suffix(internal);
            let local_name = self.local_type_name(name);
            self.udts
                .entry(local_name.clone())
                .or_insert_with(|| base_canonical_type(canonical).to_string());
            return format!("{local_name}{suffix}");
        }

        internal.to_string()
    }

    /// Assign a valid, unique local Solidity identifier to a type path such
    /// as `AgentInfo.Info` or `IPayment.Proof`. Qualified paths cannot be
    /// declared inside an interface, so the last path segment is used as the
    /// local name and, when that name is already taken, the sanitized
    /// qualifier is prepended.
    fn local_type_name(&mut self, full_name: &str) -> String {
        if let Some(name) = self.type_names.get(full_name) {
            return name.clone();
        }
        let (qualifier, last) = full_name.rsplit_once('.').unwrap_or(("", full_name));
        let mut candidate = last.to_string();
        if self.used_names.contains(&candidate) {
            let base = if qualifier.is_empty() {
                candidate
            } else {
                format!("{}{}", sanitize_identifier(qualifier), last)
            };
            candidate = base.clone();
            let mut counter = 2;
            while self.used_names.contains(&candidate) {
                candidate = format!("{base}_{counter}");
                counter += 1;
            }
        }
        self.used_names.insert(candidate.clone());
        self.type_names
            .insert(full_name.to_string(), candidate.clone());
        candidate
    }

    /// Collect the members of an enum referenced by ABI parameters.
    ///
    /// Members come from the queried artifact's AST first (`enum_defs`);
    /// qualified enums such as `EmergencyPause.Level` are resolved from the
    /// artifact of the library that declares them.
    fn enum_members_for(&mut self, full_name: &str) -> Vec<String> {
        if let Some(members) = self.enum_defs.get(full_name) {
            return members.clone();
        }
        let Some((qualifier, name)) = full_name.rsplit_once('.') else {
            return Vec::new();
        };
        if let Some(cached) = self.enum_lookup_cache.get(full_name) {
            return cached.clone().unwrap_or_default();
        }
        let members = load_enum_members(self.project, qualifier, name);
        self.enum_lookup_cache
            .insert(full_name.to_string(), members.clone());
        members.unwrap_or_default()
    }
}

/// A struct definition recovered from ABI tuple components.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StructType {
    name: String,
    members: Vec<StructMember>,
}

/// A single member of a struct definition.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StructMember {
    name: String,
    r#type: String,
}

/// An enum definition referenced by ABI parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EnumType {
    name: String,
    members: Vec<String>,
    canonical: String,
}

/// Render the complete Solidity interface source for an ABI.
fn render_interface(
    interface_name: &str,
    abi: &Abi,
    enum_defs: &BTreeMap<String, Vec<String>>,
    project: &Project,
) -> String {
    let mut resolver = TypeResolver::new(enum_defs.clone(), project);
    let mut functions: Vec<&AbiFunction> = abi
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some(function),
            _ => None,
        })
        .collect();
    functions.sort_by(|a, b| a.name.cmp(&b.name));

    let mut rendered_functions = Vec::with_capacity(functions.len());
    for function in &functions {
        rendered_functions.push(render_function(&mut resolver, function));
    }

    let mut out = String::new();
    out.push_str("// SPDX-License-Identifier: MIT\n");
    out.push_str("pragma solidity ^0.8.0;\n");
    out.push_str(&format!("\ninterface {interface_name} {{\n"));

    for enum_type in resolver.enums.values() {
        if enum_type.members.is_empty() {
            out.push_str(&format!(
                "    type {} is {};\n",
                enum_type.name, enum_type.canonical
            ));
        } else {
            out.push_str(&render_enum(enum_type));
        }
    }
    for (name, canonical) in &resolver.udts {
        out.push_str(&format!("    type {name} is {canonical};\n"));
    }
    let has_aliases = !resolver.enums.is_empty() || !resolver.udts.is_empty();
    if has_aliases {
        out.push('\n');
    }

    let structs: Vec<&StructType> = resolver.structs.values().collect();
    for (i, struct_type) in structs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&render_struct(struct_type));
    }

    let has_types = has_aliases || !structs.is_empty();
    if has_types && !rendered_functions.is_empty() {
        out.push('\n');
    }
    if !rendered_functions.is_empty() {
        for function in rendered_functions {
            out.push_str(&function);
        }
    }

    out.push_str("}\n");
    out
}

/// Render an enum definition with 8-space member indentation.
fn render_enum(enum_type: &EnumType) -> String {
    let mut out = format!("    enum {} {{\n", enum_type.name);
    for (i, member) in enum_type.members.iter().enumerate() {
        out.push_str(&format!("        {member}"));
        if i + 1 < enum_type.members.len() {
            out.push(',');
            out.push('\n');
        }
    }
    out.push('\n');
    out.push_str("    }\n");
    out
}

/// Render a struct definition with 4-space member indentation.
fn render_struct(struct_type: &StructType) -> String {
    let mut out = format!("    struct {} {{\n", struct_type.name);
    for member in &struct_type.members {
        out.push_str(&format!("        {} {};\n", member.r#type, member.name));
    }
    out.push_str("    }\n");
    out
}

/// Render a single ABI function as an interface function declaration.
fn render_function(resolver: &mut TypeResolver, function: &AbiFunction) -> String {
    let params = render_params(resolver, &function.inputs);
    let mut line = format!(
        "    function {}({}) external{}",
        function.name,
        params,
        mutability_suffix(&function.state_mutability)
    );
    if !function.outputs.is_empty() {
        let returns = render_params(resolver, &function.outputs);
        line.push_str(&format!(" returns ({returns})"));
    }
    line.push_str(";\n");
    line
}

/// Render ABI parameters as a comma-separated Solidity parameter list.
fn render_params(resolver: &mut TypeResolver, params: &[Param]) -> String {
    params
        .iter()
        .map(|param| {
            let ty = resolver.register_param(param);
            let location = if needs_data_location(&param.r#type) {
                " memory"
            } else {
                ""
            };
            if param.name.is_empty() {
                format!("{ty}{location}")
            } else {
                format!("{ty}{location} {}", param.name)
            }
        })
        .collect::<Vec<String>>()
        .join(", ")
}

/// Return `true` if the canonical ABI type requires a data location in a
/// function parameter or return declaration.
fn needs_data_location(canonical_type: &str) -> bool {
    canonical_type.starts_with("tuple")
        || canonical_type.ends_with(']')
        || canonical_type == "string"
        || canonical_type == "bytes"
}

/// Return `true` if the ABI type is an elementary (value) type.
fn is_value_type(canonical_type: &str) -> bool {
    let base = base_canonical_type(canonical_type);
    !base.starts_with("tuple") && !base.ends_with(']') && base != "string" && base != "bytes"
}

/// Strip any array suffix from a canonical ABI type.
fn base_canonical_type(canonical_type: &str) -> &str {
    canonical_type.split('[').next().unwrap_or(canonical_type)
}

/// Split a type name into its base name and array suffix (e.g.
/// `Deposit[2][]` -> (`Deposit`, `[2][]`)).
fn split_array_suffix(name: &str) -> (&str, &str) {
    match name.find('[') {
        Some(index) => (&name[..index], &name[index..]),
        None => (name, ""),
    }
}

/// Return `true` if an `internalType` is a Solidity built-in type rather than
/// a user-defined value type.
fn is_builtin_type(internal: &str) -> bool {
    let base = base_canonical_type(internal);
    matches!(
        base,
        "address" | "bool" | "string" | "bytes" | "function" | "mapping"
    ) || (base.starts_with("uint") && base[4..].chars().all(|c| c.is_ascii_digit()))
        || (base.starts_with("int") && base[3..].chars().all(|c| c.is_ascii_digit()))
        || (base.starts_with("bytes") && base[5..].chars().all(|c| c.is_ascii_digit()))
        || ((base.starts_with("fixed") || base.starts_with("ufixed"))
            && base[5..].chars().all(|c| c.is_ascii_digit() || c == 'x'))
}

/// Render the state mutability suffix for an interface function declaration.
///
/// `nonpayable` is the default for interface functions and is omitted.
fn mutability_suffix(mutability: &StateMutability) -> &'static str {
    match mutability {
        StateMutability::Pure => " pure",
        StateMutability::View => " view",
        StateMutability::Payable => " payable",
        StateMutability::Nonpayable => "",
    }
}

/// Format an ambiguity error message listing the artifact candidates.
fn format_ambiguity_error(candidates: &[PathBuf], contract_name: &str) -> String {
    let mut sorted = candidates.to_vec();
    sorted.sort();

    let mut msg = format!(
        "found {} \"{}\"\n\nSelect one of the following:\n",
        sorted.len(),
        contract_name
    );
    for candidate in &sorted {
        let qualified = ArtifactIndex::qualified_name(candidate, contract_name);
        msg.push_str(&format!("\nsolray gen interface {qualified}"));
    }
    msg.push('\n');
    msg
}

/// Minimal artifact wrapper for reading the ABI.
#[derive(Deserialize)]
struct Artifact {
    #[serde(default)]
    ast: Option<SourceUnit>,
    abi: Option<Abi>,
}

impl Artifact {
    fn parse(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse artifact `{}`", path.display()))
    }
}

/// Collect enum member lists from an artifact AST, including enums declared
/// at the source-unit level and inside contracts.
fn enum_members(ast: Option<SourceUnit>) -> BTreeMap<String, Vec<String>> {
    let Some(ast) = ast else {
        return BTreeMap::new();
    };

    let mut enums = BTreeMap::new();
    for node in ast.nodes {
        match node {
            SourceUnitNode::EnumDefinition(enum_def) => {
                enums.insert(
                    enum_def.name,
                    enum_def.members.into_iter().map(|m| m.name).collect(),
                );
            }
            SourceUnitNode::ContractDefinition(contract) => {
                for node in contract.nodes {
                    if let ContractDefinitionNode::EnumDefinition(enum_def) = node {
                        enums.insert(
                            enum_def.name,
                            enum_def
                                .members
                                .into_iter()
                                .map(|member| member.name)
                                .collect(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    enums
}

/// Read the members of the enum `name` declared by the contract or library
/// `qualifier` from its artifact AST.
fn load_enum_members(project: &Project, qualifier: &str, name: &str) -> Option<Vec<String>> {
    let index = ArtifactIndex::build(project.out_dir());
    let candidates = index.get(qualifier)?;
    for path in candidates {
        let artifact = Artifact::parse(path).ok()?;
        let Some(ast) = artifact.ast else {
            continue;
        };
        for node in ast.nodes {
            match node {
                SourceUnitNode::EnumDefinition(enum_def) if enum_def.name == name => {
                    return Some(enum_def.members.into_iter().map(|m| m.name).collect());
                }
                SourceUnitNode::ContractDefinition(contract) if contract.name == qualifier => {
                    for node in contract.nodes {
                        if let ContractDefinitionNode::EnumDefinition(enum_def) = node
                            && enum_def.name == name
                        {
                            return Some(enum_def.members.into_iter().map(|m| m.name).collect());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Strip everything but identifier characters from a type qualifier so it
/// can be embedded in a generated local name.
fn sanitize_identifier(qualifier: &str) -> String {
    qualifier
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::project::Project;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/gen-interface")
    }

    fn ambiguous_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/inspect-inheritance-graph-ambiguous")
    }

    #[test]
    fn generate_interface_for_pool() {
        let generator = InterfaceGenerator::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Pool");
        let output = generator.generate(&id).unwrap();
        assert_eq!(output.interface_name(), "IPool");
        assert_eq!(
            output.to_string(),
            include_str!("../../fixtures/gen-interface/expected/IPool.txt")
        );
    }

    #[test]
    fn generate_interface_for_library_qualified_types() {
        let generator = InterfaceGenerator::new(Project::open(fixture_path()));
        let id = ArtifactId::new("TypeHolder");
        let output = generator.generate(&id).unwrap();
        assert_eq!(output.interface_name(), "ITypeHolder");
        assert_eq!(
            output.to_string(),
            include_str!("../../fixtures/gen-interface/expected/ITypeHolder.txt")
        );
    }

    #[test]
    fn generate_interface_for_spol() {
        let generator = InterfaceGenerator::new(Project::open(fixture_path()));
        let id = ArtifactId::new("sPOL");
        let output = generator.generate(&id).unwrap();
        assert_eq!(output.interface_name(), "IsPOL");
        assert_eq!(
            output.to_string(),
            include_str!("../../fixtures/gen-interface/expected/IsPOL.txt")
        );
    }

    #[test]
    fn generate_interface_errors_for_unknown_contract() {
        let generator = InterfaceGenerator::new(Project::open(fixture_path()));
        let id = ArtifactId::new("Missing");
        let err = generator.generate(&id).unwrap_err().to_string();
        assert_eq!(err, "\"Missing\" not found.");
    }

    #[test]
    fn generate_interface_errors_for_ambiguous_contract() {
        let generator = InterfaceGenerator::new(Project::open(ambiguous_fixture_path()));
        let id = ArtifactId::new("Dupe");
        let err = generator.generate(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            "found 2 \"Dupe\"\n\nSelect one of the following:\n\n\
             solray gen interface src/Dupe.sol:Dupe\n\
             solray gen interface src/lib/Dupe.sol:Dupe\n"
        );
    }
}
