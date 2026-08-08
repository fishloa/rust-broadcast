//! Parse an LLS envelope binary file (ATSC A/331).
//!
//! Usage:
//!   cargo run --example parse_lls -- <lls.bin>
//!
//! Run against the real captured fixture (issue #926/#943 — see
//! `fixtures/atsc3/PROVENANCE.md`):
//!   cargo run -p atsc3 --example parse_lls -- fixtures/atsc3/slt-lls-2019-01-07.bin

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
