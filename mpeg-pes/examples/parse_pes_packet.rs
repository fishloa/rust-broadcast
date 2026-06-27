//! Basic: parse one PES packet from raw bytes and read its stream_id + PTS.
//!
//! Run with: `cargo run -p mpeg-pes --example parse_pes_packet`

use mpeg_pes::{PesPacket, StreamId};

fn main() {
    // A minimal PES packet: start_code(00 00 01), stream_id 0xE0 (video),
    // PES_packet_length, flags, PES_header_data_length, a 5-byte PTS, then payload.
    let bytes = [
        0x00, 0x00, 0x01, 0xE0, // packet_start_code_prefix + stream_id
        0x00, 0x0A, // PES_packet_length = 10
        0x80, 0x80, 0x05, // '10' marker, PTS_DTS_flags=10, PES_header_data_length=5
        0x21, 0x00, 0x01, 0x00, 0x01, // PTS = 0 (33-bit, 90 kHz)
        0xAA, 0xBB, // elementary-stream payload
    ];

    let pkt = PesPacket::parse(&bytes).expect("valid PES packet");

    println!("stream_id      : {:#04X}", pkt.stream_id.0);
    println!("is_video       : {}", pkt.stream_id == StreamId(0xE0));
    println!("packet_length  : {}", pkt.pes_packet_length);

    let header = pkt
        .header
        .as_ref()
        .expect("video PES carries an optional header");
    match header.pts {
        Some(pts) => println!(
            "PTS            : {} ticks ({:.6}s)",
            pts.ticks(),
            pts.seconds()
        ),
        None => println!("PTS            : (none)"),
    }
    println!("payload        : {:02X?}", pkt.payload);
}
