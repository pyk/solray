You are working on a `hawk` project.

## Overview

`hawk` is CLI that can be used to inspect any foundry project.

The main goal of the `hawk` is to help security researcher to do audit faster.

## Project Structure

- `hawk` is designed to be library first, which mean the CLI is the consumer of the
  library.

## `hawk` commands

- `hawk inspect contracts`: List all deployables (non-abstract) contracts.
- `hawk inspect abstracts`: List all abtract contracts.
- `hawk inspect libraries`: List all libraries.
- `hawk inspect callgraph Contract::function`: Show the complete callgraph of a
  function.
- `hawk inspect source Contract::function`: Show the complete source code of a function.

## Non-negoitable rules

- You must run `make check` and `make test` before finishing a task.
- You must follow the [project guidelines](docs/project-guidelines.md).
