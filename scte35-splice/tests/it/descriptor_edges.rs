//! Small SCTE 35 descriptors (avail/DTMF/time/audio) and the splice
//! descriptor loop: maximum sizes, the shared header's length gate, and the
//! raw/typed iterator paths (ANSI/SCTE 35 2023r1 §10.1-§10.3).

use broadcast_common::{Parse, Serialize};
use scte35_splice::Error;
use scte35_splice::descriptor_loop::SpliceDescriptorLoop;
use scte35_splice::descriptors::{
    AudioComponent, AudioDescriptor, AvailDescriptor, DtmfDescriptor, TimeDescriptor,
};
use scte35_splice::time::duration_to_ticks;

const CUEI: u32 = 0x4355_4549;

#[test]
fn avail_descriptor_has_its_fixed_twelve_bytes() {
    let descriptor = AvailDescriptor {
        identifier: CUEI,
        provider_avail_id: 0xABCD_1234,
    };
    #[rustfmt::skip]
    let expected: [u8; 10] = [
        0x00, 0x08, 0x43, 0x55, 0x45, 0x49,
        0xAB, 0xCD, 0x12, 0x34,
    ];
    let bytes = descriptor.try_to_bytes().unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(AvailDescriptor::parse(&expected).unwrap(), descriptor);
}

#[test]
fn dtmf_descriptor_holds_the_maximum_seven_characters() {
    let descriptor = DtmfDescriptor {
        identifier: CUEI,
        preroll: 40,
        dtmf_chars: b"1234567".to_vec(),
    };
    #[rustfmt::skip]
    let expected: [u8; 15] = [
        0x01, 0x0D, 0x43, 0x55, 0x45, 0x49,
        0x28,
        0xFF,
        b'1', b'2', b'3', b'4', b'5', b'6', b'7',
    ];
    let bytes = descriptor.try_to_bytes().unwrap();
    assert_eq!(bytes, expected);
    let parsed = DtmfDescriptor::parse(&expected).unwrap();
    assert_eq!(parsed, descriptor);
    assert_eq!(
        parsed.preroll_duration(),
        core::time::Duration::from_secs(4)
    );

    let too_many = DtmfDescriptor {
        dtmf_chars: b"12345678".to_vec(),
        ..descriptor
    };
    assert_eq!(
        too_many.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "DTMF_descriptor.dtmf_count",
            value: 8,
            max: 7,
        }))
    );
}

#[test]
fn time_descriptor_holds_the_maximum_forty_eight_bit_tai_seconds() {
    let descriptor = TimeDescriptor {
        identifier: CUEI,
        tai_seconds: 0x0000_FFFF_FFFF_FFFF,
        tai_ns: 0xFFFF_FFFF,
        utc_offset: 0xFFFF,
    };
    #[rustfmt::skip]
    let expected: [u8; 18] = [
        0x03, 0x10, 0x43, 0x55, 0x45, 0x49,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF,
    ];
    let bytes = descriptor.try_to_bytes().unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(TimeDescriptor::parse(&expected).unwrap(), descriptor);

    let over = TimeDescriptor {
        tai_seconds: 1 << 48,
        ..descriptor
    };
    assert_eq!(
        over.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "time_descriptor.tai_seconds",
            value: 1 << 48,
            max: (1 << 48) - 1,
        }))
    );
}

#[test]
fn audio_descriptor_holds_the_maximum_fifteen_components() {
    let component = AudioComponent {
        component_tag: 0x11,
        iso_code: *b"eng",
        bit_stream_mode: 5,
        num_channels: 9,
        full_srvc_audio: true,
    };
    let descriptor = AudioDescriptor {
        identifier: CUEI,
        components: vec![component; 15],
    };
    let bytes = descriptor.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 82);
    assert_eq!(bytes[1], 80, "descriptor_length");
    assert_eq!(bytes[6], 0xFF, "4-bit audio_count 15 + reserved");
    assert_eq!(&bytes[7..12], &[0x11, b'e', b'n', b'g', 0xB3]);
    let parsed = AudioDescriptor::parse(&bytes).unwrap();
    assert_eq!(parsed, descriptor);
    assert_eq!(parsed.components[0].bit_stream_mode, 5);
    assert_eq!(parsed.components[0].num_channels, 9);
    assert!(parsed.components[0].full_srvc_audio);

    let too_many = AudioDescriptor {
        components: vec![component; 16],
        ..descriptor
    };
    assert_eq!(
        too_many.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "audio_descriptor.audio_count",
            value: 16,
            max: 15,
        }))
    );
}

