//! A consumer that closes the pipe early (`dvb-tools t2mi ... | head`) is the
//! normal way to stop reading, not a failure (r11-W-DT-3, #1100): the tool
//! must stop at the first `BrokenPipe`, exit successfully, and not panic in
//! `println!` or fail later on the final flush.
//!
//! Real capture: `fixtures/dvb-t2mi/colombia-capital-t2mi.ts` (T2-MI PID
//! 0x0040, thousands of packets, so output is guaranteed to outlive the
//! reader we drop right after spawning).

use std::process::{Command, Stdio};

fn fixture() -> String {
    test_fixtures::path("dvb-t2mi/colombia-capital-t2mi.ts")
        .to_string_lossy()
        .into_owned()
}

/// Spawn `dvb-tools t2mi <fixture> <args>` with stdout piped, close the read
/// end immediately, and return (success, stderr).
fn run_with_closed_stdout(args: &[&str]) -> (bool, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dvb-tools"))
        .arg("t2mi")
        .arg(fixture())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dvb-tools");
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("wait dvb-tools");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn t2mi_event_dump_stops_quietly_when_stdout_closes() {
    let (ok, stderr, code) = run_with_closed_stdout(&["--pid", "0x0040"]);
    assert!(
        ok,
        "closed stdout must not fail (code {code:?}); stderr: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn t2mi_inner_stops_quietly_when_stdout_closes() {
    let (ok, stderr, code) = run_with_closed_stdout(&["--pid", "0x0040", "--inner"]);
    assert!(
        ok,
        "closed stdout must not fail (code {code:?}); stderr: {stderr}"
    );
    assert!(!stderr.contains("flush stdout"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

/// A read error mid-file (here: the "file" is a directory, so `open`
/// succeeds and the first `read` fails with EISDIR) is a reported failure,
/// never a silent truncated-but-successful run.
#[cfg(unix)]
#[test]
fn a_read_error_is_reported_and_fails() {
    let dir = env!("CARGO_MANIFEST_DIR");
    for sub in ["pids", "dump", "services", "epg"] {
        let out = Command::new(env!("CARGO_BIN_EXE_dvb-tools"))
            .args([sub, dir])
            .output()
            .expect("spawn dvb-tools");
        assert!(!out.status.success(), "{sub} on a directory must fail");
        assert!(!out.stderr.is_empty(), "{sub} must say why");
    }
}
