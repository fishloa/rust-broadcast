//! RFC 4326 Appendix B test vector: parse the committed `appendix_b.bin` (an
//! ICMPv6-over-IPv6 SNDU with D=0, NPA 00:01:02:03:04:05, Type 0x86DD, and the
//! RFC's stated CRC-32 `0x7C171763`), assert the decoded fields, verify a
//! byte-exact round-trip, and confirm the CRC matches `broadcast_common::crc32_mpeg2`.

use std::fs;

use ule::{Sndu, TypeField};

fn fixture() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/appendix_b.bin");
    fs::read(path).expect("fixture appendix_b.bin must be committed")
}

#[test]
fn parses_appendix_b_fields() {
    let data = fixture();
    assert_eq!(data.len(), 67);

    let sndu = Sndu::parse(&data).expect("Appendix B SNDU must parse (CRC valid)");

    // D=0 → NPA present.
    assert!(!sndu.d_bit());
    assert_eq!(
        sndu.dest_address,
        Some([0x00, 0x01, 0x02, 0x03, 0x04, 0x05]),
        "Destination ULE NPA Address"
    );

    // Type 0x86DD (IPv6 EtherType).
    assert_eq!(sndu.type_field(), TypeField::EtherType(0x86DD));

    // Length field = 0x003F = 63 (NPA 6 + PDU 53 + CRC 4).
    assert_eq!(sndu.length_field(), 0x3F);
    assert_eq!(sndu.pdu().len(), 53);

    // The PDU begins with the IPv6 version/traffic-class nibble 0x60.
    assert_eq!(sndu.pdu()[0], 0x60);
}

#[test]
fn appendix_b_crc_matches_mpeg2() {
    let data = fixture();
    // The SNDU bytes excluding the 4-byte CRC trailer.
    let body = &data[..data.len() - 4];
    let computed = broadcast_common::crc32_mpeg2::compute(body);
    let found = u32::from_be_bytes([data[63], data[64], data[65], data[66]]);
    assert_eq!(found, 0x7C17_1763, "RFC 4326 Appendix B stated CRC-32");
    assert_eq!(
        computed, 0x7C17_1763,
        "crc32_mpeg2 must reproduce the RFC's worked CRC"
    );
    assert_eq!(computed, found);
}

#[test]
fn appendix_b_byte_exact_round_trip() {
    let data = fixture();
    let sndu = Sndu::parse(&data).unwrap();

    let mut out = vec![0u8; sndu.serialized_len()];
    let n = sndu.serialize_into(&mut out).unwrap();
    assert_eq!(n, data.len());
    assert_eq!(out, data, "serialize must be byte-identical to the fixture");

    // serialize → parse → equal.
    assert_eq!(Sndu::parse(&out).unwrap(), sndu);
}

#[test]
fn corrupted_crc_is_rejected() {
    let mut data = fixture();
    let last = data.len() - 1;
    data[last] ^= 0xFF; // flip the low CRC byte
    assert!(
        matches!(Sndu::parse(&data), Err(ule::Error::CrcMismatch { .. })),
        "a bad CRC must be rejected"
    );
}
