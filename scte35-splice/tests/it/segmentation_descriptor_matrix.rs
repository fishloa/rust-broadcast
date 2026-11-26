//! Segmentation descriptor value matrix (ANSI/SCTE 35 2023r1 §10.3.3,
//! Table 20): multi-byte fields whose byte order and width a mutant can swap,
//! the optional sections' truncation boundaries, and the MID() walker.

use broadcast_core::{Parse, Serialize};
use scte35_splice::Error;
use scte35_splice::descriptors::segmentation::{
    DeliveryRestrictions, MidUpid, SegmentationComponent, SegmentationDescriptor, SubSegments,
};
use scte35_splice::descriptors::segmentation_enums::{
    DeviceRestrictions, SegmentationTypeId, SegmentationUpidType,
};

const MID_UPID: [u8; 10] = [0x03, 0x04, 0x11, 0x22, 0x33, 0x44, 0x09, 0x02, 0xAA, 0xBB];

/// `02 2D` tag/length, "CUEI", then the layout of Table 20 hand-assembled
/// from the spec: event id 0x01020304, compliance, no delivery restrictions,
/// duration 0x0102030405, two components, a MID() UPID with entries
/// (AdId, 4 bytes) and (Adi, 2 bytes), type 0x34 (Provider Placement
/// Opportunity Start), segment 2 of 5, sub-segment 1 of 3.
#[rustfmt::skip]
const MATRIX_BYTES: [u8; 47] = [
    0x02, 0x2D, 0x43, 0x55, 0x45, 0x49,
    0x01, 0x02, 0x03, 0x04,
    0x7F,
    0x7F,
    0x02,
    0x11, 0xFF, 0x23, 0x45, 0x67, 0x89,
    0x22, 0xFE, 0xFE, 0xDC, 0xBA, 0x98,
    0x01, 0x02, 0x03, 0x04, 0x05,
    0x0D, 0x0A, 0x03, 0x04, 0x11, 0x22, 0x33, 0x44, 0x09, 0x02, 0xAA, 0xBB,
    0x34, 0x02, 0x05,
    0x01, 0x03,
];

fn matrix_descriptor() -> SegmentationDescriptor<'static> {
    SegmentationDescriptor {
        identifier: 0x4355_4549,
        segmentation_event_id: 0x0102_0304,
        segmentation_event_cancel_indicator: false,
        segmentation_event_id_compliance_indicator: true,
        program_segmentation_flag: false,
        delivery_restrictions: None,
        components: vec![
            SegmentationComponent {
                component_tag: 0x11,
                pts_offset: 0x1_2345_6789,
            },
            SegmentationComponent {
                component_tag: 0x22,
                pts_offset: 0x00_FE_DC_BA_98,
            },
        ],
        segmentation_duration: Some(0x01_0203_0405),
        segmentation_upid_type: SegmentationUpidType::Mid,
        segmentation_upid: &MID_UPID,
        segmentation_type_id: SegmentationTypeId::ProviderPlacementOpportunityStart,
        segment_num: 2,
        segments_expected: 5,
        sub_segments: Some(SubSegments {
            sub_segment_num: 1,
            sub_segments_expected: 3,
        }),
        trailing: &[],
    }
}

#[test]
fn the_full_value_matrix_round_trips_byte_exactly() {
    let descriptor = matrix_descriptor();
    let bytes = descriptor.try_to_bytes().unwrap();
    assert_eq!(bytes, MATRIX_BYTES);

    let parsed = SegmentationDescriptor::parse(&MATRIX_BYTES).unwrap();
    assert_eq!(parsed, descriptor);
    assert_eq!(parsed.identifier, 0x4355_4549);
    assert_eq!(parsed.segmentation_event_id, 0x0102_0304);
    assert_eq!(parsed.segmentation_duration, Some(0x01_0203_0405));
    assert_eq!(parsed.components[0].pts_offset, 0x1_2345_6789);
    assert_eq!(parsed.components[1].pts_offset, 0x00_FE_DC_BA_98);
    assert_eq!(parsed.segment_num, 2);
    assert_eq!(parsed.segments_expected, 5);
    assert_eq!(parsed.sub_segments.unwrap().pair(), (1, 3));
}

