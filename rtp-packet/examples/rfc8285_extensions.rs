//! Build an RTP packet carrying RFC 8285 one-byte-form multiplexed header-
//! extension elements, serialize it, then parse it back and decode the
//! extension elements — RFC 8285 §4.1.2 / §4.2.
//!
//! Run with `cargo run -p rtp-packet --example rfc8285_extensions --features rfc8285`.

use broadcast_common::{Parse, Serialize};
use rtp_packet::rfc8285::{
    ExtensionElements, OneByteElement, OneByteElements, OneByteId, parse_extensions,
};
use rtp_packet::{HeaderExtension, RtpPacket};

fn main() {
    // Two named extension elements: a 1-byte "audio level" (id=1) and a
    // 3-byte "MID" identifier (id=2) — the kind of thing WebRTC streams
    // multiplex via RFC 8285 in practice.
    let elements = OneByteElements(vec![
        OneByteElement {
            id: OneByteId::new(1).expect("1 is in the valid 1..=14 range"),
            data: &[0x2A], // audio level, one byte
        },
        OneByteElement {
            id: OneByteId::new(2).expect("2 is in the valid 1..=14 range"),
            data: b"mid", // 3-byte MID value
        },
    ]);

    // Serialize the elements to the padded byte sequence that becomes the
    // RFC 3550 §5.3.1 HeaderExtension's `data`.
    let mut ext_data = vec![0u8; elements.serialized_len()];
    elements
        .serialize_into(&mut ext_data)
        .expect("serialize extension elements");
    println!(
        "encoded {} extension-data bytes (word-aligned): {}",
        ext_data.len(),
        ext_data
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    let pkt = RtpPacket {
        marker: true,
        payload_type: 96,
        sequence_number: 1,
        timestamp: 3600,
        ssrc: 0x1234_5678,
        csrc: vec![],
        extension: Some(HeaderExtension {
            profile_id: rtp_packet::rfc8285::ONE_BYTE_PROFILE_ID,
            data: &ext_data,
        }),
        padding: None,
        payload: b"example payload",
    };

    let mut wire = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut wire).expect("serialize RTP packet");
    println!("serialized RTP packet: {} bytes", wire.len());

    // Parse it back and decode the RFC 8285 elements from the extension.
    let reparsed = RtpPacket::parse(&wire).expect("parse RTP packet");
    let ext = reparsed.extension.expect("X=1");
    let decoded = parse_extensions(&ext).expect("recognized RFC 8285 profile_id");
    let ExtensionElements::OneByte(one_byte) = decoded else {
        panic!("expected the one-byte form (profile_id 0xBEDE)");
    };

    println!("decoded {} extension elements:", one_byte.elements().len());
    for e in one_byte.elements() {
        println!("  id={:>2} data={:02x?}", e.id.get(), e.data);
    }

    // Re-serializing the decoded elements reproduces the exact original
    // extension data byte-for-byte, including the zero-padding tail.
    let mut re_ext_data = vec![0u8; one_byte.serialized_len()];
    one_byte
        .serialize_into(&mut re_ext_data)
        .expect("serialize decoded elements");
    assert_eq!(re_ext_data, ext_data);
    println!(
        "round trip byte-identical: OK ({} bytes)",
        re_ext_data.len()
    );
}
