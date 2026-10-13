//! Byte-exact round-trip of the adaptation-field extension and stuffing
//! (umbrella #1139 T11, audit finding r01-W2).
//!
//! ISO/IEC 13818-1 Table 2-6 gives the extension: `adaptation_field_extension_length`,
//! `ltw_flag | piecewise_rate_flag | seamless_splice_flag |
//! af_descriptor_not_present_flag | reserved(4)`, the gated sub-fields, then an
//! `af_descriptor()` loop (flag `0`) or `reserved` bytes (flag `1`), then the
//! adaptation-field `stuffing_byte`s. Before the fix the reserved bits and the
//! trailing bytes were dropped on parse, so such a packet re-serialized with a
//! shorter extension and a shifted payload.
//!
//! ## Evidence
//! No committed real capture carries `adaptation_field_extension_flag == 1`
//! (scanned every `.ts` fixture in the repo, see
//! `no_committed_fixture_carries_an_adaptation_field_extension`), so the
//! extension vectors below are **spec-built** from Table 2-6, not real data.
//! The exhaustive gate [`every_committed_ts_adaptation_field_round_trips`]
//! is the real-data check: every adaptation field in every committed `.ts`
//! fixture must re-serialize byte-identically from its parsed form.

use mpeg_ts::ts::{AdaptationField, AdaptationFieldExtension, Ltw, TS_PACKET_SIZE, TsPacket};
use std::fs;
use std::path::{Path, PathBuf};

/// Serialize an adaptation field into its on-wire body (without the length byte).
fn body_of(af: &AdaptationField<'_>) -> Vec<u8> {
    let mut buf = vec![0u8; af.serialized_len()];
    let n = af.serialize_into(&mut buf).unwrap();
    assert_eq!(n, buf.len());
    buf
}

/// Build a full 188-byte packet around an adaptation-field body.
fn packet_with_af(body: &[u8]) -> [u8; TS_PACKET_SIZE] {
    let mut p = [0xAAu8; TS_PACKET_SIZE];
    p[0] = 0x47;
    p[1] = 0x01;
    p[2] = 0x00;
    p[3] = 0x30; // adaptation_field_control = 11 (AF + payload)
    p[4] = body.len() as u8;
    p[5..5 + body.len()].copy_from_slice(body);
    p
}

fn parse_af(pkt: &[u8; TS_PACKET_SIZE]) -> AdaptationField<'_> {
    TsPacket::parse(pkt)
        .unwrap()
        .adaptation_field()
        .expect("AF present")
        .expect("AF parses")
}

fn assert_round_trips(body: &[u8]) {
    let pkt = packet_with_af(body);
    let af = parse_af(&pkt);
    assert_eq!(body_of(&af), body, "AF body must re-serialize byte-exactly");
}

/// Flags byte `0x01` (only `adaptation_field_extension_flag`), extension
/// length 1, extension flags `0x0F` = all four low bits set with no sub-field
/// flags: `af_descriptor_not_present_flag = 0`? No — bit 4 (`0x10`) is the
/// flag and is `0` here, so the extension is `reserved(4)=1111` plus an empty
/// `af_descriptor()` loop.
#[test]
fn extension_flags_byte_only_round_trips() {
    // length=1, flags = ltw0 pw0 ss0 | af_descriptor_not_present=0 | reserved=1111
    assert_round_trips(&[0x01, 0x01, 0x0F]);
}

/// Reserved flag bits that are NOT all `1` (older / non-conforming encoders)
/// were rewritten to `0` before the fix.
#[test]
fn extension_reserved_bits_zero_round_trips() {
    // ext length 1, flags 0x10 = af_descriptor_not_present_flag=1, reserved=0000.
    assert_round_trips(&[0x01, 0x01, 0x10]);
    // reserved = 0101, flag 0.
    assert_round_trips(&[0x01, 0x01, 0x05]);
}

/// Trailing `reserved` bytes (`af_descriptor_not_present_flag == 1`) after the
/// optional sub-fields were silently dropped, shortening the AF.
#[test]
fn extension_trailing_reserved_bytes_round_trip() {
    // ext length 4: flags(0x1F: not_present=1, reserved=1111) + 3 reserved bytes.
    assert_round_trips(&[0x01, 0x04, 0x1F, 0xAB, 0xCD, 0xEF]);
}

/// `af_descriptor()` loop (`af_descriptor_not_present_flag == 0`) is carried
/// opaquely in `trailing` and round-trips.
#[test]
fn extension_af_descriptor_bytes_round_trip() {
    // ext length 5: flags 0x0F (descriptors present) + one descriptor
    // (tag 0x80, length 2, 2 bytes).
    let body = [0x01, 0x05, 0x0F, 0x80, 0x02, 0x11, 0x22];
    let pkt = packet_with_af(&body);
    let af = parse_af(&pkt);
    let ext = af.extension.as_ref().unwrap();
    assert!(!ext.af_descriptor_not_present_flag);
    assert_eq!(ext.trailing, &[0x80, 0x02, 0x11, 0x22]);
    assert_eq!(body_of(&af), body);
}

