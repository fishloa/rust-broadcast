//! DVB-TA (ETSI TS 103 752-1) compact encodings and stream-event payload
//! bounds: the bit-width arithmetic, the `bit_err` mapping, the DAS
//! `descriptor_length` gate, and every truncation boundary.

use broadcast_common::{Parse, Serialize};
use scte35_splice::commands::{AnyCommand, SpliceNull};
use scte35_splice::descriptor_loop::SpliceDescriptorLoop;
use scte35_splice::dvb_ta::{
    CompactDas, CompactSpliceInsert, CompactTimeSignal, DasNibbleOrder, DvbDasDescriptor,
    EquivalentSegmentationType, PrivateData, Scte35Carriage, StreamEventPayload, TimelineType,
};
use scte35_splice::Error;
use scte35_splice::SpliceInfoSection;

fn time_signal() -> CompactTimeSignal {
    CompactTimeSignal {
        encrypted_packet: false,
        encryption_algorithm: 0x2A,
        cw_index: 0xC3,
        pts_time: 0x1_2345_6789,
        segmentation_event_id: 0x0102_0304,
        segmentation_duration: 0x01_0203_0405,
        segmentation_type_id: 0x34,
        segmentation_upid: b"urn:example".to_vec(),
        segments_num: 2,
        segments_expected: 5,
        e_crc_32: None,
    }
}

#[test]
fn compact_time_signal_round_trips_its_fixed_width_fields() {
    let signal = time_signal();
    let bytes = signal.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 17 + 11 + 2);
    let parsed = CompactTimeSignal::parse(&bytes).unwrap();
    assert_eq!(parsed, signal);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn compact_time_signal_rejects_a_short_fixed_prefix() {
    assert_eq!(
        CompactTimeSignal::parse(&[0u8; 16]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 17,
                have: 16,
                what: "compact_time_signal fixed prefix",
            }
        ))
    );
}

#[test]
fn compact_time_signal_reports_a_truncated_upid_with_real_byte_counts() {
    let mut bytes = time_signal().try_to_bytes().unwrap();
    bytes.truncate(17);
    bytes[16] = 1;
    assert_eq!(
        CompactTimeSignal::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 18,
                have: 17,
                what: "compact_time_signal",
            }
        ))
    );
}

#[test]
fn compact_time_signal_field_maxima_are_the_bit_widths() {
    let max = CompactTimeSignal {
        pts_time: (1 << 33) - 1,
        segmentation_duration: (1 << 40) - 1,
        encryption_algorithm: 0x3F,
        ..time_signal()
    };
    let bytes = max.try_to_bytes().unwrap();
    assert_eq!(CompactTimeSignal::parse(&bytes).unwrap(), max);

    assert_eq!(
        CompactTimeSignal {
            pts_time: 1 << 33,
            ..max.clone()
        }
        .try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "compact_time_signal.pts_time",
            value: 1 << 33,
            max: (1 << 33) - 1,
        }))
    );
    assert_eq!(
        CompactTimeSignal {
            segmentation_duration: 1 << 40,
            ..max.clone()
        }
        .try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "compact_time_signal.segmentation_duration",
            value: 1 << 40,
            max: (1 << 40) - 1,
        }))
    );
    assert_eq!(
        CompactTimeSignal {
            encryption_algorithm: 0x40,
            ..max
        }
        .try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "compact_time_signal.encryption_algorithm",
            value: 0x40,
            max: 0x3F,
        }))
    );
}

fn compact_insert() -> CompactSpliceInsert {
    CompactSpliceInsert {
        encrypted_packet: false,
        encryption_algorithm: 1,
        cw_index: 0x22,
        pts_time: 0x1_0000_0001,
        splice_event_id: 0x0A0B_0C0D,
        duration: 0x00FF_FFFF_FF,
        unique_program_id: 0x1111,
        avail_num: 3,
        avails_expected: 4,
        reserved: 0x3F,
        das: Some(CompactDas {
            break_num: 1,
            breaks_expected: 2,
            equivalent_segmentation_type: EquivalentSegmentationType::ProviderPlacementOpportunity,
            reserved: 0x0F,
            upid: b"urn:abc".to_vec(),
        }),
        e_crc_32: None,
    }
}

