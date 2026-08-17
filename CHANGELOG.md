# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

- Upgraded solc dependency to v0.3.1
- `solray inspect storage-layout` now prints `No storage slots found.` for
  contracts with an empty storage layout instead of printing nothing

### Fixed

- `solray inspect function-source` now resolves cross-file virtual calls to the
  most-derived override on the queried contract instead of the base declaration
- `solray inspect storage-layout` no longer fails for contracts that declare
  only immutable state variables
- `solray inspect storage-layout` error messages now print artifact paths
  relative to the project root instead of machine-specific absolute paths

## [0.9.0] - 2026-08-15

### Added

- `solray inspect contracts` now supports solc < 0.6 projects, whose ASTs omit
  the `abstract` flag; implicitly abstract contracts (e.g. the function-only
  `*Like` adapters in MakerDAO DSS) are no longer listed as deployable.
- `solray inspect abstracts` now supports solc < 0.6 projects, whose ASTs omit
  the `abstract` flag; implicitly abstract contracts (functions declared
  without bodies) are now listed.

### Changed

- Upgraded solc dependency to v0.1.1
- `solray inspect call-path` help text no longer advertises the unsupported
  `Contract::function` target syntax
- Public types are now re-exported at the crate root instead of through public
  module paths; internal modules are private
- `ERC20TransferSink`, `ERC20TransferSinkScanner`, and
  `ERC20TransferSinkScannerOutput` were renamed from the `Erc20*` spelling, and
  `AssetTransferKind` variants were renamed from `Erc20Transfer*` to
  `ERC20Transfer*`

### Fixed

- `solray scan erc20-transfer-sink` no longer reports native ETH transfers
  (`address payable.transfer`) as ERC20 transfer sinks; unresolved members are
  builtins of elementary types and are skipped.
- `solray inspect call-graph` now includes native ETH transfers via
  `address.transfer` and `address.send`, which the AST leaves unresolved.
- `solray inspect storage-layout` now fails with an explicit error instead of
  printing nothing when the artifact's storage layout is empty even though the
  contract declares storage variables (solc < 0.6 does not emit storage layout
  output).
- `solray inspect call-path` now reports every call path from an entry function
  to the target, including direct calls that an indirect modifier or helper
  branch would otherwise hide.
- `solray inspect function-source` now follows symbols referenced in constant
  and state-variable initializers, such as `FEE_SIZE` via `NEXT_OFFSET` and
  `DEFAULT_AMOUNT_IN_CACHED` via `amountInCached`.
- `solray inspect call-graph` now follows Solidity 0.6/0.7 base constructor
  specifiers when the AST omits `ModifierInvocation.kind`.
- `solray inspect function-source` now follows Solidity 0.6/0.7 base
  constructor specifiers when the AST omits `ModifierInvocation.kind`.
- `solray inspect inheritance-graph` now resolves libraries such as `Address`
  and `StorageSlot`.
- `solray inspect call-graph` now walks `emit` arguments and `try`/`catch`
  bodies.
- `solray inspect function-source` now follows base constructor specifiers into
  the parent constructor body.
- `solray inspect call-graph` now prints function-type parameters as
  `function(uint256,uint256)`.
- `solray inspect function-source` now collects types from struct members.
- `solray inspect call-path` now expands each project contract with its own
  inheritance chain and drops duplicate inherited roots.
- `solray inspect function-source` now follows unqualified virtual calls to the
  queried contract's most-derived override.
- `solray inspect external-functions` now resolves inherited functions along
  the queried contract's inheritance chain.
- `solray inspect external-functions` now maps inherited overloads to their
  declaring function.
- `solray scan erc20-transfer-sink` now reports `transfer` and `safeTransfer`
  calls inside single-statement `if`, `while`, and `for` bodies.
- `solray scan asset-transfers` now reports transfer calls inside
  single-statement `if`, `while`, and `for` bodies.
- `solray inspect call-graph` now prints Solidity 0.7 user-defined types and
  fixed-size arrays instead of `unknown`.

## [0.8.0] - 2026-08-12

### Fixed

- `solray gen interface` now emits valid Solidity for library-qualified types:
  structs, enums, and user-defined value types declared in libraries (for
  example `AgentInfo.Info`, `EmergencyPause.Level`, `IPayment.Proof`) are
  declared locally under unique names instead of dotted paths that fail to
  parse, and enum members are recovered from the declaring library's artifact.
- `solray inspect call-path` now resolves names declared by an abstract base
  and overridden by a derived contract (for example
  `Proxy._implementation`/`ERC1967Proxy._implementation`) to the most-derived
  override instead of reporting a phantom "multiple overloads" error.
- `solray inspect call-path` now reports paths rooted in every compiled project
  contract, including files outside the configured `src` directory (only the
  `test` directory is excluded), so inherited functions such as
  `Ownable.renounceOwnership` are valid roots; virtual calls from those roots
  resolve to the queried contract's most-derived override.
- `solray inspect call-graph` now includes contract creation calls
  (`new Contract(...)`), expanding the created contract's constructor; such
  calls were previously dropped because `NewExpression` was not handled.
- `solray inspect call-graph` now resolves inherited functions to the
  implementing declaration instead of an interface listed earlier in the
  inheritance order.
