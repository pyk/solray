//! Solray: a Solidity source code explorer for Foundry projects.
//!
//! Answers questions about contract codebases straight from `forge build`
//! artifacts:
//!
//! - Which contracts are deployable, abstract, interfaces, or libraries?
//! - What does the inheritance graph of a contract look like?
//! - Which functions are externally callable, and which modifiers apply?
//! - What storage slots does a contract use?
//! - What is the complete call graph or source of a function?
//! - Where are ERC20 transfers or asset transfers made?
//!
//! The crate is library-first: the CLI is a thin consumer of the public
//! types below. Every command is backed by an inspector or scanner type you
//! can drive programmatically.
//!
//! # Installation
//!
//! Install the CLI from crates.io:
//!
//! ```bash
//! cargo install solray
//! ```
//!
//! Or from source:
//!
//! ```bash
//! git clone https://github.com/pyk/solray.git
//! cd solray
//! make bin
//! ```
//!
//! # Prerequisites
//!
//! Solray works on Foundry-based projects. Build the project first so
//! artifacts exist in `out/`, and set `ast = true` in the default profile of
//! `foundry.toml`:
//!
//! ```toml
//! [profile.default]
//! ast = true
//! ```
//!
//! Storage layout inspection additionally requires `storageLayout` in
//! `extra_output`:
//!
//! ```toml
//! [profile.default]
//! extra_output = ["storageLayout"]
//! ```
//!
//! # Command-line usage
//!
//! The CLI has three commands: `inspect`, `scan`, and `gen`.
//!
//! Explore the structure of a project with `solray inspect`:
//!
//! ```bash
//! solray inspect contracts                       # list deployable contracts
//! solray inspect abstracts                       # list abstract contracts
//! solray inspect interfaces                      # list interfaces
//! solray inspect libraries                       # list libraries
//! solray inspect inheritance-graph Token         # show the inheritance graph
//! solray inspect external-functions Token        # list externally callable functions
//! solray inspect modifiers Token                 # list modifiers, including inherited
//! solray inspect storage-layout Token            # show the storage layout
//! solray inspect call-graph Token transfer       # show the call graph of a function
//! solray inspect call-path Token _burn           # show call paths to a function
//! solray inspect function-source Token transfer  # show the complete function source
//! ```
//!
//! Scan for patterns with `solray scan`:
//!
//! ```bash
//! solray scan erc20-transfer-sink # find ERC20 transfer call sites
//! solray scan asset-transfers     # find ERC20, ETH, and value transfers
//! ```
//!
//! Generate Solidity source with `solray gen`:
//!
//! ```bash
//! solray gen interface Token # generate an interface for a contract
//! ```
//!
//! All commands accept `--project` (defaults to the current directory). When
//! a contract name is ambiguous, use the `File.sol:Name` artifact ID syntax,
//! e.g. `solray inspect inheritance-graph "src/Token.sol:Token"`.
//!
//! # Library usage
//!
//! The public API is organized around types, not functions. Open a [`Project`]
//! and validate it, then drive the inspector or scanner that matches the
//! question you want answered:
//!
//! ```rust,no_run
//! use solray::ContractInspector;
//! use solray::Project;
//!
//! let project = Project::open("path/to/forge-project");
//! project.validate().expect("foundry.toml must set ast = true");
//!
//! let inspector = ContractInspector::new(project);
//! let output = inspector.inspect().expect("failed to inspect contracts");
//! println!("{output}");
//! ```
//!
//! The available inspectors, scanners, and generators are
//! [`AbstractInspector`], [`InterfaceInspector`], [`LibraryInspector`],
//! [`InheritanceGraphInspector`], [`ExternalFunctionInspector`],
//! [`ModifierInspector`], [`StorageLayoutInspector`], [`CallGraphInspector`],
//! [`CallPathInspector`], [`FunctionSourceInspector`], [`InterfaceGenerator`],
//! [`ERC20TransferSinkScanner`], and [`AssetTransferScanner`].

pub use artifact_index::ArtifactIndex;

pub use build_info::BuildInfo;

pub use scanners::asset_transfers::AssetTransfer;
pub use scanners::asset_transfers::AssetTransferKind;
pub use scanners::asset_transfers::AssetTransferScanner;
pub use scanners::asset_transfers::AssetTransferScannerOutput;
pub use scanners::erc20_transfer_sink::ERC20TransferSink;
pub use scanners::erc20_transfer_sink::ERC20TransferSinkScanner;
pub use scanners::erc20_transfer_sink::ERC20TransferSinkScannerOutput;

pub use call_graph::CallGraph;
pub use call_graph::CallGraphNode;
pub use call_graph::CallPaths;
pub use call_graph::FunctionId;
pub use generators::interface::InterfaceGenerator;
pub use generators::interface::InterfaceGeneratorOutput;
pub use inspectors::r#abstract::Abstract;
pub use inspectors::r#abstract::AbstractInspector;
pub use inspectors::r#abstract::AbstractInspectorOutput;
pub use inspectors::artifact_id::ArtifactId;
pub use inspectors::call_graph::CallGraphInspector;
pub use inspectors::call_graph::CallGraphInspectorOutput;
pub use inspectors::call_path::CallPathInspector;
pub use inspectors::call_path::CallPathInspectorOutput;
pub use inspectors::contract::Contract;
pub use inspectors::contract::ContractInspector;
pub use inspectors::contract::ContractInspectorOutput;
pub use inspectors::external_function::ExternalFunctionInfo;
pub use inspectors::external_function::ExternalFunctionInspector;
pub use inspectors::external_function::ExternalFunctionInspectorOutput;
pub use inspectors::external_function::FunctionCategory;
pub use inspectors::external_function::SourceInfo;
pub use inspectors::function_source::FunctionSourceInspector;
pub use inspectors::function_source::FunctionSourceInspectorOutput;
pub use inspectors::function_source::ResolvedSymbol;
pub use inspectors::inheritance_graph::InheritanceGraphInspector;
pub use inspectors::inheritance_graph::InheritanceGraphInspectorOutput;
pub use inspectors::interface::Interface;
pub use inspectors::interface::InterfaceInspector;
pub use inspectors::interface::InterfaceInspectorOutput;
pub use inspectors::library::Library;
pub use inspectors::library::LibraryInspector;
pub use inspectors::library::LibraryInspectorOutput;
pub use inspectors::modifier::ModifierInfo;
pub use inspectors::modifier::ModifierInspector;
pub use inspectors::modifier::ModifierInspectorOutput;
pub use inspectors::storage_layout::StorageEntry;
pub use inspectors::storage_layout::StorageLayout;
pub use inspectors::storage_layout::StorageLayoutId;
pub use inspectors::storage_layout::StorageLayoutInspector;
pub use inspectors::storage_layout::StorageLayoutInspectorOutput;
pub use inspectors::storage_layout::StorageType;
pub use project::Declaration;
pub use project::DeclarationKind;
pub use project::Project;
pub use project::ProjectDirectories;

mod artifact_index;
mod build_info;
mod call_graph;

mod generators;
mod inspectors;
mod project;
mod scanners;
