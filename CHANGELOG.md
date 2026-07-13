# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `solray inspect function-source` now resolves `@inheritdoc` directives by
  looking up the referenced contract's NatSpec documentation for the matching
  function instead of displaying the raw `@inheritdoc` line

### Changed

- `solray inspect function-source` symbol header format changed from
  `Contract::function` to `Contract.function` for consistency with Solidity
  call-site syntax

### Fixed

- `solray inspect function-source` now correctly prefixes cross-file function
  and variable symbols with their contract name instead of showing the bare
  identifier. For example, `_afterAddLiquidity` is now displayed as
  `ExtensionCalling._afterAddLiquidity`.
- `solray inspect modifiers` no longer stack-overflows when a contract inherits
  from an interface defined in a separate file. The modifier inspector now
  includes interfaces (alongside contracts and libraries) when parsing
  artifacts, preventing infinite recursion in parent resolution.
- Replaced an `unwrap()` with a `context()` call in
  `ExternalFunctionInspector::resolve_artifact_path`, eliminating the last
  `unwrap_usage` suppression in the codebase.

## [0.2.0] - 2026-07-12

### Added

- `solray scan asset-transfers`: scan the source tree for asset transfer calls
  and ETH receivers across ERC20 and native ETH transfers
- `AssetTransferScanner`: library type for programmatic asset transfer
  detection
- CLI help text updated for consistency

### Changed

- Re-export `AssetTransfer`, `AssetTransferKind`, `AssetTransferScanner`,
  `AssetTransferScannerOutput` from `solray` crate root

## [0.1.0] - 2026-07-12

### Added

- `solray inspect contracts`: list all deployable contracts in a Foundry
  project
- `solray inspect abstracts`: list all abstract contracts
- `solray inspect interfaces`: list all interfaces
- `solray inspect libraries`: list all libraries
- `solray inspect inheritance-graph <contract>`: visualize the inheritance
  chain of any contract or interface
- `solray inspect external-functions <contract>`: list all externally callable
  functions from a contract's ABI, including `receive` and `fallback`; supports
  `--include-read-only` to include view/pure functions
- `solray inspect modifiers <contract>`: list all modifiers on a contract,
  including inherited ones
- `solray inspect storage-layout <contract>`: show the storage layout of a
  contract
- `solray inspect call-graph <contract> <function>`: show the complete call
  graph of a function, including reverse call graph support
- `solray inspect call-path <contract> <function>`: show call paths from entry
  functions to a target function
- `solray inspect function-source <contract> <function>`: display the complete
  resolved source code of a function, including inherited modifiers
- `solray scan erc20-transfer-sink`: scan the source tree for ERC20 `transfer`
  and `safeTransfer` call sites
- All `inspect` and `scan` commands accept `--project <path>` to target a
  specific Foundry project directory (defaults to `.`)
- Artifact ID syntax (`File.sol:Name`) for disambiguating contracts with the
  same name across files
- Library-first public API with dedicated inspector and scanner types for
  programmatic use
- Support for incremental builds, cross-file references, and NatSpec blocks in
  function source resolution

[unreleased]: https://github.com/pyk/solray/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/pyk/solray/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/pyk/solray/releases/tag/v0.1.0
