//! Solray: inspect Foundry projects.

pub use scanners::asset_transfers::AssetTransfer;
pub use scanners::asset_transfers::AssetTransferKind;
pub use scanners::asset_transfers::AssetTransferScanner;
pub use scanners::asset_transfers::AssetTransferScannerOutput;
pub use scanners::erc20_transfer_sink::Erc20TransferSink;
pub use scanners::erc20_transfer_sink::Erc20TransferSinkScanner;
pub use scanners::erc20_transfer_sink::Erc20TransferSinkScannerOutput;

pub use call_graph::CallGraph;
pub use call_graph::CallGraphNode;
pub use call_graph::FunctionId;
pub use inspectors::r#abstract::AbstractInspector;
pub use inspectors::r#abstract::AbstractInspectorOutput;
pub use inspectors::artifact_id::ArtifactId;
pub use inspectors::call_graph::CallGraphInspector;
pub use inspectors::call_graph::CallGraphInspectorOutput;
pub use inspectors::call_path::CallPathInspector;
pub use inspectors::call_path::CallPathInspectorOutput;
pub use inspectors::contract::ContractInspector;
pub use inspectors::contract::ContractInspectorOutput;
pub use inspectors::external_function::ExternalFunctionInfo;
pub use inspectors::external_function::ExternalFunctionInspector;
pub use inspectors::external_function::ExternalFunctionInspectorOutput;
pub use inspectors::external_function::FunctionCategory;
pub use inspectors::external_function::SourceInfo;
pub use inspectors::function_source::FunctionSourceInspector;
pub use inspectors::function_source::FunctionSourceInspectorOutput;
pub use inspectors::inheritance_graph::InheritanceGraphInspector;
pub use inspectors::inheritance_graph::InheritanceGraphInspectorOutput;
pub use inspectors::interface::InterfaceInspector;
pub use inspectors::interface::InterfaceInspectorOutput;
pub use inspectors::library::LibraryInspector;
pub use inspectors::library::LibraryInspectorOutput;
pub use inspectors::modifier::ModifierInfo;
pub use inspectors::modifier::ModifierInspector;
pub use inspectors::modifier::ModifierInspectorOutput;
pub use inspectors::storage_layout::StorageLayoutId;
pub use inspectors::storage_layout::StorageLayoutInspector;
pub use inspectors::storage_layout::StorageLayoutInspectorOutput;
pub use project::Project;

pub mod artifact_index;
pub mod build_info;
pub mod call_graph;

pub mod inspectors;
pub mod project;
pub mod scanners;
