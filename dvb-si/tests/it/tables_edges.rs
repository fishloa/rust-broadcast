//! Rarely-carried tables: exact-minimum and maximal sections, declared-loop
//! overruns, and the hand-written `DsmccSection` equality.
//!
//! Every section here is laid out from its syntax table with an independent
//! CRC-32, never through the production serializer.

use broadcast_common::{Parse, Serialize};
use dvb_si::error::Error;
use test_fixtures::psi::crc32_mpeg2;

const CRC_LEN: usize = 4;

/// A long-form section: `body` is everything after the 3-byte header and
/// before the CRC-32; byte 1 carries SSI=1, `private_indicator`=1 and the
/// `reserved(2)` fill.
fn section(table_id: u8, body: &[u8]) -> Vec<u8> {
    let section_length = body.len() + CRC_LEN;
    let mut section = vec![
        table_id,
        0xF0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
    ];
    section.extend_from_slice(body);
    section.extend_from_slice(&[0; CRC_LEN]);
    let end = section.len() - CRC_LEN;
    let crc = crc32_mpeg2(&section[..end]).to_be_bytes();
    section[end..].copy_from_slice(&crc);
    section
}

/// Declare the section one byte shorter than it is, leaving the buffer as is.
fn declare_one_short(section: &[u8]) -> Vec<u8> {
    let mut short = section.to_vec();
    short[2] -= 1;
    short
}

fn re_serialize<T: Serialize>(value: &T) -> Vec<u8>
where
    <T as Serialize>::Error: core::fmt::Debug,
{
    let mut out = vec![0u8; value.serialized_len()];
    value.serialize_into(&mut out).unwrap();
    out
}

// ---------------------------------------------------------------- DFIS (0x7C)

#[test]
fn downloadable_font_info_maximal_section_parses_and_re_serializes() {
    use dvb_si::tables::downloadable_font_info::{DownloadableFontInfoSection, FontInfo};
    let mut body = vec![0x12, 0x34, 0xF5, 0x07, 0x02];
    body.extend_from_slice(&[0x00, 0xE4]);
    body.extend_from_slice(&[0x01, 0x0A, 0x03, b'a', b'b', b'c']);
    body.extend_from_slice(&[0x02, 0x01, 0x20, 0x04, b'1', b'2', b'p', b't']);
    body.extend_from_slice(&[0x03, 0x05, b'S', b'e', b'r', b'i', b'f']);
    body.extend_from_slice(&[0x0A, 0x01, b'x']);
    let raw = section(0x7C, &body);

    let parsed = DownloadableFontInfoSection::parse(&raw).unwrap();
    assert_eq!(parsed.font_id_extension, 0x24);
    assert_eq!(parsed.font_id, 0x34);
    assert_eq!(parsed.version_number, 26);
    assert!(parsed.current_next_indicator);
    assert_eq!(parsed.section_number, 7);
    assert_eq!(parsed.last_section_number, 2);
    assert_eq!(parsed.font_info.len(), 5);
    assert!(matches!(
        parsed.font_info[0],
        FontInfo::StyleWeight {
            style: 7,
            weight: 2,
            ..
        }
    ));
    assert!(matches!(
        parsed.font_info[1],
        FontInfo::FileUri {
            format: 0x0A,
            uri: b"abc",
            ..
        }
    ));
    assert!(matches!(
        parsed.font_info[2],
        FontInfo::FontSize {
            size: 288,
            info: b"12pt"
        }
    ));
    assert!(matches!(
        parsed.font_info[3],
        FontInfo::LengthDelimited {
            font_info_type: 0x03,
            info: b"Serif"
        }
    ));
    assert!(matches!(
        parsed.font_info[4],
        FontInfo::LengthDelimited {
            font_info_type: 0x0A,
            info: b"x"
        }
    ));
    assert_eq!(re_serialize(&parsed), raw);
}

