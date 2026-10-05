//! Proof that [`test_bounded::output_bounded`] enforces its deadline: a child
//! that OUTLIVES the bound is killed and reported as `TimedOut` rather than
//! letting the caller hang. Unix-only (the stub is a `#!/bin/sh` script).
//!
//! Also proves the runner returns as soon as the tool itself exits even when
//! a forked grandchild still holds stdout/stderr, which is the reason this
//! helper exists instead of `Command::output()`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// How long the stub's grandchild keeps the pipes open.
const GRANDCHILD_SLEEP_SECS: u64 = 6;
/// Deadline handed to the bounded runner for the overrun case.
const OVERRUN_DEADLINE: Duration = Duration::from_millis(300);

fn stub(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("test-bounded-stub-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("stub dir");
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[test]
fn overrunning_child_times_out_instead_of_hanging() {
    let tool = stub("slow-tool", "exec sleep 30");
    let start = Instant::now();
    let err = test_bounded::output_bounded(&mut Command::new(&tool), OVERRUN_DEADLINE)
        .expect_err("a child outliving the deadline must be killed and reported");
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    assert!(
        err.to_string().contains("slow-tool"),
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
    let tool = stub(
        "leaky-tool",
        &format!("sleep {GRANDCHILD_SLEEP_SECS} &\necho hello\nexit 0"),
    );
    let start = Instant::now();
    let out = test_bounded::output_bounded(&mut Command::new(&tool), Duration::from_secs(4))
        .expect("run");
    assert!(out.status.success());
    assert_eq!(out.stdout, b"hello\n");
    assert!(
        start.elapsed() < Duration::from_secs(GRANDCHILD_SLEEP_SECS),
        "returned only after the grandchild exited: {:?}",
        start.elapsed()
    );
}
