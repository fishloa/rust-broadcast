//! Real-fixture integration tests for typed PES optional-header fields.
//!
//! ## Coverage
//!
//! ### `pes-optional-fields.ts`
//! Extracted from TSDuck test vectors:
//!
//! - Packet 0: `test-002.ts` pkt 101753, pid=702, stream_id=0xBD (private stream 1)
//!   - `f2 = 0x1C`, header_data_length = 36 (includes 31 stuffing bytes)
//!   - Proven fields: **`es_rate`**, **`dsm_trick_mode`**, **`additional_copy_info`**
//!   - Semantic round-trip only (serializer writes minimal encoding, no stuffing)
//!
//! - Packet 1: `test-062.ts` pkt 7686, pid=304, stream_id=0xBD (private stream 1)
//!   - `f2 = 0x81`, header_data_length = 23 (includes 1 stuffing byte)
//!   - Proven fields: **`pes_extension`**
//!   - Semantic round-trip only
//!
//! ## Fields NOT found in any available real capture
//! - `escr` (MPEG-2 PES ESCR): not present in any parseable packet across all
//!   available test vectors (test-001..205, france-tnt-uhf32.ts, hotbird-mhp.ts).
//!   Remains synthetic-only.
//! - `pes_crc` (PES packet CRC): same — not found in any real capture.

use mpeg_pes::{PesPacket, StreamId};
use std::fs;

const TS_PKT: usize = 188;

