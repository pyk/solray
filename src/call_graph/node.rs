//! Call graph node for Solidity function call analysis.
//!
//! Represents a function and the calls it makes to other functions
//! in a recursive tree structure, used to visualize call graphs.

use std::fmt;
use std::path::PathBuf;

/// A node in a call graph.
///
/// Each node represents a function call that may contain child calls.
#[derive(Debug, Clone)]
pub struct CallGraphNode {
    /// Human-readable signature, e.g. `Main::execute(uint256)`
    pub signature: String,
    /// The contract name that defines this function
    pub contract_name: String,
    /// The source file path
    pub file: PathBuf,
    /// Visibility: `external`, `public`, `internal`, `private`
    pub visibility: String,
    /// Source location range for the function (for the Sources section)
    pub src: String,
    /// Calls made within this function
    pub children: Vec<CallGraphNode>,
}

impl CallGraphNode {
    /// Create a new call graph node.
    pub fn new(
        signature: &str,
        contract_name: &str,
        file: PathBuf,
        visibility: &str,
        src: &str,
        children: Vec<CallGraphNode>,
    ) -> Self {
        CallGraphNode {
            signature: signature.to_string(),
            contract_name: contract_name.to_string(),
            file,
            visibility: visibility.to_string(),
            src: src.to_string(),
            children,
        }
    }

    /// Flatten the call graph into a depth-first list of `(file, src)` pairs
    /// for the sources section. The caller is responsible for formatting paths.
    pub fn flatten_sources(&self) -> Vec<(PathBuf, String)> {
        let mut result = Vec::new();
        self.flatten_sources_recursive(&mut result);
        result
    }

    fn flatten_sources_recursive(&self, out: &mut Vec<(PathBuf, String)>) {
        out.push((self.file.clone(), self.src.clone()));
        for child in &self.children {
            child.flatten_sources_recursive(out);
        }
    }
}

impl fmt::Display for CallGraphNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} ({})", self.signature, self.visibility)?;
        fmt_children(&self.children, f, "")
    }
}

fn fmt_children(
    children: &[CallGraphNode],
    f: &mut fmt::Formatter<'_>,
    prefix: &str,
) -> fmt::Result {
    let len = children.len();
    for (i, child) in children.iter().enumerate() {
        let is_last = i == len - 1;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
        let continuation = if is_last { "    " } else { "\u{2502}   " };

        writeln!(
            f,
            "{}{}{} ({})",
            prefix, connector, child.signature, child.visibility
        )?;
        let child_prefix = format!("{}{}", prefix, continuation);
        fmt_children(&child.children, f, &child_prefix)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn display_simple_function() {
        let node = CallGraphNode::new(
            "Main::execute(uint256)",
            "Main",
            PathBuf::from("src/Main.sol"),
            "public",
            "276:110",
            vec![],
        );
        assert_eq!(node.to_string(), "Main::execute(uint256) (public)\n");
    }

    #[test]
    fn display_nested_function_calls() {
        let node = CallGraphNode::new(
            "Main::execute(uint256)",
            "Main",
            PathBuf::from("src/Main.sol"),
            "public",
            "276:110",
            vec![
                CallGraphNode::new(
                    "Helper::assist(uint256)",
                    "Helper",
                    PathBuf::from("src/Helper.sol"),
                    "public",
                    "109:72",
                    vec![],
                ),
                CallGraphNode::new(
                    "Main::internalWork()",
                    "Main",
                    PathBuf::from("src/Main.sol"),
                    "internal",
                    "392:79",
                    vec![CallGraphNode::new(
                        "Base::baseWork()",
                        "Base",
                        PathBuf::from("src/Main.sol"),
                        "internal",
                        "226:42",
                        vec![],
                    )],
                ),
            ],
        );
        let expected = concat!(
            "Main::execute(uint256) (public)\n",
            "\u{251c}\u{2500}\u{2500} Helper::assist(uint256) (public)\n",
            "\u{2514}\u{2500}\u{2500} Main::internalWork() (internal)\n",
            "    \u{2514}\u{2500}\u{2500} Base::baseWork() (internal)\n",
        );
        assert_eq!(node.to_string(), expected);
    }

    #[test]
    fn flatten_sources_collects_depth_first() {
        let node = CallGraphNode::new(
            "Main::execute(uint256)",
            "Main",
            PathBuf::from("src/Main.sol"),
            "public",
            "276:110",
            vec![CallGraphNode::new(
                "Helper::assist(uint256)",
                "Helper",
                PathBuf::from("src/Helper.sol"),
                "public",
                "109:72",
                vec![],
            )],
        );
        let sources = node.flatten_sources();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].0, PathBuf::from("src/Main.sol"));
        assert_eq!(sources[0].1, "276:110");
        assert_eq!(sources[1].0, PathBuf::from("src/Helper.sol"));
        assert_eq!(sources[1].1, "109:72");
    }
}
