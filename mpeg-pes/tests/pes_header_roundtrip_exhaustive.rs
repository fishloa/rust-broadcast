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
/// Name of the gitignored local archive of real captures (repo root).
const ARCHIVE_DIR: &str = ".test-streams";

fn collect_ts_files(dir: &Path, include_archive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if matches!(
                name.as_ref(),
                "target" | ".git" | ".claude" | "node_modules" | ".delegate"
            ) || (name == ARCHIVE_DIR && !include_archive)
            {
                continue;
            }
            collect_ts_files(&p, include_archive, out);
        } else if name.ends_with(".ts") {
            out.push(p);
        }
    }
}

/// 188-stride `.ts` files: the committed fixtures, plus the local
/// `.test-streams` archive when `include_archive`.
fn ts_files(include_archive: bool) -> Vec<(PathBuf, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    collect_ts_files(&root, include_archive, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let b = fs::read(&p).ok()?;
            let ok = b.len() >= PKT * 2 && (0..2).all(|k| b[k * PKT] == 0x47);
            ok.then_some((p, b))
        })
        .collect()
}

/// Reassemble every PES packet in a TS buffer, per PID (skipping packets whose
/// sync byte is damaged or whose adaptation field is invalid).
fn pes_packets(ts: &[u8]) -> Vec<Vec<u8>> {
    let mut asms: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut out = Vec::new();
    for p in ts.chunks_exact(PKT) {
        if p[0] != 0x47 {
            continue; // lost sync
        }
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

fn hex(b: &[u8]) -> String {
    b.iter().take(48).map(|x| format!("{x:02x}")).collect()
}

#[derive(Default)]
struct Report {
    checked: usize,
    with_stuffing: usize,
    with_nonff_stuffing: Vec<String>,
    with_ext: usize,
    mismatches: Vec<String>,
}

fn scan(include_archive: bool) -> Report {
    let mut r = Report::default();
    for (path, ts) in ts_files(include_archive) {
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
            if out[..n] != raw[..want_len] {
                r.mismatches.push(format!(
                    "{}: PES #{i} want={} got={}",
                    path.display(),
                    hex(&raw[..want_len]),
                    hex(&out[..n])
                ));
                continue;
            }
            r.checked += 1;
            if let Some(h) = &pes.header {
                r.with_stuffing += usize::from(!h.header_stuffing.is_empty());
                if h.header_stuffing.iter().any(|&b| b != 0xFF) {
                    r.with_nonff_stuffing
                        .push(format!("{}: PES #{i}", path.display()));
                }
                r.with_ext += usize::from(h.pes_extension.is_some());
            }
        }
    }
    r
}

/// Fast gate over the files committed to git: every PES packet reassembled from
/// every committed `.ts` fixture re-serializes byte-exactly.
#[test]
fn every_pes_header_in_every_committed_ts_round_trips() {
    let r = scan(false);
    assert!(r.mismatches.is_empty(), "{:#?}", r.mismatches);
    assert!(r.checked > 3_000, "only {} PES packets checked", r.checked);
    assert!(
        r.with_stuffing >= 1,
        "no real PES header stuffing exercised"
    );
    assert!(r.with_ext >= 1, "no real PES extension exercised");
}

/// Full-archive gate (slow; local only, needs the gitignored `.test-streams`):
/// `cargo test -p mpeg-pes --test pes_header_roundtrip_exhaustive -- --ignored`.
///
/// Real non-`0xFF` PES header stuffing exists in
/// `tsduck-test/input/test-002.ts`, and real `ES_rate` marker bits of `0`
/// (the same stream: `ES_rate` bytes `25 0D 37`, leading marker `0`).
#[test]
#[ignore = "slow: scans the whole local .test-streams archive"]
fn full_archive_pes_headers_round_trip() {
    let t = std::time::Instant::now();
    let r = scan(true);
    eprintln!(
        "checked={} stuffed={} nonff_stuffing={} ext={} mismatches={} in {:?}",
        r.checked,
        r.with_stuffing,
        r.with_nonff_stuffing.len(),
        r.with_ext,
        r.mismatches.len(),
        t.elapsed()
    );
    assert!(r.mismatches.is_empty(), "{:#?}", r.mismatches);
    if Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(ARCHIVE_DIR)
        .exists()
    {
        assert!(
            r.with_nonff_stuffing
                .iter()
                .any(|e| e.contains("test-002.ts")),
            "expected real non-0xFF stuffing in test-002.ts"
        );
    }
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

/// The committed corpus has no non-`0xFF` stuffing and no `0b111` extension
/// reserved bits (the real non-`0xFF` stuffing is in the local archive); if one
/// appears, promote it to primary evidence.
#[test]
fn no_committed_fixture_has_nonff_stuffing_or_111_reserved() {
    for (path, ts) in ts_files(false) {
        for raw in pes_packets(&ts) {
            let Ok(pes) = PesPacket::parse(&raw) else {
                continue;
            };
            let Some(h) = pes.header else { continue };
            assert!(
                h.header_stuffing.iter().all(|&b| b == 0xFF),
                "{}: committed non-0xFF PES stuffing now exists",
                path.display()
            );
            if let Some(e) = &h.pes_extension {
                assert_ne!(e.reserved_bits, 0b111, "{}: real 0b111", path.display());
            }
        }
    }
}

/// Primary real-data evidence for non-`0xFF` stuffing and a zero `ES_rate`
/// leading marker bit: TSDuck's BSD-2 `test-002.ts` PES #7751-style header
/// `84 1C 24 | 25 0D 37 | D8 | E7 | 31 stuffing bytes with non-0xFF content`
/// (`ES_rate` + `DSM_trick_mode` + `additional_copy_info`). Skips loudly when
/// the local archive is absent.
#[test]
fn real_archive_es_rate_marker_and_nonff_stuffing_round_trip() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(ARCHIVE_DIR)
        .join("tsduck-test/input/test-002.ts");
    let Ok(ts) = fs::read(&path) else {
        eprintln!("SKIP: {} not available", path.display());
        return;
    };
    let mut seen = 0;
    for raw in pes_packets(&ts) {
        let Ok(pes) = PesPacket::parse(&raw) else {
            continue;
        };
        let Some(h) = &pes.header else { continue };
        if h.es_rate.is_some() && h.header_stuffing.iter().any(|&b| b != 0xFF) {
            let want = &raw[..(PES_FIXED + usize::from(pes.pes_packet_length)).min(raw.len())];
            let mut out = vec![0u8; pes.serialized_len()];
            pes.serialize_into(&mut out).unwrap();
            assert_eq!(out, want);
            seen += 1;
        }
    }
    assert!(
        seen > 0,
        "no ES_rate + non-0xFF stuffing PES in test-002.ts"
    );
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

/// Table 2-21 `ES_rate = marker_bit(1) | ES_rate(22) | marker_bit(1)`: all four
/// marker combinations round-trip (real data has a `0` leading marker).
#[test]
fn es_rate_marker_bits_round_trip_all_values() {
    for (b0, b2, want) in [
        (0x25u8, 0x37u8, 0b01u8),
        (0xA5, 0x37, 0b11),
        (0x25, 0x36, 0b00),
        (0xA5, 0x36, 0b10),
    ] {
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x08, 0x80, 0x10, 0x03, // ES_rate only
            b0, 0x0D, b2, 0xAA, 0xBB,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        assert_eq!(pkt.header.as_ref().unwrap().es_rate_marker_bits, want);
        round_trip(&b);
    }
}

/// Table 2-21: the first header flag byte starts with the fixed `'10'`; a PES
/// that does not (MPEG-1 PES, or damage - e.g. TSDuck `test-003.ts` PES #3648,
/// first flag byte `0x38`) is rejected instead of misparsed.
#[test]
fn non_10_header_marker_is_rejected() {
    let b = [
        0x00, 0x00, 0x01, 0xE0, 0x00, 0x06, 0x38, 0x00, 0x00, 0xAA, 0xBB, 0xCC,
    ];
    assert_eq!(PesPacket::parse(&b), Err(mpeg_pes::Error::BadHeaderMarker));
}

/// A header that does not fit inside a bounded `PES_packet_length` is rejected
/// (TSDuck `test-072.ts`: length 2 but flags + `PES_header_data_length`
/// follow); it previously re-serialized with bytes beyond the declared length.
#[test]
fn header_overrunning_pes_packet_length_is_rejected() {
    let b = [
        0x00, 0x00, 0x01, 0xE0, 0x00, 0x02, 0x85, 0x80, 0x05, 0x21, 0x53, 0x53, 0xB1, 0x81,
    ];
    assert!(matches!(
        PesPacket::parse(&b),
        Err(mpeg_pes::Error::BufferTooShort { .. })
    ));
}
