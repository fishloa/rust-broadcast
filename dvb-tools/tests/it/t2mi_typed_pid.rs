//! `dvb-tools t2mi --pid` takes a typed PID (#1159): a value outside the
//! 13-bit range (ISO/IEC 13818-1 §2.4.3.2) is a usage error rather than a PID
//! that silently matches nothing, and the real capture's T2-MI PID still
//! produces packets.

use std::process::Command;

fn fixture() -> String {
    test_fixtures::path("dvb-t2mi/colombia-capital-t2mi.ts")
        .to_string_lossy()
        .into_owned()
}

fn run(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dvb-tools"))
        .args(args)
        .output()
        .expect("spawn dvb-tools");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn out_of_range_t2mi_pid_is_a_usage_error() {
    let path = fixture();
    for bad in ["0x2000", "8192", "0xFFFF"] {
        let (ok, stdout, stderr) = run(&["t2mi", &path, "--pid", bad]);
        assert!(!ok, "--pid {bad} must be rejected");
        assert!(stdout.is_empty(), "no output for a rejected --pid {bad}");
        assert!(stderr.contains("--pid expects"), "{stderr}");
    }
}

#[test]
fn the_captures_t2mi_pid_decodes_and_another_pid_yields_nothing() {
    let path = fixture();
    let (ok, stdout, stderr) = run(&["t2mi", &path, "--pid", "0x0040"]);
    assert!(ok, "t2mi on the real capture failed: {stderr}");
    assert!(
        stdout.contains("type=BBFRAME"),
        "the capture's T2-MI PID (0x0040) must yield packets, got: {stdout}"
    );
    let (ok, other, _) = run(&["t2mi", &path, "--pid", "0x0041"]);
    assert!(ok);
    assert!(
        !other.contains("type=BBFRAME"),
        "a PID that is not the T2-MI PID must yield no packets"
    );
}

/// #1034: each T2 PLP carries an independent TS, so `--inner` on a multi-PLP
/// feed must not merge them into one output. The fixture has two PLPs (ids
/// zero and one) on T2-MI PID 0x1000, each carrying one real TS packet split
/// across two BBFrames (built from `fixtures/ts/scte35-real.ts` and
/// `fixtures/mpeg-ts/af-pcr-stuffing.ts` by `dvb-t2mi`'s
/// `inner_ts_plp_carryover` test, which pins this file byte-for-byte).
#[test]
fn inner_on_multi_plp_feed_requires_plp_selection() {
    let path = format!(
        "{}/../fixtures/dvb-t2mi/two-plp-synthetic.ts",
        env!("CARGO_MANIFEST_DIR")
    );

    let (ok, stdout, stderr) = run(&["t2mi", &path, "--pid", "0x1000", "--inner"]);
    assert!(!ok, "multi-PLP --inner without --plp must fail");
    assert!(
        stdout.is_empty(),
        "no merged TS written: {} bytes",
        stdout.len()
    );
    assert!(stderr.contains("2 PLPs found (0, 1)"), "{stderr}");
    assert!(stderr.contains("--plp"), "{stderr}");

    // Binary stdout: read raw bytes (the lossy-String helper above would
    // inflate non-UTF-8 bytes).
    for plp in ["0", "1"] {
        let out = Command::new(env!("CARGO_BIN_EXE_dvb-tools"))
            .args(["t2mi", &path, "--pid", "0x1000", "--inner", "--plp", plp])
            .output()
            .expect("spawn dvb-tools");
        assert!(out.status.success(), "--plp {plp} must succeed");
        assert_eq!(out.stdout.len(), 188, "exactly PLP {plp}'s one packet");
    }
}
