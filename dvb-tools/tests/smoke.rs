//! Smoke tests: run the built `dvb-tools` binary against real fixtures.
//!
//! These tests are deliberately minimal — exit status + non-empty stdout —
//! because the upstream sealed fixtures may change as broadcasters rotate,
//! and pinning text would lock the CLI to its first-day output forever. The
//! rich assertions live in the per-crate test suites (`dvb-si/tests/*`).
//!
//! Each integration test's CWD is the crate directory (`dvb-tools/`), so
//! fixture paths are anchored at the workspace root via `CARGO_MANIFEST_DIR`.

use std::process::{Command, Stdio};

/// Workspace-root-relative fixture (resolved at compile time from the
/// crate's manifest dir, which is `dvb-tools/`).
fn fixture(rel: &str) -> String {
    format!("{}{}", env!("CARGO_MANIFEST_DIR"), rel)
}

/// Run the built binary and assert: exit success + at least one byte on
/// stdout. Stderr is captured but not asserted on (stats lines change as
/// the demux evolves).
fn run(args: &[&str]) -> (bool, Vec<u8>, String) {
    let bin = env!("CARGO_BIN_EXE_dvb-tools");
    let output = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn dvb-tools");
    let ok = output.status.success();
    (
        ok,
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn dump_m6_single() {
    let path = fixture("/../fixtures/ts/m6-single.ts");
    let (ok, stdout, stderr) = run(&["dump", &path]);
    assert!(ok, "dump m6-single failed: stderr={stderr}");
    assert!(
        !stdout.is_empty(),
        "dump m6-single produced empty stdout: stderr={stderr}"
    );
}

#[test]
fn dump_tnt_isi6() {
    let path = fixture("/../fixtures/dvb-si/tnt-5w-12732v-isi6-10s.ts");
    let (ok, stdout, stderr) = run(&["dump", &path]);
    assert!(ok, "dump tnt-isi6 failed: stderr={stderr}");
    assert!(
        !stdout.is_empty(),
        "dump tnt-isi6 produced empty stdout: stderr={stderr}"
    );
}

#[test]
fn services_m6_single_exits_success() {
    // m6-single.ts is a brief capture with no SDT — `services` should still
    // exit cleanly and write a note about the missing table.
    let path = fixture("/../fixtures/ts/m6-single.ts");
    let (ok, _, stderr) = run(&["services", &path]);
    assert!(
        ok,
        "services m6-single should exit 0 even with no SDT; stderr={stderr}"
    );
}

#[test]
fn pids_m6_single() {
    let path = fixture("/../fixtures/ts/m6-single.ts");
    let (ok, stdout, stderr) = run(&["pids", &path]);
    assert!(ok, "pids m6-single failed: stderr={stderr}");
    assert!(
        !stdout.is_empty(),
        "pids m6-single produced empty stdout: stderr={stderr}"
    );
}