#[test]
fn downloadable_font_info_minimum_section_is_an_empty_loop() {
    use dvb_si::tables::downloadable_font_info::DownloadableFontInfoSection;
    let raw = section(0x7C, &[0x12, 0x34, 0xF5, 0x07, 0x02]);
    assert_eq!(raw.len(), 12);
    let parsed = DownloadableFontInfoSection::parse(&raw).unwrap();
    assert!(parsed.font_info.is_empty());
    assert_eq!(re_serialize(&parsed), raw);

    let one_short = declare_one_short(&raw);
    assert_eq!(
        DownloadableFontInfoSection::parse(&one_short).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 8,
            available: 9,
        }
    );
}

/// Each font-info arm rejects an entry that runs past the loop by exactly the
/// declared/available pair.
#[test]
fn downloadable_font_info_rejects_entries_past_the_loop_end() {
    use dvb_si::tables::downloadable_font_info::DownloadableFontInfoSection;
    let header = [0x12, 0x34, 0xF5, 0x07, 0x02];
    let cases: &[(&[u8], Error)] = &[
        (
            &[0x00],
            Error::SectionLengthOverflow {
                declared: 1,
                available: 0,
            },
        ),
        (
            &[0x01],
            Error::SectionLengthOverflow {
                declared: 2,
                available: 0,
            },
        ),
        (
            &[0x01, 0x00, 0x03, b'a'],
            Error::SectionLengthOverflow {
                declared: 3,
                available: 1,
            },
        ),
        (
            &[0x01, 0x00, 0x05, b'a', b'b', b'c', b'd'],
            Error::SectionLengthOverflow {
                declared: 5,
                available: 4,
            },
        ),
        (
            &[0x02, 0x01, 0x20],
            Error::SectionLengthOverflow {
                declared: 3,
                available: 2,
            },
        ),
        (
            &[0x02, 0x01, 0x20, 0x02, b'x'],
            Error::SectionLengthOverflow {
                declared: 2,
                available: 1,
            },
        ),
        (
            &[0x02, 0x01, 0x20, 0x05, b'x', b'y', b'z'],
            Error::SectionLengthOverflow {
                declared: 5,
                available: 3,
            },
        ),
        (
            &[0x03],
            Error::SectionLengthOverflow {
                declared: 1,
                available: 0,
            },
        ),
        (
            &[0x03, 0x05, b'x', b'y', b'z'],
            Error::SectionLengthOverflow {
                declared: 5,
                available: 3,
            },
        ),
        (
            &[0x03, 0x02, b'x'],
            Error::SectionLengthOverflow {
                declared: 2,
                available: 1,
            },
        ),
    ];
    for (entries, expected) in cases {
        let mut body = header.to_vec();
        body.extend_from_slice(entries);
        let raw = section(0x7C, &body);
        assert_eq!(
            DownloadableFontInfoSection::parse(&raw).unwrap_err(),
            *expected,
            "entries {entries:?}"
        );
    }
}

