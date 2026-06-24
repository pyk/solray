You are working on a `hawk` project.

## Non-negoitable rules

- You must run `make check` and `make test` before finishing a task.
- You must follow the [project guidelines](docs/project-guidelines.md).
- You must use `cargo txt` to view crate documentation.
- You must separate I/O from a logic.
- You must not create fixture artifact manually, run `forge build` to generate the
  artifact.

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
