# Solray

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
- **Pattern scanning**: scan the codebase for patterns of interest (e.g., ERC20
  transfer sinks).

## Installation

### From source

```sh
git clone https://github.com/pyk/solray.git
cd solray
make bin
```

This runs `cargo install --path . --locked` and installs the `solray` binary to
your Cargo bin directory.

### Prerequisites

- [Rust](https://rustup.rs/) (edition 2024)
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

```sh
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

```sh
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

```sh
# Show the complete call graph of a function
solray inspect call-graph Token transfer

# Show call paths from entry functions to a target function
solray inspect call-path Token _burn

# Show the complete source code of a function
solray inspect function-source Token transfer
```

#### Specifying a project path

All commands accept `--project` (defaults to the current directory):

```sh
solray inspect contracts --project /path/to/forge-project
```

#### Artifact IDs

When a contract name is ambiguous (same name in multiple files), use the
`File.sol:Name` syntax:

```sh
solray inspect inheritance-graph "src/Token.sol:Token"
```

### `solray scan`

Scan for patterns of interest across the codebase.

```sh
# Find all ERC20 transfer/safeTransfer call sites
solray scan erc20-transfer-sink
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

## Development

```sh
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

## Project structure

```
src/
  main.rs                  CLI entry point
  lib.rs                   Public API re-exports
  project.rs               Project opening and validation
  build_info.rs            Build-info file parsing for source ID resolution
  artifact_index.rs        Artifact index
  call_graph.rs            Call graph data types
  inspectors/              Inspector types (one per domain concept)
  scanners/                Scanner types for pattern detection
fixtures/                  Test fixtures (Foundry projects)
```