/// A font-info entry that ends exactly at the loop end is complete, not
/// truncated: the boundary is `>` against the loop end, not `>=`.
#[test]
fn downloadable_font_info_accepts_entries_that_end_at_the_loop_end() {
    use dvb_si::tables::downloadable_font_info::{DownloadableFontInfoSection, FontInfo};
    let header = [0x12, 0x34, 0xF5, 0x07, 0x02];
    let cases: &[(&[u8], fn(&FontInfo<'_>) -> bool)] = &[
        (&[0x01, 0x00, 0x00], |f| {
            matches!(f, FontInfo::FileUri { format: 0, uri: b"", .. })
        }),
        (&[0x01, 0x00, 0x01, b'a'], |f| {
            matches!(f, FontInfo::FileUri { format: 0, uri: b"a", .. })
        }),
        (&[0x02, 0x01, 0x20, 0x00], |f| {
            matches!(f, FontInfo::FontSize { size: 288, info: b"" })
        }),
        (&[0x03, 0x00], |f| {
            matches!(
                f,
                FontInfo::LengthDelimited {
                    font_info_type: 0x03,
                    info: b""
                }
            )
        }),
    ];
    for (entries, is_expected) in cases {
        let mut body = header.to_vec();
        body.extend_from_slice(entries);
        let raw = section(0x7C, &body);
        let parsed = DownloadableFontInfoSection::parse(&raw).unwrap();
        assert_eq!(parsed.font_info.len(), 1, "entries {entries:?}");
        assert!(is_expected(&parsed.font_info[0]), "entries {entries:?}");
        assert_eq!(re_serialize(&parsed), raw, "entries {entries:?}");
    }
}

/// The serializer, not just the parser, refuses an over-255-byte URI with the
/// declared/available pair rather than a wrapped length byte.
#[test]
fn downloadable_font_info_serializer_rejects_a_256_byte_uri() {
    use broadcast_common::reserved::ReservedZero;
    use dvb_si::tables::downloadable_font_info::{DownloadableFontInfoSection, FontInfo};
    let uri = [b'u'; 256];
    let section = DownloadableFontInfoSection::new(
        0,
        0,
        0,
        true,
        0,
        0,
        vec![FontInfo::FileUri {
            format: 0,
            uri: &uri,
            reserved_zero_future_use: ReservedZero::from_bits(0),
        }],
    );
    let mut buf = vec![0u8; section.serialized_len()];
    assert_eq!(
        section.serialize_into(&mut buf).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 256,
            available: 255,
        }
    );
}

// ------------------------------------------------------------------ EIT (0x4E)

const EIT_HEADER: [u8; 11] = [
    0x00, 0x01, 0xF5, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x00, 0x4E,
];

#[test]
fn eit_maximal_section_parses_and_re_serializes() {
    use dvb_si::tables::eit::{EitKind, EitSection, TABLE_ID_PF_ACTUAL};
    let mut body = EIT_HEADER.to_vec();
    body.extend_from_slice(&[
        0x12, 0x34, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0xB0, 0x02, 0x4D, 0x00,
    ]);
    let raw = section(TABLE_ID_PF_ACTUAL, &body);

    let parsed = EitSection::parse(&raw).unwrap();
    assert_eq!(parsed.kind, EitKind::PresentFollowingActual);
    assert_eq!(parsed.service_id, 1);
    assert_eq!(parsed.version_number, 26);
    assert!(parsed.current_next_indicator);
    assert_eq!(parsed.section_number, 0);
    assert_eq!(parsed.last_section_number, 0);
    assert_eq!(parsed.transport_stream_id, 0x1234);
    assert_eq!(parsed.original_network_id, 0x5678);
    assert_eq!(parsed.segment_last_section_number, 0);
    assert_eq!(parsed.last_table_id, TABLE_ID_PF_ACTUAL);
    assert_eq!(parsed.events.len(), 1);
    let event = &parsed.events[0];
    assert_eq!(event.event_id, 0x1234);
    assert_eq!(event.start_time_raw(), [0xE0, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(event.duration_raw(), [0x01, 0x02, 0x03]);
    assert!(event.free_ca_mode);
    assert_eq!(event.descriptors.raw(), &[0x4D, 0x00]);
    assert_eq!(re_serialize(&parsed), raw);
}

#[test]
fn eit_exact_minimum_section_has_no_events() {
    use dvb_si::tables::eit::{EitSection, TABLE_ID_PF_ACTUAL};
    let raw = section(TABLE_ID_PF_ACTUAL, &EIT_HEADER);
    assert_eq!(raw.len(), 18);
    let parsed = EitSection::parse(&raw).unwrap();
    assert!(parsed.events.is_empty());
    assert_eq!(re_serialize(&parsed), raw);

    let one_short = declare_one_short(&raw);
    assert_eq!(
        EitSection::parse(&one_short).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 14,
            available: 15,
        }
    );
}

#[test]
fn eit_rejects_trailing_bytes_that_cannot_form_an_event() {
    use dvb_si::tables::eit::{EitSection, TABLE_ID_PF_ACTUAL};
    let mut body = EIT_HEADER.to_vec();
    body.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
    let raw = section(TABLE_ID_PF_ACTUAL, &body);
    assert_eq!(
        EitSection::parse(&raw).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 5,
            have: 0,
            what: "EitSection trailing event bytes",
        })
    );
}

