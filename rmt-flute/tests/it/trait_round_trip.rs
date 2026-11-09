//! ALC/LCT/FLUTE wire types round-trip through `broadcast_common::{Parse, Serialize}`
//! over the committed FLUTE FDT-packet fixture.

use broadcast_common::{Parse, Serialize};
use rmt_flute::{
    AlcPacket, CencAlgorithm, Error, ExtCenc, ExtFdt, FEC_PAYLOAD_ID_128_LEN, FecPayloadId128,
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
