//! Alternative CLUT (EN 300 743 §7.2.8, Tables 31–34) and disparity
//! signalling (§7.2.7, Tables 29–30) bit packing, offsets and ceilings.
//!
//! Bytes are hand-computed from the spec's bit layouts, so a shift/mask error
//! that is self-consistent between parse and serialize still fails.

use broadcast_common::{Parse, Serialize};
use dvb_subtitle::{
    AlternativeClutEntry, AlternativeClutSegment, ClutParameters, DisparityRegion,
    DisparityShiftInterval, DisparityShiftUpdateSequence, DisparitySignallingSegment, Subregion,
};

fn alt_segment(body: &[u8]) -> Vec<u8> {
    let mut out = vec![0x0F, 0x16, 0x00, 0x01];
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
    out
}

#[test]
fn alternative_clut_8bit_entries_round_trip_byte_exact() {
    let body = [
        0x05, // clut_id
        0x3C, // version 3, reserved 0xC
        0x91, // entry_max 2, colour_component 1, depth 0, reserved 1
        0x00, // dynamic range
        0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x01, 0x02, 0x03, 0x04,
    ];
    let bytes = alt_segment(&body);
    let seg = AlternativeClutSegment::parse(&bytes).unwrap();
    assert_eq!(seg.clut_id, 5);
    assert_eq!(seg.clut_version_number, 3);
    assert_eq!(seg.reserved, 0xC);
    assert_eq!(
        seg.clut_parameters,
        ClutParameters {
            entry_max_number: 2,
            colour_component_type: 1,
            output_bit_depth: 0,
            reserved: 1,
            dynamic_range_and_colour_gamut: 0,
        }
    );
    assert_eq!(
        seg.entries,
        vec![
            AlternativeClutEntry {
                luma_value: 0x12,
                chroma1_value: 0x34,
                chroma2_value: 0x56,
                t_value: 0x78,
            },
            AlternativeClutEntry {
                luma_value: 0x9A,
                chroma1_value: 0xBC,
                chroma2_value: 0xDE,
                t_value: 0xF0,
            },
            AlternativeClutEntry {
                luma_value: 0x01,
                chroma1_value: 0x02,
                chroma2_value: 0x03,
                t_value: 0x04,
            },
        ]
    );
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn alternative_clut_10bit_entries_pack_exactly() {
    let body = [
        0x07, // clut_id
        0xA5, // version 0xA, reserved 5
        0xE3, // entry_max 3, colour_component 2, depth 1 (10-bit), reserved 1
        0x02, // dynamic range
        // entry 1: Y=0x2AA Cb=0x155 Cr=0x3FF T=0x155
        0xAA, 0x95, 0x5F, 0xFD, 0x55, //
        // entry 2: Y=0x001 Cb=0x3FE Cr=0x002 T=0x2AA
        0x00, 0x7F, 0xE0, 0x0A, 0xAA, //
        // entry 3: Y=0x123 Cb=0x0AB Cr=0x210 T=0x001
        0x48, 0xCA, 0xB8, 0x40, 0x01,
    ];
    let bytes = alt_segment(&body);
    let seg = AlternativeClutSegment::parse(&bytes).unwrap();
    assert_eq!(seg.clut_parameters.output_bit_depth, 1);
    assert_eq!(seg.clut_parameters.entry_max_number, 3);
    assert_eq!(seg.clut_parameters.colour_component_type, 2);
    assert_eq!(seg.clut_parameters.reserved, 1);
    assert_eq!(
        seg.entries,
        vec![
            AlternativeClutEntry {
                luma_value: 0x2AA,
                chroma1_value: 0x155,
                chroma2_value: 0x3FF,
                t_value: 0x155,
            },
            AlternativeClutEntry {
                luma_value: 0x001,
                chroma1_value: 0x3FE,
                chroma2_value: 0x002,
                t_value: 0x2AA,
            },
            AlternativeClutEntry {
                luma_value: 0x123,
                chroma1_value: 0x0AB,
                chroma2_value: 0x210,
                t_value: 0x001,
            },
        ]
    );
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn alternative_clut_rejects_short_input_and_trailing_entry_bytes() {
    assert!(AlternativeClutSegment::parse(&[0x0F]).is_err());
    let mut body = [0u8; 6];
    body[2] = 0x10; // 8-bit depth
    let mut bytes = alt_segment(&body);
    bytes.push(0xAA); // one stray byte after whole 4-byte entries? none yet
    assert!(AlternativeClutSegment::parse(&bytes).is_err());
}

fn disparity_segment(body: &[u8]) -> Vec<u8> {
    let mut out = vec![0x0F, 0x15, 0x00, 0x01];
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn update_sequence(duration: u32, intervals: &[(u8, i8)]) -> Vec<u8> {
    let mut out = vec![(4 + intervals.len() * 2) as u8];
    out.push((duration >> 16) as u8);
    out.push((duration >> 8) as u8);
    out.push(duration as u8);
    out.push(intervals.len() as u8);
    for &(count, shift) in intervals {
        out.push(count);
        out.push(shift as u8);
    }
    out
}

#[test]
fn disparity_segment_round_trips_byte_exact() {
    let seq = update_sequence(0x0000_010203, &[(10, -1), (20, 1)]);
    let mut body = vec![
        0x53, // version 5, page update flag 0, reserved 3
        0xFE, // page default disparity -2
    ];
    // Region 7: 2 subregions with positions and an update sequence each.
    body.extend_from_slice(&[0x07, 0x85]); // flag 1, reserved 0x04, 2 subregions
    body.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]); // h, w
    body.extend_from_slice(&[0xFE, 0x35]); // shift -2, frac 3, reserved 5
    body.extend_from_slice(&seq);
    body.extend_from_slice(&[0x05, 0x06, 0x07, 0x08]); // h, w
    body.extend_from_slice(&[0x7F, 0xAB]); // shift 127, frac 0xA, reserved 0xB
    body.extend_from_slice(&seq);
    // Region 9: one subregion, no positions, no update sequence.
    body.extend_from_slice(&[0x09, 0x08]);
    body.extend_from_slice(&[0x80, 0xF1]); // shift -128, frac 0xF, reserved 1
    let bytes = disparity_segment(&body);

    let seg = DisparitySignallingSegment::parse(&bytes).unwrap();
    assert_eq!(seg.dss_version_number, 5);
    assert!(!seg.update_sequence_page_flag);
    assert_eq!(seg.reserved, 3);
    assert_eq!(seg.page_default_disparity_shift, -2);
    assert!(seg.page_update_sequence.is_none());
    assert_eq!(seg.regions.len(), 2);

    let r0 = &seg.regions[0];
    assert_eq!(r0.region_id, 7);
    assert!(r0.update_sequence_region_flag);
    assert_eq!(r0.reserved_flags, 0x04);
    assert_eq!(r0.subregions.len(), 2);
    assert_eq!(
        r0.subregions[0],
        Subregion {
            subregion_horizontal_position: Some(0x0102),
            subregion_width: Some(0x0304),
            subregion_disparity_shift_integer: -2,
            subregion_disparity_shift_fractional: 3,
            reserved: 5,
            update_sequence: Some(DisparityShiftUpdateSequence {
                interval_duration: 0x0000_010203,
                intervals: vec![
                    DisparityShiftInterval {
                        interval_count: 10,
                        disparity_shift_integer: -1,
                    },
                    DisparityShiftInterval {
                        interval_count: 20,
                        disparity_shift_integer: 1,
                    },
                ],
            }),
        }
    );
    assert_eq!(
        r0.subregions[1],
        Subregion {
            subregion_horizontal_position: Some(0x0506),
            subregion_width: Some(0x0708),
            subregion_disparity_shift_integer: 127,
            subregion_disparity_shift_fractional: 0xA,
            reserved: 0xB,
            update_sequence: Some(DisparityShiftUpdateSequence {
                interval_duration: 0x0000_010203,
                intervals: vec![
                    DisparityShiftInterval {
                        interval_count: 10,
                        disparity_shift_integer: -1,
                    },
                    DisparityShiftInterval {
                        interval_count: 20,
                        disparity_shift_integer: 1,
                    },
                ],
            }),
        }
    );
    let r1 = &seg.regions[1];
    assert_eq!(r1.region_id, 9);
    assert!(!r1.update_sequence_region_flag);
    assert_eq!(r1.reserved_flags, 0x08);
    assert_eq!(r1.subregions.len(), 1);
    assert_eq!(r1.subregions[0].subregion_horizontal_position, None);
    assert_eq!(r1.subregions[0].subregion_width, None);
    assert_eq!(r1.subregions[0].subregion_disparity_shift_integer, -128);
    assert_eq!(r1.subregions[0].subregion_disparity_shift_fractional, 0xF);
    assert_eq!(r1.subregions[0].reserved, 1);

    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn disparity_page_update_sequence_is_followed_by_the_regions() {
    let seq = update_sequence(0x0000_0102, &[(7, -3)]);
    let mut body = vec![
        0x58, // version 5, page update flag 1, reserved 0
        0x05, // default disparity 5
    ];
    body.extend_from_slice(&seq);
    body.extend_from_slice(&[0x03, 0x00]); // region 3, one subregion, flags 0
    body.extend_from_slice(&[0x02, 0x10]); // shift 2, frac 1, reserved 0
    let bytes = disparity_segment(&body);
    let seg = DisparitySignallingSegment::parse(&bytes).unwrap();
    assert!(seg.update_sequence_page_flag);
    let page = seg.page_update_sequence.as_ref().expect("page sequence");
    assert_eq!(page.interval_duration, 0x0000_0102);
    assert_eq!(page.intervals.len(), 1);
    assert_eq!(page.intervals[0].interval_count, 7);
    assert_eq!(page.intervals[0].disparity_shift_integer, -3);
    assert_eq!(seg.regions.len(), 1);
    assert_eq!(seg.regions[0].region_id, 3);
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn disparity_rejects_short_and_truncated_input() {
    // Shorter than the generic header plus the fixed body.
    assert!(DisparitySignallingSegment::parse(&[0x0F, 0x15]).is_err());
    // A declared update-sequence length longer than the data present.
    let mut body = vec![0x58, 0x00];
    body.extend_from_slice(&[0x0A, 0x00, 0x00, 0x00, 0x05, 1, 2, 3, 4, 5]);
    let bytes = disparity_segment(&body);
    assert!(DisparitySignallingSegment::parse(&bytes).is_err());
    // A single leftover byte after the regions is malformed.
    let mut body = vec![0x50, 0x00, 0x09, 0x00, 0x00, 0x00];
    body.push(0xEE);
    let bytes = disparity_segment(&body);
    assert!(DisparitySignallingSegment::parse(&bytes).is_err());
}

#[test]
fn disparity_serialize_limits_and_position_mismatch() {
    let subregion = Subregion {
        subregion_horizontal_position: None,
        subregion_width: None,
        subregion_disparity_shift_integer: 0,
        subregion_disparity_shift_fractional: 0,
        reserved: 0,
        update_sequence: None,
    };
    let base = DisparitySignallingSegment {
        page_id: 1,
        dss_version_number: 0,
        update_sequence_page_flag: false,
        reserved: 0,
        page_default_disparity_shift: 0,
        page_update_sequence: None,
        regions: vec![],
    };

    let mut four = base.clone();
    four.regions = vec![DisparityRegion {
        region_id: 1,
        update_sequence_region_flag: false,
        reserved_flags: 0,
        subregions: vec![
            Subregion {
                subregion_horizontal_position: Some(0x0010),
                subregion_width: Some(0x0020),
                ..subregion.clone()
            },
            Subregion {
                subregion_horizontal_position: Some(0x0030),
                subregion_width: Some(0x0040),
                ..subregion.clone()
            },
            Subregion {
                subregion_horizontal_position: Some(0x0050),
                subregion_width: Some(0x0060),
                ..subregion.clone()
            },
            Subregion {
                subregion_horizontal_position: Some(0x0070),
                subregion_width: Some(0x0080),
                ..subregion.clone()
            },
        ],
    }];
    let bytes = four.try_to_bytes().unwrap();
    assert_eq!(DisparitySignallingSegment::parse(&bytes).unwrap(), four);

    let mut empty = base.clone();
    empty.regions = vec![DisparityRegion {
        region_id: 1,
        update_sequence_region_flag: false,
        reserved_flags: 0,
        subregions: vec![],
    }];
    assert!(empty.try_to_bytes().is_err());

    let mut five = base.clone();
    five.regions = vec![DisparityRegion {
        region_id: 1,
        update_sequence_region_flag: false,
        reserved_flags: 0,
        subregions: vec![
            Subregion {
                subregion_horizontal_position: Some(0x0010),
                subregion_width: Some(0x0020),
                ..subregion.clone()
            };
            5
        ],
    }];
    assert!(five.try_to_bytes().is_err());

    // One stored coordinate without the other disagrees with the region's
    // has-positions flag.
    let mut mismatch = base;
    mismatch.regions = vec![DisparityRegion {
        region_id: 1,
        update_sequence_region_flag: false,
        reserved_flags: 0,
        subregions: vec![
            Subregion {
                subregion_horizontal_position: Some(0x0010),
                subregion_width: Some(0x0020),
                ..subregion.clone()
            },
            Subregion {
                subregion_horizontal_position: None,
                subregion_width: Some(0x0020),
                ..subregion.clone()
            },
        ],
    }];
    assert!(mismatch.try_to_bytes().is_err());
}