#[test]
fn the_shared_header_gate_rejects_each_truncation_shape() {
    // One byte is not even tag + length.
    assert_eq!(
        AvailDescriptor::parse(&[0x00]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2,
                have: 1,
                what: "avail_descriptor",
            }
        ))
    );
    // Two bytes: declared length 0 is below the 4-byte identifier.
    assert_eq!(
        AvailDescriptor::parse(&[0x00, 0x00]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 6,
                have: 2,
                what: "avail_descriptor",
            }
        ))
    );
    // Exactly the identifier: the header is complete, the body is what the
    // descriptor itself rejects.
    assert_eq!(
        DtmfDescriptor::parse(&[0x01, 0x04, 0x43, 0x55, 0x45, 0x49]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 8,
                have: 6,
                what: "DTMF_descriptor preroll/count",
            }
        ))
    );
    // Declared length one more than the bytes present.
    assert_eq!(
        AvailDescriptor::parse(&[0x00, 0x08, 0x43, 0x55, 0x45, 0x49]),
        Err(Error::LengthOverflow {
            declared: 8,
            available: 4,
            what: "avail_descriptor",
        })
    );
    // Declared length one short of the identifier.
    assert_eq!(
        AvailDescriptor::parse(&[0x00, 0x03, 0x43, 0x55, 0x45]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 6,
                have: 5,
                what: "avail_descriptor",
            }
        ))
    );
    // A short body with a matching declared length.
    assert_eq!(
        AvailDescriptor::parse(&[0x00, 0x07, 0x43, 0x55, 0x45, 0x49, 1, 2, 3]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 10,
                have: 9,
                what: "avail_descriptor provider_avail_id",
            }
        ))
    );
}

#[test]
fn a_two_descriptor_loop_walks_both_entries_and_keeps_its_bytes() {
    #[rustfmt::skip]
    let raw = [
        0x00u8, 0x08, 0x43, 0x55, 0x45, 0x49, 0, 0, 0, 1,
        0x01, 0x06, 0x43, 0x55, 0x45, 0x49, 0x28, 0x1F,
    ];
    let descriptor_loop = SpliceDescriptorLoop::new(&raw);
    assert!(!descriptor_loop.is_empty());
    assert_eq!(descriptor_loop.len(), 18);
    assert_eq!(&*descriptor_loop, &raw[..], "deref is the raw bytes");
    assert_eq!(SpliceDescriptorLoop::from(&raw[..]).raw(), &raw[..]);

    let mut entries = descriptor_loop.entries();
    let first = entries.next().unwrap().unwrap();
    assert_eq!(first.tag, 0x00);
    assert_eq!(first.body.len(), 8);
    let second = entries.next().unwrap().unwrap();
    assert_eq!(second.tag, 0x01);
    assert_eq!(second.body.len(), 6);
    assert!(entries.next().is_none());
}

#[test]
fn an_empty_loop_is_empty_and_yields_nothing() {
    let empty = SpliceDescriptorLoop::new(&[]);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(&*empty, &[] as &[u8]);
    assert_eq!(empty.entries().count(), 0);
}

#[test]
fn a_truncated_loop_entry_is_one_error_then_fused() {
    let raw = [0x00u8, 0x08, 0x43];
    let mut entries = SpliceDescriptorLoop::new(&raw).entries();
    assert!(matches!(
        entries.next(),
        Some(Err(Error::BufferTooShort(_)))
    ));
    assert!(entries.next().is_none());
}

#[cfg(feature = "serde")]
#[test]
fn serde_serializes_the_loop_as_its_descriptors() {
    #[rustfmt::skip]
    let raw = [
        0x00u8, 0x08, 0x43, 0x55, 0x45, 0x49, 0, 0, 0, 1,
        0x01, 0x06, 0x43, 0x55, 0x45, 0x49, 0x28, 0x1F,
    ];
    let json = serde_json::to_string(&SpliceDescriptorLoop::new(&raw)).unwrap();
    assert!(json.contains("\"avail\""), "{json}");
    assert!(json.contains("\"dtmf\""), "{json}");
    assert!(!json.is_empty());
}

