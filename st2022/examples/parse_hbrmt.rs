//! Parse an ST 2022-6 HBRMT payload header.
//!
//! Usage:
//!   cargo run --example parse_hbrmt -- <header.bin>

use broadcast_common::bounded::{Limit, read_file_bounded};
use broadcast_common::{Parse, Serialize};
use std::env;
use std::path::Path;
use std::process;

/// One ST 2022-6 HBRMT payload header, fixed-size in the RTP payload.
const HEADER_LIMIT: Limit = Limit::new("HBRMT payload header", 64 * 1024);

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <header.bin>", args[0]);
        process::exit(1);
    }

    let file_path = &args[1];
    let bytes = match read_file_bounded(Path::new(file_path), HEADER_LIMIT) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error reading {}: {}", file_path, e);
            process::exit(1);
        }
    };

    match st2022::PayloadHeader::parse(&bytes) {
        Ok(header) => {
            println!("PayloadHeader parsed ({} bytes)", header.serialized_len());
            println!("  VSID: {}", header.vsid.name());
            println!("  Frame count: {}", header.fr_count);
            println!("  Timestamp ref: {}", header.timestamp_ref.name());
            println!("  Scrambling: {}", header.scrambling.name());
            println!("  FEC usage: {}", header.fec_usage.name());
            println!("  Clock frequency: {}", header.clock_frequency.name());
            println!("  Reserve: {}", header.reserve);
            if let Some(vsf) = header.video_source_format {
                println!("  Video source format:");
                println!("    Map: {}", vsf.map.name());
                println!("    Frame: {}", vsf.frame.name());
                println!("    Frame rate: {}", vsf.frate.name());
                println!("    Sample: {}", vsf.sample.name());
            }
            if let Some(ts) = header.video_timestamp {
                println!("  Video timestamp: {ts} (0x{:08x})", ts.ticks());
            }
        }
        Err(e) => {
            eprintln!("Error parsing header: {}", e);
            process::exit(1);
        }
    }
}
