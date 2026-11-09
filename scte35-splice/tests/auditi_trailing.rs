use broadcast_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;
use scte35_splice::descriptors::AnySpliceDescriptor;

fn with_crc(mut v: Vec<u8>) -> Vec<u8> {
    let crc = broadcast_common::crc32_mpeg2::compute(&v);
    v.extend_from_slice(&crc.to_be_bytes());
    v
}

#[test]
fn splice_null_with_excess_command_bytes_is_rejected_or_preserved() {
    let mut v = vec![0xFC, 0x30, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0, 0xFF, 0xF0, 0x04, 0x00];
    v.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    v.extend_from_slice(&[0x00, 0x00]);
    let len = v.len() + 4 - 3;
    v[2] = len as u8;
    let v = with_crc(v);
    match SpliceInfoSection::parse(&v) {
        Err(_) => {}
        Ok(s) => assert_eq!(
            s.try_to_bytes().unwrap(),
            v,
            "splice_command_length=4 on splice_null: 4 body bytes silently dropped"
        ),
    }
}

#[test]
fn avail_descriptor_with_excess_bytes_is_rejected_or_preserved() {
    let raw = [
        0x00, 0x0A, b'C', b'U', b'E', b'I', 0x00, 0x00, 0x00, 0x01, 0xAA, 0xBB,
    ];
    let d: Vec<_> = scte35_splice::descriptors::parse_loop(&raw).collect();
    match &d[0] {
        Err(_) => {}
        Ok(d) => {
            assert!(matches!(d, AnySpliceDescriptor::Avail(_)));
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, raw, "two trailing descriptor bytes silently dropped");
        }
    }
}
