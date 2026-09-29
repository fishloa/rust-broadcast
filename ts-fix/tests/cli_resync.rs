//! End-to-end CLI tests for the `ts-fix` binary (#1101 W-TF-5).
//!
//! The engine API contract is one clean 188-byte packet at a time, but a
//! capture file is a raw TS byte stream: it can start mid-packet, carry a
//! corrupted sync byte, or be 204-byte RS-coded. Before #1101 the CLI sliced
//! the input with `chunks(188)` and propagated `push`'s error, so the whole
//! run aborted with `missing TS sync byte` and wrote nothing. The CLI must
//! instead resynchronise (`mpeg_ts::resync::TsResync`), skip/count bad
//! packets, and still write repaired output.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SYNC_BYTE: u8 = 0x47;
const TS_PACKET_SIZE: usize = 188;
/// Reed-Solomon parity appended to each 204-byte packet (DVB outer FEC).
const RS_PARITY_LEN: usize = 16;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("m6-single.ts")
}

fn pid_of(packet: &[u8]) -> u16 {
    (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16
}

/// Run the built `ts-fix` binary; returns (exit success, stderr, output bytes
/// if written).
fn run_cli(input: &Path, output: &Path) -> (bool, String) {
    let exe = assert_cmd_exe();
    let out = Command::new(&exe)
        .args([
            "--input",
            &input.display().to_string(),
            "--output",
            &output.display().to_string(),
            "--repair-continuity",
            "--drop-nulls",
        ])
        .output()
        .expect("ts-fix binary must run");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn assert_cmd_exe() -> PathBuf {
    let exe = env!("CARGO_BIN_EXE_ts-fix");
    assert!(
        Path::new(exe).exists(),
        "cargo must build the ts-fix binary for integration tests"
    );
    PathBuf::from(exe)
}

/// A capture that starts mid-packet must still be repaired: the leading
/// partial packet is dropped, the remainder is output intact.
#[test]
fn cli_repairs_capture_starting_mid_packet() {
    let fixture = fs::read(fixture_path()).expect("fixture");
    let mut input = Vec::with_capacity(fixture.len() + 7);
    input.extend_from_slice(&[0xAB; 7]); // junk before the first full packet
    input.extend_from_slice(&fixture);

    let (input_path, output_path) = temp_paths("midpacket");
    fs::write(&input_path, &input).expect("write input");
    let (ok, stderr_text) = run_cli(&input_path, &output_path);
    assert!(
        ok,
        "CLI must not abort on a mid-packet start: {stderr_text}"
    );

    let output = fs::read(&output_path).expect("output must be written");
    assert!(
        !output.is_empty(),
        "a mid-packet capture must still produce repaired output"
    );
    assert_eq!(output.len() % TS_PACKET_SIZE, 0);
    assert!(
        output
            .chunks(TS_PACKET_SIZE)
            .all(|p| p[0] == SYNC_BYTE && pid_of(p) != 0x1FFF),
        "output must be aligned 188-byte packets with nulls dropped"
    );
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
}

/// A single corrupted sync byte mid-file must not abort the whole run: the
/// corrupt packet is lost (re-sync), the rest is still written.
#[test]
fn cli_survives_corrupt_sync_byte_mid_stream() {
    let mut input = fs::read(fixture_path()).expect("fixture");
    let corrupt_at = 50 * TS_PACKET_SIZE;
    input[corrupt_at] = 0x00;

    let (input_path, output_path) = temp_paths("corruptsync");
    fs::write(&input_path, &input).expect("write input");
    let (ok, stderr_text) = run_cli(&input_path, &output_path);
    assert!(ok, "CLI must not abort on one bad sync byte: {stderr_text}");

    let output = fs::read(&output_path).expect("output must be written");
    assert!(
        output.len() / TS_PACKET_SIZE >= 50,
        "packets after the corrupt sync byte must still be repaired and written"
    );
    assert_eq!(output.len() % TS_PACKET_SIZE, 0);
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
}

/// A 204-byte (RS-coded) capture must be accepted: the parity bytes are
/// stripped and 188-byte output is produced.
#[test]
fn cli_accepts_204_byte_rs_coded_capture() {
    let fixture = fs::read(fixture_path()).expect("fixture");
    let mut input = Vec::with_capacity(fixture.len() + fixture.len() / 11);
    for chunk in fixture.chunks(TS_PACKET_SIZE) {
        input.extend_from_slice(chunk);
        input.extend_from_slice(&[0x5A; RS_PARITY_LEN]);
    }

    let (input_path, output_path) = temp_paths("rs204");
    fs::write(&input_path, &input).expect("write input");
    let (ok, stderr_text) = run_cli(&input_path, &output_path);
    assert!(ok, "CLI must accept a 204-byte RS capture: {stderr_text}");

    let output = fs::read(&output_path).expect("output must be written");
    assert!(
        !output.is_empty(),
        "an RS-coded capture must produce repaired output"
    );
    assert_eq!(output.len() % TS_PACKET_SIZE, 0);
    assert!(
        output.chunks(TS_PACKET_SIZE).all(|p| p[0] == SYNC_BYTE),
        "parity bytes must be stripped: every output packet starts with 0x47"
    );
    let _ = fs::remove_file(&input_path);
    let _ = fs::remove_file(&output_path);
}

fn temp_paths(prefix: &str) -> (PathBuf, PathBuf) {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir();
    (
        dir.join(format!("tsfix-cli-{prefix}-{}.ts", std::process::id() + n)),
        dir.join(format!("tsfix-cli-{prefix}-{}.out", std::process::id() + n)),
    )
}
