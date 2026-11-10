//! ALC/LCT/FLUTE wire types round-trip through `broadcast_common::{Parse, Serialize}`
//! over the committed FLUTE FDT-packet fixture.

use broadcast_common::{Parse, Serialize};
use rmt_flute::{
    AlcPacket, CencAlgorithm, Error, ExtCenc, ExtFdt, FEC_PAYLOAD_ID_128_LEN, FecPayloadId128,
    NormCmd,
};

fn fixture() -> Vec<u8> {
    test_fixtures::read("flute/flute-fdt.bin")
}

#[test]
fn alc_packet_serializes_byte_identically_through_the_trait() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();
    assert_eq!(Serialize::try_to_bytes(&pkt).unwrap(), data);
    assert_eq!(Serialize::serialized_len(&pkt), data.len());
}

#[test]
fn lct_header_and_extension_serialize_through_the_trait() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();
    let hdr_bytes = pkt.lct.hdr_len() * 4;
    assert_eq!(
        Serialize::try_to_bytes(&pkt.lct).unwrap(),
        &data[..hdr_bytes]
    );

    let ext = &pkt.lct.extensions[0];
    let wire = Serialize::try_to_bytes(ext).unwrap();
    assert_eq!(wire.len(), 4);
    assert_eq!(wire, &data[hdr_bytes - 4..hdr_bytes]);
}

#[test]
fn fec_payload_id_and_ext_fdt_round_trip_through_the_traits() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();

    let fpid = <FecPayloadId128 as Parse>::parse(pkt.fec_payload_id).unwrap();
    assert_eq!(Serialize::try_to_bytes(&fpid).unwrap(), pkt.fec_payload_id);

    let content = pkt.lct.extensions[0].content;
    let fdt = <ExtFdt as Parse>::parse(content).unwrap();
    let wire = Serialize::try_to_bytes(&fdt).unwrap();
    assert_eq!(wire, content);
    assert_eq!(<ExtFdt as Parse>::parse(&wire).unwrap(), fdt);
}

#[test]
fn ext_cenc_round_trips_through_the_traits() {
    let content = [0x01u8, 0x00, 0x00];
    let cenc = <ExtCenc as Parse>::parse(&content).unwrap();
    assert_eq!(cenc.algorithm, CencAlgorithm::from_u8(1));
    let wire = Serialize::try_to_bytes(&cenc).unwrap();
    assert_eq!(wire, content);
}

#[test]
fn short_output_is_output_buffer_too_small() {
    let fdt = ExtFdt {
        version: 1,
        instance_id: 5,
    };
    assert_eq!(
        Serialize::serialize_into(&fdt, &mut [0u8; 2]),
        Err(Error::OutputBufferTooSmall(
            broadcast_common::error::OutputBufferTooSmall { need: 3, have: 2 }
        ))
    );
}

const LCT_RES_MASK: u8 = 0x0C;

#[test]
fn lct_header_with_reserved_bits_set_serializes_byte_identically() {
    let mut data = fixture();
    data[1] |= LCT_RES_MASK;
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();
    assert_eq!(pkt.lct.reserved, 0x03);
    let hdr_bytes = pkt.lct.hdr_len() * 4;
    assert_eq!(
        Serialize::try_to_bytes(&pkt.lct).unwrap(),
        &data[..hdr_bytes]
    );
}

fn norm_cmd_wire(sub_type: u8, b1: u8, b2: u8, b3: u8, extra: &[u8]) -> Vec<u8> {
    let mut v = vec![
        0x13, 0, 0x12, 0x34, 0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0xFF, 0x40, 0xA5, sub_type, b1, b2, b3,
    ];
    v.extend_from_slice(extra);
    v[1] = (v.len() / 4) as u8;
    v
}

fn assert_norm_cmd_round_trips(name: &str, wire: Vec<u8>) {
    let cmd = NormCmd::parse(&wire, 0).unwrap();
    let out = Serialize::try_to_bytes(&cmd).unwrap();
    assert_eq!(out, wire, "{name}: reserved bytes lost");
}

#[test]
fn norm_cmd_eot_reserved_word_round_trips_byte_exact() {
    assert_norm_cmd_round_trips("EOT", norm_cmd_wire(2, 0xAA, 0xBB, 0xCC, &[]));
}

#[test]
fn norm_cmd_cc_reserved_byte_round_trips_byte_exact() {
    assert_norm_cmd_round_trips("CC", norm_cmd_wire(4, 0xAA, 0x00, 0x07, &[0; 8]));
}

#[test]
fn norm_cmd_repair_adv_reserved_bytes_round_trip_byte_exact() {
    assert_norm_cmd_round_trips("REPAIR_ADV", norm_cmd_wire(5, 0x01, 0xBB, 0xCC, &[]));
}

#[test]
fn norm_cmd_ack_req_reserved_byte_round_trips_byte_exact() {
    assert_norm_cmd_round_trips("ACK_REQ", norm_cmd_wire(6, 0xAA, 0x01, 0x09, &[]));
}

#[test]
fn norm_cmd_application_reserved_word_round_trips_byte_exact() {
    assert_norm_cmd_round_trips("APPLICATION", norm_cmd_wire(7, 0xAA, 0xBB, 0xCC, &[]));
}

#[test]
fn lct_header_parses_and_round_trips_through_the_trait() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();
    let hdr_bytes = pkt.lct.hdr_len() * 4;
    let hdr = <rmt_flute::LctHeader as Parse>::parse(&data[..hdr_bytes]).unwrap();
    let wire = Serialize::try_to_bytes(&hdr).unwrap();
    assert_eq!(wire, &data[..hdr_bytes]);
    let mut changed = hdr.clone();
    changed.codepoint ^= 0x01;
    assert_ne!(Serialize::try_to_bytes(&changed).unwrap(), wire);
}

#[test]
fn header_extension_parses_and_round_trips_through_the_trait() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();
    let ext = &pkt.lct.extensions[0];
    let wire = Serialize::try_to_bytes(ext).unwrap();
    let parsed = <rmt_flute::HeaderExtension as Parse>::parse(&wire).unwrap();
    assert_eq!(Serialize::try_to_bytes(&parsed).unwrap(), wire);
    let mut changed = parsed.clone();
    changed.het ^= 0x01;
    // het change may alter length; just assert bytes differ when possible
    if let Ok(w2) = Serialize::try_to_bytes(&changed) {
        assert_ne!(w2, wire);
    }
}

#[test]
fn ext_time_parses_and_round_trips_through_the_trait() {
    let content = [0x01, 0x03, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x20];
    let t = <rmt_flute::ExtTime as Parse>::parse(&content).unwrap();
    let wire = Serialize::try_to_bytes(&t).unwrap();
    assert_eq!(wire, content);
    let mut changed = t.clone();
    changed.pi_specific ^= 0x01;
    assert_ne!(Serialize::try_to_bytes(&changed).unwrap(), wire);
}
