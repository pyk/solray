You are working on a `hawk` project.

## Non-negotiable rules

List of CRITICAL rules that you must follow every time. Failing to do so will have a
severe negative impact on the project and the user.

### General Rules

- You must run `make check` and `make test` before finishing a task.
- You must use `cargo txt` to view crate documentation.
- You must not create fixture artifacts manually. Run `forge build` to generate them.

### Code Design Rules

- You must separate I/O from logic.
- You must not add comment block header.
- You must design the public API around types, not functions.
- You must organize modules around domain concepts.
- You must keep one primary type per module (`project.rs` -> Project).
- You must re-export public types at the module level.
- You must keep implementation details private.
- You must not create `utils.rs`, `helpers.rs`, or `common.rs`.
- You must put behavior on the type that owns the state.
- You must use constructors as entry points (e.g. `Project::open(path)`).
- You must not prefix function names with the type name (bad: `build_project`).
- You must use free functions only when there is no natural owner.
- You must use option structs for methods with many parameters.
- You must use operation types (Analyzer, Linker, Builder) for complex workflows.
- You must use context objects for internal workflows to prevent parameter explosion.
- You must avoid deep module hierarchies.

### Testing Rules

- You must assert exact error messages in tests with `assert_eq!`, never `.contains()`.

## Project Overview

`hawk` is CLI that can be used to inspect any foundry project.

The main goal of the `hawk` is to help security researcher to do audit faster.

## Project Structure

- `hawk` is designed to be library first, which mean the CLI is the consumer of the
  library.

## `hawk` commands

- `hawk inspect contracts`: List all deployable contracts.
- `hawk inspect abstracts`: List all abtract contracts.
- `hawk inspect libraries`: List all libraries.
- `hawk inspect interfaces`: List all interfaces.
- `hawk inspect inheritances <declaration>`: Show inheritance graph of a
  contract/interface.
- `hawk inspect calls Contract::function`: Show the complete callgraph of a function.
- `hawk inspect source Contract::function`: Show the complete source code of a function.

## cargo txt

1. Build documentation: `cargo txt build <crate>`
2. List all items: `cargo txt list <lib_name>`
3. View a specific item: `cargo txt show <lib_name>::<item>`

For example:

```sh
# Build the serde crate documentation
cargo txt build serde

# List all items in serde
cargo txt list serde

# View serde crate overview
cargo txt show serde

# View serde::Deserialize trait documentation
cargo txt show serde::Deserialize
```

## checkrs

To suppress the lint use the `// checkrs: allow(<name>)` for example:

```rust
// checkrs: allow(clone_in_loops)
let mut fresh_chain = self.chain.clone();
```

or

```rust
let mut fresh_chain = self.chain.clone(); // checkrs: allow(clone_in_loops)
```
