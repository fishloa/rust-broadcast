//! Build an RTP packet from typed fields (fixed header + CSRC list + header
//! extension) and serialize it to wire bytes — RFC 3550 §5.1 / §5.3.1.
//!
//! Run with `cargo run -p rtp-packet --example build_packet`.

use broadcast_common::Serialize;
use rtp_packet::{HeaderExtension, RtpPacket};

fn main() {
    let extension_data = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02];
    let pkt = RtpPacket {
        marker: true,
        payload_type: 96, // dynamic payload type, e.g. H.264 (RFC 6184)
        sequence_number: 42,
        timestamp: 90_000,
        ssrc: 0xCAFEBABE,
        csrc: vec![0x1111_1111, 0x2222_2222],
        extension: Some(HeaderExtension {
            profile_id: 0xBEDE, // example one-byte header extension profile id
            data: &extension_data,
        }),
        padding: None,
        payload: b"example RTP payload bytes",
    };

    let mut bytes = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut bytes).expect("serialize");

    println!("serialized {} bytes:", bytes.len());
    println!(
        "{}",
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "V={} P={} X={} CC={}",
        bytes[0] >> 6,
        (bytes[0] >> 5) & 1,
        (bytes[0] >> 4) & 1,
        bytes[0] & 0x0F
    );
}