- `solray inspect call-graph` now expands virtual calls to the implementing
  declaration instead of an interface declared earlier in the inheritance
  chain.
- `solray inspect call-graph` no longer redirects explicitly-qualified base
  calls (`BaseContract.func(...)`, like `super.func(...)`) to a derived
  override, which previously produced a self-loop.
- `solray inspect function-source` now renders the contract created by a
  `new Contract(...)` expression as a resolved symbol (header and NatSpec, plus
  base contracts); the created contract was previously omitted.
- `solray inspect function-source` no longer pulls in symbols from unrelated
  library or contract members when a container is referenced by identifier;
  container symbols render only their own declaration header, and their base
  contracts and interfaces are no longer resolved as separate sections.
- The CLI writes output through a broken-pipe-aware writer, so piping into
  `head`/`less` exits quietly instead of panicking.

## [0.7.0] - 2026-08-09

### Added

- `solray inspect call-graph`, `call-path`, and `external-functions` support
  `--debug` resolution tracing.
- `solray scan erc20-transfer-sink` supports `--debug` scan tracing.
- `solray scan asset-transfers` supports `--debug` scan tracing.

### Changed

- `make build-fixtures` runs `forge clean` before rebuilding fixtures for
  deterministic artifacts.
- `solray inspect call-path` renders paths with dot notation (`Lpd.transfer`).

### Fixed

- `solray inspect call-graph` includes function calls nested inside
  index-access expressions and member-call bases, for example
  `_allowances[_msgSender()].sub(...)`.
- `solray inspect external-functions` resolves inherited ABI entries to base
  interface declarations when the queried contract is an interface.
- `solray inspect call-graph` includes low-level calls made with
  `{value: ...}`.
- `solray inspect modifiers` reports correct line numbers for CRLF projects.
- `solray inspect call-path` rejects functions that are not exposed by the
  queried contract instead of counting unrelated interface declarations as
  overloads.
- `solray inspect call-graph` rejects functions that are not exposed by the
  queried contract instead of resolving them to unrelated interface
  declarations in flattened projects.
- `solray inspect external-functions` resolves inherited functions and getters
  to the implementing contract instead of an interface, with deterministic
  selection for flattened projects.
- `solray inspect call-graph` resolves inherited overridden functions to the
  implementing base contract instead of an interface when the queried contract
  does not redeclare them.
- `solray inspect call-path` resolves inherited overridden functions to the
  implementing base contract instead of counting interface declarations as
  overloads; bare names and full signatures both work.
- `solray inspect` ignores import-only artifacts that declare nothing, so
  plain-name lookups like `IERC20` no longer fail with false ambiguity.
- `solray inspect function-source` accepts interface function declarations as
  roots (for example `IERC20 transfer`).
- `solray gen interface` emits real `enum` declarations with their original
  members instead of user-defined value types.
- `solray inspect function-source` no longer leaks unrelated declarations after
  incremental builds.
- `solray inspect function-source` resolves CRLF sources correctly instead of
  emitting shifted or truncated blocks.
- `solray inspect function-source`, `call-graph`, and `call-path` resolve
  functions inherited from base contracts in other files.
- `solray inspect external-functions` reports the declaring source line for
  inherited `fallback` and `receive` functions instead of `:0`.
- `solray inspect external-functions` reports `fallback` state mutability from
  the artifact ABI instead of always printing `nonpayable`.
- `solray inspect function-source` labels abstract contracts as "Abstract
  Contract" instead of "Interface".
- `solray inspect function-source` resolves inherited `receive` and `fallback`
  functions from their declaring base contract.
- `solray inspect function-source` omits inherited constructors from
  resolution.
- `solray inspect function-source` resolves inherited overridden functions to
  the most-derived declaration instead of failing with duplicate identical
  suggestions.
- `solray inspect call-graph` resolves virtual calls and inherited overrides to
  the most-derived implementation instead of base declarations, while
  preserving explicit `super` calls.
- `solray inspect external-functions`, `call-graph`, and `call-path` report
  correct line numbers for CRLF projects.
- `solray inspect call-graph` resolves overridden functions to the queried
  contract's own declaration instead of inherited interface declarations.
- `solray inspect call-path` resolves overridden functions instead of treating
  inherited interface declarations as overloads; full signatures are accepted
  by `call-path` and `call-graph`.
- `solray inspect call-graph` and `call-path` resolve `constructor` lookups and
  render constructor roots correctly.
- `solray inspect call-graph`, `call-path`, and `function-source` resolve
  public state-variable getters from the ABI.
- `solray inspect call-graph` and `call-path` expand calls made through
  modifiers and base constructors.
- `solray inspect external-functions` maps every overload to its own source
  line using normalized parameter signatures.
- Ambiguity suggestions across `inspect` and `gen` commands show the artifact's
  AST source path, and file-qualified IDs accept those paths.
- `solray scan erc20-transfer-sink` reports correct transfer snippets and lines
  for CRLF projects.
- `solray scan asset-transfers` reports correct transfer expressions and lines
  for CRLF projects.

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

[unreleased]: https://github.com/pyk/solray/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/pyk/solray/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/pyk/solray/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/pyk/solray/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/pyk/solray/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/pyk/solray/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/pyk/solray/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/pyk/solray/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/pyk/solray/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/pyk/solray/releases/tag/v0.1.0
