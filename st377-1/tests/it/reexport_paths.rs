//! Pins the paths `st377-1/CHANGELOG.md` tells a consumer to use.
//!
//! The BER/KLV codec moved to `broadcast_common::klv` (#2341); st377-1's
//! `ber` and `klv` modules are private, so only the crate-root re-exports
//! resolve. Every path the `[Unreleased]` entry names is used here once, so
//! the entry can never again promise a path that does not compile, and the
//! BER functions it re-exports resolve from the crate root (not only from
//! `broadcast_common::klv`).

use broadcast_common::Parse;
use st377_1::{
    BerLength, KlvItem, ber_length_size_for, collect_klv_items, decode_ber_length,
    encode_ber_length_as, walk_klv_items,
};

fn fixture() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x02]);
    bytes.extend_from_slice(&[0x03, 0x01, 0x02, 0x10, 0x01, 0x00, 0x00, 0x00]);
    bytes.push(0x03);
    bytes.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
    bytes.extend_from_slice(&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01]);
    bytes.extend_from_slice(&[0x0D, 0x01, 0x02, 0x01, 0x01, 0x02, 0x00, 0x00]);
    bytes.push(0x01);
    bytes.push(0x7F);
    bytes
}

#[test]
fn crate_root_reexports_resolve_and_work() {
    let bytes = fixture();

    let (first, consumed) = KlvItem::parse_prefix(&bytes).expect("first KLV item");
    assert_eq!(first.value, &[0xAA, 0xBB, 0xCC]);
    assert_eq!(first.len_size, BerLength::fixed_from_consumed(1));
    assert_eq!(consumed, 20);

    let mut walked = Vec::new();
    walk_klv_items(&bytes, |offset, item| {
        walked.push((offset, item.value.to_vec()));
        Ok(())
    })
    .expect("walk");
    assert_eq!(walked, vec![(0, vec![0xAA, 0xBB, 0xCC]), (20, vec![0x7F])]);

    let collected = collect_klv_items(&bytes).expect("collect");
    assert_eq!(collected.len(), 2);
    assert_eq!(
        KlvItem::parse(&bytes[20..]).expect("exact-fit parse").value,
        &[0x7F]
    );

    let (len, used) = decode_ber_length(&[0x03]).expect("a short-form length");
    assert_eq!((len, used), (3, 1));
    let width = BerLength::fixed_from_consumed(1);
    assert_eq!(ber_length_size_for(len, width), 1);
    let mut token = [0u8; 1];
    assert_eq!(encode_ber_length_as(len, width, &mut token), Ok(1));
    assert_eq!(token, [0x03]);
    assert_eq!(
        broadcast_common::klv::ber_length_size_for(len, width),
        st377_1::ber_length_size_for(len, width),
        "the re-export names the same function"
    );
}
