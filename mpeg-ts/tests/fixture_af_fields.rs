//! Real-fixture integration tests for typed adaptation-field optional fields.
//!
//! ## Coverage
//!
//! ### `af-transport-private-data.ts`
//! Extracted from:
//! - TSDuck test vector `test-074.hevc.ts` (pkts 159, 215): `transport_private_data`-only
//!   adaptation fields (flag byte `0x02`, no PCR) — **byte-identical round-trip**.
//! - TSDuck test vector `test-128.ts` (pkts 484, 485): PCR + `transport_private_data`
//!   (flag byte `0x12`, PCR reserved bits = `0x7E` as per spec) — **byte-identical round-trip**.
//!
//! ### `af-splice-extension.ts`
//! Extracted from TSDuck test vector `test-002.ts`:
//! - pkt 16854 (pid 174): `splice_countdown = 104`
//! - pkt 13933 (pid 166): `adaptation_field_extension` with `seamless_splice`
//! - pkt 51707 (pid 418): `transport_private_data` + `adaptation_field_extension`
//!   with all three sub-fields (`ltw`, `piecewise_rate`, `seamless_splice`)
//!
//! **Note on byte-identical round-trip**: the `af-splice-extension.ts` packets
//! carry stuffing bytes appended after the parsed fields (standard practice for
//! encoders that pad AF bodies to a fixed size). The serializer produces minimal
//! encoding (no stuffing), so byte-identical round-trip is not expected here.
//! The tests assert **semantic round-trip** (parse → serialize → re-parse →
//! fields equal) and that the named fields are provably `Some(...)` on real
//! broadcast bytes.

use mpeg_ts::ts::{TsPacket, TS_PACKET_SIZE};
use std::fs;

