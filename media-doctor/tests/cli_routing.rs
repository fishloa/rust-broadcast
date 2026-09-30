//! End-to-end CLI tests for the `media-doctor` binary (issue #1112, audit
//! MD-W7).
//!
//! `check` has to decide which diagnostic set to run — the TS checks
//! (sync/PAT/PMT/CC/PCR/PTS/SCTE-35/codec) or the container checks — and it
//! used to do that with a strict "`0x47` at byte 0 **and** byte 188" sniff.
//! Several real, common TS shapes fail that test:
//!
//! - a capture that does not start on a packet boundary (`tcpdump`/`dd` cuts),
//! - 192-byte M2TS and 204-byte RS-framed transport streams,
//! - a file with any leading junk byte.
//!
//! All of those were routed to the container path, which bails at its ISOBMFF
//! sniff and printed **"No issues found."** for a TS full of errors — the
//! worst possible answer from a diagnostic tool.
//!
//! Detection is now `container-probe`'s (a stride×phase lattice search over
//! 188/192/204/208 framings). `container-probe` reports the offset of the
//! first **sync byte**, which already lies past any wrapper, so the packet
//! bytes are extracted as `bytes[off..off + 188]` at `off += stride` — the
//! same 188 bytes whatever the framing.
//!
//! Every assertion here is **exact equality** against the aligned-188 run:
//! same per-rule counts, and zero `sync-byte` findings. A weaker "not clean"
//! check passed even with the extraction bug that read wrappers as packet
//! content and produced ~1260 spurious `sync-byte` errors.
//!
//! # Fixtures
//!
//! Every input here is derived at test time from the committed real capture
//! `fixtures/ts/m6-duplicate.ts` — no new binary fixture is checked in. The
//! exact transforms (the same ones the code below performs):
//!
//! ```text
//! # 192-byte M2TS framing (4-byte TP_extra_header before each packet)
//! python3 -c "d=open('fixtures/ts/m6-duplicate.ts','rb').read(); out=bytearray(); [out.extend(bytes(4)+d[i:i+188]) for i in range(0,len(d),188)]; open('m2ts.ts','wb').write(bytes(out))"
//! # 204-byte RS framing (16 parity bytes after each packet)
//! python3 -c "d=open('fixtures/ts/m6-duplicate.ts','rb').read(); out=bytearray(); [out.extend(d[i:i+188]+bytes([0xAA]*16)) for i in range(0,len(d),188)]; open('rs204.ts','wb').write(bytes(out))"
//! # 208-byte M2TS-over-RS (4-byte header + 188 + 16 parity)
//! python3 -c "d=open('fixtures/ts/m6-duplicate.ts','rb').read(); out=bytearray(); [out.extend(bytes(4)+d[i:i+188]+bytes([0xAA]*16)) for i in range(0,len(d),188)]; open('m2ts208.ts','wb').write(bytes(out))"
//! # mid-packet cut (starts 7 bytes into a packet)
//! python3 -c "d=open('fixtures/ts/m6-duplicate.ts','rb').read(); open('mid-packet.ts','wb').write(d[7:])"
//! # leading junk
//! python3 -c "d=open('fixtures/ts/m6-duplicate.ts','rb').read(); open('leading-junk.ts','wb').write(bytes([0xDE,0xAD,0xBE])+d)"
//! ```
//!
//! Independent confirmation that the re-framed files really are MPEG-2 TS:
//!
//! ```text
//! $ ffprobe -v error -show_entries format=format_name -of csv=p=0 f188.ts
//! mpegts
//! $ ffprobe -v error -show_entries format=format_name -of csv=p=0 m2ts.ts
//! mpegts
//! $ ffprobe -v error -show_entries format=format_name -of csv=p=0 rs204.ts
//! mpegts
//! $ tsanalyze m2ts.ts | grep -c "Transport stream"
//! 1
//! $ tsanalyze rs204.ts | grep -c "Transport stream"
//! 1
//! ```
//!
//! (`ffprobe` does not accept the 208-byte M2TS-over-RS variant — it is a
//! recorder-specific wrapper, not a standard one — which is why the
//! 208-framing case is asserted against the aligned-188 run rather than
//! against an external tool.)

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TS_PACKET_SIZE: usize = 188;
/// Reed-Solomon parity a 204-byte (DVB outer-FEC) packet appends.
const RS_PARITY_LEN: usize = 16;
/// `TP_extra_header` an M2TS (192-byte) record prefixes each packet with.
const M2TS_HEADER_LEN: usize = 4;
/// The exact number of `cc-anomaly` findings `fixtures/ts/m6-duplicate.ts`
/// carries — pinned independently by `tests/integration.rs`.
const M6_DUPLICATE_CC_ANOMALIES: usize = 879;

fn read_fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/ts")
        .join(name);
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// A scratch file under the workspace `target/` (already gitignored).
fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/media-doctor-cli");
    fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    path
}

/// Re-frame a 188-byte packet stream as `prefix + 188 + suffix`-byte records.
fn reframe(packets: &[u8], prefix: usize, suffix: usize) -> Vec<u8> {
    assert_eq!(packets.len() % TS_PACKET_SIZE, 0, "whole packets in");
    let stride = prefix + TS_PACKET_SIZE + suffix;
    let mut out = Vec::with_capacity(packets.len() / TS_PACKET_SIZE * stride);
    for packet in packets.chunks_exact(TS_PACKET_SIZE) {
        out.extend(std::iter::repeat_n(0x00, prefix));
        out.extend_from_slice(packet);
        // Parity/trailer bytes: never 0x47, so they cannot be mistaken for a
        // sync byte by anything downstream.
        out.extend(std::iter::repeat_n(0xAA, suffix));
    }
    out
}

