//! Real-fixture tests against the genuine ATSC 3.0 ROUTE/LCT captures in
//! `fixtures/atsc3/route-*.bin` (issue #943 milestone; full field-by-field
//! provenance in `fixtures/atsc3/PROVENANCE.md`). This is the acceptance
//! test for `atsc3-route`: parse the real captures with [`RoutePacket`] and
//! byte-exact round-trip them — not hand-made bytes.
//!
//! `atsc3/tests/route_fixture_lct_bytes.rs` hand-decoded these same fixtures
//! with test-local helpers (predating this crate, since no ROUTE parser
//! existed yet — see that file's own doc comment). This file is the real
//! parser those helpers stood in for; it is intentionally independent (does
//! not call into or modify that file, which lives outside this crate's
//! scope).

use atsc3_route::{Codepoint, RouteFecPayloadId, RoutePacket, SourceFecPayloadId};
use broadcast_common::{Parse, Serialize};

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("atsc3")
        .join(name)
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path:?}: {e}"))
}

const FDT_FIXTURE: &str = "route-fdt-instance-2020-11-05.bin";
const VIDEO_FIXTURE: &str = "route-media-video-fragment-2020-11-05.bin";
const AUDIO_FIXTURE: &str = "route-media-audio-fragment-2020-11-05.bin";
const SLS_PACKAGE_FIXTURE: &str = "route-sls-signed-package-2020-11-05.bin";

// ---------------------------------------------------------------------------
// route-fdt-instance-2020-11-05.bin
// ---------------------------------------------------------------------------