#[test]
fn compact_splice_insert_round_trips_its_das_body() {
    let insert = compact_insert();
    let bytes = insert.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 19 + 1 + 3 + 7);
    let parsed = CompactSpliceInsert::parse(&bytes).unwrap();
    assert_eq!(parsed, insert);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);

    let das = parsed.das.as_ref().unwrap();
    assert_eq!(das.break_num, 1);
    assert_eq!(das.breaks_expected, 2);
    assert_eq!(das.upid, b"urn:abc");
}

#[test]
fn compact_splice_insert_nibble_order_changes_the_das_type() {
    let bytes = compact_insert().try_to_bytes().unwrap();
    let table_8 = CompactSpliceInsert::parse_with_order(&bytes, DasNibbleOrder::TABLE_8).unwrap();
    let table_1 = CompactSpliceInsert::parse_with_order(&bytes, DasNibbleOrder::TABLE_1).unwrap();
    assert_eq!(
        table_8.das.unwrap().equivalent_segmentation_type,
        EquivalentSegmentationType::ProviderPlacementOpportunity
    );
    assert_ne!(
        table_1.das.unwrap().equivalent_segmentation_type,
        EquivalentSegmentationType::ProviderPlacementOpportunity
    );
}

#[test]
fn compact_splice_insert_rejects_a_short_fixed_prefix() {
    assert_eq!(
        CompactSpliceInsert::parse(&[0u8; 18]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 19,
                have: 18,
                what: "compact_splice_insert fixed fields",
            }
        ))
    );
}

#[test]
fn compact_splice_insert_rejects_a_das_length_below_three() {
    let mut bytes = compact_insert().try_to_bytes().unwrap();
    bytes[19] = 2;
    assert_eq!(
        CompactSpliceInsert::parse(&bytes),
        Err(Error::InvalidValue(
            broadcast_common::error::InvalidValue {
                field: "compact_splice_insert.descriptor_length",
                value: 2,
                reason: "DAS body length must be at least 3 (break_num, breaks_expected, type)",
            }
        ))
    );
}

#[test]
fn compact_splice_insert_rejects_a_das_length_past_the_body() {
    let mut bytes = compact_insert().try_to_bytes().unwrap();
    bytes[19] = 11;
    assert_eq!(
        CompactSpliceInsert::parse(&bytes),
        Err(Error::LengthOverflow {
            declared: 11,
            available: 10,
            what: "compact_splice_insert DAS body",
        })
    );
}

#[test]
fn compact_splice_insert_das_length_may_exactly_fill_the_body() {
    let bytes = compact_insert().try_to_bytes().unwrap();
    assert_eq!(bytes[19], 10);
    assert!(CompactSpliceInsert::parse(&bytes).is_ok());
}

#[test]
fn an_over_wide_reserved_row_is_a_typed_field_overflow() {
    let insert = CompactSpliceInsert {
        reserved: 0x7F,
        ..compact_insert()
    };
    let mut out = vec![0u8; insert.serialized_len()];
    assert_eq!(
        insert.serialize_into(&mut out),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "compact_splice_insert",
            value: 0x7F,
            max: 0x3F,
        }))
    );
}

#[test]
fn the_das_descriptor_rejects_a_body_shorter_than_its_fixed_fields() {
    #[rustfmt::skip]
    let bytes = [0xF0, 0x04, 0x44, 0x56, 0x42, 0x5F];
    assert_eq!(
        DvbDasDescriptor::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 9,
                have: 6,
                what: "DVB_DAS_descriptor body",
            }
        ))
    );
}

fn inline_section() -> Vec<u8> {
    SpliceInfoSection::new_clear(AnyCommand::from(SpliceNull), SpliceDescriptorLoop::new(&[]))
        .try_to_bytes()
        .unwrap()
}

#[test]
fn stream_event_payload_carousel_reference_round_trips() {
    let payload = StreamEventPayload {
        timeline_type: TimelineType::None,
        temi: None,
        reserved_dvb_data: &[],
        private_data: None,
        carriage: Scte35Carriage::CarouselObject(b"obj"),
    };
    #[rustfmt::skip]
    let expected: [u8; 7] = [
        0x01,
        0xF0,
        0x00,
        0x03, b'o', b'b', b'j',
    ];
    let bytes = payload.try_to_bytes().unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(StreamEventPayload::parse(&expected).unwrap(), payload);
}

