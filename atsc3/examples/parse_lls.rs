//! Parse an LLS envelope binary file (ATSC A/331).
//!
//! Usage:
//!   cargo run --example parse_lls -- <lls.bin>

use broadcast_common::Parse;
use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <lls.bin>", args[0]);
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

    match atsc3::lls::LlsEnvelope::parse(&bytes) {
        Ok(envelope) => {
            println!("LLS Envelope parsed ({} bytes)", bytes.len());
            println!("  Table ID: {:?}", envelope.table_id);
            println!("  Group ID: {}", envelope.group_id);
            println!("  Group count: {}", envelope.group_count());
            println!("  Table version: {}", envelope.table_version);
            println!("  Payload size: {} bytes", envelope.payload.len());
        }
        Err(e) => {
            eprintln!("Error parsing LLS envelope: {}", e);
            process::exit(1);
        }
    }
}
