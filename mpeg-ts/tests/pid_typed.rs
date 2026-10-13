//! Typed-PID gate (#1159): `TsHeader.pid` is a [`Pid`], not a bare `u16`.
//!
//! The real-data gate parses **every packet of every committed `.ts`
//! fixture** (and, in an `#[ignore]`d test, of the local `.test-streams`
//! capture archive) and checks, per packet:
//!
//! * `header.pid.value()` equals the 13-bit PID computed *independently* from
//!   the raw wire bytes (ISO/IEC 13818-1 §2.4.3.2: `byte1[4:0] << 8 | byte2`)
//!   — so a regression in the wire→`Pid` mapping (a wrong mask, a swapped
//!   byte order) fails here, not in a downstream crate;
//! * the header serialises back to the original four bytes.
//!
//! The per-API tests prove the boundary rule: a value above `0x1FFF` is
//! refused by [`Pid::try_new`] before it can reach a serializer, and a valid
//! PID round-trips through every public API that takes one.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mpeg_ts::mux::{SectionPacketiser, SiMux};
use mpeg_ts::pid::{Pid, PidOutOfRange, well_known};
use mpeg_ts::pusi::PusiReassembler;
use mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE, TsHeader, TsPacket};

/// `transport_priority` bit in header byte 1 (ISO/IEC 13818-1 §2.4.3.2).
const TRANSPORT_PRIORITY: u8 = 0x20;

/// Independent 13-bit PID extraction straight from the wire bytes.
fn raw_pid(packet: &[u8]) -> u16 {
    (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2])
}

/// Recursively collect files with extension `ext` under `dir` (following the
/// `.test-streams` symlink).
fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect(&p, ext, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
            out.push(p);
        }
    }
}

/// Parse every 188-byte-aligned packet of `data` (only when the buffer is
/// 188-stride aligned at offset 0: M2TS / mid-packet captures are skipped).
/// Returns the number of packets verified.
fn verify_stream(name: &str, data: &[u8]) -> usize {
    let aligned = data.len() >= 3 * TS_PACKET_SIZE
        && [0, TS_PACKET_SIZE, 2 * TS_PACKET_SIZE]
            .iter()
            .all(|&o| data[o] == TS_SYNC_BYTE);
    if !aligned {
        return 0;
    }
    let mut verified = 0;
    for (i, chunk) in data.chunks_exact(TS_PACKET_SIZE).enumerate() {
        if chunk[0] != TS_SYNC_BYTE {
            continue; // mid-stream resync loss: not this test's subject
        }
        let header = TsHeader::parse(&chunk[..4])
            .unwrap_or_else(|e| panic!("{name} packet {i}: header parse failed: {e}"));
        assert_eq!(
            header.pid.value(),
            raw_pid(chunk),
            "{name} packet {i}: typed pid disagrees with the wire bytes"
        );
        // Full packet parse sees the same header.
        let pkt = TsPacket::parse(chunk)
            .unwrap_or_else(|e| panic!("{name} packet {i}: packet parse failed: {e}"));
        assert_eq!(pkt.header.pid, header.pid, "{name} packet {i}");
        let mut ser = [0u8; 4];
        header.serialize_into(&mut ser).unwrap();
        // `TsHeader` does not carry `transport_priority` (byte 1 bit 5, §2.4.3.2:
        // a pre-existing, PID-independent gap), so compare with that single bit
        // masked; every other header bit - including all 13 PID bits - must
        // round-trip exactly.
        let mut expected = [chunk[0], chunk[1], chunk[2], chunk[3]];
        expected[1] &= !TRANSPORT_PRIORITY;
        assert_eq!(
            ser, expected,
            "{name} packet {i}: header does not round-trip (transport_priority masked)"
        );
        verified += 1;
    }
    verified
}

#[test]
fn every_committed_ts_fixture_pid_matches_the_wire() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures");
    let mut files = Vec::new();
    collect(&root, "ts", &mut files);
    assert!(
        files.len() >= 10,
        "expected the committed .ts fixtures, found {}",
        files.len()
    );
    let mut total = 0;
    let mut checked_files = 0;
    for f in &files {
        let data = fs::read(f).unwrap();
        let n = verify_stream(&f.display().to_string(), &data);
        if n > 0 {
            checked_files += 1;
        }
        total += n;
    }
    assert!(checked_files >= 10, "only {checked_files} aligned fixtures");
    assert!(total > 10_000, "only {total} packets verified");
}