// ------------------------------------------------------------------ INT (0x4C)

const INT_FIXED: [u8; 9] = [0x01, 0x5A, 0xF5, 0x03, 0x04, 0x12, 0x34, 0x56, 0x07];

#[test]
fn int_maximal_section_parses_and_re_serializes() {
    use dvb_si::tables::int::{IntActionType, IntSection};
    let mut body = INT_FIXED.to_vec();
    body.extend_from_slice(&[0xF0, 0x04, 0x40, 0x02, 0xAA, 0xBB]);
    body.extend_from_slice(&[0xF0, 0x03, 0x11, 0x22, 0x33]);
    body.extend_from_slice(&[0xF0, 0x02, 0x44, 0x55]);
    body.extend_from_slice(&[0xF0, 0x01, 0x66]);
    body.extend_from_slice(&[0xF0, 0x03, 0x77, 0x88, 0x99]);
    let raw = section(0x4C, &body);

    let parsed = IntSection::parse(&raw).unwrap();
    assert_eq!(parsed.action_type, IntActionType::IpMacStreamLocation);
    assert_eq!(parsed.platform_id_hash, 0x5A);
    assert_eq!(parsed.version_number, 26);
    assert!(parsed.current_next_indicator);
    assert_eq!(parsed.section_number, 3);
    assert_eq!(parsed.last_section_number, 4);
    assert_eq!(parsed.platform_id, 0x0012_3456);
    assert_eq!(parsed.processing_order, 7);
    assert_eq!(parsed.platform_descriptors.raw(), &[0x40, 0x02, 0xAA, 0xBB]);
    assert_eq!(parsed.loops.len(), 2);
    assert_eq!(parsed.loops[0].target_descriptors.raw(), &[0x11, 0x22, 0x33]);
    assert_eq!(parsed.loops[0].operational_descriptors.raw(), &[0x44, 0x55]);
    assert_eq!(parsed.loops[1].target_descriptors.raw(), &[0x66]);
    assert_eq!(
        parsed.loops[1].operational_descriptors.raw(),
        &[0x77, 0x88, 0x99]
    );
    assert_eq!(re_serialize(&parsed), raw);
}

#[test]
fn int_exact_minimum_section_has_no_loops() {
    use dvb_si::tables::int::IntSection;
    let mut body = INT_FIXED.to_vec();
    body.extend_from_slice(&[0xF0, 0x00]);
    let raw = section(0x4C, &body);
    assert_eq!(raw.len(), 18);
    let parsed = IntSection::parse(&raw).unwrap();
    assert!(parsed.platform_descriptors.raw().is_empty());
    assert!(parsed.loops.is_empty());
    assert_eq!(re_serialize(&parsed), raw);

    let one_short = declare_one_short(&raw);
    assert_eq!(
        IntSection::parse(&one_short).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 14,
            available: 15,
        }
    );
}

#[test]
fn int_rejects_loop_lengths_that_run_past_the_payload() {
    use dvb_si::tables::int::IntSection;
    let mut platform = INT_FIXED.to_vec();
    platform.extend_from_slice(&[0xF0, 0x05, 0xAA]);
    assert_eq!(
        IntSection::parse(&section(0x4C, &platform)).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 5,
            available: 1,
        }
    );

    let mut target_field = INT_FIXED.to_vec();
    target_field.extend_from_slice(&[0xF0, 0x00, 0x00]);
    assert_eq!(
        IntSection::parse(&section(0x4C, &target_field)).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 16,
            have: 15,
            what: "IntSection target_descriptor_loop length",
        })
    );

    let mut target_len = INT_FIXED.to_vec();
    target_len.extend_from_slice(&[0xF0, 0x00, 0xF0, 0x05, 0xAA]);
    assert_eq!(
        IntSection::parse(&section(0x4C, &target_len)).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 5,
            available: 1,
        }
    );

    let mut op_field = INT_FIXED.to_vec();
    op_field.extend_from_slice(&[0xF0, 0x00, 0xF0, 0x00, 0x00]);
    assert_eq!(
        IntSection::parse(&section(0x4C, &op_field)).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 18,
            have: 17,
            what: "IntSection operational_descriptor_loop length",
        })
    );

    let mut op_len = INT_FIXED.to_vec();
    op_len.extend_from_slice(&[0xF0, 0x00, 0xF0, 0x05, 0xAA]);
    assert_eq!(
        IntSection::parse(&section(0x4C, &op_len)).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 5,
            available: 1,
        }
    );
}