#[test]
fn duration_to_ticks_accepts_exactly_the_maximum() {
    // 11 112 ns is just over one 90 kHz tick (11 111.11 ns).
    let one_tick = core::time::Duration::from_nanos(11_112);
    assert_eq!(duration_to_ticks(one_tick, 1), Some(1));
    assert_eq!(duration_to_ticks(one_tick, 0), None);
}

#[test]
fn a_loop_left_with_a_single_header_byte_is_one_error() {
    let mut entries = SpliceDescriptorLoop::new(&[0x00]).entries();
    assert_eq!(
        entries.next(),
        Some(Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2,
                have: 1,
                what: "splice_descriptor header in loop",
            }
        )))
    );
    assert!(entries.next().is_none());
}

#[test]
fn an_audio_descriptor_with_an_empty_body_reports_the_count_need() {
    assert_eq!(
        AudioDescriptor::parse(&[0x04, 0x04, 0x43, 0x55, 0x45, 0x49]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 7,
                have: 6,
                what: "audio_descriptor audio_count",
            }
        ))
    );
}

#[test]
fn an_audio_component_cut_short_reports_the_absolute_need() {
    // audio_count 2, one full component, two bytes of the second.
    #[rustfmt::skip]
    let bytes: [u8; 14] = [
        0x04, 0x0C, 0x43, 0x55, 0x45, 0x49,
        0x20,
        0x11, b'e', b'n', b'g', 0x1F,
        0x22, b'd',
    ];
    assert_eq!(
        AudioDescriptor::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 17,
                have: 14,
                what: "audio_descriptor component",
            }
        ))
    );
}

#[test]
fn a_dtmf_body_short_of_its_declared_chars_reports_the_need() {
    // preroll, a count byte declaring two characters, one character present.
    #[rustfmt::skip]
    let bytes: [u8; 9] = [
        0x01, 0x07, 0x43, 0x55, 0x45, 0x49,
        0x28,
        0x5F,
        b'1',
    ];
    assert_eq!(
        DtmfDescriptor::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 10,
                have: 9,
                what: "DTMF_descriptor chars",
            }
        ))
    );
}

#[test]
fn a_time_descriptor_short_of_its_twelve_byte_body_reports_the_need() {
    let mut bytes = vec![0x03u8, 0x0F, 0x43, 0x55, 0x45, 0x49];
    bytes.extend_from_slice(&[0u8; 11]);
    assert_eq!(
        TimeDescriptor::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 18,
                have: 17,
                what: "time_descriptor body",
            }
        ))
    );
}

#[test]
fn a_loop_entry_with_a_zero_length_body_is_valid() {
    let mut entries = SpliceDescriptorLoop::new(&[0x00, 0x00]).entries();
    let entry = entries.next().unwrap().unwrap();
    assert_eq!(entry.tag, 0x00);
    assert!(entry.body.is_empty());
    assert!(entries.next().is_none());
}

#[test]
fn an_over_long_avail_body_keeps_the_first_four_bytes() {
    // descriptor_length 9: four identifier bytes + five body bytes; the
    // provider_avail_id is the first four of those.
    #[rustfmt::skip]
    let bytes: [u8; 11] = [
        0x00, 0x09, 0x43, 0x55, 0x45, 0x49,
        0xAB, 0xCD, 0x12, 0x34, 0xFF,
    ];
    let parsed = AvailDescriptor::parse(&bytes).unwrap();
    assert_eq!(parsed.provider_avail_id, 0xABCD_1234);
}

#[test]
fn an_over_long_time_body_keeps_the_first_twelve_bytes() {
    let mut bytes = vec![0x03u8, 0x11, 0x43, 0x55, 0x45, 0x49];
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x05]);
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x09, 0x00, 0x25]);
    bytes.push(0xFF);
    let parsed = TimeDescriptor::parse(&bytes).unwrap();
    assert_eq!(parsed.tai_seconds, 5);
    assert_eq!(parsed.tai_ns, 9);
    assert_eq!(parsed.utc_offset, 0x0025);
}

#[test]
fn a_dtmf_body_short_of_three_declared_chars_reports_the_need() {
    // preroll, a count byte declaring three characters, two present.
    #[rustfmt::skip]
    let bytes: [u8; 10] = [
        0x01, 0x08, 0x43, 0x55, 0x45, 0x49,
        0x28,
        0x7F,
        b'1', b'2',
    ];
    assert_eq!(
        DtmfDescriptor::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 11,
                have: 10,
                what: "DTMF_descriptor chars",
            }
        ))
    );
}
