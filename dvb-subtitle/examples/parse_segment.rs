//! Basic: parse a single subtitling segment from inline bytes.
//!
//! Run with: `cargo run -p dvb-subtitle --example parse_segment`

use dvb_common::Parse;
use dvb_subtitle::{AnySegment, DataIdentifier, EndOfPesMarker, PesDataField, SyncByte};

fn main() {
    // A complete PES data field with one display definition segment (no window).
    let bytes = [
        DataIdentifier, // 0x20
        0x00,           // subtitle_stream_id = 0x00
        // Display definition segment (segment_type 0x14, no window):
        SyncByte, // 0x0F
        0x14,     // segment_type = display_definition
        0x00,
        0x01, // page_id = 1
        0x00,
        0x05, // segment_length = 5
        0x30, // dds_version_number = 3, display_window_flag = 0
        0x02,
        0xCF, // display_width  = 720-1 = 719
        0x01,
        0x1F,           // display_height = 288-1 = 287
        EndOfPesMarker, // 0xFF
    ];

    let field = PesDataField::parse(&bytes).expect("valid PES data field");
    println!("Segments: {}", field.segments.len());

    match &field.segments[0] {
        AnySegment::DisplayDefinition(dds) => {
            println!("Display width      : {}", dds.display_width + 1);
            println!("Display height     : {}", dds.display_height + 1);
            println!("Version            : {}", dds.dds_version_number);
            println!("Window flag        : {}", dds.display_window_flag);
        }
        seg => println!("Unexpected segment: {}", seg.name()),
    }
}
