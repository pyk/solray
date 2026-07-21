//! Call graph inspection for Foundry projects.
//!
//! [`CallGraphInspector`] reads a contract artifact, finds the specified
//! function, and produces a tree showing every function it calls recursively.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::call_graph::{CallGraph, CallGraphNode, FunctionId};
use crate::inspectors::call_graph::source_renderer::offset_to_line_range;
use crate::project::Project;

pub mod source_renderer;

/// The output of a [`CallGraphInspector`] inspection.
#[derive(Debug)]
pub struct CallGraphInspectorOutput {
    root: CallGraphNode,
    project_root: PathBuf,
}

impl CallGraphInspectorOutput {
    /// Create a new [`CallGraphInspectorOutput`] from a root call graph node.
    pub fn new(root: CallGraphNode, project_root: PathBuf) -> Self {
        Self { root, project_root }
    }
}

impl std::fmt::Display for CallGraphInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sources = self.root.flatten_sources();
        let cwd = std::env::current_dir().unwrap_or_else(|_| self.project_root.clone());
        let project_abs =
            std::path::absolute(&self.project_root).unwrap_or_else(|_| self.project_root.clone());

        let mut line_maps: HashMap<PathBuf, Vec<usize>> = HashMap::new();

        writeln!(f, "Call graph:\n")?;
        write!(f, "{}", self.root)?;
        writeln!(f, "\nResolved from {} sources:\n", sources.len())?;

        for (i, (file, src)) in sources.iter().enumerate() {
            let full_path = project_abs.join(file);
            let rel_path = full_path.strip_prefix(&cwd).unwrap_or(&full_path);
            let line_range = offset_to_line_range(&full_path, src, &mut line_maps);
            writeln!(f, "{}. {}#{}", i + 1, rel_path.display(), line_range)?;
        }

        Ok(())
    }
}

/// Inspect a Foundry project for the call graph of a single function.
pub struct CallGraphInspector {
    engine: CallGraph,
}

impl CallGraphInspector {
    /// Build a [`CallGraphInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self {
            engine: CallGraph::new(project),
        }
    }

    /// Inspect the call graph for the given [`FunctionId`].
    pub fn inspect(&self, id: &FunctionId) -> Result<CallGraphInspectorOutput> {
        // Resolve the artifact and check for ambiguity before building the tree.
        let (_, ambiguity_candidates) = self
            .engine
            .resolve_artifact_with_candidates(id.artifact_id())?;

        let root = self
            .engine
            .build_call_tree(id, ambiguity_candidates.as_deref())?;
        let project_root = self.engine.project_root().to_path_buf();
        Ok(CallGraphInspectorOutput::new(root, project_root))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::inspectors::artifact_id::ArtifactId;
    use crate::project::Project;

    fn fixture_project() -> Project {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/calls");
        Project::open(root)
    }

    fn fixture_ambiguous_project() -> Project {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/inheritance-graph-ambiguous");
        Project::open(root)
    }

    fn make_id(artifact_id: &str, function: &str) -> FunctionId {
        let aid = ArtifactId::new(artifact_id);
        FunctionId::new(aid, function)
    }

    #[test]
    fn call_graph_for_readonly() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "readOnly");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_for_readonly.txt")
        );
    }

    #[test]
    fn call_graph_for_execute() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "execute");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_for_execute.txt")
        );
    }

    #[test]
    fn call_graph_errors_for_unknown_contract() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Unknown", "function");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/calls/expected/call_graph_errors_for_unknown_contract.txt"
            )
            .trim_end()
        );
    }

    #[test]
    fn call_graph_errors_for_unknown_function() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "unknownFunction");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!(
                "../../../fixtures/calls/expected/call_graph_errors_for_unknown_function.txt"
            )
            .trim_end()
        );
    }

    #[test]
    fn ambiguity_shows_suggestions() {
        let inspector = CallGraphInspector::new(fixture_ambiguous_project());
        let id = make_id("Dupe", "someFunction");
        let err = inspector.inspect(&id).unwrap_err().to_string();
        assert_eq!(
            err,
            include_str!("../../../fixtures/calls/expected/ambiguity_shows_suggestions.txt")
        );
    }

    #[test]
    fn call_graph_for_interface_call() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("Main", "callViaInterface");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_for_interface_call.txt")
        );
    }

    #[test]
    fn call_graph_includes_low_level_call() {
        let inspector = CallGraphInspector::new(fixture_project());
        let id = make_id("LowLevelCaller", "callWithPayload");
        let output = inspector.inspect(&id).unwrap().to_string();
        assert_eq!(
            output,
            include_str!("../../../fixtures/calls/expected/call_graph_includes_low_level_call.txt")
        );
    }
}
