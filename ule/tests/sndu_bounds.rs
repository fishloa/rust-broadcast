//! `Sndu::parse` `Length` boundaries (RFC 4326 §4.2): the smallest legal
//! SNDUs (empty payload with and without the NPA address), and the paths that
//! must report an error instead of slicing past the buffer.

use ule::{Error, Sndu};

fn with_crc(mut body: Vec<u8>) -> Vec<u8> {
    let crc = broadcast_common::crc32_mpeg2::compute(&body);
    body.extend_from_slice(&crc.to_be_bytes());
    body
}

#[test]
fn a_length_one_past_the_available_bytes_is_rejected_without_panicking() {
    // 4-byte header + 4 bytes present, Length = 5 (= data.len() - 3).
    let data = [0x80u8, 0x05, 0x08, 0x00, 0xAA, 0xBB, 0xCC, 0xDD];
    assert_eq!(
        Sndu::parse(&data).unwrap_err(),
        Error::InvalidLength {
            length: 5,
            reason: "Length field exceeds available bytes",
        }
    );
}

#[test]
fn a_crc_only_d1_sndu_is_valid_with_an_empty_payload() {
    let data = with_crc(vec![0x80, 0x04, 0x08, 0x00]);
    let sndu = Sndu::parse(&data).expect("D=1 Length=4 is the smallest CRC-only SNDU");
    assert!(sndu.d_bit());
    assert_eq!(sndu.length_field(), 4);
    assert!(sndu.pdu().is_empty());
    assert_eq!(
        sndu.serialize_into(&mut vec![0u8; sndu.serialized_len()])
            .unwrap(),
        8
    );
}

#[test]
fn a_npa_and_crc_only_d0_sndu_is_valid_with_an_empty_payload() {
    let data = with_crc(vec![0x00, 0x0A, 0x08, 0x00, 0, 1, 2, 3, 4, 5]);
    let sndu = Sndu::parse(&data).expect("D=0 Length=10 is the smallest NPA+CRC SNDU");
    assert!(!sndu.d_bit());
    assert_eq!(sndu.dest_address, Some([0, 1, 2, 3, 4, 5]));
    assert_eq!(sndu.length_field(), 10);
    assert!(sndu.pdu().is_empty());
}

#[test]
fn a_d0_length_that_cannot_hold_the_npa_and_crc_is_rejected() {
    let data = [0x00u8, 0x04, 0x08, 0x00, 0, 1, 2, 3];
    assert_eq!(
        Sndu::parse(&data).unwrap_err(),
        Error::InvalidLength {
            length: 4,
            reason: "Length cannot hold the NPA address + CRC",
        }
    );
}

#[test]
fn a_d0_length_boundary_at_the_npa_and_crc_size_is_rejected() {
    // Length = 6: room for the NPA but not for the CRC after it.
    let data = [0x00u8, 0x06, 0x08, 0x00, 0, 1, 2, 3, 4, 5];
    assert_eq!(
        Sndu::parse(&data).unwrap_err(),
        Error::InvalidLength {
            length: 6,
            reason: "Length leaves no room for the payload",
        }
    );
}

#[test]
fn a_d0_length_shorter_than_npa_plus_crc_is_rejected() {
    let data = [0x00u8, 0x09, 0x08, 0x00, 0, 1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(
        Sndu::parse(&data).unwrap_err(),
        Error::InvalidLength {
            length: 9,
            reason: "Length leaves no room for the payload",
        }
    );
}

#[test]
fn a_length_shorter_than_the_crc_trailer_is_rejected() {
    let data = [0x80u8, 0x03, 0x08, 0x00, 0xAA, 0xBB, 0xCC];
    assert_eq!(
        Sndu::parse(&data).unwrap_err(),
        Error::InvalidLength {
            length: 3,
            reason: "Length shorter than the CRC trailer",
        }
    );
}
