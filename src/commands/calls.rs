//! Show the call graph of a Solidity function.
//!
//! This module is the CLI-facing layer for the `hawk inspect calls` command.
//! The core logic lives in [`crate::call_graph::CallGraphLoader`].

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::call_graph::{CallGraphLoader, CallGraphNode};
use crate::project::Project;

/// Run the call graph inspection for the given function ID.
///
/// `function_id` should be in the format `Contract::function`.
///
/// Returns the formatted output on success. Returns an error with a
/// user-friendly message when the declaration is not found, when
/// multiple declarations share the same name, or when the function
/// is overloaded.
pub fn run(function_id: &str, path: impl AsRef<Path>) -> Result<String> {
    let project = Project::open(path.as_ref())?;
    let loader = CallGraphLoader::new(project.path(), project.out_dir());
    let node = loader.call_graph(function_id)?;
    format_output(node, project.path())
}

fn format_output(tree: CallGraphNode, project_root: &Path) -> Result<String> {
    let sources = tree.flatten_sources();
    let cwd = std::env::current_dir()?;
    let project_abs = std::path::absolute(project_root)?;

    // Build line-offset maps for each unique source file.
    let mut line_maps: HashMap<PathBuf, Vec<usize>> = HashMap::new();

    let mut output = String::new();

    output.push_str("Call graph:\n\n");
    output.push_str(&tree.to_string());

    output.push_str(&format!("\nResolved from {} sources:\n\n", sources.len()));

    for (i, (file, src)) in sources.iter().enumerate() {
        let full_path = project_abs.join(file);
        let rel_path = full_path.strip_prefix(&cwd).unwrap_or(&full_path);

        let line_range = offset_to_line_range(&full_path, src, &mut line_maps);
        output.push_str(&format!(
            "{}. {}#{}\n",
            i + 1,
            rel_path.display(),
            line_range
        ));
    }

    Ok(output)
}

/// Parse `src` as `offset:length` and return a human-readable line range like `L5-L7`.
/// Uses a cache of line-offset maps to avoid re-reading files.
fn offset_to_line_range(
    file_path: impl AsRef<Path>,
    src: &str,
    cache: &mut HashMap<PathBuf, Vec<usize>>,
) -> String {
    let file_path = file_path.as_ref();
    let (offset_str, length_str) = match src.split_once(':') {
        Some((o, l)) => (o, l),
        None => return src.to_string(),
    };

    let offset: usize = match offset_str.parse() {
        Ok(o) => o,
        Err(_) => return src.to_string(),
    };
    let length: usize = match length_str.parse() {
        Ok(l) => l,
        Err(_) => return src.to_string(),
    };

    let line_offsets = cache.entry(file_path.to_path_buf()).or_insert_with(|| {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        build_line_offsets(&content)
    });

    if line_offsets.is_empty() {
        return src.to_string();
    }

    let start_line = byte_offset_to_line(offset, line_offsets);
    let end_line = byte_offset_to_line(
        offset.saturating_add(length).saturating_sub(1),
        line_offsets,
    );

    if start_line == end_line {
        format!("L{}", start_line)
    } else {
        format!("L{}-L{}", start_line, end_line)
    }
}

/// Build a vector where `line_offsets[n]` is the byte offset of the start of line `n`
/// (1-indexed: `line_offsets[1]` is the offset of line 1).
fn build_line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0, 0]; // offsets[0] is dummy, offsets[1] = start of line 1
    for (i, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Given a byte offset and a line-offsets vector, return the 1-indexed line number.
fn byte_offset_to_line(offset: usize, line_offsets: &[usize]) -> usize {
    match line_offsets.binary_search(&offset) {
        Ok(line) => line.max(1),
        Err(line) => line.saturating_sub(1).max(1),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/calls")
    }

    #[test]
    fn run_shows_call_graph_for_readonly() {
        let result = run("Main::readOnly", fixture_path()).unwrap();
        assert!(result.contains("Call graph:"));
        assert!(result.contains("Main::readOnly()"));
    }

    #[test]
    fn run_shows_call_graph_for_execute() {
        let result = run("Main::execute", fixture_path()).unwrap();
        assert!(result.contains("Call graph:"));
        assert!(result.contains("Main::execute(uint256)"));
    }

    #[test]
    fn run_errors_for_unknown_contract() {
        let result = run("Unknown::function", fixture_path());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("\"Unknown\" not found"));
    }

    #[test]
    fn run_errors_for_unknown_function() {
        let result = run("Main::unknownFunction", fixture_path());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("\"unknownFunction\" not found in \"Main\""));
    }

    #[test]
    fn byte_offset_to_line_finds_correct_line() {
        let content = "line1\nline2\nline3\n";
        let offsets = build_line_offsets(content);
        // offsets: [0, 0, 6, 12]
        assert_eq!(byte_offset_to_line(0, &offsets), 1); // start of line 1
        assert_eq!(byte_offset_to_line(3, &offsets), 1); // middle of line 1
        assert_eq!(byte_offset_to_line(6, &offsets), 2); // start of line 2
        assert_eq!(byte_offset_to_line(8, &offsets), 2); // middle of line 2
        assert_eq!(byte_offset_to_line(12, &offsets), 3); // start of line 3
        assert_eq!(byte_offset_to_line(14, &offsets), 3); // middle of line 3
    }

    #[test]
    fn offset_to_line_range_single_line() {
        let content = "line1\nline2\nline3\n";
        let offsets = build_line_offsets(content);
        assert_eq!(byte_offset_to_line(0, &offsets), 1);
        assert_eq!(byte_offset_to_line(6, &offsets), 2);
        assert_eq!(byte_offset_to_line(12, &offsets), 3);
    }
}
