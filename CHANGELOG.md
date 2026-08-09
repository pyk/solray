# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

- `make build-fixtures` now runs `forge clean` for every fixture project before
  rebuilding, so local and CI artifact states are always deterministic.

### Fixed

- `solray inspect function-source` no longer leaks unrelated declarations into
  resolved output after incremental builds. The build-info resolver previously
  matched artifacts by source path alone, so a recompiled file could be scoped
  to an older compilation unit when build-info files were discovered in a
  different order. It now prefers the exact file-index match, which is
  deterministic and keeps each artifact in its own node-ID namespace.
- `solray inspect function-source` now resolves source ranges correctly in
  projects with CRLF line endings. Previously it sliced raw CRLF source bytes
  with LF-normalized solc AST offsets, so function and symbol blocks were
  truncated or misaligned. Added a CRLF regression fixture to the
  `function-source` test suite.
- `solray inspect function-source`, `solray inspect call-graph`, and
  `solray inspect call-path` now resolve functions inherited from base
  contracts defined in other source files. Previously only the queried
  contract's own artifact AST was searched, so inherited functions such as
  OpenZeppelin's `owner` failed with `"not found"`.
- `solray inspect function-source` now labels abstract contracts as "Abstract
  Contract" instead of "Interface"; interfaces and libraries keep their own
  headings.
- `solray inspect function-source` no longer lists inherited constructors,
  `receive`, or `fallback` functions when resolving a derived contract.
- `solray inspect external-functions`, `solray inspect call-graph`, and
  `solray inspect call-path` now report correct source line numbers for
  projects with CRLF line endings. Previously the line helpers read raw CRLF
  bytes while solc AST offsets are LF-normalized, so functions were reported
  several lines early. Added CRLF regression fixtures to all three test suites.
- `solray inspect call-graph` now deterministically resolves overridden
  functions to the queried contract's own declaration instead of an inherited
  interface with the same name. Previously the matching declaration was picked
  from HashMap iteration order, so ERC-20 functions such as `transfer`, `name`,
  and `decimals` could resolve to `IERC20`/`IERC20Metadata`. Added an
  override-preference regression fixture and `--debug` support for
  `inspect call-graph`.

## [0.6.0] - 2026-08-08

### Added

- `solray gen interface <contract>` generates a valid Solidity interface for
  any contract from its ABI, including inherited functions and public variable
  getters. Structs, enums, and user-defined value types referenced by the
  functions are resolved inline.

### Changed

- Upgraded solc dependency to v0.1.0
- `solray inspect external-functions` summary and section labels now use
  "mutable functions" and "view functions" instead of "state-changing
  functions" and "read-only functions". Summary order is also updated to list
  mutable and view counts before callback and special.
- `solray inspect external-functions` now always shows view and pure functions.
  The `--include-read-only` flag has been removed.

### Fixed

- Artifact JSON deserialization failures now report the failing artifact path
  across inspect, gen, call-graph, and function-source paths (for example
  missing solc AST fields), matching the context already added for scan
  commands and `Project` parsing.

## [0.5.0] - 2026-08-03

### Added

- `solray inspect function-source` now supports `--debug` for opt-in resolver
  tracing, matching the existing flags on `inspect inheritance-graph` and
  `inspect modifiers`.

### Changed

- Upgraded solc dependency to v0.0.14

### Fixed

- `solray inspect function-source` now correctly resolves cross-file modifiers
  after incremental builds. The build-info resolver previously matched
  artifacts to compilation units by exact file index, which breaks when Foundry
  reassigns source IDs across recompilations. It now prefers source-path
  matching so all artifacts from the same source file consistently resolve to
  the same build-info regardless of file-index drift.

- Error messages across all `inspect` commands now use `solray` instead of the
  stale `hawk` project name. The `function-source` overloaded-function
  suggestions also now wrap function signatures in quotes and strip the
  contract-name prefix for clean, copy-pasteable output.

