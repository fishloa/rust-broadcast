//! `ts-fix --keep-pids` takes typed PIDs (#1159): a value outside the 13-bit
//! range (ISO/IEC 13818-1 §2.4.3.2) is rejected by the argument parser instead
//! of silently never matching a packet, and a valid list filters a real
//! capture to exactly those PIDs (plus the always-kept PAT PID).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TS_PACKET_SIZE: usize = 188;
const PAT_PID: u16 = 0x0000;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("m6-single.ts")
}

/// Independent 13-bit PID extraction from the wire bytes.
fn raw_pid(packet: &[u8]) -> u16 {
    (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2])
}

fn run(input: &Path, output: &Path, keep: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ts-fix"))
        .args([
            "--input",
            &input.display().to_string(),
            "--output",
            &output.display().to_string(),
            "--keep-pids",
            keep,
        ])
        .output()
        .expect("ts-fix binary must run");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn temp(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ts-fix-keep-pids-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    (dir.join("in.ts"), dir.join("out.ts"))
}

#[test]
fn out_of_range_keep_pid_is_rejected_not_silently_ignored() {
    let (input, output) = temp("reject");
    fs::copy(fixture_path(), &input).unwrap();
    for bad in ["8192", "65535", "256,8192"] {
        let (ok, stderr) = run(&input, &output, bad);
        assert!(!ok, "--keep-pids {bad} must be rejected");
        assert!(
            stderr.contains("0x2000") || stderr.contains("0xFFFF") || stderr.contains("13-bit"),
            "stderr must name the out-of-range PID, got: {stderr}"
        );
    }
    assert!(
        !output.exists(),
        "a rejected invocation must not write output"
    );
}

#[test]
fn valid_keep_pids_filter_a_real_capture_to_exactly_those_pids() {
    let data = fs::read(fixture_path()).unwrap();
    let present: BTreeSet<u16> = data
        .chunks_exact(TS_PACKET_SIZE)
        .map(raw_pid)
        .filter(|&p| p != PAT_PID && p != 0x1FFF)
        .collect();
    let keep = *present.iter().next().expect("fixture carries non-PAT PIDs");

    let (input, output) = temp("filter");
    fs::copy(fixture_path(), &input).unwrap();
    let (ok, stderr) = run(&input, &output, &keep.to_string());
    assert!(ok, "valid --keep-pids must succeed: {stderr}");

    let out = fs::read(&output).unwrap();
    assert!(!out.is_empty(), "output must not be empty");
    let got: BTreeSet<u16> = out.chunks_exact(TS_PACKET_SIZE).map(raw_pid).collect();
    let want: BTreeSet<u16> = [PAT_PID, keep].into();
    assert!(
        got.is_subset(&want) && got.contains(&keep),
        "output PIDs {got:?} must be within {want:?} and include {keep:#06x}"
    );
}
