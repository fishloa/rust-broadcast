//! Basic: build a PAT section, serialize it to wire bytes, and parse it back.
//!
//! Run with: `cargo run -p dvb-si --example build_and_parse_pat`
//!
//! Shows the symmetric build/parse contract without needing a TS source —
//! every table type works the same way.

use broadcast_common::{Parse, Serialize};
use dvb_si::tables::pat::{PatEntry, PatSection};

fn main() {
    let pat = PatSection {
        transport_stream_id: 1,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        entries: vec![
            PatEntry {
                program_number: 1,
                pid: 0x0100,
            },
            PatEntry {
                program_number: 2,
                pid: 0x0200,
            },
        ],
    };

    // Serialize to the on-wire section bytes (CRC-32 appended automatically).
    let mut bytes = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut bytes).expect("buffer is sized");
    println!("serialized PAT : {} bytes", bytes.len());
    println!("{bytes:02X?}");

    // Parse them back and prove equality (the round-trip invariant).
    let parsed = PatSection::parse(&bytes).expect("valid PAT");
    assert_eq!(parsed, pat);

    println!("\ntransport_stream_id = {}", parsed.transport_stream_id);
    for e in &parsed.entries {
        println!("  program {} → PMT PID {:#06X}", e.program_number, e.pid);
    }
}
