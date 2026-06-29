//! Real-fixture **byte-identical** round-trip proof for the typed
//! adaptation-field optional fields (ISO/IEC 13818-1:2007 §2.4.3.4).
//!
//! Every adaptation field — including its `0xFF` stuffing — must round-trip
//! parse → serialize → byte-identical, the workspace's hard wire-codec
//! invariant. `AdaptationField::stuffing_len` captures the trailing stuffing so
//! the serializer reproduces the body exactly.
//!
//! ## Fixtures (all genuine, unscrambled, byte-identical)
//!
//! ### `af-transport-private-data.ts`
//! - `test-074.hevc.ts` pkts 159, 215: `transport_private_data`-only (flag `0x02`).
//! - `test-128.ts` pkts 484, 485: PCR + `transport_private_data` (flag `0x12`).
//!
//! ### `af-pcr-stuffing.ts` (the canonical stuffed adaptation field)
//! Extracted from the France TNT UHF-32 DVB mux:
//! - a **pure-stuffing** AF (flag `0x00`, body = all `0xFF`);
//! - a **PCR + stuffing** AF (flag `0x10`, 6-byte PCR then 115 `0xFF` stuffing
//!   bytes filling the packet) — the most common adaptation-field pattern in any
//!   real mux, and the strongest proof that stuffing now round-trips exactly.
//!
//! ### `m6-single.ts` (committed broadcast capture, shared with `dvb-si`)
//! Swept wholesale: **every** unscrambled adaptation field in the capture is
//! asserted byte-identical, and at least one is a stuffed AF.
//!
//! ## Fields proven only synthetically (in-crate unit tests)
//! `splice_countdown` and the `adaptation_field_extension` sub-fields (`ltw`,
//! `piecewise_rate`, `seamless_splice`) do **not** occur in genuine, unscrambled,
//! byte-identical-roundtrippable form in any available capture (the France TNT
//! and Hot Bird muxes — ~515 MB combined — contain zero; the only TSDuck
//! "candidates" are scrambled or structurally-impossible packets whose bytes
//! merely *parse* as misinterpreted fields and never round-trip byte-identical).
//! These fields are covered by the build-from-fields round-trip unit tests in
//! `mpeg-ts/src/ts.rs`.

use mpeg_ts::ts::{TsPacket, TS_PACKET_SIZE};
use std::fs;

fn fixture(name: &str) -> Vec<u8> {
    fs::read(format!(
        "{}/tests/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    ))
    .unwrap_or_else(|e| panic!("fixture {name} must be present: {e}"))
}

/// The original adaptation-field body bytes of a TS packet (the bytes after the
/// `adaptation_field_length` byte, up to its declared length).
fn af_body(pkt: &[u8]) -> &[u8] {
    let af_len = (pkt[4] as usize).min(TS_PACKET_SIZE - 5);
    &pkt[5..5 + af_len]
}

/// Assert every unscrambled adaptation field in `buf` round-trips byte-identical.
/// Returns `(total_afs, stuffed_afs)`.
fn assert_all_byte_identical(buf: &[u8]) -> (usize, usize) {
    assert_eq!(
        buf.len() % TS_PACKET_SIZE,
        0,
        "fixture must be a whole number of 188-byte packets"
    );
    let mut total = 0usize;
    let mut stuffed = 0usize;
    for (i, pkt) in buf.chunks_exact(TS_PACKET_SIZE).enumerate() {
        assert_eq!(pkt[0], 0x47, "pkt {i}: lost sync");
        // Skip scrambled packets — their "adaptation field" bytes are ciphertext.
        if (pkt[3] & 0xC0) >> 6 != 0 {
            continue;
        }
        let Some(Ok(af)) = TsPacket::parse(pkt)
            .expect("packet parses")
            .adaptation_field()
        else {
            continue;
        };
        total += 1;
        if af.stuffing_len > 0 {
            stuffed += 1;
        }
        let orig = af_body(pkt);
        let mut out = vec![0u8; af.serialized_len()];
        let n = af.serialize_into(&mut out).expect("AF serialize");
        assert_eq!(
            &out[..n],
            orig,
            "pkt {i}: adaptation field NOT byte-identical\n  orig: {}\n  out:  {}",
            orig.iter().map(|b| format!("{b:02X}")).collect::<String>(),
            out[..n]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<String>(),
        );
    }
    (total, stuffed)
}

/// `transport_private_data` round-trips byte-identical on real bytes; at least
/// two instances exist, both with and without an accompanying PCR.
#[test]
fn transport_private_data_byte_identical() {
    let buf = fixture("af-transport-private-data.ts");
    let (total, _) = assert_all_byte_identical(&buf);

    let mut tpd = 0usize;
    let mut tpd_with_pcr = 0usize;
    let mut tpd_no_pcr = 0usize;
    for pkt in buf.chunks_exact(TS_PACKET_SIZE) {
        let Some(Ok(af)) = TsPacket::parse(pkt).unwrap().adaptation_field() else {
            continue;
        };
        if af.transport_private_data.is_some() {
            tpd += 1;
            if af.pcr.is_some() {
                tpd_with_pcr += 1;
            } else {
                tpd_no_pcr += 1;
            }
        }
    }
    assert_eq!(total, 4, "fixture should hold 4 adaptation fields");
    assert!(tpd >= 2, "expected >=2 transport_private_data, got {tpd}");
    assert!(tpd_with_pcr >= 1, "expected a tpd+PCR packet");
    assert!(tpd_no_pcr >= 1, "expected a tpd-only packet");
}

/// The canonical stuffed adaptation fields (pure-stuffing and PCR+stuffing) from
/// a real DVB mux round-trip byte-identical, stuffing bytes included.
#[test]
fn pcr_and_stuffing_byte_identical() {
    let buf = fixture("af-pcr-stuffing.ts");
    let (total, stuffed) = assert_all_byte_identical(&buf);
    assert!(total >= 2, "expected >=2 adaptation fields, got {total}");
    assert_eq!(
        stuffed, total,
        "every packet in this fixture is a stuffed adaptation field"
    );

    // At least one carries a PCR followed by a substantial stuffing run.
    let mut pcr_stuffed = 0usize;
    let mut max_stuffing = 0usize;
    for pkt in buf.chunks_exact(TS_PACKET_SIZE) {
        let Some(Ok(af)) = TsPacket::parse(pkt).unwrap().adaptation_field() else {
            continue;
        };
        max_stuffing = max_stuffing.max(af.stuffing_len);
        if af.pcr.is_some() && af.stuffing_len > 0 {
            pcr_stuffed += 1;
        }
    }
    assert!(
        pcr_stuffed >= 1,
        "expected a PCR + stuffing adaptation field"
    );
    assert!(
        max_stuffing >= 100,
        "expected a large stuffing run (got max {max_stuffing}); proves stuffing is reproduced, not dropped"
    );
}

/// Sweep the whole committed `m6-single.ts` capture: every unscrambled
/// adaptation field round-trips byte-identical, and the capture genuinely
/// exercises stuffing.
#[test]
fn m6_single_all_adaptation_fields_byte_identical() {
    let buf = fixture("m6-single.ts");
    let (total, stuffed) = assert_all_byte_identical(&buf);
    assert!(
        total >= 50,
        "m6-single.ts should contain many adaptation fields, got {total}"
    );
    assert!(
        stuffed >= 1,
        "m6-single.ts should contain at least one stuffed adaptation field, got {stuffed}"
    );
}