#[test]
fn stream_event_payload_temi_and_private_data_round_trip() {
    let payload = StreamEventPayload {
        timeline_type: TimelineType::Temi,
        temi: Some((0x11, 0x22)),
        reserved_dvb_data: &[0xAA],
        private_data: Some(PrivateData {
            specifier: 0x4355_4549,
            bytes: &[1, 2, 3],
        }),
        carriage: Scte35Carriage::CarouselObject(b"obj"),
    };
    #[rustfmt::skip]
    let expected: [u8; 17] = [
        0x04,
        0xF2, 0x11, 0x22, 0xAA,
        0x07, 0x43, 0x55, 0x45, 0x49, 0x01, 0x02, 0x03,
        0x03, b'o', b'b', b'j',
    ];
    let bytes = payload.try_to_bytes().unwrap();
    assert_eq!(bytes, expected);
    let parsed = StreamEventPayload::parse(&expected).unwrap();
    assert_eq!(parsed, payload);
    assert_eq!(parsed.timeline_type, TimelineType::Temi);
    assert_eq!(parsed.temi, Some((0x11, 0x22)));
    assert_eq!(parsed.private_data.unwrap().specifier, 0x4355_4549);
}

#[test]
fn stream_event_payload_inline_section_round_trips() {
    let section = inline_section();
    let payload = StreamEventPayload {
        timeline_type: TimelineType::VideoPts,
        temi: None,
        reserved_dvb_data: &[0xB0, 0xB1],
        private_data: None,
        carriage: Scte35Carriage::Inline(SpliceInfoSection::parse(&section).unwrap()),
    };
    let bytes = payload.try_to_bytes().unwrap();
    assert_eq!(&bytes[..4], &[0x03, 0xE1, 0xB0, 0xB1]);
    assert_eq!(bytes[4], 0x00, "private_data_length");
    assert_eq!(&bytes[5..], &section[..]);
    let parsed = StreamEventPayload::parse(&bytes).unwrap();
    assert_eq!(parsed, payload);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn stream_event_payload_truncations_are_errors() {
    let section = inline_section();
    let payload = StreamEventPayload {
        timeline_type: TimelineType::Temi,
        temi: Some((1, 2)),
        reserved_dvb_data: &[],
        private_data: Some(PrivateData {
            specifier: 7,
            bytes: &[9],
        }),
        carriage: Scte35Carriage::Inline(SpliceInfoSection::parse(&section).unwrap()),
    };
    let bytes = payload.try_to_bytes().unwrap();
    for cut in 0..bytes.len() {
        assert!(
            StreamEventPayload::parse(&bytes[..cut]).is_err(),
            "prefix of {cut} bytes must not parse"
        );
    }
}

#[test]
fn stream_event_payload_declared_lengths_past_the_buffer_overflow() {
    // DVB_data_length 8 with only two bytes present.
    assert_eq!(
        StreamEventPayload::parse(&[8, 0xF0]),
        Err(Error::LengthOverflow {
            declared: 8,
            available: 1,
            what: "DSM-CC_stream_event_payload_binary DVB_data_length",
        })
    );

    // A TEMI timeline whose two fields are missing.
    assert_eq!(
        StreamEventPayload::parse(&[1, 0xE2]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 4,
                have: 2,
                what: "DSM-CC_stream_event_payload_binary temi fields",
            }
        ))
    );

    // A private_data_length below the 4-byte specifier.
    assert_eq!(
        StreamEventPayload::parse(&[1, 0xF0, 0x03, 0x00]),
        Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
            field: "DSM-CC_stream_event_payload_binary.private_data_length",
            value: 3,
            reason: "must be >= 4 (32-bit private_data_specifier) when present",
        }))
    );

    // A private_data_length past the buffer.
    assert_eq!(
        StreamEventPayload::parse(&[1, 0xF0, 0x06, 1, 2, 3]),
        Err(Error::LengthOverflow {
            declared: 6,
            available: 3,
            what: "DSM-CC_stream_event_payload_binary private_data",
        })
    );

    // A carousel name length past the buffer.
    assert_eq!(
        StreamEventPayload::parse(&[1, 0xF0, 0x00, 0x05, b'o']),
        Err(Error::LengthOverflow {
            declared: 5,
            available: 1,
            what: "DSM-CC_stream_event_payload_binary carousel_object_name",
        })
    );
}

#[test]
fn a_dvb_data_length_one_past_a_short_buffer_overflows() {
    // Declared 3, only 3 bytes present (need 4).
    assert_eq!(
        StreamEventPayload::parse(&[3, 0xF0, 0x00]),
        Err(Error::LengthOverflow {
            declared: 3,
            available: 2,
            what: "DSM-CC_stream_event_payload_binary DVB_data_length",
        })
    );
}

