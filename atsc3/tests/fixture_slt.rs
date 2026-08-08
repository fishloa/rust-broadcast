//! Real-fixture test — parses and byte-exact round-trips a genuine ATSC 3.0
//! LLS envelope carrying a gzip-compressed SLT XML body, extracted from a
//! real capture (issue #926/#943).
//!
//! Requires the `std` feature (default) for `LlsEnvelope::decompress`.
//!
//! Fixtures live in the workspace-shared `fixtures/atsc3/` directory, not
//! inside this crate — per project convention (see e.g.
//! `dvb-mabr/tests/round_trip.rs`). Full provenance, including why this is
//! a genuine capture and not a synthetic sample, is in
//! `fixtures/atsc3/PROVENANCE.md`.

#![cfg(feature = "std")]

use std::fs;
use std::path::PathBuf;

use atsc3::LlsTableId;
use atsc3::lls::LlsEnvelope;
use atsc3::slt::{ServiceCategory, SlsProtocol, Slt};
use broadcast_common::{Parse, Serialize};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("atsc3")
        .join(name)
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path:?}: {e}"))
}

const FIXTURE: &str = "slt-lls-2019-01-07.bin";

/// Parses the real LLS envelope header fields (issue #926/#943 -- this is
/// the fixture that replaces the pre-existing hand-invented
/// `[0x01, 0x02, 0x03, 0x04]` + `b"payload-bytes"` test data).
#[test]
fn parses_real_lls_envelope_header() {
    let bytes = read_fixture(FIXTURE);
    assert_eq!(bytes.len(), 363);

    let env = LlsEnvelope::parse(&bytes).expect("parse real LLS envelope");
    assert_eq!(env.table_id, LlsTableId::Slt);
    assert_eq!(env.group_id, 1);
    assert_eq!(env.group_count_minus1, 0);
    assert_eq!(env.group_count(), 1);
    assert_eq!(env.table_version, 0x15);
    assert_eq!(env.payload.len(), 359);
    // The payload is real gzip (RFC 1952 magic), unlike `b"payload-bytes"`.
    assert_eq!(&env.payload[..2], [0x1f, 0x8b]);
}

/// Byte-exact round trip on the real capture (kills a lossy parser or a
/// raw-passthrough serializer).
#[test]
fn round_trips_real_lls_envelope_byte_exact() {
    let bytes = read_fixture(FIXTURE);
    let env = LlsEnvelope::parse(&bytes).expect("parse");

    let mut out = vec![0u8; env.serialized_len()];
    let written = env.serialize_into(&mut out).expect("serialize");
    assert_eq!(written, bytes.len());
    assert_eq!(
        out, bytes,
        "serialize did not reproduce the real capture bytes"
    );

    let reparsed = LlsEnvelope::parse(&out).expect("re-parse");
    assert_eq!(reparsed, env);
}

/// Decompresses the real gzip payload (RFC 1952) into valid UTF-8 XML --
/// the actual decode path `b"payload-bytes"` could never exercise, since it
/// is neither valid gzip nor valid XML.
#[test]
fn decompresses_real_payload_into_valid_xml() {
    let bytes = read_fixture(FIXTURE);
    let env = LlsEnvelope::parse(&bytes).expect("parse");

    let xml_bytes = env.decompress().expect("gunzip real payload");
    let xml = std::str::from_utf8(&xml_bytes).expect("decompressed body is UTF-8");
    assert!(xml.starts_with("<?xml"));
    assert!(xml.contains("<SLT"));
}

/// Full pipeline: envelope -> gunzip -> `Slt::parse`, asserting on the real
/// service data recorded in `fixtures/atsc3/PROVENANCE.md` (two ROUTE
/// services, "NATNL"/"NATN2", real destination multicast addressing).
#[test]
fn parses_real_slt_from_decompressed_payload() {
    let bytes = read_fixture(FIXTURE);
    let env = LlsEnvelope::parse(&bytes).expect("parse");
    let xml_bytes = env.decompress().expect("gunzip");
    let xml = std::str::from_utf8(&xml_bytes).expect("utf8");

    let slt = Slt::parse(xml).expect("parse real SLT XML");
    assert_eq!(slt.bsid, vec![0]);
    assert_eq!(slt.services.len(), 2);

    let svc1 = &slt.services[0];
    assert_eq!(svc1.service_id, 11);
    assert_eq!(svc1.major_channel_no, Some(45));
    assert_eq!(svc1.minor_channel_no, Some(1));
    assert_eq!(svc1.service_category, ServiceCategory::LinearAv);
    assert_eq!(svc1.short_service_name.as_deref(), Some("NATNL"));
    assert!(!svc1.hidden);
    let bss1 = svc1.broadcast_svc_signaling.as_ref().expect("BSS present");
    assert_eq!(bss1.sls_protocol, SlsProtocol::Route);
    assert_eq!(
        bss1.sls_destination_ip_address,
        std::net::Ipv4Addr::new(239, 255, 1, 1)
    );
    assert_eq!(bss1.sls_destination_udp_port, 49152);
    assert_eq!(
        bss1.sls_source_ip_address,
        Some(std::net::Ipv4Addr::new(192, 168, 59, 62))
    );

    let svc2 = &slt.services[1];
    assert_eq!(svc2.service_id, 12);
    assert_eq!(svc2.short_service_name.as_deref(), Some("NATN2"));
}

/// Bite-proof: corrupting one byte of the committed fixture's gzip payload
/// breaks decompression -- proving the test is actually wired to the
/// fixture, not passing regardless of content.
#[test]
fn corrupting_the_gzip_payload_breaks_decompression() {
    let mut bytes = read_fixture(FIXTURE);
    // Flip a byte inside the gzip-compressed body (well past the 4-byte
    // envelope header), corrupting the DEFLATE stream.
    let corrupt_at = 4 + 20;
    bytes[corrupt_at] ^= 0xFF;

    let env = LlsEnvelope::parse(&bytes).expect("envelope header still parses");
    assert!(
        env.decompress().is_err(),
        "corrupting the gzip payload must break decompression"
    );
}
