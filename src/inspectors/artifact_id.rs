//! Shared artifact identifier types.
//!
//! [`ArtifactId`] is the common way to refer to a contract by name and
//! optional source file across inspectors.

/// Identifies an artifact by contract name and optional source file.
#[derive(Debug, Clone)]
pub struct ArtifactId {
    /// The contract name (required).
    pub name: String,
    /// The source file path (optional).
    pub file: Option<String>,
}

impl ArtifactId {
    /// Parse an artifact ID from a string like `Name` or `File.sol:Name`.
    pub fn new(id: &str) -> Self {
        match id.rsplit_once(':') {
            Some((path, name)) if !path.is_empty() && !name.is_empty() => Self {
                name: name.to_string(),
                file: Some(path.to_string()),
            },
            _ => Self {
                name: id.to_string(),
                file: None,
            },
        }
    }
}