/// The local real-capture archive (gitignored; skips cleanly when absent).
#[test]
#[ignore = "needs the local .test-streams capture archive; run with --ignored"]
fn every_test_streams_capture_pid_matches_the_wire() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.test-streams");
    if !root.is_dir() {
        eprintln!("skipping: {} not present", root.display());
        return;
    }
    let mut files = Vec::new();
    collect(&root, "ts", &mut files);
    let mut total = 0usize;
    for f in &files {
        // Cap very large captures to the first 256 MiB: the property is
        // per-packet, so a prefix is as probing as the whole file.
        let data = fs::read(f).unwrap();
        let data = &data[..data.len().min(256 << 20)];
        total += verify_stream(&f.display().to_string(), data);
    }
    eprintln!("verified {total} packets across {} files", files.len());
    if !files.is_empty() {
        assert!(total > 0, "no aligned packets found in the archive");
    }
}

#[test]
fn try_new_rejects_out_of_range_and_new_masks_wire_values() {
    assert_eq!(Pid::try_new(0x1FFF).unwrap(), well_known::NULL);
    assert_eq!(Pid::try_new(0x0000).unwrap(), well_known::PAT);
    for bad in [0x2000u16, 0x2064, 0x8000, 0xFFFF] {
        let err: PidOutOfRange = Pid::try_new(bad).unwrap_err();
        assert_eq!(err.value, bad);
        assert!(Pid::try_from(bad).is_err());
        // `new` is the wire-parsing constructor: it masks, never errors.
        assert_eq!(Pid::new(bad).value(), bad & 0x1FFF);
    }
    assert_eq!(u16::from(Pid::try_new(0x0100).unwrap()), 0x0100);
    let msg = Pid::try_new(0x2000).unwrap_err().to_string();
    assert!(msg.contains("0x2000"), "{msg}");
    // Display / hex formatting stay `0x%04X`-compatible with the old u16 prints.
    let pid = Pid::try_new(0x01F0).unwrap();
    assert_eq!(format!("{pid}"), "0x01F0");
    assert_eq!(format!("{pid:04X}"), "01F0");
    assert_eq!(format!("{pid:#06x}"), "0x01f0");
}

/// A minimal valid long-form section (table_id 0x42, empty body) with CRC.
fn tiny_section() -> Vec<u8> {
    let mut s = vec![
        0x42, 0xF0, 0x09, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    // section_length counts the bytes after the 3-byte header, including CRC.
    let crc = broadcast_common::crc32_mpeg2::compute(&s[..s.len() - 4]);
    let n = s.len();
    s[n - 4..].copy_from_slice(&crc.to_be_bytes());
    s
}

#[test]
fn packetiser_takes_a_typed_pid_and_emits_it_on_every_packet() {
    let section = tiny_section();
    for raw in [0x0000u16, 0x0100, 0x1000, 0x1FFE] {
        let pid = Pid::try_new(raw).unwrap();
        let mut p = SectionPacketiser::new(pid);
        assert_eq!(p.pid(), pid);
        let pkts = p.packetise(&[&section]);
        assert!(!pkts.is_empty());
        for pkt in &pkts {
            assert_eq!(raw_pid(pkt), raw, "PID on the wire");
            assert_eq!(TsPacket::parse(pkt).unwrap().header.pid, pid);
        }
    }
    // The over-range value never reaches the packetiser: it cannot be built.
    assert!(Pid::try_new(0x2000).is_err());
}

#[test]
fn si_mux_upsert_takes_typed_pids() {
    let section = tiny_section();
    let pid = Pid::try_new(0x0123).unwrap();
    let mut mux = SiMux::new();
    mux.upsert(pid, section.clone(), Duration::from_millis(100));
    let pkts = mux.poll(Duration::ZERO);
    assert!(!pkts.is_empty());
    assert!(pkts.iter().all(|p| raw_pid(p) == 0x0123));
    // PMT PID goes through the same typed path.
    let mut mux = SiMux::new();
    mux.upsert_pmt(Pid::try_new(0x1000).unwrap(), section);
    let pkts = mux.poll(Duration::ZERO);
    assert!(pkts.iter().all(|p| raw_pid(p) == 0x1000));
}

#[test]
fn pusi_reassembler_filters_on_a_typed_pid() {
    let pid = Pid::try_new(0x0004).unwrap();
    let other = Pid::try_new(0x0005).unwrap();
    let mut r = PusiReassembler::new(pid);
    assert!(r.push(other, true, b"ignored").is_none());
    assert!(r.push(pid, true, b"unit-a").is_none());
    assert_eq!(
        r.push(pid, true, b"unit-b").as_deref(),
        Some(&b"unit-a"[..])
    );
}