fn af_tpd_fixture() -> Vec<u8> {
    fs::read(format!(
        "{}/tests/fixtures/af-transport-private-data.ts",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("af-transport-private-data.ts must be present — fixture not found")
}

fn af_splice_fixture() -> Vec<u8> {
    fs::read(format!(
        "{}/tests/fixtures/af-splice-extension.ts",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("af-splice-extension.ts must be present — fixture not found")
}

/// Re-parse a serialized adaptation-field body by wrapping it in a minimal TS
/// packet and calling `adaptation_field()`.
fn reparse_af(serialized: &[u8]) -> mpeg_ts::ts::AdaptationField<'_> {
    // We need the bytes to live long enough — put them in a local array.
    // The AF bytes are at most 183 bytes (188 - sync(1) - hdr(4) - af_len(1)).
    let len = serialized.len();
    assert!(len <= 183, "AF too large for TS packet");
    let mut ts_buf = vec![0u8; TS_PACKET_SIZE];
    ts_buf[0] = 0x47;
    ts_buf[1] = 0x00;
    ts_buf[2] = 0x01;
    ts_buf[3] = 0x30; // has_adaptation | has_payload
    ts_buf[4] = len as u8;
    ts_buf[5..5 + len].copy_from_slice(serialized);
    // fill rest with payload bytes (0xFF = valid stuffing)
    for b in ts_buf[5 + len + 1..].iter_mut() {
        *b = 0xFF;
    }
    let ts_buf_box: Box<[u8; TS_PACKET_SIZE]> = ts_buf.into_boxed_slice().try_into().unwrap();
    // Leak deliberately to tie lifetime to 'static — acceptable in tests
    let ts_buf_static: &'static [u8; TS_PACKET_SIZE] = Box::leak(ts_buf_box);
    let ts = TsPacket::parse(ts_buf_static).expect("re-parse TS packet");
    ts.adaptation_field()
        .expect("AF present in re-parsed packet")
        .expect("AF parsed without error")
}

// ─── transport_private_data: BYTE-IDENTICAL round-trip ───────────────────────

/// Walk all packets in the af-transport-private-data fixture.
/// Every packet with a `transport_private_data` field must round-trip
/// **byte-identically** to the original adaptation-field body bytes.
/// At least 2 packets must be found — the test fails (not silently skips) if
/// the fixture carries no `transport_private_data` fields.
#[test]
fn transport_private_data_byte_identical_roundtrip() {
    let buf = af_tpd_fixture();
    assert_eq!(
        buf.len() % TS_PACKET_SIZE,
        0,
        "fixture must be a multiple of {TS_PACKET_SIZE}"
    );
    let mut tpd_count = 0usize;

    for chunk in buf.chunks_exact(TS_PACKET_SIZE) {
        let ts = TsPacket::parse(chunk).expect("packet must parse");
        let Some(Ok(af)) = ts.adaptation_field() else {
            continue;
        };
        let Some(tpd) = af.transport_private_data else {
            continue;
        };

        tpd_count += 1;

        // The original AF body bytes (exactly the bytes the parse saw).
        let af_len = (chunk[4] as usize).min(TS_PACKET_SIZE - 5);
        let af_body = &chunk[5..5 + af_len];

        // Serialize back.
        let mut ser_buf = [0u8; 188];
        let written = af
            .serialize_into(&mut ser_buf)
            .expect("serialize must succeed");

        let serialized = &ser_buf[..written];
        assert_eq!(
            serialized,
            af_body,
            "packet with tpd_len={} failed byte-identical round-trip:\n  original:   {}\n  serialized: {}",
            tpd.len(),
            af_body.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
            serialized.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
        );
    }

    assert!(
        tpd_count >= 2,
        "expected at least 2 packets with transport_private_data, found {tpd_count}; \
         fixture may be corrupt or wrong"
    );
    eprintln!("transport_private_data byte-identical round-trip: {tpd_count} packets exercised");
}

/// All packets in the tpd fixture with PCR must also have `pcr` == `Some`.
/// Exercises the PCR field alongside tpd on real bytes.
#[test]
fn transport_private_data_with_pcr_parses() {
    let buf = af_tpd_fixture();
    let mut with_pcr = 0usize;
    let mut without_pcr = 0usize;

    for chunk in buf.chunks_exact(TS_PACKET_SIZE) {
        let ts = TsPacket::parse(chunk).expect("packet must parse");
        let Some(Ok(af)) = ts.adaptation_field() else {
            continue;
        };
        if af.transport_private_data.is_none() {
            continue;
        }

        if af.pcr.is_some() {
            with_pcr += 1;
        } else {
            without_pcr += 1;
        }
    }
    // Both variants exist in the fixture (tpd-only from test-074, PCR+tpd from test-128).
    assert!(
        with_pcr >= 1,
        "expected at least 1 tpd packet WITH PCR, got {with_pcr}"
    );
    assert!(
        without_pcr >= 1,
        "expected at least 1 tpd-only packet (no PCR), got {without_pcr}"
    );
    eprintln!("tpd with PCR: {with_pcr}, tpd without PCR: {without_pcr}");
}

// ─── splice_countdown + adaptation_field_extension: semantic round-trip ──────

/// Walk all packets in the af-splice-extension fixture.
///
/// For each packet whose AF carries a `splice_countdown`:
/// 1. Assert the field is `Some(...)`.
/// 2. Parse → serialize → re-parse → assert fields equal (semantic round-trip).
///
/// Fail if zero such packets are found.
#[test]
fn splice_countdown_semantic_roundtrip_real_data() {
    let buf = af_splice_fixture();
    assert_eq!(buf.len() % TS_PACKET_SIZE, 0);
    let mut splice_count = 0usize;

    for chunk in buf.chunks_exact(TS_PACKET_SIZE) {
        let ts = TsPacket::parse(chunk).expect("packet must parse");
        let Some(Ok(ref af)) = ts.adaptation_field() else {
            continue;
        };
        let Some(sc) = af.splice_countdown else {
            continue;
        };

        splice_count += 1;

        // Serialize
        let mut ser_buf = [0u8; 188];
        let written = af.serialize_into(&mut ser_buf).expect("serialize");
        let serialized = &ser_buf[..written];

        // Re-parse and compare fields
        let af2 = reparse_af(serialized);
        assert_eq!(
            af2.splice_countdown,
            Some(sc),
            "splice_countdown {sc} did not survive semantic round-trip"
        );
        assert_eq!(af2.pcr, af.pcr, "PCR changed across round-trip");
    }

    assert!(
        splice_count >= 1,
        "expected at least 1 packet with splice_countdown in the fixture; got {splice_count}"
    );
    eprintln!("splice_countdown semantic round-trip: {splice_count} instances");
}

/// Walk the af-splice-extension fixture for `adaptation_field_extension` fields.
/// Assert that at least one `ltw`, one `piecewise_rate`, and one `seamless_splice`
/// sub-field are found and survive a semantic round-trip.
#[test]
fn adaptation_field_extension_semantic_roundtrip_real_data() {
    let buf = af_splice_fixture();
    assert_eq!(buf.len() % TS_PACKET_SIZE, 0);

    let mut ltw_count = 0usize;
    let mut pw_count = 0usize;
    let mut ss_count = 0usize;

    for chunk in buf.chunks_exact(TS_PACKET_SIZE) {
        let ts = TsPacket::parse(chunk).expect("packet must parse");
        let Some(Ok(ref af)) = ts.adaptation_field() else {
            continue;
        };
        let Some(ref ext) = af.extension else {
            continue;
        };

        // Serialize and re-parse
        let mut ser_buf = [0u8; 188];
        let written = af.serialize_into(&mut ser_buf).expect("serialize");
        let af2 = reparse_af(&ser_buf[..written]);
        let ext2 = af2
            .extension
            .as_ref()
            .expect("extension must survive round-trip");

        if let Some(ltw) = ext.ltw {
            ltw_count += 1;
            assert_eq!(ext2.ltw, Some(ltw), "ltw changed across round-trip");
        }
        if let Some(pw) = ext.piecewise_rate {
            pw_count += 1;
            assert_eq!(
                ext2.piecewise_rate,
                Some(pw),
                "piecewise_rate changed across round-trip"
            );
        }
        if let Some(ss) = ext.seamless_splice {
            ss_count += 1;
            assert_eq!(
                ext2.seamless_splice,
                Some(ss),
                "seamless_splice changed across round-trip"
            );
        }
    }

    assert!(
        ltw_count >= 1,
        "expected ≥1 packet with af_ext.ltw, found {ltw_count}; \
         fixture may be wrong or parser has a regression"
    );
    assert!(
        pw_count >= 1,
        "expected ≥1 packet with af_ext.piecewise_rate, found {pw_count}"
    );
    assert!(
        ss_count >= 1,
        "expected ≥1 packet with af_ext.seamless_splice, found {ss_count}"
    );

    eprintln!(
        "af_ext real data: ltw={ltw_count}, piecewise_rate={pw_count}, seamless_splice={ss_count}"
    );
}

/// A packet in the splice+extension fixture that carries `transport_private_data`
/// alongside an `adaptation_field_extension` must parse both simultaneously.
/// This covers the co-occurrence case (pkt 51707 from test-002.ts).
#[test]
fn tpd_and_af_extension_coexist() {
    let buf = af_splice_fixture();
    let mut both_count = 0usize;

    for chunk in buf.chunks_exact(TS_PACKET_SIZE) {
        let ts = TsPacket::parse(chunk).expect("packet must parse");
        let Some(Ok(ref af)) = ts.adaptation_field() else {
            continue;
        };
        if af.transport_private_data.is_some() && af.extension.is_some() {
            both_count += 1;

            // Semantic round-trip: fields must survive serialize → re-parse
            let mut ser_buf = [0u8; 188];
            let written = af.serialize_into(&mut ser_buf).expect("serialize");
            let af2 = reparse_af(&ser_buf[..written]);
            assert_eq!(
                af2.transport_private_data.map(|b| b.len()),
                af.transport_private_data.map(|b| b.len()),
                "tpd length changed across round-trip"
            );
            assert_eq!(
                af2.extension.as_ref().map(|e| (
                    e.ltw.is_some(),
                    e.piecewise_rate.is_some(),
                    e.seamless_splice.is_some()
                )),
                af.extension.as_ref().map(|e| (
                    e.ltw.is_some(),
                    e.piecewise_rate.is_some(),
                    e.seamless_splice.is_some()
                )),
                "af_extension presence flags changed across round-trip"
            );
        }
    }

    assert!(
        both_count >= 1,
        "expected ≥1 packet carrying both tpd and af_extension, got {both_count}"
    );
    eprintln!("tpd+af_extension co-occurrence: {both_count} packets");
}