- `solray scan asset-transfers`, `solray scan erc20-transfer-sink`, and
  artifact parsing in `Project` now report the artifact path when `solc` AST
  deserialization fails. Previously parse errors such as
  `missing field   'eventSelector'` surfaced without any context.

## [0.4.0] - 2026-07-27

### Changed

- `solray inspect function-source` output format changed to markdown
- `solray inspect function-source` resolved symbols are now sorted
  alphabetically by kind and then by display name

### Fixed

- `solray inspect function-source` now resolves interface types used as state
  variable types (e.g. `ILoans public LOANS` now also resolves `ILoans`), used
  in explicit type conversions (e.g. `ILoansAuth(address(...))` now also
  resolves `ILoansAuth`), and inherited parent interfaces (e.g.
  `interface IChild is IParentA, IParentB` now also resolves `IParentA` and
  `IParentB`). Interface definitions are shown with their header signature
  only, without the full body. Previously all interface-type references were
  silently dropped because `ContractDefinition` nodes were not indexed in the
  symbol index and state variable type names were not traversed during
  resolution.
- `solray inspect function-source` now correctly resolves all intermediate
  calls in chained function-call expressions (e.g. `a().b().c()`). Previously
  only the outermost call was resolved; calls nested inside `MemberAccess`
  expressions (such as `.asBoolean()` inside
  `_reentrancyGuardStorageSlot().asBoolean().tstore(false)`) were silently
  skipped.
- `solray inspect function-source` now resolves symbols inside `revert`
  statements (e.g. error definitions referenced from modifier bodies), and
  includes support for `emit`, `try`/`catch`, and `unchecked` blocks. Missing
  statement-type handlers caused errors, events, and other symbols inside those
  constructs to be silently dropped.
- `solray inspect function-source` now resolves top-level declarations (errors,
  events, structs, enums, functions, variables, and UDVTs declared outside any
  contract) both in the symbol index and during in-AST resolution. Previously
  these declarations were invisible to the resolver, causing references to them
  (including errors from modifiers like `ReentrancyGuardReentrantCall`) to be
  silently skipped.

## [0.3.0] - 2026-07-21

### Added

- `solray inspect function-source` now resolves `@inheritdoc` directives by
  looking up the referenced contract's NatSpec documentation for the matching
  function instead of displaying the raw `@inheritdoc` line
- `solray inspect function-source` now supports `constructor`, `receive`, and
  `fallback`, including recursive symbol resolution for same-contract,
  inherited, and imported-parent functions
- Added `--debug` to `solray inspect inheritance-graph` and
  `solray inspect modifiers` for opt-in resolver tracing

### Changed

- `solray inspect inheritance-graph` now displays each contract's source path
  inline and no longer emits a separate `Sources` section
- `solray inspect call-graph` now uses dot syntax for contract functions,
  displays project-relative source paths with start lines inline, and no longer
  emits a separate resolved-sources section
- `solray inspect function-source` symbol header format changed from
  `Contract::function` to `Contract.function` for consistency with Solidity
  call-site syntax
- Upgraded solc dependency to v0.0.12

### Fixed

- `solray inspect function-source` now correctly prefixes cross-file function
  and variable symbols with their contract name instead of showing the bare
  identifier. For example, `_afterAddLiquidity` is now displayed as
  `ExtensionCalling._afterAddLiquidity`.
- `solray inspect modifiers` no longer stack-overflows when duplicate artifacts
  include import-only files. Parent resolution now selects an artifact whose
  AST declares the requested contract, preventing infinite recursion.
- Replaced an `unwrap()` with a `context()` call in
  `ExternalFunctionInspector::resolve_artifact_path`, eliminating the last
  `unwrap_usage` suppression in the codebase.
- `solray inspect inheritance-graph` now skips empty duplicate artifacts and
  resolves shared ancestors without reporting false circular inheritance.
- Added regression coverage for duplicate artifacts and diamond inheritance
  graphs.

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

[unreleased]: https://github.com/pyk/solray/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/pyk/solray/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/pyk/solray/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/pyk/solray/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/pyk/solray/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/pyk/solray/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/pyk/solray/releases/tag/v0.1.0
