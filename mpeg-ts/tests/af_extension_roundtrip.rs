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
        13, ext_flags, 0x81, 0x23, // ltw_valid=1, ltw_offset=0x0123
        0x01, 0x02, 0x03, // piecewise: reserved '00', rate = 0x010203 & 0x3FFFFF
        0x2B, 0x00, 0x01, 0x00,
        0x01, // seamless splice: type 2, DTS[32:30]=5, rest 0, markers 1
        0xDE, 0xAD, // trailing reserved bytes
        0x00, 0x5A, 0xFF, // AF stuffing, deliberately not all 0xFF
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

// ── exhaustive real-data gates ───────────────────────────────────────────────

fn collect_ts_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if matches!(
                name.as_ref(),
                "target"
                    | ".git"
                    | ".claude"
                    | "node_modules"
                    | ".delegate"
                    | "private"
                    | ".test-streams"
            ) {
                continue;
            }
            collect_ts_files(&p, out);
        } else if name.ends_with(".ts") {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Report {
    checked: usize,
    stuffed: usize,
    files_with_af: usize,
    /// Parsed Ok but did not re-serialize byte-exactly (always a failure).
    mismatches: Vec<String>,
    /// Packets with a real adaptation-field extension that round-tripped.
    ext_ok: Vec<String>,
    /// Skipped: `transport_error_indicator == 1` (corrupted by definition).
    skipped_tei: usize,
    /// Skipped: `adaptation_field_length > 183` (invalid, Table 2-6 / §2.4.3.4).
    skipped_len: usize,
    /// Skipped: sync byte missing (capture glitch).
    skipped_sync: usize,
    /// Parse errors on otherwise clean (TEI = 0, length <= 183) packets.
    parse_errors: Vec<String>,
}

fn scan(include_archive: bool) -> Report {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    collect_ts_files(&root, &mut files);
    if include_archive {
        // The whole real-capture corpus: private/fixtures/** + fetched .test-streams/.
        files.extend(test_fixtures::corpus(&["ts"]));
    }
    files.sort();
    let mut r = Report::default();
    for path in files {
        let Ok(buf) = fs::read(&path) else { continue };
        // A 188-stride TS file: the first three sync bytes must line up.
        if buf.len() < TS_PACKET_SIZE * 3
            || (0..3).any(|k| buf[k * TS_PACKET_SIZE] != mpeg_ts::ts::TS_SYNC_BYTE)
        {
            continue;
        }
        let mut file_had_af = false;
        for (i, raw) in buf.chunks_exact(TS_PACKET_SIZE).enumerate() {
            if raw[0] != mpeg_ts::ts::TS_SYNC_BYTE {
                r.skipped_sync += 1;
                continue;
            }
            let pkt = TsPacket::parse(raw).unwrap();
            let Some(res) = pkt.adaptation_field() else {
                continue;
            };
            if raw[4] == 0 {
                continue;
            }
            if pkt.header.tei {
                r.skipped_tei += 1;
                continue;
            }
            if usize::from(raw[4]) > TS_PACKET_SIZE - 5 {
                r.skipped_len += 1;
                continue;
            }
            let af = match res {
                Ok(af) => af,
                Err(e) => {
                    r.parse_errors
                        .push(format!("{}: pkt {i}: {e}", path.display()));
                    continue;
                }
            };
            let want = &raw[5..5 + usize::from(raw[4])];
            if body_of(&af) != want {
                r.mismatches.push(format!(
                    "{}: pkt {i} sc={} want={} got={}",
                    path.display(),
                    pkt.header.scrambling,
                    hex(want),
                    hex(&body_of(&af))
                ));
                continue;
            }
            r.checked += 1;
            file_had_af = true;
            r.stuffed += usize::from(!af.stuffing.is_empty());
            if af.extension.is_some() {
                r.ext_ok.push(format!("{}: pkt {i}", path.display()));
            }
        }
        r.files_with_af += usize::from(file_had_af);
    }
    r
}

/// Fast gate over the files committed to git (no real-capture corpus):
/// every adaptation field must re-serialize byte-exactly.
#[test]
fn every_committed_ts_adaptation_field_round_trips() {
    let r = scan(false);
    assert!(r.mismatches.is_empty(), "{:#?}", r.mismatches);
    assert!(r.parse_errors.is_empty(), "{:#?}", r.parse_errors);
    // Non-vacuity: the corpus must actually exercise stuffed AFs.
    assert!(r.checked > 5_000, "only {} AFs checked", r.checked);
    assert!(r.stuffed > 500, "only {} stuffed AFs", r.stuffed);
    assert!(r.files_with_af >= 20, "only {} files", r.files_with_af);
    // The committed corpus has no real AF extension (hence the spec-built
    // vectors above); the real evidence lives in the private real-capture corpus and is
    // exercised by the ignored full-corpus test below.
    assert!(
        r.ext_ok.is_empty(),
        "committed real AF-extension now exists"
    );
}

/// Full-corpus gate (slow; needs the private repo, see `test_fixtures::corpus`):
/// `cargo test -p mpeg-ts --test af_extension_roundtrip -- --ignored`.
///
/// Real evidence for `adaptation_field_extension_flag == 1` (TEI = 0, parses):
/// `tsduck-test/input/test-143.ts` pkts 451 and 3994 and
/// `tsduck-test/reference/test-147/test-147.ts` pkt 8098 (`test-002.ts` and its
/// `reference/test-045/*` copies carry more, but all TEI-flagged).
///
/// Skip policy (parse stays strict; the gate skips, with counts):
/// * `transport_error_indicator == 1` — corrupted by definition;
/// * `adaptation_field_length > 183` — invalid per §2.4.3.4;
/// * a parse error on any other packet is listed and must be a *genuinely
///   damaged / non-adaptation-field* packet. TSDuck's `test-143.ts` /
///   `test-147.ts` are scrambling-test streams whose adaptation-field bytes are
///   pseudo-random (non-DVB `transport_scrambling_control` values, AF-only
///   packets with random flags), so a private-data length overruns the AF;
///   likewise e.g. `hotbird-mhp.ts` pkt 391355: PID 0x1125, scrambled,
///   whose bytes 4.. are `47 03 B8 1F ..` — a duplicated header of the *next*
///   packet spliced in (capture glitch), so `transport_private_data_length`
///   overruns the AF. That is stream damage, not a sender convention, so the
///   parser keeps rejecting it rather than guess.
#[test]
#[ignore = "slow: scans the whole real-capture corpus (private/fixtures + .test-streams)"]
fn full_archive_adaptation_fields_round_trip() {
    let t = std::time::Instant::now();
    let r = scan(true);
    eprintln!(
        "checked={} stuffed={} files={} ext_ok={} skipped(tei={}, len>183={}, sync={}) parse_errors={} in {:?}",
        r.checked,
        r.stuffed,
        r.files_with_af,
        r.ext_ok.len(),
        r.skipped_tei,
        r.skipped_len,
        r.skipped_sync,
        r.parse_errors.len(),
        t.elapsed()
    );
    for e in &r.parse_errors {
        eprintln!("parse error: {e}");
    }
    for e in &r.ext_ok {
        eprintln!("real AF extension round-tripped: {e}");
    }
    assert!(r.mismatches.is_empty(), "{:#?}", r.mismatches);
    // Only the documented damaged packets may fail to parse.
    for e in &r.parse_errors {
        assert!(
            e.contains("hotbird-mhp.ts: pkt 391355")
                || e.contains("tsduck-test/input/test-143.ts")
                || e.contains("tsduck-test/reference/test-147/"),
            "undocumented AF parse error: {e}"
        );
    }
    if test_fixtures::private_root().is_some() {
        assert!(
            r.ext_ok
                .iter()
                .any(|e| e.contains("test-143.ts") || e.contains("test-147.ts")),
            "expected real AF-extension packets in the archive"
        );
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Primary real-data evidence for the adaptation-field extension: three
/// specific packets of TSDuck's BSD-2 test streams (pinned `tsduck-test` clone
/// or the `private/fixtures/tsduck/` mirror; skips loudly only when the private
/// repo is not checked out). Each must re-serialize byte-exactly.
///
/// * `reference/test-147/test-147.ts` pkt 8098: flags `0x87`
///   (discontinuity + splice countdown + private data + extension) with a
///   `seamless_splice` whose three `marker_bit`s are `0` on the wire.
/// * `input/test-143.ts` pkts 451 and 3994.
#[test]
fn real_archive_af_extension_packets_round_trip() {
    let cases = [
        ("reference/test-147/test-147.ts", 8098usize),
        ("input/test-143.ts", 451),
        ("input/test-143.ts", 3994),
    ];
    for (file, idx) in cases {
        let Some(path) = test_fixtures::tsduck(file, "af_extension_roundtrip") else {
            continue;
        };
        let buf = fs::read(path).expect("read capture");
        let raw: &[u8; TS_PACKET_SIZE] = buf[idx * TS_PACKET_SIZE..(idx + 1) * TS_PACKET_SIZE]
            .try_into()
            .unwrap();
        let af = parse_af(raw);
        let ext = af.extension.as_ref().expect("real packet has an extension");
        assert_eq!(
            body_of(&af),
            &raw[5..5 + usize::from(raw[4])],
            "{file} pkt {idx}"
        );
        if file.ends_with("test-147.ts") {
            let ss = ext.seamless_splice.expect("seamless_splice present");
            assert_eq!(ss.marker_bits, 0, "real marker bits are 0, not the spec 1");
        }
    }
}
