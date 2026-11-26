//! Region Composition (EN 300 743 Table 11), Page Composition (Table 9), CLUT
//! Definition (Table 16), Display Definition (Table 8) and PES data field
//! (Table 3) field positions and boundaries.

use broadcast_core::{Parse, Serialize};
use dvb_subtitle::{
    ClutDefinitionSegment, ClutEntry, DisplayDefinitionSegment, Error, ObjectProviderFlag,
    ObjectType, PageCompositionSegment, PageRegionEntry, PageState, PesDataField,
    RegionCompositionSegment, RegionDepth, RegionLevelOfCompatibility, RegionObjectEntry,
};

fn segment(segment_type: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0x0F, segment_type, 0x00, 0x01];
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(body);
    out
}

#[test]
fn region_composition_fields_and_objects() {
    let body = [
        0x05, // region_id
        0xAD, // version 0xA, fill flag, reserved 5
        0x01, 0xE0, // width 480
        0x02, 0xD0, // height 720
        0x69, // level 3, depth 2, reserved 1
        0x12, // clut_id
        0x55, // 8-bit pixel code
        0xA9, // 4-bit 0xA, 2-bit 2, reserved 1
        // Basic bitmap, ROM provider, position (4095, 2748)
        0x01, 0x23, 0x1F, 0xFF, 0x0A, 0xBC, //
        // Basic character, in-stream, position (291, 1110)
        0x04, 0x56, 0x41, 0x23, 0x04, 0x56, 0x55, 0xAA, //
        // Composite string, reserved provider 2, position (0, 1)
        0x07, 0x89, 0xA0, 0x00, 0x00, 0x01, 0x01, 0x02,
    ];
    let bytes = segment(0x11, &body);
    let seg = RegionCompositionSegment::parse(&bytes).unwrap();
    assert_eq!(seg.page_id, 1);
    assert_eq!(seg.region_id, 5);
    assert_eq!(seg.region_version_number, 0xA);
    assert!(seg.region_fill_flag);
    assert_eq!(seg.reserved_byte1, 5);
    assert_eq!(seg.region_width, 480);
    assert_eq!(seg.region_height, 720);
    assert_eq!(
        seg.region_level_of_compatibility,
        RegionLevelOfCompatibility::Clut8Bit
    );
    assert_eq!(seg.region_depth, RegionDepth::Depth4Bit);
    assert_eq!(seg.reserved_byte6, 1);
    assert_eq!(seg.clut_id, 0x12);
    assert_eq!(seg.region_8bit_pixel_code, 0x55);
    assert_eq!(seg.region_4bit_pixel_code, 0xA);
    assert_eq!(seg.region_2bit_pixel_code, 2);
    assert_eq!(seg.reserved_byte9, 1);
    assert_eq!(
        seg.objects,
        vec![
            RegionObjectEntry {
                object_id: 0x0123,
                object_type: ObjectType::BasicBitmap,
                object_provider_flag: ObjectProviderFlag::InRom,
                object_horizontal_position: 0xFFF,
                object_vertical_position: 0xABC,
                foreground_pixel_code: None,
                background_pixel_code: None,
            },
            RegionObjectEntry {
                object_id: 0x0456,
                object_type: ObjectType::BasicCharacter,
                object_provider_flag: ObjectProviderFlag::InStream,
                object_horizontal_position: 0x123,
                object_vertical_position: 0x456,
                foreground_pixel_code: Some(0x55),
                background_pixel_code: Some(0xAA),
            },
            RegionObjectEntry {
                object_id: 0x0789,
                object_type: ObjectType::CompositeString,
                object_provider_flag: ObjectProviderFlag::Reserved(2),
                object_horizontal_position: 0,
                object_vertical_position: 1,
                foreground_pixel_code: Some(0x01),
                background_pixel_code: Some(0x02),
            },
        ]
    );
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn region_composition_rejects_short_input() {
    assert!(RegionCompositionSegment::parse(&[0x0F]).is_err());
}

#[test]
fn page_composition_fields_and_regions() {
    let body = [
        0x07, // page_time_out
        0xBB, // version 0xB, state 2 (mode change), reserved 3
        0x01, 0x00, 0x00, 0x10, 0x00, 0x30, //
        0x02, 0xFF, 0x01, 0x00, 0x00, 0xF0, //
        0x03, 0x55, 0x02, 0x00, 0x01, 0xA0,
    ];
    let bytes = segment(0x10, &body);
    let seg = PageCompositionSegment::parse(&bytes).unwrap();
    assert_eq!(seg.page_time_out, 7);
    assert_eq!(seg.page_version_number, 0xB);
    assert_eq!(seg.page_state, PageState::ModeChange);
    assert_eq!(seg.reserved, 3);
    assert_eq!(
        seg.regions,
        vec![
            PageRegionEntry {
                region_id: 1,
                reserved: 0x00,
                region_horizontal_address: 0x0010,
                region_vertical_address: 0x0030,
            },
            PageRegionEntry {
                region_id: 2,
                reserved: 0xFF,
                region_horizontal_address: 0x0100,
                region_vertical_address: 0x00F0,
            },
            PageRegionEntry {
                region_id: 3,
                reserved: 0x55,
                region_horizontal_address: 0x0200,
                region_vertical_address: 0x01A0,
            },
        ]
    );
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
}

#[test]
fn page_composition_region_loop_remainder() {
    let mut body = vec![
        0x07, 0x10, 0x01, 0x00, 0x00, 0x10, 0x00, 0x30, 0x02, 0x00, 0x00, 0x20, 0x00, 0x40, 0x03,
        0x00, 0x00, 0x30, 0x00, 0x50,
    ];
    body.push(0xEE); // one leftover byte
    let bytes = segment(0x10, &body);
    match PageCompositionSegment::parse(&bytes) {
        Err(Error::BufferTooShort(e)) => {
            assert_eq!(e.need, 24);
            assert_eq!(e.have, 19);
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
    assert!(PageCompositionSegment::parse(&[0x0F]).is_err());
}

#[test]
fn clut_definition_full_and_reduced_entries() {
    let body = [
        0x04, // clut_id
        0xB5, // version 0xB, reserved 5
        0x01, 0xFF, 0x12, 0x34, 0x56, 0x78, // full-range entry
        0x02, 0x24, 0xAB, 0x16, // reduced-range entry
    ];
    let bytes = segment(0x12, &body);
    let seg = ClutDefinitionSegment::parse(&bytes).unwrap();
    assert_eq!(seg.clut_id, 4);
    assert_eq!(seg.clut_version_number, 0xB);
    assert_eq!(seg.reserved, 5);
    assert_eq!(
        seg.entries,
        vec![
            ClutEntry {
                clut_entry_id: 1,
                flag_2bit: true,
                flag_4bit: true,
                flag_8bit: true,
                reserved_flags: 0x1E,
                full_range_flag: true,
                y_value: 0x12,
                cr_value: 0x34,
                cb_value: 0x56,
                t_value: 0x78,
            },
            ClutEntry {
                clut_entry_id: 2,
                flag_2bit: false,
                flag_4bit: false,
                flag_8bit: true,
                reserved_flags: 0x04,
                full_range_flag: false,
                y_value: 0x2A,
                cr_value: 0x0C,
                cb_value: 0x05,
                t_value: 0x02,
            },
        ]
    );
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
    assert!(ClutDefinitionSegment::parse(&[0x0F]).is_err());

    // Zero entries is valid: only the two fixed body bytes are present.
    let empty_body = [0x04, 0xB5];
    let empty = segment(0x12, &empty_body);
    let parsed = ClutDefinitionSegment::parse(&empty).unwrap();
    assert!(parsed.entries.is_empty());
    assert_eq!(parsed.try_to_bytes().unwrap(), empty);
}

#[test]
fn display_definition_window_fields() {
    let body = [
        0xBB, // version 0xB, window flag, reserved 3
        0x02, 0xCF, // width 719
        0x01, 0xDF, // height 479
        0x00, 0x10, // horizontal minimum
        0x02, 0xCF, // horizontal maximum
        0x00, 0x05, // vertical minimum
        0x01, 0xDF, // vertical maximum
    ];
    let bytes = segment(0x14, &body);
    let seg = DisplayDefinitionSegment::parse(&bytes).unwrap();
    assert_eq!(seg.page_id, 1);
    assert_eq!(seg.dds_version_number, 0xB);
    assert!(seg.display_window_flag);
    assert_eq!(seg.reserved, 3);
    assert_eq!(seg.display_width, 719);
    assert_eq!(seg.display_height, 479);
    assert_eq!(seg.display_window_horizontal_position_minimum, Some(0x0010));
    assert_eq!(seg.display_window_horizontal_position_maximum, Some(0x02CF));
    assert_eq!(seg.display_window_vertical_position_minimum, Some(0x0005));
    assert_eq!(seg.display_window_vertical_position_maximum, Some(0x01DF));
    assert_eq!(seg.try_to_bytes().unwrap(), bytes);
    assert!(DisplayDefinitionSegment::parse(&[0x0F]).is_err());
}

#[test]
fn pes_data_field_header_and_boundaries() {
    // A complete segment whose length exactly reaches the end of the buffer.
    let exact = [0x20u8, 0x00, 0x0F, 0x80, 0x00, 0x01, 0x00, 0x00];
    let field = PesDataField::parse(&exact).unwrap();
    assert_eq!(field.segments.len(), 1);
    assert_eq!(field.try_to_bytes().unwrap(), exact);

    // With the end marker.
    let with_marker = [0x20u8, 0x00, 0x0F, 0x80, 0x00, 0x01, 0x00, 0x00, 0xFF];
    let field = PesDataField::parse(&with_marker).unwrap();
    assert_eq!(field.segments.len(), 1);
    assert_eq!(field.try_to_bytes().unwrap(), with_marker);

    // The data_identifier must be 0x20.
    assert!(matches!(
        PesDataField::parse(&[0x21, 0x00, 0xFF]),
        Err(Error::BadDataIdentifier(0x21))
    ));

    // A lone trailing byte must be the end marker.
    assert!(matches!(
        PesDataField::parse(&[0x20, 0x00, 0xFE]),
        Err(Error::BadEndOfPesMarker(0xFE))
    ));
}

#[test]
fn region_field_labels() {
    assert_eq!(RegionLevelOfCompatibility::Reserved0.name(), "reserved");
    assert_eq!(RegionLevelOfCompatibility::Clut2Bit.name(), "2-bit CLUT");
    assert_eq!(RegionLevelOfCompatibility::Clut4Bit.name(), "4-bit CLUT");
    assert_eq!(RegionLevelOfCompatibility::Clut8Bit.name(), "8-bit CLUT");
    assert_eq!(RegionLevelOfCompatibility::Reserved(4).name(), "reserved");
    assert_eq!(RegionDepth::Reserved0.name(), "reserved");
    assert_eq!(RegionDepth::Depth2Bit.name(), "2-bit");
    assert_eq!(RegionDepth::Depth4Bit.name(), "4-bit");
    assert_eq!(RegionDepth::Depth8Bit.name(), "8-bit");
    assert_eq!(RegionDepth::Reserved(5).name(), "reserved");
    assert_eq!(ObjectType::BasicBitmap.name(), "basic_bitmap");
    assert_eq!(ObjectType::BasicCharacter.name(), "basic_character");
    assert_eq!(ObjectType::CompositeString.name(), "composite_string");
    assert_eq!(ObjectType::Reserved(3).name(), "reserved");
    assert_eq!(ObjectProviderFlag::InStream.name(), "in_stream");
    assert_eq!(ObjectProviderFlag::InRom.name(), "in_rom");
    assert_eq!(ObjectProviderFlag::Reserved(2).name(), "reserved");
    assert_eq!(PageState::Normal.name(), "normal");
    assert_eq!(PageState::Acquisition.name(), "acquisition");
    assert_eq!(PageState::ModeChange.name(), "mode_change");
    assert_eq!(PageState::Reserved(3).name(), "reserved");
}

#[test]
fn segment_parsers_reject_short_buffers_without_panicking() {
    assert!(PageCompositionSegment::parse(&[0x0F, 0x10, 0x00, 0x01]).is_err());
    assert!(PageCompositionSegment::parse(&[0x0F, 0x10, 0x00, 0x01, 0x00, 0x02]).is_err());
    assert!(ClutDefinitionSegment::parse(&[0x0F, 0x12]).is_err());
    assert!(ClutDefinitionSegment::parse(&[0x0F, 0x12, 0x00, 0x01, 0x00]).is_err());
}
