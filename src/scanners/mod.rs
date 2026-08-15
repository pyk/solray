//! Scanners for Foundry projects.
//!
//! Scanners analyze the source code at the AST level to find patterns of
//! interest, such as ERC20 Transfer Sinks and asset transfers.

pub mod asset_transfers;
pub mod erc20_transfer_sink;
