//! Inheritance tree node for Solidity contracts.
//!
//! Represents a contract and its base contracts in a recursive tree
//! structure, used to visualize inheritance graphs.

use std::fmt;
use std::path::PathBuf;

/// A node in an inheritance tree.
#[derive(Debug, Clone)]
pub struct InheritanceNode {
    pub name: String,
    pub file: PathBuf,
    pub parents: Vec<InheritanceNode>,
}

impl InheritanceNode {
    /// Flatten the inheritance tree into a depth-first list of `(file, name)` pairs.
    pub fn flatten_sources(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();
        self.flatten_recursive(&mut result);
        result
    }

    fn flatten_recursive(&self, out: &mut Vec<(String, String)>) {
        out.push((self.file.display().to_string(), self.name.clone()));
        for parent in &self.parents {
            parent.flatten_recursive(out);
        }
    }
}

impl fmt::Display for InheritanceNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn fmt_children(
            children: &[InheritanceNode],
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

                writeln!(f, "{}{}{}", prefix, connector, child.name)?;
                if !child.parents.is_empty() {
                    let child_prefix = format!("{}{}", prefix, continuation);
                    fmt_children(&child.parents, f, &child_prefix)?;
                }
            }
            Ok(())
        }

        writeln!(f, "{}", self.name)?;
        fmt_children(&self.parents, f, "")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn display_single_node() {
        let node = InheritanceNode {
            name: "Root".into(),
            file: PathBuf::from("src/Root.sol"),
            parents: vec![],
        };
        assert_eq!(node.to_string(), "Root\n");
    }

    #[test]
    fn display_with_children() {
        let leaf = InheritanceNode {
            name: "Leaf".into(),
            file: PathBuf::from("src/Leaf.sol"),
            parents: vec![],
        };
        let root = InheritanceNode {
            name: "Root".into(),
            file: PathBuf::from("src/Root.sol"),
            parents: vec![leaf],
        };
        assert_eq!(root.to_string(), "Root\n\u{2514}\u{2500}\u{2500} Leaf\n");
    }

    #[test]
    fn display_nested() {
        let base = InheritanceNode {
            name: "Base".into(),
            file: PathBuf::from("src/Base.sol"),
            parents: vec![],
        };
        let middle = InheritanceNode {
            name: "Middle".into(),
            file: PathBuf::from("src/Middle.sol"),
            parents: vec![base],
        };
        let child = InheritanceNode {
            name: "Child".into(),
            file: PathBuf::from("src/Child.sol"),
            parents: vec![middle],
        };
        assert_eq!(
            child.to_string(),
            "Child\n\u{2514}\u{2500}\u{2500} Middle\n    \u{2514}\u{2500}\u{2500} Base\n"
        );
    }

    #[test]
    fn flatten_sources_depth_first() {
        let base = InheritanceNode {
            name: "Base".into(),
            file: PathBuf::from("src/Base.sol"),
            parents: vec![],
        };
        let middle = InheritanceNode {
            name: "Middle".into(),
            file: PathBuf::from("src/Middle.sol"),
            parents: vec![base],
        };
        let child = InheritanceNode {
            name: "Child".into(),
            file: PathBuf::from("src/Child.sol"),
            parents: vec![middle],
        };
        let sources = child.flatten_sources();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0].1, "Child");
        assert_eq!(sources[1].1, "Middle");
        assert_eq!(sources[2].1, "Base");
    }
}
