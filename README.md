<p align="center">
  <strong>
    Solray
  </strong>
</p>

<p align="center">
    Solidity Source Code Explorer
<p>

<p align="center">
  <img src="https://img.shields.io/crates/v/solray.svg?colorA=00f&colorB=fff&style=flat&logo=rust" alt="Crates.io">
  <img src="https://img.shields.io/crates/d/solray?colorA=00f&colorB=fff&style=flat&logo=rust" alt="Downloads">
  <img src="https://img.shields.io/github/license/pyk/solray?colorA=00f&colorB=fff&style=flat" alt="MIT License">
</p>

**Solray** is a Solidity source code explorer for Foundry-based projects. It
helps security reviewers and developers understand contract codebases by
resolving inheritance hierarchies, external functions, call graphs, storage
layouts, and more, without leaving the terminal.

## Features

- **Project inspection**: list deployable contracts, abstract contracts,
  libraries, and interfaces.
- **Inheritance graph**: visualize the inheritance chain of any contract or
  interface.
- **External functions**: list all externally callable functions (including
  `receive` and `fallback`) from a contract's ABI.
- **Modifiers**: list all modifiers on a contract, including inherited ones.
- **Storage layout**: show the storage layout of a contract.
- **Call graph**: show the complete call graph of a function.
- **Call path**: show call paths from entry functions to a target function.
- **Function source**: resolve and display the complete source code of a
  function.
- **Pattern scanning**: scan the codebase for code patterns.

## Installation

### From crates.io

```bash
cargo install solray
```

### From source

```bash
git clone https://github.com/pyk/solray.git
cd solray
make bin
```

This runs `cargo install --path . --locked` and installs the `solray` binary to
your Cargo bin directory.

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2024)
- Run `forge build` first. Solray reads the compiled artifacts from the
  Foundry project's output directory.
- A Foundry project with `ast = true` set in `foundry.toml`:

```toml
[profile.default]
ast = true
```

For storage layout inspection, also set:

```toml
[profile.default]
extra_output = ["storageLayout"]
```

## Usage

Solray has two main commands: `inspect` and `scan`.

### `solray inspect`

Explore the structure of a Foundry project.

```bash
# List all deployable contracts
solray inspect contracts

# List all abstract contracts
solray inspect abstracts

# List all interfaces
solray inspect interfaces

# List all libraries
solray inspect libraries
```

#### Inspecting a single contract

```bash
# Show the inheritance graph of a contract
solray inspect inheritance-graph Token

# List all external functions (use --include-read-only to include view/pure)
solray inspect external-functions Token
solray inspect external-functions Token --include-read-only

# List all modifiers (including inherited)
solray inspect modifiers Token

# Show the storage layout
solray inspect storage-layout Token
```

#### Function-level inspection

```bash
# Show the complete call graph of a function
solray inspect call-graph Token transfer

# Show call paths from entry functions to a target function
solray inspect call-path Token _burn

# Show the complete source code of a function
solray inspect function-source Token transfer
```

#### Specifying a project path

All commands accept `--project` (defaults to the current directory):

```bash
solray inspect contracts --project /path/to/forge-project
```

#### Artifact IDs

When a contract name is ambiguous (same name in multiple files), use the
`File.sol:Name` syntax:

```bash
solray inspect inheritance-graph "src/Token.sol:Token"
```

### `solray scan`

Scan for patterns of interest across the codebase.

```bash
# Find all ERC20 transfer/safeTransfer call sites
solray scan erc20-transfer-sink

# Find all asset transfer calls (ERC20, ETH) and ETH receivers
solray scan asset-transfers
```

The scan only inspects source files under the project's `src/` directory (as
configured in `foundry.toml`), excluding test and library code.

## Library

Solray is designed to be **library-first**. The `solray` crate exposes all its
inspectors and scanners as public types, so you can use them programmatically
in your own Rust tooling.

```rust
use solray::Project;
use solray::ContractInspector;

let project = Project::open("path/to/forge-project");
project.validate()?;

let inspector = ContractInspector::new(project);
let output = inspector.inspect()?;
println!("{output}");
```

The public API is organized around types, not functions. Each domain concept
has its own inspector or scanner type:

| Type                        | Purpose                             |
| --------------------------- | ----------------------------------- |
| `Project`                   | Open and validate a Foundry project |
| `ContractInspector`         | List deployable contracts           |
| `AbstractInspector`         | List abstract contracts             |
| `InterfaceInspector`        | List interfaces                     |
| `LibraryInspector`          | List libraries                      |
| `InheritanceGraphInspector` | Show inheritance graph              |
| `ExternalFunctionInspector` | List external functions             |
| `ModifierInspector`         | List modifiers                      |
| `StorageLayoutInspector`    | Show storage layout                 |
| `CallGraphInspector`        | Show call graph of a function       |
| `CallPathInspector`         | Show call paths to a function       |
| `FunctionSourceInspector`   | Show function source code           |
| `Erc20TransferSinkScanner`  | Scan for ERC20 transfer calls       |
| `AssetTransferScanner`      | Scan for all asset transfer patterns |

## Development

```bash
# Run checks (format, clippy, checkrs, markdown)
make check

# Run tests
make test

# Build and install the binary
make bin

# Rebuild test fixtures
make build-fixtures
```

The project requires `ast = true` in `foundry.toml` for all fixtures. Fixtures
are checked into the repository and rebuilt with `make build-fixtures`.

## License

MIT
