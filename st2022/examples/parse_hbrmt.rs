//! Parse an ST 2022-6 HBRMT payload header.
//!
//! Usage:
//!   cargo run --example parse_hbrmt -- <header.bin>

use broadcast_common::{Parse, Serialize};
use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <header.bin>", args[0]);
        process::exit(1);
    }

    let file_path = &args[1];
    let bytes = match fs::read(file_path) {
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
                println!("  Video timestamp: 0x{:08x}", ts);
            }
        }
        Err(e) => {
            eprintln!("Error parsing header: {}", e);
            process::exit(1);
        }
    }
}
