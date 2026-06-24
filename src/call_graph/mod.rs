//! Call graph types for Solidity function call analysis.
//!
//! [`CallGraphNode`] represents a node in a call graph tree. [`CallGraphResolver`]
//! resolves call graphs from Foundry artifact files.

pub use node::CallGraphNode;
pub use resolver::CallGraphResolver;

pub mod node;
pub mod resolver;
