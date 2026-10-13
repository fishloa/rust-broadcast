//! Byte-exact PES header round-trip (umbrella #1139 T11, audit finding r01-W2).
//!
//! Two lossy spots in the PES header (ISO/IEC 13818-1 Table 2-21):
//! * `stuffing_byte`s after the optional fields were kept only as a count and
//!   re-emitted as `0xFF`;
//! * the 3 `reserved` bits of the PES-extension flags byte were not kept and
//!   re-emitted as `000`.
//!
//! ## Evidence
//! * Real stuffing: `fixtures/mpeg-pes/pes-pts-dts-stuffing.ts` (also covered
//!   by `fixture_pes_header.rs`).
//! * Real reserved bits: `fixtures/mpeg-pes/dirac-vc2-extended-stream-id.ts`
//!   carries PES-extension flags byte `0x01`, i.e. `reserved == 0b000`, **not**
//!   the `0b111` the spec-convention suggests (so the old writer happened to
//!   match; a `0b111` writer would have broken it).
//! * No committed capture carries *non*-`0xFF` stuffing or `reserved == 0b111`
//!   in the extension byte (checked by the scan test below), so those two cases
//!   are **spec-built** vectors from Table 2-21.
//!
//! [`every_pes_header_in_every_committed_ts_round_trips`] is the exhaustive
//! real-data gate: every PES packet reassembled from every committed `.ts`
//! fixture must re-serialize byte-identically.

use mpeg_pes::{PesAssembler, PesPacket};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const PKT: usize = 188;
/// `packet_start_code_prefix(3) + stream_id(1) + PES_packet_length(2)`.
const PES_FIXED: usize = 6;

fn collect_ts_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if matches!(
                name.as_ref(),
                "target" | ".git" | ".claude" | "node_modules" | ".delegate"
            ) {
                continue;
            }
            collect_ts_files(&p, out);
        } else if name.ends_with(".ts") {
            out.push(p);
        }
    }
}

fn committed_ts_fixtures() -> Vec<(PathBuf, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    collect_ts_files(&root, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let b = fs::read(&p).ok()?;
            let ok = b.len() >= PKT * 2
                && b.len() % PKT == 0
                && b.chunks_exact(PKT).all(|c| c[0] == 0x47);
            ok.then_some((p, b))
        })
        .collect()
}

/// Reassemble every PES packet in a TS buffer, per PID.
fn pes_packets(ts: &[u8]) -> Vec<Vec<u8>> {
    let mut asms: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut out = Vec::new();
    for p in ts.chunks_exact(PKT) {
        let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
        let pusi = p[1] & 0x40 != 0;
        let start = match (p[3] >> 4) & 3 {
            1 => 4,
            3 => 5 + usize::from(p[4]),
            _ => continue,
        };
        if start >= PKT {
            continue;
        }
        if let Some(v) = asms.entry(pid).or_default().feed(pusi, &p[start..]) {
            out.push(v);
        }
    }
    for (_, mut a) in asms {
        if let Some(v) = a.flush() {
            out.push(v);
        }
    }
    out
}

fn round_trip(b: &[u8]) {
    let pkt = PesPacket::parse(b).expect("parses");
    let mut out = vec![0u8; pkt.serialized_len()];
    let n = pkt.serialize_into(&mut out).expect("serializes");
    assert_eq!(&out[..n], b, "PES packet did not round-trip byte-exactly");
}

