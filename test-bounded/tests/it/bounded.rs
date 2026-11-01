//! Proof that [`test_bounded::output_bounded`] enforces its deadline: a child
//! that OUTLIVES the bound is killed and reported as `TimedOut` rather than
//! letting the caller hang. Unix-only.
//!
//! The stubs are `sh -c "<script text>"` rather than script files written to
//! disk: writing a file and then exec'ing it from parallel test threads is the
//! classic ETXTBSY ("text file busy") flake, so no file is ever created here
//! (and no temp dir is left behind).
//!
//! Also proves the runner returns as soon as the tool itself exits even when
//! a forked grandchild still holds stdout/stderr, which is the reason this
//! helper exists instead of `Command::output()`.
#![cfg(unix)]

use std::process::Command;
use std::time::{Duration, Instant};

/// The interpreter every stub runs through (`sh -c "<script text>"`).
const SHELL: &str = "/bin/sh";
/// How long the stub's grandchild keeps the pipes open.
const GRANDCHILD_SLEEP_SECS: u64 = 6;
/// Deadline handed to the bounded runner for the overrun case.
const OVERRUN_DEADLINE: Duration = Duration::from_millis(300);

/// Build a command that runs `body` through `sh -c` — no file on disk.
fn stub(body: &str) -> Command {
    let mut cmd = Command::new(SHELL);
    cmd.arg("-c").arg(body);
    cmd
}

#[test]
fn overrunning_child_times_out_instead_of_hanging() {
    let mut tool = stub("exec sleep 30");
    let start = Instant::now();
    let err = test_bounded::output_bounded(&mut tool, OVERRUN_DEADLINE)
        .expect_err("a child outliving the deadline must be killed and reported");
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    assert!(
        err.to_string().contains(SHELL),
        "error must name the program: {err}"
    );
    assert!(
        err.to_string().contains("hard deadline"),
        "error must say it hit the deadline: {err}"
    );
    // Bounded by the deadline, not by the child's own 30 s sleep.
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "returned only after the child's own sleep: {:?}",
        start.elapsed()
    );
}

#[test]
fn pipe_holding_grandchild_does_not_block_the_runner() {
    let mut tool = stub(&format!(
        "sleep {GRANDCHILD_SLEEP_SECS} &\necho hello\nexit 0"
    ));
    let start = Instant::now();
    let out = test_bounded::output_bounded(&mut tool, Duration::from_secs(4)).expect("run");
    assert!(out.status.success());
    assert_eq!(out.stdout, b"hello\n");
    assert!(
        start.elapsed() < Duration::from_secs(GRANDCHILD_SLEEP_SECS),
        "returned only after the grandchild exited: {:?}",
        start.elapsed()
    );
}
