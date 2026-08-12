//! Process-level regression tests for the CLI binary.

use std::process::{Command, Stdio};

/// The CLI must terminate quietly when a consumer closes stdout early
/// (e.g. `solray inspect function-source ... | head`). Regression test for
/// BUG-32: the Rust runtime ignores SIGPIPE, so an early-closing pipe used to
/// make `print!` panic with a broken-pipe error.
#[test]
fn closed_stdout_pipe_does_not_panic() {
    let (reader, writer) = std::io::pipe().expect("create pipe");
    // Close the read end before spawning so every child write fails with
    // EPIPE, regardless of pipe-buffer sizes or timing.
    drop(reader);

    let mut child = Command::new(env!("CARGO_BIN_EXE_solray"))
        .args([
            "inspect",
            "function-source",
            "LibraryScopeUser",
            "run",
            "--project",
            "fixtures/inspect-function-source",
        ])
        .stdout(Stdio::from(writer))
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn solray binary");

    let stderr =
        std::io::read_to_string(child.stderr.take().expect("stderr")).expect("read child stderr");
    let status = child.wait().expect("wait for solray binary");

    assert!(
        !stderr.contains("panicked"),
        "solray panicked on a closed stdout pipe (status {status}):\n{stderr}"
    );
}
