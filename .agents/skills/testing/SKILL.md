---
name: solray-testing
description: Try solray, check output and report bugs.
---

# Solray Testing Skill

You try all solray commands, check output against the project's Solidity source
and Foundry artifacts, and report bugs. A bug is any output that contradicts
the verified source or artifacts, such as an abstract contract or interface
listed by `inspect contracts`, or a missing symbol or invalid source emitted by
`inspect function-source`.

-------------------------------------------------------------------------------

## Input

| #   | Name            | Path                    |
| --: | :-------------- | :---------------------- |
|   1 | Foundry project | Path to foundry project |

-------------------------------------------------------------------------------

## Output

| #   | Name        | Path                    |
| --: | :---------- | :---------------------- |
|   1 | Bug reports | `solray-bug-reports.md` |

-------------------------------------------------------------------------------

## Rules

| ID     | Rule                                                                                                                                                     |
| :----- | :------------------------------------------------------------------------------------------------------------------------------------------------------- |
| STT-01 | For bug reports, you MUST convert the command into `cargo run --`                                                                                        |
| STT-02 | Expected output MUST be derived from the project's Solidity source and Foundry artifacts                                                                 |
| STT-03 | A bug MUST be reported when any output contradicts the verified Solidity source or artifact data                                                         |
| STT-04 | A declaration MUST be listed only by the command matching its Solidity kind: `contracts`, `abstracts`, `interfaces`, or `libraries`                      |
| STT-05 | `inspect contracts` output MUST list every deployable (non-abstract, non-interface, non-library) contract                                                |
| STT-06 | `inspect function-source` output MUST include the requested function's source code                                                                       |
| STT-07 | `inspect function-source` output MUST include every symbol referenced by the function, recursively                                                       |
| STT-08 | `inspect function-source` output MUST NOT omit or drop any referenced symbol                                                                             |
| STT-09 | `inspect function-source` code blocks MUST be valid Solidity source                                                                                      |
| STT-10 | `inspect function-source` code blocks MUST match the original source after intended dedent and NatSpec resolution                                        |
| STT-11 | `inspect function-source` output MUST NOT contain unresolved placeholders such as `// unable to read` or `unknown` unless present in the original source |
| STT-12 | A command crash, panic, or unexpected error exit MUST be reported as a bug                                                                               |
| STT-13 | A command that exits successfully with wrong output MUST be reported as a bug                                                                            |
| STT-14 | A command that fails when it should succeed MUST be reported as a bug                                                                                    |
| STT-15 | A command that succeeds when it should fail MUST be reported as a bug                                                                                    |
| STT-16 | You MUST NOT clean the Foundry project before testing                                                                                                    |
| STT-17 | You MUST NOT rebuild the Foundry project before testing                                                                                                  |
| STT-18 | You MUST test against the project's current build state, including incremental and stale artifacts                                                       |
| STT-19 | Bugs reproduced only with incremental build artifacts MUST be reported                                                                                   |

-------------------------------------------------------------------------------

## Workflow

1. Run `solray --version` to make sure `solray` is available.
2. Run `solray --help` to check all available commands.
3. Try each command one by one against the project:
   - Convert the command into `cargo run --`.
   - Keep the project's existing artifacts; do not clean or rebuild the
     project.
   - Verify the output against the Solidity source and Foundry artifacts.
   - Classify every discrepancy as a bug using the Rules.
4. Write the bug reports to `solray-bug-reports.md` using the Template.

-------------------------------------------------------------------------------

## Template

You MUST use the following template:

````markdown

# Solray Bug Reports

Project Path: `<absolute_path>`

-------------------------------------------------------------------------------

## BUG-01: <Bug Title>

Command:

```bash
cargo run -- inspect contracts --project <absolute_path>
```

Expected output:

<description about expected output>

Actual output:

<description about actual output and examples>

-------------------------------------------------------------------------------

## BUG-##: <Bug Title>

Command:

```bash
cargo run -- inspect contracts --project <absolute_path>
```

Expected output:

<description about expected output>

Actual output:

<description about actual output and examples>

-------------------------------------------------------------------------------

````
