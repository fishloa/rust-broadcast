//! `L1FuturePayload::l1_dynamic_next` (ETSI TS 102 773 §5.2.3 L1-future packet:
//! a 2-byte `L1DYN_NEXT_LEN` in bits, then that many bits of dynamic L1-post).

use dvb_t2mi::Error;
use dvb_t2mi::payload::{L1FuturePayload, L1PostDynamic};

const DYNAMIC_BITS_NO_PLP_NO_AUX: u16 = 79;
const DYNAMIC_BYTES_NO_PLP_NO_AUX: usize = 10;

fn payload(data: &[u8]) -> L1FuturePayload<'_> {
    L1FuturePayload {
        frame_idx: 1,
        l1_future_data: data,
    }
}

fn dynamic_body() -> [u8; DYNAMIC_BYTES_NO_PLP_NO_AUX] {
    [0x2A, 0x00, 0x01, 0x40, 0x00, 0x00, 0x90, 0x55, 0x00, 0x00]
}

#[test]
fn a_zero_length_block_is_absent() {
    assert_eq!(payload(&[0, 0]).l1_dynamic_next(0, 0), Ok(None));
    assert_eq!(payload(&[0, 0, 0xEE, 0xEE]).l1_dynamic_next(3, 1), Ok(None));
}

#[test]
fn the_block_is_decoded_and_trailing_bytes_are_left_alone() {
    let body = dynamic_body();
    let mut data = DYNAMIC_BITS_NO_PLP_NO_AUX.to_be_bytes().to_vec();
    data.extend_from_slice(&body);
    let expected = L1PostDynamic::parse(&body, 0, 0).unwrap();
    assert_eq!(expected.frame_idx, 0x2A);
    assert_eq!(expected.len_bits(), usize::from(DYNAMIC_BITS_NO_PLP_NO_AUX));

    assert_eq!(
        payload(&data).l1_dynamic_next(0, 0),
        Ok(Some(expected.clone()))
    );

    data.extend_from_slice(&[0xAB; 7]);
    assert_eq!(payload(&data).l1_dynamic_next(0, 0), Ok(Some(expected)));
}

#[test]
fn a_missing_length_field_is_reported() {
    assert_eq!(
        payload(&[0x00]).l1_dynamic_next(0, 0),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2,
                have: 1,
                what: "L1DYN_NEXT_LEN",
            }
        ))
    );
    assert_eq!(
        payload(&[]).l1_dynamic_next(0, 0),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2,
                have: 0,
                what: "L1DYN_NEXT_LEN",
            }
        ))
    );
}

#[test]
fn a_block_shorter_than_its_declared_bits_is_reported() {
    let mut data = DYNAMIC_BITS_NO_PLP_NO_AUX.to_be_bytes().to_vec();
    data.extend_from_slice(&dynamic_body()[..DYNAMIC_BYTES_NO_PLP_NO_AUX - 1]);
    assert_eq!(
        payload(&data).l1_dynamic_next(0, 0),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2 + DYNAMIC_BYTES_NO_PLP_NO_AUX,
                have: data.len(),
                what: "L1DYN_NEXT",
            }
        ))
    );
}

#[test]
fn the_declared_bit_length_rounds_up_to_whole_bytes() {
    let mut data = 9u16.to_be_bytes().to_vec();
    data.push(0xFF);
    assert_eq!(
        payload(&data).l1_dynamic_next(0, 0),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 4,
                have: 3,
                what: "L1DYN_NEXT",
            }
        ))
    );
}