// ------------------------------------------------------------------ CIT (0x77)

const CIT_FIXED: [u8; 10] = [0x00, 0x01, 0xF5, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x00];

#[test]
fn cit_maximal_section_parses_and_re_serializes() {
    use dvb_si::tables::cit::CitSection;
    let mut body = CIT_FIXED.to_vec();
    body[9] = 3;
    body.extend_from_slice(b"ab\0");
    body.extend_from_slice(&[0x00, 0x01, 0x00, 0x02, b'c', b'd']);
    body.extend_from_slice(&[0x00, 0x02, 0xFF, 0x01, b'e']);
    let raw = section(0x77, &body);

    let parsed = CitSection::parse(&raw).unwrap();
    assert_eq!(parsed.service_id, 1);
    assert_eq!(parsed.version_number, 26);
    assert!(parsed.current_next_indicator);
    assert_eq!(parsed.section_number, 0);
    assert_eq!(parsed.last_section_number, 0);
    assert_eq!(parsed.transport_stream_id, 0x1234);
    assert_eq!(parsed.original_network_id, 0x5678);
    assert_eq!(parsed.prepend_strings.raw(), b"ab\0");
    assert_eq!(parsed.crid_entries.len(), 2);
    assert_eq!(parsed.crid_entries[0].crid_ref, 1);
    assert_eq!(parsed.crid_entries[0].prepend_string_index, 0x00);
    assert_eq!(parsed.crid_entries[0].unique_string.raw(), b"cd");
    assert_eq!(parsed.crid_entries[1].crid_ref, 2);
    assert_eq!(parsed.crid_entries[1].prepend_string_index, 0xFF);
    assert_eq!(parsed.crid_entries[1].unique_string.raw(), b"e");
    assert_eq!(re_serialize(&parsed), raw);
}

#[test]
fn cit_minimum_section_and_a_crid_string_that_overruns() {
    use dvb_si::tables::cit::CitSection;
    let raw = section(0x77, &CIT_FIXED);
    assert_eq!(raw.len(), 17);
    let parsed = CitSection::parse(&raw).unwrap();
    assert!(parsed.prepend_strings.is_empty());
    assert!(parsed.crid_entries.is_empty());
    assert_eq!(re_serialize(&parsed), raw);

    let mut body = CIT_FIXED.to_vec();
    body.extend_from_slice(&[0x00, 0x01, 0x00, 0x03, b'x']);
    assert_eq!(
        CitSection::parse(&section(0x77, &body)).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 20,
            have: 18,
            what: "CitSection unique_string",
        })
    );
}

// ------------------------------------------------------------------ DIT (0x7E)

#[test]
fn dit_parses_its_single_body_byte() {
    use dvb_si::tables::dit::DitSection;
    let raw = [0x7Eu8, 0x00, 0x01, 0x80];
    let parsed = DitSection::parse(&raw).unwrap();
    assert!(parsed.transition_flag);
    assert_eq!(re_serialize(&parsed), raw);

    assert_eq!(
        DitSection::parse(&[0x7E, 0x00]).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 4,
            have: 2,
            what: "DitSection",
        })
    );
    assert_eq!(
        DitSection::parse(&[0x7E, 0x00, 0x01]).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 4,
            have: 3,
            what: "DitSection",
        })
    );
}

// -------------------------------------------------------------- Container (0x75)

const CONTAINER_FIXED: [u8; 5] = [0x12, 0x34, 0xF5, 0x00, 0x00];