#[test]
fn the_mid_walker_returns_both_entries_in_order() {
    let parsed = SegmentationDescriptor::parse(&MATRIX_BYTES).unwrap();
    let entries = parsed.mid().expect("MID upid").unwrap();
    assert_eq!(
        entries,
        vec![
            MidUpid {
                upid_type: SegmentationUpidType::from_u8(0x03),
                upid: &[0x11, 0x22, 0x33, 0x44],
            },
            MidUpid {
                upid_type: SegmentationUpidType::from_u8(0x09),
                upid: &[0xAA, 0xBB],
            },
        ]
    );
    assert_eq!(
        SegmentationUpidType::from_u8(0x03),
        SegmentationUpidType::AdId
    );
    assert_eq!(
        SegmentationUpidType::from_u8(0x09),
        SegmentationUpidType::Adi
    );
}

fn descriptor_with_upid<'a>(upid: &'a [u8], sub_segments: bool) -> SegmentationDescriptor<'a> {
    SegmentationDescriptor {
        segmentation_event_id: 1,
        delivery_restrictions: None,
        components: Vec::new(),
        segmentation_duration: None,
        segmentation_upid_type: SegmentationUpidType::Mid,
        segmentation_upid: upid,
        segmentation_type_id: SegmentationTypeId::ProviderPlacementOpportunityStart,
        segment_num: 1,
        segments_expected: 1,
        sub_segments: if sub_segments {
            Some(SubSegments {
                sub_segment_num: 1,
                sub_segments_expected: 1,
            })
        } else {
            None
        },
        ..Default::default()
    }
}

#[test]
fn mid_with_one_trailing_byte_is_a_buffer_too_short() {
    let descriptor = descriptor_with_upid(&[0x03, 0x01, 0xAA, 0x09], false);
    assert_eq!(
        descriptor.mid(),
        Some(Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 5,
                have: 4,
                what: "MID() entry header",
            }
        )))
    );
}

#[test]
fn mid_with_a_declared_entry_length_past_the_buffer_overflows() {
    let descriptor = descriptor_with_upid(&[0x03, 0x01, 0xAA, 0x09, 0x02], false);
    assert_eq!(
        descriptor.mid(),
        Some(Err(Error::LengthOverflow {
            declared: 2,
            available: 0,
            what: "MID() entry segmentation_upid",
        }))
    );
    let exact = descriptor_with_upid(&[0x03, 0x01, 0xAA], false);
    let entries = exact.mid().unwrap().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].upid, &[0xAA]);
}

#[test]
fn delivery_restriction_flags_come_from_their_own_bits() {
    // program_segmentation_flag=1, duration_flag=0, delivery_not_restricted=0,
    // and every restriction flag clear: flags byte 0x80.
    let descriptor = SegmentationDescriptor {
        delivery_restrictions: Some(DeliveryRestrictions {
            web_delivery_allowed: false,
            no_regional_blackout: false,
            archive_allowed: false,
            device_restrictions: DeviceRestrictions::RestrictGroup0,
        }),
        segmentation_upid_type: SegmentationUpidType::NotUsed,
        ..Default::default()
    };
    let bytes = descriptor.try_to_bytes().unwrap();
    assert_eq!(bytes[11], 0x80);
    let parsed = SegmentationDescriptor::parse(&bytes).unwrap();
    assert_eq!(
        parsed.delivery_restrictions,
        descriptor.delivery_restrictions
    );
    assert_eq!(parsed.segmentation_duration, None);
    assert!(parsed.program_segmentation_flag);
}

#[test]
fn the_40_bit_duration_maximum_round_trips_and_one_more_is_rejected() {
    let max = SegmentationDescriptor {
        segmentation_duration: Some(0x00FF_FFFF_FFFF),
        ..Default::default()
    };
    let bytes = max.try_to_bytes().unwrap();
    assert_eq!(&bytes[12..17], &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(
        SegmentationDescriptor::parse(&bytes)
            .unwrap()
            .segmentation_duration,
        Some(0x00FF_FFFF_FFFF)
    );

    let over = SegmentationDescriptor {
        segmentation_duration: Some(0x0100_0000_0000),
        ..Default::default()
    };
    assert_eq!(
        over.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_core::len::FieldOverflow {
            field: "segmentation_descriptor.segmentation_duration",
            value: 0x0100_0000_0000,
            max: 0x00FF_FFFF_FFFF,
        }))
    );
}

