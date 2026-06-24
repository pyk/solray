//! Function identifier for Solidity contract methods.
//!
//! A `FunctionID` represents a `Contract::function` string used to identify
//! a specific function in call graph resolution.

use std::fmt;

use anyhow::bail;

/// A validated function identifier of the form `Contract::function`.
///
/// Both parts must be non-empty. The `::` separator is required.
///
/// # Examples
///
/// ```
/// # use hawk::call_graph::FunctionID;
/// let fid = FunctionID::try_from("Main::execute")?;
/// assert_eq!(fid.contract_name(), "Main");
/// assert_eq!(fid.function_name(), "execute");
/// # Ok::<_, anyhow::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct FunctionID {
    contract: String,
    function: String,
}

impl FunctionID {
    /// The contract name component.
    pub fn contract_name(&self) -> &str {
        &self.contract
    }

    /// The function name component.
    pub fn function_name(&self) -> &str {
        &self.function
    }
}

impl fmt::Display for FunctionID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.contract, self.function)
    }
}

impl TryFrom<&str> for FunctionID {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value.split_once("::") {
            Some((contract, function)) if !contract.is_empty() && !function.is_empty() => {
                Ok(Self {
                    contract: contract.to_string(),
                    function: function.to_string(),
                })
            }
            _ => bail!("invalid function ID \"{value}\". Expected format: Contract::function",),
        }
    }
}

impl TryFrom<String> for FunctionID {
    type Error = anyhow::Error;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_function_id() {
        let fid = FunctionID::try_from("Main::execute").unwrap();
        assert_eq!(fid.contract_name(), "Main");
        assert_eq!(fid.function_name(), "execute");
    }

    #[test]
    fn display_roundtrip() {
        let fid = FunctionID::try_from("Main::execute").unwrap();
        assert_eq!(fid.to_string(), "Main::execute");
    }

    #[test]
    fn from_string_works() {
        let fid = FunctionID::try_from("Contract::method".to_string()).unwrap();
        assert_eq!(fid.contract_name(), "Contract");
        assert_eq!(fid.function_name(), "method");
    }

    #[test]
    fn missing_separator_errors() {
        let err = FunctionID::try_from("Main.execute").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid function ID \"Main.execute\". Expected format: Contract::function",
        );
    }

    #[test]
    fn empty_contract_errors() {
        let err = FunctionID::try_from("::execute").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid function ID \"::execute\". Expected format: Contract::function",
        );
    }

    #[test]
    fn empty_function_errors() {
        let err = FunctionID::try_from("Main::").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid function ID \"Main::\". Expected format: Contract::function",
        );
    }

    #[test]
    fn both_empty_errors() {
        let err = FunctionID::try_from("::").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid function ID \"::\". Expected format: Contract::function",
        );
    }

    #[test]
    fn empty_string_errors() {
        let err = FunctionID::try_from("").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid function ID \"\". Expected format: Contract::function",
        );
    }
}
