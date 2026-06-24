//! Call graph types for Solidity function call analysis.
//!
//! [`CallGraphNode`] represents a node in a call graph tree. [`CallGraphLoader`]
//! resolves call graphs from Foundry artifact files.

pub use loader::CallGraphLoader;
pub use node::CallGraphNode;

pub mod loader;
pub mod node;
