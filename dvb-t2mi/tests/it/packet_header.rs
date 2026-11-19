//! T2-MI packet header lengths and payload slicing — ETSI TS 102 773 §5.1.

use broadcast_common::{Parse, Serialize};
use dvb_t2mi::Error;
use dvb_t2mi::packet::{Header, PacketType};

fn header(payload_len_bits: u16) -> Header {
    Header {
        packet_type: PacketType::BasebandFrame,
        packet_count: 0x2A,
        superframe_idx: 5,
        t2mi_stream_id: 7,
        payload_len_bits,
    }
}

#[test]
fn total_bytes_counts_header_payload_and_crc() {
    // 1000 bits = 125 bytes: 6 header + 125 payload + 4 CRC-32.
    assert_eq!(header(1000).total_bytes(), 135);
    // Empty payload: header + CRC only.
    assert_eq!(header(0).total_bytes(), 10);
    // One bit still occupies a whole payload byte.
    assert_eq!(header(1).total_bytes(), 11);
}

#[test]
fn serialized_len_is_the_six_byte_header() {
    let hdr = header(1000);
    assert_eq!(hdr.serialized_len(), 6);

    let mut exact = [0u8; 6];
    assert_eq!(hdr.serialize_into(&mut exact).unwrap(), 6);
    assert_eq!(Header::parse(&exact).unwrap(), hdr);

    let mut small = [0u8; 5];
    assert_eq!(
        hdr.serialize_into(&mut small).err(),
        Some(Error::OutputBufferTooSmall(
            broadcast_common::error::OutputBufferTooSmall { need: 6, have: 5 }
        ))
    );
}

#[test]
fn payload_bytes_accept_a_buffer_without_the_crc_trailer() {
    let hdr = header(1000);
    let mut packet = vec![0u8; hdr.serialized_len()];
    hdr.serialize_into(&mut packet).unwrap();
    packet.extend_from_slice(&[0xAAu8; 125]);

    let payload = hdr.payload_bytes(&packet).unwrap();
    assert_eq!(payload.len(), 125);
    assert!(payload.iter().all(|&b| b == 0xAA));

    let raw = Header::raw_payload_bytes(&packet).unwrap();
    assert_eq!(raw, payload);

    let short = &packet[..6 + 124];
    assert!(matches!(
        hdr.payload_bytes(short),
        Err(Error::PayloadLengthMismatch { .. })
    ));
    assert!(matches!(
        Header::raw_payload_bytes(short),
        Err(Error::PayloadLengthMismatch { .. })
    ));
}

#[test]
fn stream_id_seven_is_encodable_and_eight_is_rejected() {
    let mut hdr = header(0);
    hdr.t2mi_stream_id = 7;
    let mut buf = [0u8; 6];
    hdr.serialize_into(&mut buf).unwrap();
    assert_eq!(buf[2] & 0x07, 7);
    assert_eq!(Header::parse(&buf).unwrap().t2mi_stream_id, 7);

    hdr.t2mi_stream_id = 8;
    assert_eq!(
        hdr.serialize_into(&mut buf).err(),
        Some(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "t2mi_stream_id",
            value: 8,
            max: 7,
        }))
    );
}
