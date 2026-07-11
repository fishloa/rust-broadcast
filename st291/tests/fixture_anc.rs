//! Real-ish fixture test: parse the committed `tests/fixtures/anc.bin` (a
//! 2-ANC-packet ANC data PES hand-constructed from ST 2038 Table 2), assert the
//! decoded fields against independently-known expected values, and verify a
//! byte-exact round-trip.

use std::fs;

use st291::{AncDataPacket, AncPacket};

fn fixture() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/st291/anc.bin");
    fs::read(path).expect("fixture anc.bin must be committed")
}

#[test]
fn parses_expected_fields() {
    let data = fixture();
    let pkt = AncDataPacket::parse(&data).unwrap();

    // PES header expectations.
    assert_eq!(
        data[0..4],
        [0x00, 0x00, 0x01, 0xBD],
        "start_code + stream_id"
    );
    assert_eq!(data[8], 0x05, "PES_header_data_length");
    assert_eq!(pkt.pts, 0x1_2345_6789);
    assert!(!pkt.pes_priority);

    // Two ANC packets.
    assert_eq!(pkt.anc_packets.len(), 2);

    let a0 = &pkt.anc_packets[0];
    assert_eq!(
        a0,
        &AncPacket {
            c_not_y_channel_flag: false,
            line_number: 9,
            horizontal_offset: 0,
            did: 0x161,
            sdid: 0x101,
            data_count: 0x002,
            user_data_words: vec![0x2CF, 0x101],
            checksum: 0x233,
        }
    );

    let a1 = &pkt.anc_packets[1];
    assert_eq!(
        a1,
        &AncPacket {
            c_not_y_channel_flag: true,
            line_number: 0x2A,
            horizontal_offset: 0x10,
            did: 0x241,
            sdid: 0x102,
            data_count: 0x003,
            user_data_words: vec![0x111, 0x222, 0x333],
            checksum: 0x1AB,
        }
    );

    assert_eq!(pkt.stuffing_bytes, 5);
}

#[test]
fn byte_exact_round_trip() {
    let data = fixture();
    let pkt = AncDataPacket::parse(&data).unwrap();
    let mut out = vec![0u8; pkt.serialized_len()];
    let n = pkt.serialize_into(&mut out).unwrap();
    assert_eq!(n, data.len());
    assert_eq!(out, data, "serialize must be byte-identical to the fixture");

    // And serialize -> parse -> equal.
    assert_eq!(AncDataPacket::parse(&out).unwrap(), pkt);
}
