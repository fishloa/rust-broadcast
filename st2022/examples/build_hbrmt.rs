//! Build an ST 2022-6 payload header, serialize, parse back, verify.

use broadcast_common::{Parse, Serialize};
use st2022::{
    ClockFrequency, FecUsage, FrameRate, FrameStructure, MapStructure, PayloadHeader,
    SampleStructure, Scrambling, TimestampRef, VideoSourceFormat, VideoSourceId,
};

fn main() {
    let vsf = VideoSourceFormat {
        map: MapStructure::Direct,
        frame: FrameStructure::Hd1080p,
        frate: FrameRate::Hz60,
        sample: SampleStructure::Yuv422At10Bit,
        fmt_reserve: 0,
    };

    let header = PayloadHeader::new(
        VideoSourceId::Primary,
        42,
        TimestampRef::LockedUtc,
        Scrambling::NotScrambled,
        FecUsage::ColumnAndRow,
        ClockFrequency::Mhz148_5,
        0,
        Some(vsf),
        Some(0xDEADBEEF),
        None,
    )
    .expect("valid header");

    println!("Built PayloadHeader:");
    println!("  VSID: {}", header.vsid.name());
    println!("  FR count: {}", header.fr_count);
    println!("  Clock freq: {}", header.clock_frequency.name());
    println!("  Timestamp: 0x{:08x}", header.video_timestamp.unwrap());

    let bytes = header.to_bytes();
    println!("Serialized to {} bytes", bytes.len());

    match PayloadHeader::parse(&bytes) {
        Ok(reparsed) => {
            if reparsed == header {
                println!("✓ Round-trip successful: headers match");
            } else {
                println!("✗ Round-trip failed: headers differ");
            }
        }
        Err(e) => {
            println!("✗ Parse error: {}", e);
        }
    }
}