#[test]
fn container_minimum_and_maximal_sections() {
    use dvb_si::tables::container::ContainerSection;
    let raw = section(0x75, &CONTAINER_FIXED);
    assert_eq!(raw.len(), 12);
    let parsed = ContainerSection::parse(&raw).unwrap();
    assert_eq!(parsed.container_id, 0x1234);
    assert_eq!(parsed.version_number, 26);
    assert!(parsed.current_next_indicator);
    assert!(parsed.container_data.is_empty());
    assert_eq!(re_serialize(&parsed), raw);

    let one_short = declare_one_short(&raw);
    assert_eq!(
        ContainerSection::parse(&one_short).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 8,
            available: 9,
        }
    );

    let mut body = CONTAINER_FIXED.to_vec();
    body.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let full = section(0x75, &body);
    let parsed = ContainerSection::parse(&full).unwrap();
    assert_eq!(parsed.container_data, &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(re_serialize(&parsed), full);
}

// ---------------------------------------------------------------- DSM-CC (0x3B)

const DSMCC_FIXED: [u8; 5] = [0x12, 0x34, 0xF5, 0x01, 0x02];

#[test]
fn dsmcc_minimum_and_maximal_sections() {
    use dvb_si::tables::dsmcc::DsmccSection;
    let raw = section(0x3B, &DSMCC_FIXED);
    assert_eq!(raw.len(), 12);
    let parsed = DsmccSection::parse(&raw).unwrap();
    assert_eq!(parsed.table_id, 0x3B);
    assert_eq!(parsed.extension_id, 0x1234);
    assert_eq!(parsed.version_number, 26);
    assert!(parsed.current_next_indicator);
    assert_eq!(parsed.section_number, 1);
    assert_eq!(parsed.last_section_number, 2);
    assert!(parsed.payload.is_empty());
    assert_eq!(re_serialize(&parsed), raw);

    let one_short = declare_one_short(&raw);
    assert_eq!(
        DsmccSection::parse(&one_short).unwrap_err(),
        Error::SectionLengthOverflow {
            declared: 8,
            available: 9,
        }
    );

    let mut body = DSMCC_FIXED.to_vec();
    body.extend_from_slice(&[0xCA, 0xFE]);
    let full = section(0x3F, &body);
    let parsed = DsmccSection::parse(&full).unwrap();
    assert_eq!(parsed.table_id, 0x3F);
    assert_eq!(parsed.payload, &[0xCA, 0xFE]);
    assert_eq!(re_serialize(&parsed), full);
}

/// Every compared field is part of `DsmccSection` equality; the checksum is
/// compared only for an `SSI == 0` (checksum-trailered) section.
#[test]
fn dsmcc_section_equality_compares_every_field() {
    use broadcast_common::reserved::ReservedOnes;
    use dvb_si::tables::dsmcc::DsmccSection;
    let base = || DsmccSection::new(0x3B, 0x1234, 5, true, 1, 2, &[0xAA, 0xBB]);
    assert_eq!(base(), base());

    let mut variants: Vec<DsmccSection<'_>> = Vec::new();
    let mut other = base();
    other.table_id = 0x3C;
    variants.push(other);
    let mut other = base();
    other.section_syntax_indicator = false;
    variants.push(other);
    let mut other = base();
    other.private_indicator = true;
    variants.push(other);
    let mut other = base();
    other.extension_id = 0x1235;
    variants.push(other);
    let mut other = base();
    other.version_number = 6;
    variants.push(other);
    let mut other = base();
    other.current_next_indicator = false;
    variants.push(other);
    let mut other = base();
    other.section_number = 2;
    variants.push(other);
    let mut other = base();
    other.last_section_number = 3;
    variants.push(other);
    let mut other = base();
    other.payload = &[0xAA, 0xBC];
    variants.push(other);
    let mut other = base();
    other.reserved_b1 = ReservedOnes::from_bits(0);
    variants.push(other);
    let mut other = base();
    other.reserved_b5 = ReservedOnes::from_bits(0);
    variants.push(other);
    for (index, variant) in variants.iter().enumerate() {
        assert_ne!(base(), *variant, "field variant {index} must differ");
    }

    let mut ignored = base();
    ignored.checksum = [1, 2, 3, 4];
    assert_eq!(base(), ignored, "SSI=1 ignores the checksum trailer");

    let mut checksummed = base();
    checksummed.section_syntax_indicator = false;
    let mut different = checksummed.clone();
    different.checksum = [0, 0, 0, 1];
    assert_eq!(checksummed, checksummed);
    assert_ne!(checksummed, different, "SSI=0 compares the checksum");
}

