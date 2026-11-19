//! Random Index Pack key/trailing-length/pair-offset checks (ST 377-1:2019
//! §12, `private/specs-verified/smpte/st377-1/st377-1.md` Table 29).

use broadcast_common::{Parse, Serialize};
use st377_1::{Error, PartitionLocation, RandomIndexPack, UlBytes};

fn two_pair_pack() -> RandomIndexPack {
    RandomIndexPack {
        partitions: vec![
            PartitionLocation {
                body_sid: 1,
                byte_offset: 2,
            },
            PartitionLocation {
                body_sid: 3,
                byte_offset: 4,
            },
        ],
        len_size: Default::default(),
    }
}

#[test]
fn two_pairs_serialize_to_the_hand_computed_layout() {
    let pack = two_pair_pack();
    let mut out = vec![0u8; pack.serialized_len()];
    let n = pack.serialize_into(&mut out).unwrap();
    // 16-byte key, one-byte BER length (28), two 12-byte pairs, 4-byte
    // trailing overall length.
    assert_eq!(n, 45);
    assert_eq!(out.len(), 45);
    assert_eq!(out[16], 28);
    assert_eq!(&out[17..21], &1u32.to_be_bytes());
    assert_eq!(&out[21..29], &2u64.to_be_bytes());
    assert_eq!(&out[29..33], &3u32.to_be_bytes());
    assert_eq!(&out[33..41], &4u64.to_be_bytes());
    assert_eq!(
        &out[41..45],
        &45u32.to_be_bytes(),
        "trailing overall Length"
    );

    let (parsed, consumed) = RandomIndexPack::parse_prefix(&out).unwrap();
    assert_eq!(consumed, out.len());
    assert_eq!(parsed, pack);
    assert_eq!(RandomIndexPack::parse(&out).unwrap(), pack);
}

#[test]
fn a_wrong_key_byte_is_rejected() {
    let mut out = vec![0u8; two_pair_pack().serialized_len()];
    two_pair_pack().serialize_into(&mut out).unwrap();
    out[0] ^= 0xFF;
    assert!(matches!(
        RandomIndexPack::parse(&out),
        Err(Error::KeyPrefixMismatch { .. })
    ));
    assert!(matches!(
        RandomIndexPack::parse_prefix(&out),
        Err(Error::KeyPrefixMismatch { .. })
    ));
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let mut bytes = vec![0u8; two_pair_pack().serialized_len()];
    two_pair_pack().serialize_into(&mut bytes).unwrap();
    for n in 0..bytes.len() {
        assert!(
            RandomIndexPack::parse(&bytes[..n]).is_err(),
            "a {n}-byte prefix must not parse"
        );
    }
}

#[test]
fn a_trailing_length_disagreeing_with_the_total_is_rejected() {
    let mut bytes = vec![0u8; two_pair_pack().serialized_len()];
    two_pair_pack().serialize_into(&mut bytes).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(matches!(
        RandomIndexPack::parse(&bytes),
        Err(Error::InvalidPropertyLength {
            name: "Random Index Pack trailing Length",
            ..
        })
    ));
}

#[test]
fn the_key_is_the_table_29_constant() {
    let key = RandomIndexPack::key();
    assert_eq!(
        key.as_bytes(),
        &[
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x11,
            0x01, 0x00
        ]
    );
    assert_eq!(UlBytes(*key.as_bytes()), key);
    assert!(RandomIndexPack::is_rip_key(&key));
}
