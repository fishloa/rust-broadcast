//! Build a Header Partition Pack, serialize it, then parse it back and walk
//! its Operational Pattern / Essence Container inventory — SMPTE
//! ST 377-1:2019 §7.1-§7.2.
//!
//! Run with `cargo run -p st377-1 --example parse_partition`.

use broadcast_common::{Parse, Serialize};
use st377_1::{PartitionKind, PartitionPack, PartitionStatus};

fn main() {
    let pack = PartitionPack {
        kind: PartitionKind::Header,
        status: PartitionStatus::ClosedComplete,
        major_version: 1,
        minor_version: 3,
        kag_size: 512,
        this_partition: 0,
        previous_partition: 0,
        footer_partition: 65536,
        header_byte_count: 2048,
        index_byte_count: 0,
        index_sid: 0,
        body_offset: 0,
        body_sid: 1,
        // A placeholder Operational Pattern UL (see §8) — a real encoder
        // would use one of the registered OP1a/OP-Atom/etc. values.
        operational_pattern: [
            0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x01,
            0x01, 0x00,
        ],
        essence_containers: vec![[0x11; 16]],
    };

    let bytes = pack.to_bytes();
    println!("serialized {} bytes:", bytes.len());
    println!(
        "{}",
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    let parsed = PartitionPack::parse(&bytes).expect("parse Partition Pack");
    assert_eq!(parsed, pack);

    println!("kind:   {}", parsed.kind);
    println!("status: {}", parsed.status);
    println!(
        "KAG size: {} bytes, footer at byte offset {}",
        parsed.kag_size, parsed.footer_partition
    );
    println!(
        "{} Essence Container UL(s) referenced",
        parsed.essence_containers.len()
    );
}
