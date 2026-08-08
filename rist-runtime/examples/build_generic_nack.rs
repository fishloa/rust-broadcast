//! Build a Generic NACK (RFC 4585), serialize, parse back, and print.
//!
//! Usage: cargo run -p rist-runtime --example build_generic_nack

use broadcast_common::{Parse, Serialize};
use rist_runtime::{GenericNack, NackFci};

fn main() {
    let nack = GenericNack {
        ssrc_sender: 0x1111_2222,
        ssrc_media: 0x3333_4444,
        nacks: vec![
            NackFci {
                pid: 100,
                blp: 0x0000,
            },
            NackFci {
                pid: 200,
                blp: 0x0003,
            },
        ],
    };

    let bytes = nack.to_bytes();
    println!("Serialized {} bytes", bytes.len());
    println!("Wire: {:02X?}", &bytes);

    let parsed = GenericNack::parse(&bytes).expect("round-trip parse");
    assert_eq!(parsed, nack, "round-trip mismatch");

    println!("\nGeneric NACK:");
    println!("  SSRC sender: 0x{:08X}", parsed.ssrc_sender);
    println!("  SSRC media:  0x{:08X}", parsed.ssrc_media);
    for (i, fci) in parsed.nacks.iter().enumerate() {
        println!("  NACK[{i}]: PID={}, BLP=0x{:04X}", fci.pid, fci.blp);
    }
}
