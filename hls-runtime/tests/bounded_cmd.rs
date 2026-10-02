//! Proof that the deadline-bounded runner fixes the `Command::output()` hang:
//! a stub tool forks a sleeping grandchild that holds stdout/stderr, then
//! exits at once. `output()` blocks until the grandchild dies; the bounded
//! runner returns as soon as the tool itself exits.
#![cfg(unix)]

#[path = "support/bounded.rs"]
mod bounded;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// How long the stub's grandchild keeps the pipes open.
const GRANDCHILD_SLEEP_SECS: u64 = 6;
/// Window within which the pipe-based `output()` must still be blocked.
const OUTPUT_BLOCK_WINDOW: Duration = Duration::from_secs(2);
/// Deadline handed to the bounded runner.
const DEADLINE: Duration = Duration::from_secs(4);

fn stub(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bounded-stub-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("stub dir");
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn leaky_stub() -> PathBuf {
    stub(
        "leaky-tool",
        &format!("sleep {GRANDCHILD_SLEEP_SECS} &\necho hello\nexit 0"),
    )
}

#[test]
fn pipe_based_output_hangs_on_a_pipe_holding_grandchild() {
    let tool = leaky_stub();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = Command::new(&tool).output().expect("spawn");
        let _ = tx.send(out);
    });
    assert!(
        rx.recv_timeout(OUTPUT_BLOCK_WINDOW).is_err(),
        "output() was expected to block on the grandchild-held pipe"
    );
}

#[test]
fn bounded_runner_returns_despite_pipe_holding_grandchild() {
    let tool = leaky_stub();
    let start = Instant::now();
    let out = bounded::output_bounded(&mut Command::new(&tool), DEADLINE).expect("run");
    assert!(out.status.success());
    assert_eq!(out.stdout, b"hello\n");
    assert!(
        start.elapsed() < Duration::from_secs(GRANDCHILD_SLEEP_SECS),
        "returned only after the grandchild exited: {:?}",
        start.elapsed()
    );
}

#[test]
fn bounded_runner_kills_an_overrunning_tool_with_a_clear_error() {
    let tool = stub("slow-tool", "exec sleep 30");
    let start = Instant::now();
    let err = bounded::output_bounded(&mut Command::new(&tool), Duration::from_millis(300))
        .expect_err("must time out");
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    assert!(err.to_string().contains("slow-tool"), "{err}");
    assert!(err.to_string().contains("hard deadline"), "{err}");
    assert!(start.elapsed() < Duration::from_secs(5));
}
