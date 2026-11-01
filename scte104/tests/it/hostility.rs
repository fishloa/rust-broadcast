//! No-panic hostility sweep (audit S3): the spec-derived message vectors are
//! truncated at every length and mutated at every byte position with every
//! value, and every operation body vector is wrapped under every plausible
//! `opID`, so each operation parser sees realistic-shaped but wrong bodies.
//! The contract is "`Ok` or `Err`, never a panic".

use broadcast_common::{Parse, Serialize};
use scte104::{MultipleOperationMessage, SingleOperationMessage};

#[rustfmt::skip]
const MULTI: [u8; 31] = [
    0xFF, 0xFF, 0x00, 0x1F, 0x00, 0x01, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x01, 0x01, 0x00, 0x0F,
    0x01, 0x00, 0x00, 0x00, 0x42, 0x00, 0x01, 0x13, 0x88, 0x01, 0x2C, 0x00, 0x00, 0x01, 0x00,
];

#[rustfmt::skip]
const SINGLE: [u8; 13] = [
    0x00, 0x00, 0x00, 0x0D, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x01, 0x2A, 0x00, 0x00,
];

/// Operation body vectors (the spec-derived ones in `spec_vectors.rs`).
#[rustfmt::skip]
const BODIES: &[&[u8]] = &[
    // splice_request_data (Table 9-5)
    &[0x01, 0x00, 0x00, 0x00, 0x42, 0x00, 0x01, 0x13, 0x88, 0x01, 0x2C, 0x00, 0x00, 0x01, 0x00],
    // insert_segmentation_descriptor (24 bytes)
    &[0x00, 0x00, 0x00, 0x42, 0x00, 0x0E, 0x10, 0x01, 0x03, 0x01, 0x02, 0x03, 0x30, 0x01, 0x05,
      0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
    &[0x00],
    &[0xFF; 40],
    &[],
];

fn exercise_multi(b: &[u8]) -> bool {
    match MultipleOperationMessage::parse(b) {
        Ok(m) => {
            let _ = m.try_to_bytes();
            true
        }
        Err(_) => false,
    }
}

fn exercise_single(b: &[u8]) -> bool {
    match SingleOperationMessage::parse(b) {
        Ok(m) => {
            let _ = m.try_to_bytes();
            true
        }
        Err(_) => false,
    }
}

#[test]
fn truncation_and_every_value_mutation_never_panic() {
    assert!(exercise_multi(&MULTI) && exercise_single(&SINGLE));
    let mut ok_after = 0usize;
    for n in 0..=MULTI.len() {
        exercise_multi(&MULTI[..n]);
    }
    for n in 0..=SINGLE.len() {
        exercise_single(&SINGLE[..n]);
    }
    for pos in 0..MULTI.len() {
        for v in 0u8..=255 {
            let mut m = MULTI;
            m[pos] = v;
            if exercise_multi(&m) {
                ok_after += 1;
            }
        }
    }
    for pos in 0..SINGLE.len() {
        for v in 0u8..=255 {
            let mut m = SINGLE;
            m[pos] = v;
            exercise_single(&m);
        }
    }
    assert!(ok_after > 100, "{ok_after}");
}

#[test]
fn every_op_id_with_every_body_shape_never_panics() {
    let ids = (0x0000u16..0x0040)
        .chain(0x0100..0x0140)
        .chain([0x8000, 0xFFFF]);
    let mut parsed = 0usize;
    for id in ids {
        for body in BODIES {
            for cut in [body.len(), body.len().saturating_sub(1), body.len() / 2] {
                let body = &body[..cut];
                // header(12 w/o ts) + num_ops(1) + opID(2) + len(2) + body
                let size = 12 + 2 + 2 + body.len();
                let mut m = vec![0xFF, 0xFF];
                m.extend_from_slice(&(size as u16).to_be_bytes());
                m.extend_from_slice(&[0x00, 0x01, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x01]);
                m.extend_from_slice(&id.to_be_bytes());
                m.extend_from_slice(&(body.len() as u16).to_be_bytes());
                m.extend_from_slice(body);
                if exercise_multi(&m) {
                    parsed += 1;
                }
            }
        }
    }
    assert!(parsed > 20, "{parsed}");
}