fn pes_fixture() -> Vec<u8> {
    fs::read(format!(
        "{}/tests/fixtures/pes-optional-fields.ts",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("pes-optional-fields.ts must be present — fixture not found")
}

/// Extract the PUSI PES payload from a raw 188-byte TS packet.
/// Returns None if the packet has no payload or no PUSI set.
fn extract_pes_payload(raw: &[u8]) -> Option<&[u8]> {
    assert_eq!(raw.len(), TS_PKT);
    let pusi = (raw[1] & 0x40) != 0;
    if !pusi {
        return None;
    }
    let b3 = raw[3];
    let has_adaptation = (b3 & 0x20) != 0;
    let has_payload = (b3 & 0x10) != 0;
    if !has_payload {
        return None;
    }
    let payload_start = if has_adaptation {
        let af_len = raw[4] as usize;
        5 + af_len
    } else {
        4
    };
    if payload_start >= TS_PKT {
        return None;
    }
    Some(&raw[payload_start..])
}

// ─── es_rate, dsm_trick_mode, additional_copy_info ───────────────────────────

/// Parse pkt 0 of the fixture (test-002 pkt 101753).
/// Assert ES_rate, DSM_trick_mode, and additional_copy_info are all `Some`.
/// Assert semantic round-trip: parse → serialize → re-parse → equal fields.
#[test]
fn es_rate_dsm_aci_real_data() {
    let raw = pes_fixture();
    assert_eq!(
        raw.len() % TS_PKT,
        0,
        "fixture must be multiple of {TS_PKT}"
    );
    let pkts: Vec<&[u8]> = raw.chunks_exact(TS_PKT).collect();
    assert!(!pkts.is_empty(), "fixture must have at least 1 packet");

    // pkt 0 is the es_rate/dsm/aci packet
    let payload = extract_pes_payload(pkts[0]).expect("pkt 0 must be a PUSI payload TS packet");

    let pes = PesPacket::parse(payload).expect("PES must parse from pkt 0");
    let hdr = pes.header.as_ref().expect("pkt 0 must have a PES header");

    assert!(
        hdr.es_rate.is_some(),
        "es_rate must be Some in fixture pkt 0 (test-002 pkt 101753)"
    );
    assert!(
        hdr.dsm_trick_mode.is_some(),
        "dsm_trick_mode must be Some in fixture pkt 0"
    );
    assert!(
        hdr.additional_copy_info.is_some(),
        "additional_copy_info must be Some in fixture pkt 0"
    );

    eprintln!(
        "es_rate={:?} dsm={:?} aci={:?}",
        hdr.es_rate, hdr.dsm_trick_mode, hdr.additional_copy_info
    );

    // Semantic round-trip: serialize → re-parse → compare header fields
    let ser_len = pes.serialized_len();
    let mut ser_buf = vec![0u8; ser_len];
    pes.serialize_into(&mut ser_buf)
        .expect("serialize must succeed");

    let pes2 = PesPacket::parse(&ser_buf).expect("re-parsed PES must parse");
    let hdr2 = pes2
        .header
        .as_ref()
        .expect("re-parsed PES must have header");

    assert_eq!(pes2.stream_id, pes.stream_id, "stream_id changed");
    assert_eq!(hdr2.pts, hdr.pts, "pts changed");
    assert_eq!(hdr2.dts, hdr.dts, "dts changed");
    assert_eq!(
        hdr2.es_rate, hdr.es_rate,
        "es_rate changed across round-trip"
    );
    assert_eq!(
        hdr2.dsm_trick_mode, hdr.dsm_trick_mode,
        "dsm_trick_mode changed across round-trip"
    );
    assert_eq!(
        hdr2.additional_copy_info, hdr.additional_copy_info,
        "additional_copy_info changed across round-trip"
    );
}

// ─── pes_extension ────────────────────────────────────────────────────────────

/// Parse pkt 1 of the fixture (test-062 pkt 7686).
/// Assert `pes_extension` is `Some` and survives semantic round-trip.
#[test]
fn pes_extension_real_data() {
    let raw = pes_fixture();
    let pkts: Vec<&[u8]> = raw.chunks_exact(TS_PKT).collect();
    assert!(
        pkts.len() >= 2,
        "fixture must have at least 2 packets; got {}",
        pkts.len()
    );

    // pkt 1 is the pes_extension packet
    let payload = extract_pes_payload(pkts[1]).expect("pkt 1 must be a PUSI payload TS packet");

    let pes = PesPacket::parse(payload).expect("PES must parse from pkt 1");
    let hdr = pes.header.as_ref().expect("pkt 1 must have a PES header");

    assert!(
        hdr.pes_extension.is_some(),
        "pes_extension must be Some in fixture pkt 1 (test-062 pkt 7686)"
    );

    let ext = hdr.pes_extension.as_ref().unwrap();
    eprintln!(
        "pes_extension: pes_private_data={} pack_header={} \
         sequence_counter={} p_std_buffer={} ext_field={}",
        ext.pes_private_data.is_some(),
        ext.pack_header.is_some(),
        ext.program_packet_sequence_counter.is_some(),
        ext.p_std_buffer.is_some(),
        ext.pes_extension_field.is_some(),
    );

    // Semantic round-trip
    let ser_len = pes.serialized_len();
    let mut ser_buf = vec![0u8; ser_len];
    pes.serialize_into(&mut ser_buf)
        .expect("serialize must succeed");

    let pes2 = PesPacket::parse(&ser_buf).expect("re-parsed PES must parse");
    let hdr2 = pes2
        .header
        .as_ref()
        .expect("re-parsed PES must have header");

    assert_eq!(pes2.stream_id, pes.stream_id, "stream_id changed");
    assert!(
        hdr2.pes_extension.is_some(),
        "pes_extension must survive round-trip"
    );
    let ext2 = hdr2.pes_extension.as_ref().unwrap();
    assert_eq!(
        ext.pes_private_data.is_some(),
        ext2.pes_private_data.is_some(),
        "pes_private_data presence changed"
    );
    assert_eq!(
        ext.program_packet_sequence_counter, ext2.program_packet_sequence_counter,
        "program_packet_sequence_counter changed"
    );
    assert_eq!(ext.p_std_buffer, ext2.p_std_buffer, "p_std_buffer changed");
}

// ─── stream_id sanity ─────────────────────────────────────────────────────────

/// Both fixture packets carry private stream 1 (0xBD).
/// Proves we're parsing the right fixture and stream_id is decoded correctly.
#[test]
fn stream_ids_are_private_stream1() {
    let raw = pes_fixture();
    let pkts: Vec<&[u8]> = raw.chunks_exact(TS_PKT).collect();
    let expected_sid = StreamId(0xBD);

    let mut count = 0usize;
    for (i, pkt) in pkts.iter().enumerate() {
        if let Some(payload) = extract_pes_payload(pkt) {
            if let Ok(pes) = PesPacket::parse(payload) {
                assert_eq!(
                    pes.stream_id, expected_sid,
                    "pkt {i}: stream_id must be 0xBD (private stream 1)"
                );
                count += 1;
            }
        }
    }

    assert!(
        count >= 2,
        "expected at least 2 parseable PES packets in fixture, got {count}"
    );
}
