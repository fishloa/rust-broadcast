//! #2743: the pack walker advanced past a Program Stream Map by the parsed
//! struct's re-serialized length, not by the wire `program_stream_map_length`
//! (Table 2-41). Any PSM whose ES loop carries bytes no entry claims re-
//! serializes short, so the walk landed mid-PSM and every later packet of the
//! stream was lost.

use crate::psm_vectors;

use broadcast_core::{Parse, Serialize};
use mpeg_ps::program_stream::scan_packs;
use mpeg_ps::{Error, ProgramStreamMap};

#[test]
fn reproduces_2743() {
    let data = psm_vectors::trailing_es_bytes();
    let scan = scan_packs(&data);
    assert!(scan.skipped.is_empty(), "skipped: {:?}", scan.skipped);
    assert_eq!(scan.packs.len(), 2, "both packs must parse");
    assert_eq!(scan.packs[0].pes_packets.len(), 1, "pack 1's PES");
    assert_eq!(scan.packs[1].pes_packets.len(), 1, "pack 2's PES");
    assert_eq!(scan.packs[1].psm.as_ref().map(|p| p.version), Some(2));
}

#[test]
fn control_vector_walks_identically() {
    let data = psm_vectors::two_psms();
    let scan = scan_packs(&data);
    assert!(scan.skipped.is_empty());
    assert_eq!(scan.packs.len(), 2);
    assert!(scan.remaining.is_empty());
    let versions: Vec<u8> = scan
        .packs
        .iter()
        .map(|p| p.psm.as_ref().map_or(0, |m| m.version))
        .collect();
    assert_eq!(versions, [1, 2]);
}

#[test]
fn an_unclaimed_es_loop_byte_is_rejected_not_dropped() {
    let mut wire = psm_vectors::psm(1);
    // Declare one more `elementary_stream_map_length` byte than the single
    // entry covers, add that byte, and fix `program_stream_map_length`/CRC_32.
    wire[10..12].copy_from_slice(&5u16.to_be_bytes());
    wire.insert(16, 0x00);
    let map_length = u16::from_be_bytes([wire[4], wire[5]]) + 1;
    wire[4..6].copy_from_slice(&map_length.to_be_bytes());
    let crc_at = wire.len() - 4;
    let crc = broadcast_common::crc32_mpeg2::compute(&wire[..crc_at]);
    wire[crc_at..].copy_from_slice(&crc.to_be_bytes());
    assert_eq!(
        ProgramStreamMap::parse(&wire).unwrap_err(),
        Error::ElementaryStreamMapTrailingBytes {
            declared: 5,
            parsed: 4,
        }
    );
}

#[test]
fn the_vector_psm_round_trips_byte_exactly() {
    let wire = psm_vectors::psm(3);
    let parsed = ProgramStreamMap::parse(&wire).expect("the vector PSM parses");
    assert_eq!(parsed.version, 3);
    assert_eq!(parsed.elementary_stream_map.len(), 1);
    let mut out = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out).unwrap();
    assert_eq!(out, wire);
}
