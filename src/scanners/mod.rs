//! Scanners for Foundry projects.
//!
//! Scanners analyze the source code at the AST level to find patterns of
//! interest, such as ERC20 Transfer Sinks and asset transfers.

pub use asset_transfers::AssetTransfer;
pub use asset_transfers::AssetTransferKind;
pub use asset_transfers::AssetTransferScanner;
pub use asset_transfers::AssetTransferScannerOutput;
pub use erc20_transfer_sink::Erc20TransferSink;
pub use erc20_transfer_sink::Erc20TransferSinkScanner;
pub use erc20_transfer_sink::Erc20TransferSinkScannerOutput;

pub mod asset_transfers;
pub mod erc20_transfer_sink;