/// All three typed sub-fields plus a non-`11` piecewise-rate reserved field
/// plus trailing bytes plus non-`0xFF` AF stuffing.
#[test]
fn extension_all_fields_with_reserved_and_stuffing_round_trip() {
    // flags: ltw(0x80) | piecewise(0x40) | seamless(0x20) | not_present(0x10) | reserved 0x0A
    let ext_flags = 0xE0 | 0x10 | 0x0A;
    let body = [
        0x01, // AF flags: extension only
        // extension length: flags(1) + ltw(2) + piecewise(3) + splice(5) + trailing(2)
        13,
        ext_flags,
        0x81,
        0x23, // ltw_valid=1, ltw_offset=0x0123
        0x01,
        0x02,
        0x03, // piecewise: reserved '00', rate = 0x010203 & 0x3FFFFF
        0x2B,
        0x00,
        0x01,
        0x00,
        0x01, // seamless splice: type 2, DTS[32:30]=5, rest 0, markers 1
        0xDE,
        0xAD, // trailing reserved bytes
        0x00,
        0x5A,
        0xFF, // AF stuffing, deliberately not all 0xFF
    ];
    let pkt = packet_with_af(&body);
    let af = parse_af(&pkt);
    let ext = af.extension.as_ref().unwrap();
    assert_eq!(ext.piecewise_rate_reserved, 0);
    assert_eq!(
        ext.ltw,
        Some(Ltw {
            ltw_valid_flag: true,
            ltw_offset: 0x0123
        })
    );
    assert_eq!(ext.trailing, &[0xDE, 0xAD]);
    assert_eq!(af.stuffing, &[0x00, 0x5A, 0xFF]);
    assert_eq!(body_of(&af), body);
}

/// A constructed extension uses the spec-convention reserved bits (all `1`)
/// and parses back equal.
#[test]
fn constructed_extension_uses_convention_reserved_bits() {
    let mut ext = AdaptationFieldExtension::new();
    ext.piecewise_rate = Some(0x0A_BCDE);
    let af_body = {
        let mut buf = vec![0u8; ext.serialized_len()];
        ext.serialize_into(&mut buf).unwrap();
        buf
    };
    // length 4; flags = piecewise(0x40) | not_present(0x10) | reserved(0x0F).
    assert_eq!(af_body[..3], [0x04, 0x5F, 0xC0 | 0x0A]);
    let body = [&[0x01u8][..], &af_body].concat();
    let pkt = packet_with_af(&body);
    let af = parse_af(&pkt);
    assert_eq!(af.extension.as_ref().unwrap(), &ext);
}

// ── exhaustive real-data gate ────────────────────────────────────────────────

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

/// Every committed `.ts` fixture that is a clean 188-byte-stride stream.
fn committed_ts_fixtures() -> Vec<(PathBuf, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    collect_ts_files(&root, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let b = fs::read(&p).ok()?;
            // Skip non-TS / non-188-stride files (e.g. the 204-stride probe fixture).
            let ok = b.len() >= TS_PACKET_SIZE * 3
                && b.len() % TS_PACKET_SIZE == 0
                && b.chunks_exact(TS_PACKET_SIZE).all(|c| c[0] == 0x47);
            ok.then_some((p, b))
        })
        .collect()
}

#[test]
fn every_committed_ts_adaptation_field_round_trips() {
    let fixtures = committed_ts_fixtures();
    assert!(
        fixtures.len() >= 20,
        "expected the committed fixture corpus, found {} files",
        fixtures.len()
    );
    let (mut checked, mut stuffed, mut files_with_af) = (0usize, 0usize, 0usize);
    for (path, buf) in &fixtures {
        let mut file_had_af = false;
        for (i, raw) in buf.chunks_exact(TS_PACKET_SIZE).enumerate() {
            let pkt = TsPacket::parse(raw).unwrap();
            let Some(res) = pkt.adaptation_field() else {
                continue;
            };
            // A zero-length AF has no body to compare.
            if raw[4] == 0 {
                continue;
            }
            let af = res.unwrap_or_else(|e| panic!("{}: pkt {i}: AF parse: {e}", path.display()));
            let want = &raw[5..5 + usize::from(raw[4])];
            assert_eq!(
                body_of(&af),
                want,
                "{}: pkt {i}: adaptation field did not re-serialize byte-exactly",
                path.display()
            );
            checked += 1;
            file_had_af = true;
            if !af.stuffing.is_empty() {
                stuffed += 1;
            }
        }
        files_with_af += usize::from(file_had_af);
    }
    // Non-vacuity: the corpus must actually exercise stuffed AFs.
    assert!(checked > 5_000, "only {checked} adaptation fields checked");
    assert!(stuffed > 500, "only {stuffed} stuffed adaptation fields");
    assert!(files_with_af >= 20, "only {files_with_af} files had an AF");
}

/// Documents (and bites if it ever stops being true) that the committed
/// corpus holds no real `adaptation_field_extension_flag == 1` packet, which
/// is why the extension vectors above are spec-built.
#[test]
fn no_committed_fixture_carries_an_adaptation_field_extension() {
    let mut with_ext = Vec::new();
    for (path, buf) in committed_ts_fixtures() {
        for raw in buf.chunks_exact(TS_PACKET_SIZE) {
            if let Some(Ok(af)) = TsPacket::parse(raw).unwrap().adaptation_field()
                && af.extension.is_some()
            {
                with_ext.push(path.clone());
                break;
            }
        }
    }
    assert!(
        with_ext.is_empty(),
        "real AF-extension evidence now exists, switch the extension tests to it: {with_ext:?}"
    );
}
