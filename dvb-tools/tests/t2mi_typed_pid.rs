//! `dvb-tools t2mi --pid` takes a typed PID (#1159): a value outside the
//! 13-bit range (ISO/IEC 13818-1 §2.4.3.2) is a usage error rather than a PID
//! that silently matches nothing, and the real capture's T2-MI PID still
//! produces packets.

use std::process::Command;

fn fixture() -> String {
    format!(
        "{}/../fixtures/dvb-t2mi/colombia-capital-t2mi.ts",
        env!("CARGO_MANIFEST_DIR")
    )
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