/// Rule id -> count, parsed out of the CLI's rendered findings.
///
/// Each finding line is `   N. [severity] [rule-id] packet=… message`; the
/// severity bracket is skipped and the next bracket is the rule id.
fn rule_counts(stdout: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for line in stdout.lines() {
        let mut rest = line;
        while let Some(open) = rest.find('[') {
            let Some(close) = rest[open..].find(']') else {
                break;
            };
            let token = &rest[open + 1..open + close];
            rest = &rest[open + close..];
            // The severity field is not a rule id.
            if token.is_empty() || matches!(token, "error" | "warning" | "info") {
                continue;
            }
            *counts.entry(token.to_string()).or_insert(0) += 1;
            break;
        }
    }
    counts
}

fn run_check(input: &Path) -> String {
    let exe = env!("CARGO_BIN_EXE_media-doctor");
    assert!(Path::new(exe).exists(), "cargo must build the binary");
    let out = Command::new(exe)
        .args(["check", "--input", &input.display().to_string()])
        .output()
        .expect("media-doctor binary must run");
    assert!(
        out.status.success(),
        "check must exit 0 (it reports by findings, not by status): {:?}",
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Assert a re-framed variant is diagnosed **identically** to the aligned
/// 188-byte run: same per-rule counts, and no `sync-byte` finding (a
/// mis-extracted packet always trips that first).
fn assert_same_as_aligned(aligned: &str, variant: &str, what: &str) {
    assert!(
        !variant.contains("No issues found"),
        "a TS consisting of {what} must not be reported clean; got:\n{variant}",
    );
    assert_eq!(
        rule_counts(variant),
        rule_counts(aligned),
        "findings for {what} must match the aligned-188 run exactly",
    );
    assert_eq!(
        rule_counts(variant).get("sync-byte"),
        None,
        "no `sync-byte` finding may appear for {what}: every extracted packet \
         must start on a real sync byte",
    );
}

#[test]
fn aligned_ts_is_checked() {
    let ts = read_fixture("m6-duplicate.ts");
    let path = scratch("aligned.ts", &ts);
    let stdout = run_check(&path);
    assert_eq!(
        rule_counts(&stdout).get("cc-anomaly").copied(),
        Some(M6_DUPLICATE_CC_ANOMALIES),
        "the aligned baseline must be the fixture's known 879 cc anomalies; got:\n{stdout}",
    );
}

/// A `tcpdump`-style cut that does not start on a packet boundary.
#[test]
fn mid_packet_start_is_still_checked() {
    let ts = read_fixture("m6-duplicate.ts");
    let aligned = run_check(&scratch("aligned-baseline.ts", &ts));
    // 7 bytes into a packet: the first sync byte is no longer at offset 0.
    let variant = run_check(&scratch("mid-packet.ts", &ts[7..]));
    assert_same_as_aligned(&aligned, &variant, "a capture starting mid-packet");
}

/// Leading junk before the first sync byte (a file with a stray header).
#[test]
fn leading_junk_is_still_checked() {
    let ts = read_fixture("m6-duplicate.ts");
    let aligned = run_check(&scratch("aligned-baseline.ts", &ts));
    let mut junk = vec![0xDE, 0xAD, 0xBE];
    junk.extend_from_slice(&ts);
    let variant = run_check(&scratch("leading-junk.ts", &junk));
    assert_same_as_aligned(&aligned, &variant, "leading junk before the first packet");
}

/// 192-byte M2TS framing: a 4-byte `TP_extra_header` before each packet.
#[test]
fn m2ts_192_byte_framing_is_still_checked() {
    let ts = read_fixture("m6-duplicate.ts");
    let aligned = run_check(&scratch("aligned-baseline.ts", &ts));
    let variant = run_check(&scratch("m2ts.ts", &reframe(&ts, M2TS_HEADER_LEN, 0)));
    assert_same_as_aligned(&aligned, &variant, "192-byte M2TS framing");
}

/// 204-byte RS-framed transport stream (DVB outer FEC).
#[test]
fn rs_204_byte_framing_is_still_checked() {
    let ts = read_fixture("m6-duplicate.ts");
    let aligned = run_check(&scratch("aligned-baseline.ts", &ts));
    let variant = run_check(&scratch("rs204.ts", &reframe(&ts, 0, RS_PARITY_LEN)));
    assert_same_as_aligned(&aligned, &variant, "204-byte RS framing");
}

/// 208-byte framing: a 4-byte `TP_extra_header` *and* 16 RS parity bytes.
#[test]
fn m2ts_over_rs_208_byte_framing_is_still_checked() {
    let ts = read_fixture("m6-duplicate.ts");
    let aligned = run_check(&scratch("aligned-baseline.ts", &ts));
    let variant = run_check(&scratch(
        "m2ts208.ts",
        &reframe(&ts, M2TS_HEADER_LEN, RS_PARITY_LEN),
    ));
    assert_same_as_aligned(&aligned, &variant, "208-byte M2TS-over-RS framing");
}

/// The negative control: a non-TS file must still go to the container path —
/// the routing change must not make every input a transport stream.
#[test]
fn isobmff_still_routes_to_the_container_path() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/transmux/h264_aac_prog.mp4");
    let stdout = run_check(&path);
    assert!(
        !stdout.contains("[cc-anomaly]") && !stdout.contains("[sync-byte]"),
        "an ISOBMFF file must not be read as a TS packet lattice; got:\n{stdout}",
    );
}