#[test]
fn the_33_bit_pts_offset_maximum_round_trips_and_one_more_is_rejected() {
    let max = SegmentationDescriptor {
        program_segmentation_flag: false,
        components: vec![SegmentationComponent {
            component_tag: 0x42,
            pts_offset: 0x1_FFFF_FFFF,
        }],
        ..Default::default()
    };
    let bytes = max.try_to_bytes().unwrap();
    assert_eq!(&bytes[13..19], &[0x42, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(
        SegmentationDescriptor::parse(&bytes).unwrap().components[0].pts_offset,
        0x1_FFFF_FFFF
    );

    let over = SegmentationDescriptor {
        program_segmentation_flag: false,
        components: vec![SegmentationComponent {
            component_tag: 0x42,
            pts_offset: 0x2_0000_0000,
        }],
        ..Default::default()
    };
    assert_eq!(
        over.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_core::len::FieldOverflow {
            field: "segmentation_descriptor.component.pts_offset",
            value: 0x2_0000_0000,
            max: 0x1_FFFF_FFFF,
        }))
    );
}

#[test]
fn the_descriptor_length_field_accepts_exactly_255() {
    let at_limit = SegmentationDescriptor {
        segmentation_upid: &[0xAA; 240],
        ..Default::default()
    };
    let bytes = at_limit.try_to_bytes().unwrap();
    assert_eq!(bytes[1], 255);
    assert_eq!(bytes.len(), 257);

    let over = SegmentationDescriptor {
        segmentation_upid: &[0xAA; 241],
        ..Default::default()
    };
    assert_eq!(
        over.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_core::len::FieldOverflow {
            field: "segmentation_descriptor.descriptor_length",
            value: 256,
            max: 255,
        }))
    );
}

fn truncated_to(bytes: &[u8], len: usize) -> Vec<u8> {
    let mut out = bytes[..len].to_vec();
    out[1] = (len - 2) as u8;
    out
}

#[test]
fn a_descriptor_cut_at_each_optional_section_is_a_typed_error() {
    // Header body offsets: event id 0..4, compliance 4, flags 5, component
    // count 6, components 7..19, duration 19..24, upid header 24..26, upid
    // 26..36, type/segment 36..39, sub-segments 39..41.
    assert_eq!(
        SegmentationDescriptor::parse(&truncated_to(&MATRIX_BYTES, 9)),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 11,
                have: 9,
                what: "segmentation_descriptor header",
            }
        ))
    );
    assert_eq!(
        SegmentationDescriptor::parse(&truncated_to(&MATRIX_BYTES, 11)),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 12,
                have: 11,
                what: "segmentation_descriptor flags",
            }
        ))
    );
    assert_eq!(
        SegmentationDescriptor::parse(&truncated_to(&MATRIX_BYTES, 28)),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 30,
                have: 28,
                what: "segmentation_descriptor segmentation_duration",
            }
        ))
    );

    // A component list whose declared count runs one byte past the body.
    #[rustfmt::skip]
    let component_cut: [u8; 24] = [
        0x02, 0x16, 0x43, 0x55, 0x45, 0x49,
        0x00, 0x00, 0x00, 0x01,
        0x7F,
        0x3F,
        0x02,
        0x11, 0xFF, 0x01, 0x02, 0x03, 0x04,
        0x22, 0xFE, 0xFE, 0xDC, 0xBA,
    ];
    assert_eq!(
        SegmentationDescriptor::parse(&component_cut),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 25,
                have: 24,
                what: "segmentation_descriptor component",
            }
        ))
    );

    // A declared UPID length that runs past the body.
    #[rustfmt::skip]
    let short_upid: [u8; 16] = [
        0x02, 0x0E, 0x43, 0x55, 0x45, 0x49,
        0x00, 0x00, 0x00, 0x01,
        0x7F,
        0xBF,
        0x00, 0x04, 0xAA, 0xBB,
    ];
    assert_eq!(
        SegmentationDescriptor::parse(&short_upid),
        Err(Error::LengthOverflow {
            declared: 4,
            available: 2,
            what: "segmentation_descriptor segmentation_upid",
        })
    );

    // A descriptor cut inside the type/segment triplet.
    let compact = descriptor_with_upid(&[], false).try_to_bytes().unwrap();
    let cut = truncated_to(&compact, compact.len() - 1);
    assert_eq!(
        SegmentationDescriptor::parse(&cut),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 17,
                have: 16,
                what: "segmentation_descriptor type/segment",
            }
        ))
    );

    // The sub-segment appendix is presence-signalled by leftover bytes: with
    // it cut, the descriptor is still well formed and simply has none.
    let cut = truncated_to(&MATRIX_BYTES, MATRIX_BYTES.len() - 2);
    let parsed = SegmentationDescriptor::parse(&cut).unwrap();
    assert_eq!(parsed.sub_segments, None);
    assert_eq!(parsed.trailing, &[] as &[u8]);
}
