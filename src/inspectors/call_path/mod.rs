//! Call path inspection for Foundry projects.
//!
//! [`CallPathInspector`] finds all external/public functions that can reach
//! a given target function, showing only the linear path to the target.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::call_graph::{
    CallGraph, FunctionId, Node, call_graph_reaches_target, extract_func_name_from_sig,
    function_node_matches_target,
};
use crate::inspectors::call_graph::source_renderer::offset_to_line_range;
use crate::project::Project;

/// The output of a [`CallPathInspector`] inspection.
///
/// Shows compact call paths from external functions to a target function.
#[derive(Debug)]
pub struct CallPathInspectorOutput {
    roots: Vec<Node>,
    project_root: PathBuf,
    target_function: String,
    target_file: PathBuf,
    target_line: String,
}

impl CallPathInspectorOutput {
    /// Create a new [`CallPathInspectorOutput`].
    pub fn new(
        roots: Vec<Node>,
        project_root: PathBuf,
        target_function: &str,
        target_file: PathBuf,
        target_line: &str,
    ) -> Self {
        Self {
            roots,
            project_root,
            target_function: target_function.to_string(),
            target_file,
            target_line: target_line.to_string(),
        }
    }
}

impl std::fmt::Display for CallPathInspectorOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "## Call Paths\n")?;

        let target_display = format_target_display(&self.target_function);

        writeln!(f, "Target: `{}`", target_display)?;
        if self.target_file.as_os_str().is_empty() {
            writeln!(f, "Source: Not found")?;
        } else {
            writeln!(
                f,
                "Source: `{}:{}`",
                self.target_file.display(),
                self.target_line
            )?;
        }

        if self.roots.is_empty() {
            writeln!(
                f,
                "\n0 paths found.\n\nNo external functions reach \"{}\".",
                target_display
            )?;
            return Ok(());
        }

        writeln!(f, "\n{} paths found.\n", self.roots.len())?;

        let project_abs =
            std::path::absolute(&self.project_root).unwrap_or_else(|_| self.project_root.clone());

        let mut line_maps: HashMap<PathBuf, Vec<usize>> = HashMap::new();

        for (i, root) in self.roots.iter().enumerate() {
            writeln!(f, "### Path {}\n", i + 1)?;

            let path = extract_path_to_target(root, &self.target_function);

            writeln!(f, "```")?;
            for (j, node) in path.iter().enumerate() {
                let node_name = format_call_path_node(node);
                let full_path = project_abs.join(&node.file);
                let rel_path = full_path.strip_prefix(&project_abs).unwrap_or(&full_path);
                let line = if !node.src.is_empty() {
                    let range = offset_to_line_range(&full_path, &node.src, &mut line_maps);
                    range
                        .strip_prefix('L')
                        .and_then(|r| r.split('-').next())
                        .unwrap_or(&range)
                        .to_string()
                } else {
                    String::new()
                };
                let file_display = if line.is_empty() {
                    rel_path.display().to_string()
                } else {
                    format!("{}:{}", rel_path.display(), line)
                };

                if j == 0 {
                    writeln!(f, "{}", node_name)?;
                    writeln!(f, "{}", file_display)?;
                } else {
                    let connector_indent = " ".repeat((j - 1) * 4);
                    let file_indent = " ".repeat(j * 4);
                    writeln!(
                        f,
                        "{}\u{2514}\u{2500}\u{2500} {}",
                        connector_indent, node_name
                    )?;
                    writeln!(f, "{}{}", file_indent, file_display)?;
                }
            }
            writeln!(f, "```")?;
            writeln!(f)?;
        }

        Ok(())
    }
}

/// Format a target function string for display (strip visibility, params).
fn format_target_display(target: &str) -> String {
    if let Some((contract, rest)) = target.split_once("::") {
        let func = rest.split('(').next().unwrap_or(rest);
        return format!("{}::{}", contract, func);
    }
    target.to_string()
}

/// Format a [`Node`] for call-path display (no visibility, no params).
fn format_call_path_node(node: &Node) -> String {
    let func_name = extract_func_name_from_sig(&node.signature);
    format!("{}::{}", node.contract_name, func_name)
}

/// Extract the linear path from root to the target node.
fn extract_path_to_target<'a>(root: &'a Node, target: &str) -> Vec<&'a Node> {
    if function_node_matches_target(root, target) {
        return vec![root];
    }
    for child in &root.children {
        if call_graph_reaches_target(child, target) {
            let mut path = extract_path_to_target(child, target);
            path.insert(0, root);
            return path;
        }
    }
    vec![root]
}

/// Inspect a Foundry project for call paths to a target function.
pub struct CallPathInspector {
    engine: CallGraph,
}

impl CallPathInspector {
    /// Build a [`CallPathInspector`] for the given project.
    pub fn new(project: Project) -> Self {
        Self {
            engine: CallGraph::new(project),
        }
    }

    /// Find all external/public functions that can reach the target function.
    pub fn inspect(
        &self,
        id: &FunctionId,
        target_function: &str,
    ) -> Result<CallPathInspectorOutput> {
        let paths = self.engine.find_call_paths(id, target_function)?;
        let project_root = self.engine.project_root().to_path_buf();

        // Compute line number from target src.
        let project_abs =
            std::path::absolute(&project_root).unwrap_or_else(|_| project_root.clone());
        let mut line_maps: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        let target_line = if paths.target_src.is_empty() {
            String::new()
        } else {
            let full_path = project_abs.join(&paths.target_file);
            let range = offset_to_line_range(&full_path, &paths.target_src, &mut line_maps);
            range
                .strip_prefix('L')
                .and_then(|r| r.split('-').next())
                .unwrap_or(&range)
                .to_string()
        };

        Ok(CallPathInspectorOutput::new(
            paths.roots,
            project_root,
            target_function,
            paths.target_file,
            &target_line,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::inspectors::artifact_id::ArtifactId;
    use crate::project::Project;

    fn fixture_call_path_project() -> Project {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/call-path");
        Project::open(root)
    }

    fn make_id(artifact_id: &str, function: &str) -> FunctionId {
        let aid = ArtifactId::new(artifact_id);
        FunctionId::new(aid, function)
    }

    #[test]
    fn call_path_for_target_internal() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "targetInternal");
        let output = inspector.inspect(&id, "targetInternal").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_targetInternal.txt")
        );
    }

    #[test]
    fn call_path_for_parent_work() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "parentWork");
        let output = inspector.inspect(&id, "parentWork").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_parentWork.txt")
        );
    }

    #[test]
    fn call_path_for_grandparent_work() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "grandparentWork");
        let output = inspector.inspect(&id, "grandparentWork").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_grandparentWork.txt")
        );
    }

    #[test]
    fn call_path_for_lib_lib_work() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "Lib::libWork");
        let output = inspector.inspect(&id, "Lib::libWork").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!("../../../fixtures/call-path/expected/call_path_for_Lib_libWork.txt")
        );
    }

    #[test]
    fn call_path_returns_empty_when_target_not_found() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("Target", "nonExistent");
        let output = inspector.inspect(&id, "nonExistent").unwrap();
        assert_eq!(
            output.to_string(),
            include_str!(
                "../../../fixtures/call-path/expected/call_path_returns_empty_when_target_not_found.txt"
            )
        );
    }

    #[test]
    fn call_path_errors_for_unknown_contract() {
        let inspector = CallPathInspector::new(fixture_call_path_project());
        let id = make_id("NonExistentContract", "foo");
        let err = inspector.inspect(&id, "foo").unwrap_err().to_string();
        assert_eq!(err, "\"NonExistentContract\" not found.");
    }
}
