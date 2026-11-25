//! VINT edge cases: every width, the unknown size at every width, and the
//! reserved all-one identifiers (RFC 8794 §4–§6).

use broadcast_matroska::Error;
use broadcast_matroska::ebml::{ElementHeader, ElementId, VintSize, VintWidth};

#[test]
fn every_element_id_width_round_trips() {
    for raw in [0x81u32, 0x407F, 0x203FFF, 0x101F_FFFF] {
        let id = ElementId::try_from_raw(raw).unwrap();
        let mut out = [0u8; 4];
        let written = id.write(&mut out).unwrap();
        assert_eq!(written, id.octets());
        let (back, used) = ElementId::parse(&out[..written]).unwrap();
        assert_eq!(used, written);
        assert_eq!(back, id);
    }
}

#[test]
fn reserved_all_ones_ids_are_refused() {
    for raw in [0xFFu32, 0x7FFF, 0x3F_FFFF, 0x1FFF_FFFF] {
        assert!(
            ElementId::try_from_raw(raw).is_err(),
            "0x{raw:X} must be refused"
        );
    }
    // ChapterDisplay's 0x80 is the Matroska exception to RFC 8794 §5.
    assert!(ElementId::try_from_raw(0x80).is_ok());
    // A five-octet id is outside the four octets this crate reads.
    assert_eq!(
        ElementId::parse(&[0x08, 0, 0, 0, 0x01]),
        Err(Error::ElementIdWidth { width: 5 })
    );
}

#[test]
fn unknown_size_is_preserved_at_every_width() {
    for width in 1..=8usize {
        let size = VintSize::Unknown {
            width: VintWidth::try_new(width).unwrap(),
        };
        let mut out = [0u8; 8];
        let written = size.write(&mut out).unwrap();
        assert_eq!(written, width);
        let (back, used) = VintSize::parse(&out[..width]).unwrap();
        assert_eq!(used, width);
        assert_eq!(back, size);
        assert!(back.is_unknown());
    }
}

#[test]
fn known_sizes_keep_their_encoded_width() {
    for width in 1..=8usize {
        let value = 3u64;
        let size = VintSize::Known {
            value,
            width: VintWidth::try_new(width).unwrap(),
        };
        let mut out = [0u8; 8];
        let written = size.write(&mut out).unwrap();
        assert_eq!(written, width);
        let (back, _) = VintSize::parse(&out[..width]).unwrap();
        assert_eq!(back, size);
        assert_eq!(back.value(), Some(value));
    }
}

#[test]
fn a_zero_first_octet_is_not_a_vint() {
    assert_eq!(ElementId::parse(&[0x00]), Err(Error::VintZeroStart));
    assert_eq!(VintSize::parse(&[0x00]), Err(Error::VintZeroStart));
}

#[test]
fn element_header_round_trips_through_parse_and_serialize() {
    let bytes = [0x18u8, 0x53, 0x80, 0x67, 0x01, 0, 0, 0, 0, 0x01, 0x67, 0x02];
    let (header, used) = ElementHeader::parse(&bytes).unwrap();
    assert_eq!(used, bytes.len());
    let mut out = [0u8; 12];
    let written = header.write(&mut out).unwrap();
    assert_eq!(&out[..written], &bytes);
}