#[test]
fn a_temi_timeline_needs_four_bytes_even_with_a_long_dvb_data_region() {
    // Exactly four bytes: the TEMI fields are complete, the
    // private_data_length byte is what is missing.
    assert_eq!(
        StreamEventPayload::parse(&[3, 0xE2, 0x11, 0x22]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 5,
                have: 4,
                what: "DSM-CC_stream_event_payload_binary private_data_length",
            }
        ))
    );
}

#[test]
fn a_private_data_block_ending_exactly_at_the_buffer_needs_the_carriage() {
    // dvb_data_length 1, private_data_length 4 with four bytes, nothing after.
    assert_eq!(
        StreamEventPayload::parse(&[1, 0xF0, 0x04, 0xAA, 0xBB, 0xCC]),
        Err(Error::LengthOverflow {
            declared: 4,
            available: 3,
            what: "DSM-CC_stream_event_payload_binary private_data",
        })
    );
    let mut exact = vec![1u8, 0xF0, 0x04, 0xAA, 0xBB, 0xCC, 0xDD];
    assert_eq!(
        StreamEventPayload::parse(&exact),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 8,
                have: 7,
                what: "DSM-CC_stream_event_payload_binary carousel_object_name_length",
            }
        )),
        "the private block ends exactly at the buffer; the carriage byte is missing"
    );
    exact.push(0x00);
    assert!(StreamEventPayload::parse(&exact).is_ok(), "empty carousel name");
}

#[test]
fn a_missing_carousel_length_reports_the_absolute_need() {
    assert_eq!(
        StreamEventPayload::parse(&[1, 0xF0, 0x00]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 4,
                have: 3,
                what: "DSM-CC_stream_event_payload_binary carousel_object_name_length",
            }
        ))
    );
}

#[test]
fn a_four_byte_specifier_with_no_private_bytes_is_valid() {
    let payload = StreamEventPayload {
        timeline_type: TimelineType::None,
        temi: None,
        reserved_dvb_data: &[],
        private_data: Some(PrivateData {
            specifier: 0x4355_4549,
            bytes: &[],
        }),
        carriage: Scte35Carriage::CarouselObject(b""),
    };
    #[rustfmt::skip]
    let expected: [u8; 8] = [
        0x01, 0xF0,
        0x04, 0x43, 0x55, 0x45, 0x49,
        0x00,
    ];
    let bytes = payload.try_to_bytes().unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(StreamEventPayload::parse(&expected).unwrap(), payload);
}

#[test]
fn private_data_and_carousel_name_maxima_are_255() {
    let at_limit = StreamEventPayload {
        timeline_type: TimelineType::None,
        temi: None,
        reserved_dvb_data: &[],
        private_data: Some(PrivateData {
            specifier: 1,
            bytes: &[0xAA; 251],
        }),
        carriage: Scte35Carriage::CarouselObject(&[b'a'; 255]),
    };
    let bytes = at_limit.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 1 + 1 + 1 + 255 + 1 + 255);
    assert_eq!(StreamEventPayload::parse(&bytes).unwrap(), at_limit);

    let over_private = StreamEventPayload {
        private_data: Some(PrivateData {
            specifier: 1,
            bytes: &[0xAA; 252],
        }),
        ..at_limit.clone()
    };
    assert_eq!(
        over_private.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "DSM-CC_stream_event_payload_binary.private_data_length",
            value: 256,
            max: 255,
        }))
    );

    let over_name = StreamEventPayload {
        private_data: None,
        carriage: Scte35Carriage::CarouselObject(&[b'a'; 256]),
        ..at_limit
    };
    assert_eq!(
        over_name.try_to_bytes(),
        Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
            field: "DSM-CC_stream_event_payload_binary.carousel_object_name_length",
            value: 256,
            max: 255,
        }))
    );
}

#[test]
fn a_dvb_data_region_filling_the_buffer_needs_the_private_data_length() {
    // DVB_data_length 1 and exactly two bytes: the region ends at the buffer.
    assert_eq!(
        StreamEventPayload::parse(&[1, 0xF0]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 3,
                have: 2,
                what: "DSM-CC_stream_event_payload_binary private_data_length",
            }
        ))
    );
}