#[test]
fn every_pes_header_in_every_committed_ts_round_trips() {
    let (mut checked, mut with_stuffing, mut with_ext) = (0usize, 0usize, 0usize);
    for (path, ts) in committed_ts_fixtures() {
        for (i, raw) in pes_packets(&ts).iter().enumerate() {
            // Only complete PES packets starting at a start code are meaningful
            // (an assembler may hand back a headless continuation after a
            // mid-stream join).
            let Ok(pes) = PesPacket::parse(raw) else {
                continue;
            };
            let mut out = vec![0u8; pes.serialized_len()];
            let n = pes.serialize_into(&mut out).unwrap();
            // Bytes past a bounded `PES_packet_length` are TS-level padding the
            // assembler hands back, not part of the PES packet.
            let want_len = match pes.pes_packet_length {
                0 => raw.len(),
                l => (PES_FIXED + usize::from(l)).min(raw.len()),
            };
            assert_eq!(
                &out[..n],
                &raw[..want_len],
                "{}: PES #{i} did not round-trip byte-exactly",
                path.display()
            );
            checked += 1;
            if let Some(h) = &pes.header {
                with_stuffing += usize::from(!h.header_stuffing.is_empty());
                with_ext += usize::from(h.pes_extension.is_some());
            }
        }
    }
    assert!(checked > 3_000, "only {checked} PES packets checked");
    assert!(with_stuffing >= 1, "no real PES header stuffing exercised");
    assert!(with_ext >= 1, "no real PES extension exercised");
}

/// Real evidence on the PES-extension reserved bits (see module docs).
#[test]
fn real_pes_extension_reserved_bits_are_preserved() {
    let ts = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mpeg-pes/dirac-vc2-extended-stream-id.ts"
    ))
    .unwrap();
    let pkts = pes_packets(&ts);
    assert!(!pkts.is_empty());
    for raw in &pkts {
        let pes = PesPacket::parse(raw).unwrap();
        let ext = pes.header.as_ref().unwrap().pes_extension.as_ref().unwrap();
        assert_eq!(ext.reserved_bits, 0b000, "flags byte is 0x01 on the wire");
        round_trip(raw);
    }
}

/// No committed capture has non-`0xFF` stuffing or a `0b111` reserved nibble in
/// the PES extension; if one appears, replace the spec-built vectors with it.
#[test]
fn no_committed_fixture_has_nonff_stuffing_or_111_reserved() {
    for (path, ts) in committed_ts_fixtures() {
        for raw in pes_packets(&ts) {
            let Ok(pes) = PesPacket::parse(&raw) else {
                continue;
            };
            let Some(h) = pes.header else { continue };
            assert!(
                h.header_stuffing.iter().all(|&b| b == 0xFF),
                "{}: real non-0xFF PES stuffing now exists",
                path.display()
            );
            if let Some(e) = &h.pes_extension {
                assert_ne!(e.reserved_bits, 0b111, "{}: real 0b111", path.display());
            }
        }
    }
}

/// Spec-built (Table 2-21): non-`0xFF` stuffing is preserved verbatim.
#[test]
fn non_ff_header_stuffing_round_trips() {
    let b = [
        0x00, 0x00, 0x01, 0xE0, 0x00, 0x0C, 0x80, 0x80, 0x08, // hdl = 8
        0x21, 0x00, 0x01, 0x00, 0x01, // PTS = 0
        0x00, 0xA5, 0xFF, // stuffing, not all 0xFF
        0xAA, // payload
    ];
    let pkt = PesPacket::parse(&b).unwrap();
    assert_eq!(
        pkt.header.as_ref().unwrap().header_stuffing,
        &[0x00, 0xA5, 0xFF]
    );
    round_trip(&b);
}

/// Spec-built (Table 2-21): reserved bits of the PES-extension flags byte are
/// preserved for every value, alongside a P-STD_buffer field.
#[test]
fn pes_extension_reserved_bits_round_trip_all_values() {
    for reserved in 0u8..=7 {
        let flags = 0x10 | (reserved << 1); // P-STD_buffer_flag + reserved(3)
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x01, 0x03, // hdl = 3
            flags, 0x60, 0x40, // ext flags, P-STD '01' scale=1 size=0x040
            0xAA, 0xBB, // payload
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        let ext = pkt.header.as_ref().unwrap().pes_extension.as_ref().unwrap();
        assert_eq!(ext.reserved_bits, reserved);
        round_trip(&b);
    }
}
