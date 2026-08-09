---
name: solray-bug-fixing
description: Fix solray bugs from `solray-testing` bug reports. Use when a
  solray `inspect` or `scan` output contradicts the verified Solidity source or
  Foundry artifacts.
---

# Solray Fix Bugs Skill

You fix solray bugs reported by `solray-testing`. A bug is any output that
contradicts the verified Solidity source or Foundry artifacts, such as a wrong
contract list, an unresolved modifier, or an invalid source block.

-------------------------------------------------------------------------------

## Input

| #   | Name            | Path                     |
| --: | :-------------- | :----------------------- |
|   1 | Bug report      | `solray-bug-reports.md`  |
|   2 | Foundry project | Path from the bug report |

-------------------------------------------------------------------------------

## Output

| #   | Name            | Path                                                           |
| --: | :-------------- | :------------------------------------------------------------- |
|   1 | Regression test | Test function in the affected inspector or scanner test module |
|   2 | Test fixture    | `fixtures/<feature>/src/` and `fixtures/<feature>/expected/`   |
|   3 | Fixed source    | Affected source under `src/inspectors/` or `src/scanners/`     |
|   4 | Changelog entry | `CHANGELOG.md` under `[Unreleased]`                            |

-------------------------------------------------------------------------------

## Rules

| ID     | Rule                                                                                                                       |
| :----- | :------------------------------------------------------------------------------------------------------------------------- |
| FIX-01 | You MUST reproduce the bug with the exact `Command` from the bug report before diagnosing                                  |
| FIX-02 | The reproduction command MUST use `cargo run --`                                                                           |
| FIX-03 | Expected output MUST be derived from the project's verified Solidity source and Foundry artifacts                          |
| FIX-04 | You MUST NOT clean or rebuild the reported Foundry project before reproducing the bug                                      |
| FIX-05 | You MUST identify the root cause before creating the regression test                                                       |
| FIX-06 | When diagnosing, you MAY add permanent `debug!` statements in the affected inspector or scanner                            |
| FIX-07 | When the failing command supports `--debug`, you MUST run it with `--debug` to reveal the `debug!` output                  |
| FIX-08 | When the failing command lacks `--debug`, you MUST add `--debug` support to it as part of the bug fix instead of deferring |
| FIX-09 | You MUST create the regression test before fixing the bug                                                                  |
| FIX-10 | The regression fixture MUST include a source file under `fixtures/<feature>/src/`                                          |
| FIX-11 | The regression fixture MUST include expected output under `fixtures/<feature>/expected/`                                   |
| FIX-12 | Fixture artifacts MUST be generated with `forge build`                                                                     |
| FIX-13 | Fixture artifacts MUST NOT be created or edited manually                                                                   |
| FIX-14 | The regression test MUST fail against the unfixed code                                                                     |
| FIX-15 | The regression test failure MUST reproduce the reported bug                                                                |
| FIX-16 | You MUST fix the bug only after the regression test reproduces it                                                          |
| FIX-17 | After the fix, the regression test MUST pass                                                                               |
| FIX-18 | The regression test MUST assert the full output with `assert_eq!` against the expected file                                |
| FIX-19 | The regression test MUST NOT use `.contains()`                                                                             |
| FIX-20 | Foundry artifact JSON MUST be explored with `python3 -c` one-liners                                                        |
| FIX-21 | When the fixture source or expected output already exists, you MUST abort before creating or overwriting it                |
| FIX-22 | When consulting crate documentation, you MUST use `cargo txt`                                                              |
| FIX-23 | You MUST run `make lint` before finishing                                                                                  |
| FIX-24 | You MUST run `make test` before finishing                                                                                  |
| FIX-25 | You MUST add a `### Fixed` entry for the bug to `CHANGELOG.md` under `[Unreleased]` before finishing                       |

-------------------------------------------------------------------------------

## Workflow

1. Reproduce the bug.
   - Run the exact `Command` from the bug report entry:

     ```bash
     cargo run -- inspect contracts --project /path/to/project/
     ```

   - Compare the reproduced output with the report's `Actual output`.

   - Verify the report's `Expected output` against the project's Solidity
     source and Foundry artifacts.

   - Stop when the command no longer reproduces the reported bug.

2. Find the root cause.
   - Explore the Foundry artifact JSON under the project `out/` dir with
     `python3 -c` one-liners:

     ```bash
     python3 -c "import json; a = json.load(open('/path/to/project/out/<File>.sol/<Contract>.json')); print(a['ast']['nodes'][0]['name'])"
     ```

   - Add permanent `debug!` statements in the affected inspector or scanner
     when the resolution path is unclear.

   - Run the failing command with `--debug` when the flag is supported:

     ```bash
     cargo run -- inspect modifiers <Contract> --project /path/to/project/ --debug
     ```

   - Trace the affected feature's resolution path until the root cause explains
     the wrong output.

3. Create the regression test before fixing the bug.
   - Determine the affected feature from the bug report command, for example
     `contracts`, `inspect-function-source`, or `erc20-transfer-sinks`.

   - Abort when `fixtures/<feature>/src/` or `fixtures/<feature>/expected/`
     already contains the fixture.

   - Add the fixture source under `fixtures/<feature>/src/`.

   - Add the expected output under `fixtures/<feature>/expected/`.

   - Generate the fixture artifacts with `forge build`:

     ```bash
     forge build --root fixtures/<feature> --force --quiet
     ```

   - Add a test in the affected feature's test module under `src/inspectors/`
     or `src/scanners/`, matching the module's existing test helper, and assert
     the full output with `assert_eq!` against the expected file using
     `include_str!`.

4. Confirm the regression test fails.
   - Run the new test:

     ```bash
     cargo test <test_name>
     ```

   - Verify the failure matches the reported wrong output.

   - Verify the failure is not caused by a fixture or build error.

5. Fix the bug.
   - Fix the root cause in the affected inspector or scanner source.

   - When the failing command lacks `--debug`, add `--debug` support to it
     instead of deferring.

6. Verify the fix.
   - Run the regression test and confirm it passes:

     ```bash
     cargo test <test_name>
     ```

   - Add a `### Fixed` entry for the bug to `CHANGELOG.md` under `[Unreleased]`
     before finishing.

   - Run `make lint` and `make test` before finishing:

     ```bash
     make lint
     make test
     ```

-------------------------------------------------------------------------------

## Test Template

Match the affected feature's existing test module under `src/inspectors/` or
`src/scanners/`. Each feature has its own fixture directory:

| Feature                    | Fixture directory                   |
| :------------------------- | :---------------------------------- |
| `inspect contracts`        | `fixtures/contracts/`               |
| `inspect function-source`  | `fixtures/inspect-function-source/` |
| `scan erc20-transfer-sink` | `fixtures/erc20-transfer-sinks/`    |
| `scan asset-transfers`     | `fixtures/asset-transfers/`         |

Copy an existing test from the affected module and adapt it to the new fixture.
Name the expected file after the test case.
