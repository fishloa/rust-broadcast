//! Basic: build a PAT section, serialize it to wire bytes, and parse it back.
//!
//! Run with: `cargo run -p dvb-si --example build_and_parse_pat`
//!
//! Shows the symmetric build/parse contract without needing a TS source —
//! every table type works the same way.

use mpeg_ts::pid::Pid;
use broadcast_common::{Parse, Serialize};
use dvb_si::tables::pat::{PatEntry, PatSection};

fn main() {
    let pat = PatSection::new(
        1,
        0,
        true,
        0,
        0,
        vec![PatEntry::new(1, Pid::new(0x0100)), PatEntry::new(2, Pid::new(0x0200))],
    );

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
