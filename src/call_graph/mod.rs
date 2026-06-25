//! Call graph types for Solidity function call analysis.
//!
//! [`CallGraphNode`] represents a node in a call graph tree. [`CallGraphResolver`]
//! resolves call graphs from Foundry artifact files.

pub use function_id::FunctionID;
pub use function_index::FunctionIndex;
pub use node::CallGraphNode;
pub use resolved_call_graph::ResolvedCallGraph;
pub use resolver::CallGraphResolver;

pub mod function_id;
pub mod function_index;
pub mod node;
pub mod resolved_call_graph;
pub mod resolver;
