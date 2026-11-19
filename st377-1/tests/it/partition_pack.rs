//! Partition Pack key validation, fixed-field offsets, truncation safety,
//! serialized length and the parsed-batch reuse rule (ST 377-1:2019 §7,
//! `private/specs-verified/smpte/st377-1/st377-1.md` Tables 4/6/7/8).

use broadcast_common::{Parse, Serialize};
use st377_1::{Error, PartitionKind, PartitionPack, PartitionStatus, UidBatch, UlBytes};

fn ul(byte: u8) -> UlBytes {
    UlBytes([byte; 16])
}

/// Every fixed field carries a distinct, non-trivial value so a reader taking
/// the wrong offset cannot pass.
fn distinct_pack() -> PartitionPack {
    PartitionPack {
        kind: PartitionKind::Body,
        status: PartitionStatus::ClosedComplete,
        major_version: 0x0102,
        minor_version: 0x0304,
        kag_size: 0x0506_0708,
        this_partition: 0x0102_0304_0506_0708,
        previous_partition: 0x1112_1314_1516_1718,
        footer_partition: 0x2122_2324_2526_2728,
        header_byte_count: 0x3132_3334_3536_3738,
        index_byte_count: 0x4142_4344_4546_4748,
        index_sid: 0x5152_5354,
        body_offset: 0x6162_6364_6566_6768,
        body_sid: 0x7172_7374,
        operational_pattern: ul(0x81),
        essence_containers: vec![ul(0x91), ul(0x92), ul(0xA3)],
        essence_containers_batch: None,
        len_size: Default::default(),
    }
}

#[test]
fn exact_bytes_and_offsets_of_every_fixed_field() {
    let pack = distinct_pack();
    let mut out = vec![0u8; pack.serialized_len()];
    let n = pack.serialize_into(&mut out).unwrap();
    assert_eq!(n, out.len());
    // key (16) + BER value length (0x81, 136) + 80 fixed bytes + 8-byte batch
    // header + 3 x 16-byte containers.
    assert_eq!(out.len(), 154);
    assert_eq!(&out[16..18], &[0x81, 136]);

    assert_eq!(&out[18..20], &0x0102u16.to_be_bytes());
    assert_eq!(&out[20..22], &0x0304u16.to_be_bytes());
    assert_eq!(&out[22..26], &0x0506_0708u32.to_be_bytes());
    assert_eq!(&out[26..34], &0x0102_0304_0506_0708u64.to_be_bytes());
    assert_eq!(&out[34..42], &0x1112_1314_1516_1718u64.to_be_bytes());
    assert_eq!(&out[42..50], &0x2122_2324_2526_2728u64.to_be_bytes());
    assert_eq!(&out[50..58], &0x3132_3334_3536_3738u64.to_be_bytes());
    assert_eq!(&out[58..66], &0x4142_4344_4546_4748u64.to_be_bytes());
    assert_eq!(&out[66..70], &0x5152_5354u32.to_be_bytes());
    assert_eq!(&out[70..78], &0x6162_6364_6566_6768u64.to_be_bytes());
    assert_eq!(&out[78..82], &0x7172_7374u32.to_be_bytes());
    assert_eq!(&out[82..98], &[0x81; 16]);
    assert_eq!(&out[98..106], &[0, 0, 0, 3, 0, 0, 0, 16]);
    assert_eq!(&out[106..122], &[0x91; 16]);
    assert_eq!(&out[122..138], &[0x92; 16]);
    assert_eq!(&out[138..154], &[0xA3; 16]);
    // Table 4: byte 14 = kind, byte 15 = status.
    assert_eq!(out[13], 0x03, "Body kind");
    assert_eq!(out[14], 0x04, "Closed/Complete status");

    assert_eq!(PartitionPack::parse(&out).unwrap(), pack);
}

#[test]
fn one_byte_wrong_in_any_key_segment_is_rejected() {
    let mut original = vec![0u8; distinct_pack().serialized_len()];
    distinct_pack().serialize_into(&mut original).unwrap();
    for at in [0usize, 8, 12] {
        let mut bytes = original.clone();
        bytes[at] ^= 0xFF;
        assert!(
            matches!(
                PartitionPack::parse(&bytes),
                Err(Error::KeyPrefixMismatch { .. })
            ),
            "flipping key byte {at} must not parse as a Partition Pack"
        );
    }
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    let mut bytes = vec![0u8; distinct_pack().serialized_len()];
    distinct_pack().serialize_into(&mut bytes).unwrap();
    for n in 0..bytes.len() {
        assert!(
            PartitionPack::parse(&bytes[..n]).is_err(),
            "a {n}-byte prefix must not parse"
        );
    }
}

#[test]
fn an_edited_container_list_is_written_with_its_new_batch() {
    let mut bytes = vec![0u8; distinct_pack().serialized_len()];
    distinct_pack().serialize_into(&mut bytes).unwrap();
    let mut pack = PartitionPack::parse(&bytes).unwrap();
    assert_eq!(pack.essence_containers.len(), 3);

    pack.essence_containers = vec![ul(0x77)];
    let mut out = vec![0u8; pack.serialized_len()];
    pack.serialize_into(&mut out).unwrap();
    let reparsed = PartitionPack::parse(&out).unwrap();
    assert_eq!(reparsed.essence_containers, vec![ul(0x77)]);
    assert_eq!(&out[98..106], &[0, 0, 0, 1, 0, 0, 0, 16]);
}

#[test]
fn unchanged_containers_keep_the_parsed_batch_header() {
    let mut bytes = vec![0u8; distinct_pack().serialized_len()];
    distinct_pack().serialize_into(&mut bytes).unwrap();
    let mut pack = PartitionPack::parse(&bytes).unwrap();
    // A non-canonical header (item_len 17) survives only if the serializer
    // reuses the parsed batch for an unchanged container list.
    pack.essence_containers_batch = Some(UidBatch::with_header(
        pack.essence_containers.clone(),
        Some([0, 0, 0, 3, 0, 0, 0, 17]),
    ));
    let mut out = vec![0u8; pack.serialized_len()];
    pack.serialize_into(&mut out).unwrap();
    assert_eq!(&out[98..106], &[0, 0, 0, 3, 0, 0, 0, 17]);
}
