//! Hawk: inspect Foundry projects.

pub use call_graph::CallGraphResolver;
pub use entrypoints::EntrypointsResolver;
pub use inheritance_resolver::InheritanceResolver;
pub use inspectors::r#abstract::AbstractInspector;
pub use inspectors::r#abstract::AbstractInspectorOutput;
pub use inspectors::library::LibraryInspector;
pub use inspectors::library::LibraryInspectorOutput;
pub use project::Project;
pub use storage_layout::StorageLayoutResolver;

pub mod artifact_index;
pub mod build_info;
pub mod call_graph;
pub mod commands;
pub mod entrypoints;
pub mod inheritance;
pub mod inheritance_resolver;
pub mod inspectors;
pub mod project;
pub mod source_graph;
pub mod storage_layout;
pub mod symbol_index;
