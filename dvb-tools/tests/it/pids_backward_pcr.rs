//! `dvb-tools pids` must reject a backward PCR step near zero (issue #2495).
//!
//! `PcrTrack::push` keeps its unwrapped 27 MHz value signed. Before the fix the
//! unwrapped sum was narrowed with `as u64`, so a backward step taken while the
//! running value was still near zero turned into a value near `u64::MAX` and
//! `estimate_bitrate_mbps` reported a huge, wrong bitrate instead of the
//! documented "no bitrate" rejection.
//!
//! This drives the whole CLI over a real capture whose PCR fields are rewritten
//! to 5 then 0 on the 27 MHz clock (ISO/IEC 13818-1 §2.4.3.5), so it covers the
//! wire path and not just the in-module unit test.

use std::io::Write;
use std::process::{Command, Stdio};

use mpeg_ts::ts::{AF_PCR_FLAG, Pcr, TS_PACKET_SIZE, TS_SYNC_BYTE};

/// A 4-byte TS header precedes the adaptation-field length byte.
const TS_HEADER_LEN: usize = 4;
/// Byte 3 `adaptation_field_control` high bit: an adaptation field follows
/// (H.222.0 Table 2-5).
const CONTROL_BIT: u8 = 0x20;
/// Shortest adaptation field carrying a PCR: a flags byte plus the 6-byte PCR.
const AF_FLAGS_AND_PCR_LEN: u8 = 7;
/// The length byte follows the 4-byte TS header.
const AF_LEN_AT: usize = TS_HEADER_LEN;
/// The flags byte follows the length byte.
const AF_FLAGS_AT: usize = AF_LEN_AT + 1;
/// The 6-byte PCR field follows the flags byte.
const PCR_FIELD_AT: usize = AF_FLAGS_AT + 1;

#[test]
fn pids_rejects_a_backward_pcr_step_near_zero() {
    let path = test_fixtures::path("ts/france-tnt-pcr.ts");
    let base = std::fs::read(&path).expect("fixture");
    let mut data = base.clone();
    let pcr_packets: Vec<usize> = (0..data.len() / TS_PACKET_SIZE)
        .map(|i| i * TS_PACKET_SIZE)
        .filter(|&at| {
            let af = at + TS_HEADER_LEN;
            at + TS_PACKET_SIZE <= data.len()
                && data[at] == TS_SYNC_BYTE
                && data[at + 3] & CONTROL_BIT != 0
                && data[af] >= AF_FLAGS_AND_PCR_LEN
                && data[af + 1] & AF_PCR_FLAG != 0
        })
        .collect();

    for (i, &at) in pcr_packets.iter().enumerate() {
        let ticks = if i % 2 == 0 { 5 } else { 0 };
        let field = at + PCR_FIELD_AT;
        assert_eq!(
            data[at + AF_FLAGS_AT],
            AF_PCR_FLAG,
            "rewrite must not touch the adaptation-field flags byte at {at}"
        );
        rewrite_pcr(&mut data, field, ticks);
    }
    assert!(
        !pcr_packets.is_empty(),
        "the capture must carry PCR packets to rewrite"
    );

    let baseline = run_pids(&base);
    let stderr = run_pids(&data);
    let line = stderr
        .lines()
        .find(|l| l.contains("bitrate="))
        .unwrap_or_else(|| panic!("no bitrate line in stderr: {stderr}"));
    assert!(
        baseline
            .lines()
            .any(|l| l.contains("bitrate=") && !l.contains("n/a")),
        "the unmodified capture must yield a bitrate for the rewriting to mean anything: {baseline}"
    );
    assert!(
        line.contains("n/a"),
        "a backward PCR step (5 -> 0 -> 5) must report no bitrate, got: {line}"
    );
}

/// Replace the 6-byte PCR field at `at` (`Pcr::from_27mhz` wire form).
fn rewrite_pcr(data: &mut [u8], at: usize, ticks: u64) {
    data[at..at + 6].copy_from_slice(&Pcr::from_27mhz(ticks).to_field_bytes());
}

/// Feed `data` to `dvb-tools pids` on stdin and return its stderr.
fn run_pids(data: &[u8]) -> String {
    let bin = env!("CARGO_BIN_EXE_dvb-tools");
    let mut child = Command::new(bin)
        .arg("pids")
        .arg("/dev/stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dvb-tools");
    let mut stdin = child.stdin.take().expect("stdin piped");
    let data = data.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&data).expect("write ts"));
    let output = child.wait_with_output().expect("wait for dvb-tools");
    writer.join().expect("writer thread");
    String::from_utf8_lossy(&output.stderr).into_owned()
}
