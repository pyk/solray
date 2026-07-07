//! Scanners for Foundry projects.
//!
//! Scanners analyze the source code at the AST level to find patterns of
//! interest, such as ERC20 Transfer Sinks.

pub use erc20_transfer_sink::Erc20TransferSink;
pub use erc20_transfer_sink::Erc20TransferSinkScanner;
pub use erc20_transfer_sink::Erc20TransferSinkScannerOutput;

pub mod erc20_transfer_sink;
