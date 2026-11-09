//! Audit M: parse -> serialize must be byte-identical for every input the
//! parser accepts (the crate README promises reserved bits are preserved).

use broadcast_common::{Parse, Serialize};
use rmt_flute::{AlcPacket, ExtTime, FEC_PAYLOAD_ID_128_LEN, LctHeader, NormCmd};

const LCT_RES_MASK: u8 = 0x0C;

#[test]
fn lct_header_reserved_bits_survive_parse_serialize() {
    let mut data = test_fixtures::read("flute/flute-fdt.bin");
    data[1] |= LCT_RES_MASK;
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();
    let hdr_bytes = pkt.lct.hdr_len() * 4;
    let out = Serialize::try_to_bytes(&pkt.lct).unwrap();
    assert_eq!(out, &data[..hdr_bytes], "Res bits dropped by LctHeader");
    let _ = LctHeader::parse(&data).unwrap();
}

fn norm_cmd(sub_type: u8, b1: u8, b2: u8, b3: u8, extra: &[u8]) -> Vec<u8> {
    let mut v = vec![
        0x13, 0, 0x12, 0x34, 0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0xFF, 0x40, 0xA5, sub_type, b1, b2, b3,
    ];
    v.extend_from_slice(extra);
    v[1] = (v.len() / 4) as u8;
    v
}

fn assert_cmd_round_trips(name: &str, wire: Vec<u8>) {
    let cmd = NormCmd::parse(&wire, 0).unwrap();
    let out = Serialize::try_to_bytes(&cmd).unwrap();
    assert_eq!(out, wire, "{name}: reserved bytes lost");
}

#[test]
fn norm_cmd_eot_reserved_24_bits_survive() {
    assert_cmd_round_trips("EOT", norm_cmd(2, 0xAA, 0xBB, 0xCC, &[]));
}

#[test]
fn norm_cmd_cc_reserved_8_bits_survive() {
    assert_cmd_round_trips("CC", norm_cmd(4, 0xAA, 0x00, 0x07, &[0; 8]));
}

#[test]
fn norm_cmd_repair_adv_reserved_16_bits_survive() {
    assert_cmd_round_trips("REPAIR_ADV", norm_cmd(5, 0x01, 0xBB, 0xCC, &[]));
}

#[test]
fn norm_cmd_ack_req_reserved_8_bits_survive() {
    assert_cmd_round_trips("ACK_REQ", norm_cmd(6, 0xAA, 0x01, 0x09, &[]));
}

#[test]
fn norm_cmd_application_reserved_24_bits_survive() {
    assert_cmd_round_trips("APPLICATION", norm_cmd(7, 0xAA, 0xBB, 0xCC, &[]));
}

#[test]
fn ext_time_serialize_output_is_always_parseable() {
    let t = ExtTime {
        sct_low: Some(1),
        ..Default::default()
    };
    ExtTime::parse(&t.to_content()).expect("serializer emitted SCT-Low without SCT-High");
}
