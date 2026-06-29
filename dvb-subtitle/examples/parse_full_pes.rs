//! Advanced: parse a full PES data field with several segments from inline bytes.
//!
//! Run with: `cargo run -p dvb-subtitle --example parse_full_pes`

use broadcast_common::Parse;
use dvb_subtitle::{
    AnySegment, DataIdentifier, EndOfPesMarker, ObjectDataPayload, PesDataField, SyncByte,
};

fn main() {
    // A multi-segment PES data field: DDS, PCS, ODS, EDS.
    let bytes = [
        DataIdentifier, // 0x20
        0x00,           // subtitle_stream_id
        // Display definition segment (no window)
        SyncByte,
        0x14,
        0x00,
        0x01,
        0x00,
        0x05,
        0x30,
        0x02,
        0xCF,
        0x01,
        0x1F,
        // Page composition segment (one region, page state = acquisition)
        SyncByte,
        0x10,
        0x00,
        0x01,
        0x00,
        0x08,
        0x05, // page_time_out = 5 seconds
        0x04, // version=0, state=acquisition
        0x01,
        0x00, // region_id=1, reserved=0
        0x00,
        0x64,
        0x00,
        0x32, // region at (100, 50)
        // Object data segment (character string)
        SyncByte,
        0x13,
        0x00,
        0x01,
        0x00,
        0x08,
        0x00,
        0x0A, // object_id = 10
        0x04, // version=0, coding=characters
        0x02, // number_of_codes = 2
        0x00,
        0x41,
        0x00,
        0x42, // 'A', 'B'
        // End of display set
        SyncByte,
        0x80,
        0x00,
        0x01,
        0x00,
        0x00,
        EndOfPesMarker, // 0xFF
    ];

    let field = PesDataField::parse(&bytes).expect("valid PES data field");

    println!("{} segments found:\n", field.segments.len());
    for seg in &field.segments {
        match seg {
            AnySegment::DisplayDefinition(dds) => {
                println!(
                    "  Display Definition: {}x{}",
                    dds.display_width + 1,
                    dds.display_height + 1
                );
            }
            AnySegment::PageComposition(pcs) => {
                println!(
                    "  Page Composition: timeout={}s, state={}",
                    pcs.page_time_out, pcs.page_state
                );
                for r in &pcs.regions {
                    println!(
                        "    Region {} @ ({}, {})",
                        r.region_id, r.region_horizontal_address, r.region_vertical_address
                    );
                }
            }
            AnySegment::ObjectData(ods) => {
                println!(
                    "  Object Data: id={}, coding={}",
                    ods.object_id, ods.object_coding_method
                );
                if let ObjectDataPayload::Characters {
                    character_codes, ..
                } = &ods.payload
                {
                    for code in character_codes {
                        if let Ok(ch) = char::try_from(u32::from(*code)) {
                            print!("    '{}'", ch);
                        }
                    }
                    println!();
                }
            }
            AnySegment::EndOfDisplaySet(_) => {
                println!("  End of Display Set");
            }
            other => println!("  {}: (type=?)", other.name()),
        }
    }
}