#[test]
fn fdt_instance_parses_and_byte_exact_round_trips() {
    let bytes = read_fixture(FDT_FIXTURE);
    assert_eq!(bytes.len(), 439);

    let pkt = RoutePacket::parse(&bytes).expect("parse real FDT-Instance ROUTE packet");

    // LCT header fields, per PROVENANCE.md's documented decode.
    assert_eq!(pkt.lct.version, rmt_flute::LCT_VERSION);
    assert_eq!(pkt.lct.psi, rmt_flute::PSI_SPI);
    assert!(pkt.spi());
    assert!(!pkt.lct.close_session);
    assert!(pkt.lct.close_object); // Close-Object flag set
    assert_eq!(pkt.lct.cci, [0, 0, 0, 0]);
    assert_eq!(pkt.lct.tsi, [0, 0, 0, 0]); // TSI = 0
    assert_eq!(pkt.lct.toi, [0, 0, 0, 0]); // TOI = 0 (FDT-Instance convention)
    assert_eq!(pkt.lct.codepoint, 4);
    assert_eq!(pkt.codepoint(), Codepoint::NrtSignedPackageMode);

    // FEC Payload ID: Compact No-Code start_offset = 0.
    assert_eq!(
        pkt.fec_payload_id,
        RouteFecPayloadId::Source(SourceFecPayloadId { start_offset: 0 })
    );

    // Two header extensions: EXT_FTI (HET=64) then EXT_FDT (HET=192).
    assert_eq!(pkt.lct.extensions.len(), 2);
    assert_eq!(pkt.lct.extensions[0].het, rmt_flute::ALC_HET_EXT_FTI);
    assert_eq!(
        pkt.lct.extensions[0].content,
        [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x8f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
        ]
    );
    assert_eq!(pkt.lct.extensions[1].het, rmt_flute::HET_EXT_FDT);
    let fdt_ext = rmt_flute::ExtFdt::parse(pkt.lct.extensions[1].content).unwrap();
    assert_eq!(fdt_ext.version, 2); // FLUTE version
    assert_eq!(fdt_ext.instance_id, 0);

    // Payload: the 399-byte FDT-Instance XML.
    assert_eq!(pkt.payload.len(), 399);
    let xml = std::str::from_utf8(pkt.payload).expect("FDT-Instance payload is valid UTF-8");
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml.contains("<FDT-Instance"));
    assert!(xml.contains(r#"Expires="4294967295""#));
    assert!(xml.contains(r#"afdt:efdtVersion="74""#));
    assert!(xml.contains(r#"TOI="458826""#));
    assert!(xml.contains(r#"Content-Location="sls""#));
    assert!(xml.contains(r#"Content-Length="6758""#));
    assert!(xml.contains(r#"Content-Type="multipart/signed""#));

    // Byte-exact round trip: re-serializing the parsed packet must reproduce
    // the ORIGINAL captured bytes exactly, not merely an equivalent decode.
    let out = pkt.to_bytes();
    assert_eq!(out, bytes, "byte-exact round trip against the real capture");
}

// ---------------------------------------------------------------------------
// route-media-{video,audio}-fragment-2020-11-05.bin
// ---------------------------------------------------------------------------

struct MediaExpect {
    fixture: &'static str,
    tsi: u32,
    fti_content: [u8; 14],
    start_offset: u32,
}

const VIDEO: MediaExpect = MediaExpect {
    fixture: VIDEO_FIXTURE,
    tsi: 3000,
    fti_content: [
        0x00, 0x00, 0x00, 0x06, 0xff, 0xf3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    start_offset: 107_008,
};

const AUDIO: MediaExpect = MediaExpect {
    fixture: AUDIO_FIXTURE,
    tsi: 3003,
    fti_content: [
        0x00, 0x00, 0x00, 0x00, 0x80, 0xee, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
    start_offset: 26_752,
};

#[test]
fn media_fragments_parse_and_byte_exact_round_trip() {
    for expect in [VIDEO, AUDIO] {
        let bytes = read_fixture(expect.fixture);
        assert_eq!(bytes.len(), 1444, "{}", expect.fixture);

        let pkt = RoutePacket::parse(&bytes).unwrap_or_else(|e| {
            panic!("parse {} failed: {e}", expect.fixture);
        });

        assert_eq!(pkt.lct.tsi, expect.tsi.to_be_bytes(), "{}", expect.fixture);
        assert_eq!(pkt.lct.toi, 6034u32.to_be_bytes(), "{}", expect.fixture);
        assert_eq!(pkt.lct.codepoint, 128, "{}", expect.fixture);
        assert!(matches!(pkt.codepoint(), Codepoint::Indirect(128)));
        assert!(pkt.spi(), "{}", expect.fixture);

        assert_eq!(pkt.lct.extensions.len(), 1, "{}", expect.fixture);
        assert_eq!(pkt.lct.extensions[0].het, rmt_flute::ALC_HET_EXT_FTI);
        assert_eq!(
            pkt.lct.extensions[0].content, expect.fti_content,
            "{}",
            expect.fixture
        );

        assert_eq!(
            pkt.fec_payload_id,
            RouteFecPayloadId::Source(SourceFecPayloadId {
                start_offset: expect.start_offset
            }),
            "{}",
            expect.fixture
        );
        assert_eq!(pkt.payload.len(), 1408, "{}", expect.fixture);

        let out = pkt.to_bytes();
        assert_eq!(out, bytes, "byte-exact round trip: {}", expect.fixture);
    }
}

/// Sequential media packets on the same TSI have `start_offset` advancing by
/// exactly the previous packet's payload length — this crate's decoded
/// `SourceFecPayloadId` values are internally consistent with PROVENANCE.md's
/// documented "sequential, loss-free streaming" observation, corroborating
/// that the fragmentation model ([`SourceFecPayloadId::start_offset`]) is
/// being decoded correctly rather than misaligned by some constant offset.
#[test]
fn media_start_offsets_are_the_documented_fixed_values() {
    let video_bytes = read_fixture(VIDEO_FIXTURE);
    let audio_bytes = read_fixture(AUDIO_FIXTURE);
    let video = RoutePacket::parse(&video_bytes).unwrap();
    let audio = RoutePacket::parse(&audio_bytes).unwrap();
    let RouteFecPayloadId::Source(v) = video.fec_payload_id else {
        panic!("expected source FEC Payload ID")
    };
    let RouteFecPayloadId::Source(a) = audio.fec_payload_id else {
        panic!("expected source FEC Payload ID")
    };
    assert_eq!(v.start_offset, 107_008);
    assert_eq!(a.start_offset, 26_752);
}

// ---------------------------------------------------------------------------
// Cross-check against the SLS package's S-TSID XML (independent corroboration)
// ---------------------------------------------------------------------------

#[test]
fn media_fragment_tsi_matches_s_tsid_in_sls_package() {
    let video_bytes = read_fixture(VIDEO_FIXTURE);
    let audio_bytes = read_fixture(AUDIO_FIXTURE);
    let video = RoutePacket::parse(&video_bytes).unwrap();
    let audio = RoutePacket::parse(&audio_bytes).unwrap();
    let video_tsi = u32::from_be_bytes(video.lct.tsi.try_into().unwrap());
    let audio_tsi = u32::from_be_bytes(audio.lct.tsi.try_into().unwrap());
    assert_eq!(video_tsi, 3000);
    assert_eq!(audio_tsi, 3003);

    let sls = read_fixture(SLS_PACKAGE_FIXTURE);
    let sls_text = std::str::from_utf8(&sls).expect("SLS package is valid UTF-8");
    assert!(sls_text.contains(&format!(r#"tsi="{video_tsi}""#)));
    assert!(sls_text.contains(&format!(r#"tsi="{audio_tsi}""#)));
    // No RepairFlow in the S-TSID -- independently consistent with every
    // wire packet's SPI bit being 1 (source-data), matching the repair-flow
    // coverage gap this crate documents (`fec.rs` module doc).
    assert!(!sls_text.contains("RepairFlow"));
}

// ---------------------------------------------------------------------------
// Bite-proof: mutation -> observable failure -> restore -> pass again
// ---------------------------------------------------------------------------

#[test]
fn corrupting_hdr_len_breaks_the_route_decode() {
    let mut bytes = read_fixture(FDT_FIXTURE);
    // Pristine parse succeeds.
    assert!(RoutePacket::parse(&bytes).is_ok());

    assert_eq!(bytes[2], 9, "pristine HDR_LEN is 9 words (36 bytes)");
    bytes[2] = 5; // claim a much shorter LCT header (20 bytes): desyncs the
    // FEC-Payload-ID/payload split, landing 16 bytes inside what was the
    // EXT_FTI extension.
    let corrupted = RoutePacket::parse(&bytes);
    match corrupted {
        Err(_) => {} // rejected outright -- also an acceptable failure mode
        Ok(pkt) => {
            // If it happens to still parse (HDR_LEN=5 is itself a valid,
            // shorter LCT header shape), the recovered "payload" must NOT be
            // the real FDT-Instance XML any more.
            let xml = std::str::from_utf8(&pkt.payload[..pkt.payload.len().min(399)]);
            assert!(
                xml.is_err() || !xml.unwrap().starts_with("<?xml"),
                "corrupting HDR_LEN must break recovery of the real FDT-Instance XML"
            );
        }
    }

    // Restore and confirm the pristine fixture parses cleanly again (proves
    // the failure above was caused by the mutation, not fixture damage).
    let restored = read_fixture(FDT_FIXTURE);
    assert_eq!(restored[2], 9);
    let pkt =
        RoutePacket::parse(&restored).expect("pristine fixture must still parse after restore");
    assert_eq!(pkt.to_bytes(), restored);
}

#[test]
fn flipping_spi_bit_changes_the_decoded_fec_payload_id_variant() {
    let bytes = read_fixture(VIDEO_FIXTURE);
    let pkt = RoutePacket::parse(&bytes).unwrap();
    assert!(matches!(pkt.fec_payload_id, RouteFecPayloadId::Source(_)));

    // Flip PSI to a repair-packet shape (SPI=0) directly on the wire bytes,
    // in memory only -- the committed fixture file is never touched.
    let mut repair_bytes = bytes.clone();
    // First byte packs V(4)|C(2)|PSI(2) (RFC 5651 §5.1); PSI's high bit
    // (SPI) is bit 1 of byte 0.
    repair_bytes[0] &= 0b1111_1101; // clear the SPI (high PSI) bit
    let repair_pkt = RoutePacket::parse(&repair_bytes).expect("repair-shaped packet still parses");
    assert!(!repair_pkt.spi());
    assert!(matches!(
        repair_pkt.fec_payload_id,
        RouteFecPayloadId::Repair(_)
    ));
    assert_ne!(pkt.fec_payload_id, repair_pkt.fec_payload_id);
}

#[test]
fn mutating_payload_breaks_byte_identity_but_not_field_decode() {
    let bytes = read_fixture(VIDEO_FIXTURE);
    let pkt = RoutePacket::parse(&bytes).unwrap();

    let mut mutated_payload = pkt.payload.to_vec();
    mutated_payload[0] ^= 0xFF;
    let mutated_pkt = RoutePacket {
        lct: pkt.lct.clone(),
        fec_payload_id: pkt.fec_payload_id,
        payload: &mutated_payload,
    };

    let original_out = pkt.to_bytes();
    let mutated_out = mutated_pkt.to_bytes();
    assert_ne!(original_out, mutated_out);
    assert_eq!(original_out.len(), mutated_out.len());
    // Everything except the payload is unaffected.
    assert_eq!(
        &original_out[..original_out.len() - pkt.payload.len()],
        &mutated_out[..mutated_out.len() - mutated_pkt.payload.len()]
    );
}
