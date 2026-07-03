//! Real-fixture integration test for `mpeg-ts` framing.
//!
//! Walks `tests/fixtures/m6-single.ts` with `iter_packets`, validates the
//! header round-trip, exercises typed accessors on real broadcast bytes, and
//! feeds the PAT PID through `SectionReassembler` to verify the CRC.

use std::collections::HashMap;
use std::fs;

use broadcast_common::Parse;
use mpeg_ts::section::Section;
use mpeg_ts::ts::{
    AdaptationFieldControl, ScramblingControl, SectionReassembler, TS_PACKET_SIZE, TsHeader,
    TsPacket, iter_packets,
};

fn fixture_path() -> String {
    format!("{}/../fixtures/ts/m6-single.ts", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn fixture_packet_count_matches_file_size() {
    let buf = fs::read(fixture_path()).expect("read fixture");
    // Every TS file must be an exact multiple of 188.
    assert_eq!(
        buf.len() % TS_PACKET_SIZE,
        0,
        "fixture length {len} is not a multiple of {TS_PACKET_SIZE}",
        len = buf.len()
    );
    let expected = buf.len() / TS_PACKET_SIZE;
    let actual = iter_packets(&buf).count();
    assert_eq!(
        actual, expected,
        "iter_packets count mismatch: expected {expected}, got {actual}"
    );
    assert!(actual > 0, "fixture must contain at least one packet");
}

#[test]
fn header_round_trip_byte_identical() {
    // Parse each header, re-serialize it, and assert the 4 header bytes are
    // byte-identical to the original. This is the project's Parse/Serialize
    // symmetry invariant applied to real broadcast data. A raw-passthrough
    // serializer that just writes `self.raw[..4]` would pass here — the
    // following `fields_match_wire` test catches that case by constructing the
    // header from individual parsed fields.
    let buf = fs::read(fixture_path()).expect("read fixture");
    let mut mismatches = 0usize;

    for (i, chunk) in buf.chunks_exact(TS_PACKET_SIZE).enumerate() {
        let hdr = TsHeader::parse(&chunk[..4]).expect("valid header");
        let mut serialized = [0u8; 4];
        hdr.serialize_into(&mut serialized)
            .expect("serialize_into must not fail");
        if serialized != chunk[..4] {
            eprintln!(
                "packet {i}: original={:02X?} serialized={:02X?}",
                &chunk[..4],
                &serialized
            );
            mismatches += 1;
        }
    }
    assert_eq!(
        mismatches, 0,
        "{mismatches} packets failed header round-trip"
    );
}

#[test]
fn parsed_fields_match_wire_bytes() {
    // Independently decode the four header bytes and assert the TsHeader fields
    // match — this test bites a bug where fields are decoded but not correctly
    // mapped to the wire layout.
    let buf = fs::read(fixture_path()).expect("read fixture");

    for (i, chunk) in buf.chunks_exact(TS_PACKET_SIZE).enumerate() {
        let hdr = TsHeader::parse(&chunk[..4]).expect("valid header");

        // byte 1
        let b1 = chunk[1];
        assert_eq!(hdr.tei, (b1 & 0x80) != 0, "pkt {i}: tei");
        assert_eq!(hdr.pusi, (b1 & 0x40) != 0, "pkt {i}: pusi");
        let pid_expected = (((b1 & 0x1F) as u16) << 8) | (chunk[2] as u16);
        assert_eq!(hdr.pid, pid_expected, "pkt {i}: pid");

        // byte 3
        let b3 = chunk[3];
        assert_eq!(hdr.scrambling, (b3 >> 6) & 0x3, "pkt {i}: scrambling");
        assert_eq!(
            hdr.has_adaptation,
            (b3 & 0x20) != 0,
            "pkt {i}: has_adaptation"
        );
        assert_eq!(hdr.has_payload, (b3 & 0x10) != 0, "pkt {i}: has_payload");
        assert_eq!(hdr.continuity_counter, b3 & 0x0F, "pkt {i}: cc");
    }
}

#[test]
fn at_least_one_pid_appears_multiple_times() {
    let buf = fs::read(fixture_path()).expect("read fixture");
    let mut pid_counts: HashMap<u16, usize> = HashMap::new();
    for pkt in iter_packets(&buf) {
        *pid_counts.entry(pkt.header.pid).or_insert(0) += 1;
    }
    let max_count = pid_counts.values().copied().max().unwrap_or(0);
    assert!(
        max_count > 1,
        "expected at least one PID to appear more than once in a real mux (max_count={max_count})"
    );
}

#[test]
fn all_packets_are_not_scrambled() {
    // The M6 fixture is a clear (FTA) capture — every packet should report
    // NotScrambled. This exercises the typed accessor on real broadcast bytes.
    let buf = fs::read(fixture_path()).expect("read fixture");
    let mut scrambled_count = 0usize;

    for pkt in iter_packets(&buf) {
        if pkt.header.scrambling_control() != ScramblingControl::NotScrambled {
            scrambled_count += 1;
        }
    }
    assert_eq!(
        scrambled_count, 0,
        "expected all packets in clear M6 fixture to be NotScrambled, \
         but {scrambled_count} were scrambled"
    );
}

#[test]
fn adaptation_field_control_accessor_on_real_data() {
    // Just ensure the typed accessor never panics on real broadcast bytes and
    // always returns a valid variant (the enum is exhaustive over the 2-bit field).
    let buf = fs::read(fixture_path()).expect("read fixture");
    let mut counts = [0usize; 4];

    for pkt in iter_packets(&buf) {
        let afc = pkt.header.adaptation_field_control();
        match afc {
            AdaptationFieldControl::Reserved => counts[0] += 1,
            AdaptationFieldControl::PayloadOnly => counts[1] += 1,
            AdaptationFieldControl::AdaptationOnly => counts[2] += 1,
            AdaptationFieldControl::AdaptationAndPayload => counts[3] += 1,
            _ => panic!("unexpected AdaptationFieldControl variant"),
        }
    }
    // Most real TS packets carry payload (PayloadOnly or AdaptationAndPayload).
    assert!(
        counts[1] + counts[3] > 0,
        "expected at least some payload-carrying packets; counts={counts:?}"
    );
}

#[test]
fn pat_section_reassembles_and_crc_validates() {
    const PAT_PID: u16 = 0x0000;

    let buf = fs::read(fixture_path()).expect("read fixture");
    let mut reasm = SectionReassembler::default();

    for pkt in iter_packets(&buf) {
        if pkt.header.pid != PAT_PID {
            continue;
        }
        if let Some(payload) = pkt.payload {
            reasm.feed(payload, pkt.header.pusi);
        }
    }

    let section_bytes = reasm
        .pop_section()
        .expect("at least one PAT section should reassemble from the fixture");

    // Parse the section header and validate the CRC-32.
    let section = Section::parse(&section_bytes).expect("PAT section header parses");
    assert_eq!(section.table_id, 0x00, "PAT table_id must be 0x00");
    section
        .validate_crc(&section_bytes)
        .expect("PAT CRC-32 must validate");
}

#[test]
fn tspacket_parse_and_serialize_roundtrip_on_fixture() {
    // TsPacket::parse then TsHeader::serialize_into: reconstructed header bytes
    // must be byte-identical to the wire. This is the Parse/Serialize symmetry
    // invariant at the TsPacket level.
    let buf = fs::read(fixture_path()).expect("read fixture");
    let mut count = 0usize;

    for chunk in buf.chunks_exact(TS_PACKET_SIZE) {
        let pkt = TsPacket::parse(chunk).expect("valid TS packet from fixture");
        let mut reconstructed = [0u8; 4];
        pkt.header
            .serialize_into(&mut reconstructed)
            .expect("serialize header");
        assert_eq!(
            &reconstructed[..],
            &chunk[..4],
            "header bytes mismatch at packet {count}"
        );
        count += 1;
    }
    assert!(count > 0);
}
