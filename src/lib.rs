//! Hawk: inspect Foundry projects.

pub use call_graph::CallGraphResolver;
pub use inspectors::r#abstract::AbstractInspector;
pub use inspectors::r#abstract::AbstractInspectorOutput;
pub use inspectors::artifact_id::ArtifactId;
pub use inspectors::contract::ContractInspector;
pub use inspectors::contract::ContractInspectorOutput;
pub use inspectors::external_function::ExternalFunctionInspector;
pub use inspectors::external_function::ExternalFunctionInspectorOutput;
pub use inspectors::function_source::FunctionSourceInspector;
pub use inspectors::function_source::FunctionSourceInspectorOutput;
pub use inspectors::inheritance_graph::InheritanceGraphInspector;
pub use inspectors::inheritance_graph::InheritanceGraphInspectorOutput;
pub use inspectors::interface::InterfaceInspector;
pub use inspectors::interface::InterfaceInspectorOutput;
pub use inspectors::library::LibraryInspector;
pub use inspectors::library::LibraryInspectorOutput;
pub use inspectors::storage_layout::StorageLayoutId;
pub use inspectors::storage_layout::StorageLayoutInspector;
pub use inspectors::storage_layout::StorageLayoutInspectorOutput;
pub use project::Project;

pub mod artifact_index;
pub mod build_info;
pub mod call_graph;
pub mod commands;

pub mod inspectors;
pub mod project;