/// A table_id outside the DSM-CC range is rejected, never treated as one.
#[test]
fn dsmcc_rejects_a_table_id_outside_its_range() {
    use dvb_si::tables::dsmcc::DsmccSection;
    let raw = section(0x40, &DSMCC_FIXED);
    assert!(matches!(
        DsmccSection::parse(&raw),
        Err(Error::UnexpectedTableId { table_id: 0x40, .. })
    ));
}

/// `IntActionType`'s spec labels (`dvb-tools` prints them) and the lossless
/// `Display` for the reserved catch-all.
#[test]
fn int_action_type_labels_are_the_spec_tokens() {
    use dvb_si::tables::int::IntActionType;
    assert_eq!(IntActionType::Reserved.name(), "Reserved");
    assert_eq!(
        IntActionType::IpMacStreamLocation.name(),
        "IP/MAC Stream Location"
    );
    assert_eq!(IntActionType::DvbReserved(0x7F).name(), "DVB Reserved");
    assert_eq!(IntActionType::Reserved.to_string(), "Reserved");
    assert_eq!(
        IntActionType::IpMacStreamLocation.to_string(),
        "IP/MAC Stream Location"
    );
    assert_eq!(
        IntActionType::DvbReserved(0x7F).to_string(),
        "DVB Reserved(0x7F)"
    );
    assert_eq!(IntActionType::from_u8(0x01), IntActionType::IpMacStreamLocation);
    assert_eq!(IntActionType::from_u8(0x42).to_u8(), 0x42);
}

/// A 255-byte URI is still a valid 8-bit `uri_length`; one byte more is not
/// (`u8::MAX`, not `u8::MAX - 1`).
#[test]
fn downloadable_font_info_accepts_a_255_byte_uri() {
    use broadcast_common::reserved::ReservedZero;
    use dvb_si::tables::downloadable_font_info::{DownloadableFontInfoSection, FontInfo};
    let uri = [b'v'; 255];
    let section = DownloadableFontInfoSection::new(
        0,
        0,
        0,
        true,
        0,
        0,
        vec![FontInfo::FileUri {
            format: 0,
            uri: &uri,
            reserved_zero_future_use: ReservedZero::from_bits(0),
        }],
    );
    let mut buf = vec![0u8; section.serialized_len()];
    assert_eq!(section.serialize_into(&mut buf).unwrap(), buf.len());
    let parsed = DownloadableFontInfoSection::parse(&buf).unwrap();
    assert_eq!(parsed.font_info.len(), 1);
    assert!(matches!(
        parsed.font_info[0],
        FontInfo::FileUri {
            format: 0,
            uri,
            ..
        } if uri.len() == 255 && uri.iter().all(|b| *b == b'v')
    ));
    assert_eq!(re_serialize(&parsed), buf);
}

/// After the platform loop, a lone 2-byte target-loop length field is read
/// (the check is `>`, not `>=`) and then fails for want of the operational
/// length field: the boundary itself is not an overrun.
#[test]
fn int_reads_a_two_byte_loop_length_field_at_the_payload_end() {
    use dvb_si::tables::int::IntSection;
    let mut body = INT_FIXED.to_vec();
    body.extend_from_slice(&[0xF0, 0x00, 0x00, 0x00]);
    assert_eq!(
        IntSection::parse(&section(0x4C, &body)).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 18,
            have: 16,
            what: "IntSection operational_descriptor_loop length",
        })
    );
}
