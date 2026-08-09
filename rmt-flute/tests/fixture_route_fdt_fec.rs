//! Real-fixture test: feed a corroborated Transfer-Length pulled from a real
//! ATSC 3.0 ROUTE FDT-Instance ALC/LCT packet's `EXT_FTI` extension into
//! [`SourceBlockPartition`] (RFC 5052 §9.1's Block Partitioning Algorithm).
//!
//! `fixtures/atsc3/route-fdt-instance-2020-11-05.bin` is a real, unfragmented
//! ALC/LCT packet (frame 174 of `ROUTE_SLS1.pcap`), byte-decoded in
//! `fixtures/atsc3/PROVENANCE.md`. Its `EXT_FTI` (HET 64) carries a 14-byte
//! opaque HEC whose *exact* bit-packing is FEC-scheme-defined (RFC 5052
//! §6.2.1/§6.3) and — per `PROVENANCE.md`'s own caveat — not independently
//! confirmed against a vendored FEC-scheme spec for this packet's scheme
//! (RFC 5445 Compact No-Code is not vendored in this repo). What **is**
//! independently corroborated, without assuming any scheme's bit layout: the
//! 16-bit big-endian value at content-bytes 4-5 (`0x018F` = 399) exactly
//! equals the byte length of the FDT-Instance XML payload carried in this
//! same packet — strong evidence it *is* a Transfer-Length field, confirmed
//! by measuring the payload independently of decoding the HEC.
//!
//! This test does not decode the rest of the HEC (that would be guessing a
//! FEC scheme this repo has no vendored spec for) — it only uses that one
//! corroborated Transfer-Length as a real-world `L` input to the
//! scheme-agnostic Block Partitioning Algorithm, exactly the shape
//! `docs/fec.md` §9 describes: `SourceBlockPartition` operates purely on
//! symbol counts the caller supplies, never on FEC-scheme-specific bytes.

use std::fs;

use rmt_flute::{ALC_HET_EXT_FTI, AlcPacket, HET_EXT_FDT, SourceBlockPartition};

/// The FEC Payload ID in this packet is a 4-octet Compact-No-Code
/// `start_offset` (ROUTE §3.1 source-flow format, `PROVENANCE.md`), confirmed
/// by the packet's total length: LCT header (36 bytes, `HDR_LEN`=9) + 4-byte
/// FEC Payload ID + 399-byte FDT-Instance XML payload = 439 bytes, the exact
/// size of the fixture file.
const FEC_PAYLOAD_ID_LEN: usize = 4;

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/atsc3/route-fdt-instance-2020-11-05.bin"
    );
    fs::read(path).expect("fixture route-fdt-instance-2020-11-05.bin must be committed")
}

#[test]
fn parses_ext_fti_and_fdt_from_real_packet() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_LEN).unwrap();

    assert_eq!(pkt.lct.hdr_len(), 9);
    assert_eq!(pkt.lct.extensions.len(), 2);

    let fti = &pkt.lct.extensions[0];
    assert_eq!(fti.het, ALC_HET_EXT_FTI);
    assert!(!fti.is_fixed(), "EXT_FTI (HET 64) is variable-length");
    assert_eq!(fti.hel(), 4); // HEL=4 -> 16-byte extension, 14-byte HEC.
    assert_eq!(fti.content.len(), 14);
    assert_eq!(
        fti.content,
        [
            0x00, 0x00, 0x00, 0x00, 0x01, 0x8f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
        ]
    );

    let fdt = &pkt.lct.extensions[1];
    assert_eq!(fdt.het, HET_EXT_FDT);
    assert!(fdt.is_fixed());

    // The FDT-Instance XML payload this packet carries.
    assert_eq!(pkt.payload.len(), 399);
    let xml = core::str::from_utf8(pkt.payload).unwrap();
    assert!(xml.starts_with("<?xml"));
}

#[test]
fn ext_fti_content_bytes_4_5_corroborate_the_real_payload_length() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_LEN).unwrap();
    let hec = pkt.lct.extensions[0].content;

    // Independent corroboration (no FEC-scheme assumption): the 16-bit
    // big-endian value at HEC bytes 4-5 equals the payload length measured
    // from the parsed packet, not from decoding the HEC itself.
    let candidate_transfer_length = u16::from_be_bytes([hec[4], hec[5]]) as u64;
    assert_eq!(candidate_transfer_length, 399);
    assert_eq!(candidate_transfer_length, pkt.payload.len() as u64);
}

#[test]
fn real_transfer_length_drives_the_block_partitioning_algorithm() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_LEN).unwrap();
    let hec = pkt.lct.extensions[0].content;
    let transfer_length = u16::from_be_bytes([hec[4], hec[5]]) as u64;
    assert_eq!(transfer_length, pkt.payload.len() as u64);

    // E and B are not recoverable from this packet (FEC-scheme-specific,
    // undecoded per the module doc); use representative values to exercise
    // the algorithm end-to-end on a real, corroborated L.
    //
    // L=399, E=100, B=2:
    //   T = ceil(399/100) = 4
    //   N = ceil(4/2) = 2
    //   A_large = ceil(4/2) = 2, A_small = floor(4/2) = 2, I = 4-2*2 = 0
    let partition = SourceBlockPartition::new(transfer_length, 100, 2).unwrap();
    assert_eq!(partition.source_symbols, 4);
    assert_eq!(partition.num_blocks, 2);
    assert_eq!(partition.larger_block_len, 2);
    assert_eq!(partition.smaller_block_len, 2);
    assert_eq!(partition.larger_blocks, 0);
    assert_eq!(partition.block_len(0), Some(2));
    assert_eq!(partition.block_len(1), Some(2));
    assert_eq!(partition.block_len(2), None);

    // 399 = 3*100 + 99, so the trailing remainder is 99 octets.
    assert_eq!(partition.last_symbol_len(), Some(99));
}
