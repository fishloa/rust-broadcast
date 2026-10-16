//! `transport_and_framing` — the two wire formats a caller meets before any
//! session logic: the typed `Transport` header (RFC 2326 §12.39) and
//! `$`-interleaved RTP/RTCP framing over TCP (§10.12).
//!
//! Parses a multi-spec Transport offer, shows the typed fields, serializes it
//! back, then demultiplexes a byte stream that arrives in arbitrary chunks —
//! the situation every TCP reader is in.
//!
//! ```text
//! cargo run -p rtsp-runtime --example transport_and_framing
//! ```

use rtsp_runtime::Transport;
use rtsp_runtime::interleaved::{InterleavedFrame, parse_frames};

fn main() {
    // A client offering UDP first and TCP interleaved as the fallback.
    let offer = "RTP/AVP;unicast;client_port=5000-5001,RTP/AVP/TCP;unicast;interleaved=0-1";
    let transport = Transport::parse(offer).expect("valid Transport header");
    println!("offer: {offer}");
    for (i, spec) in transport.specs.iter().enumerate() {
        println!(
            "  spec {i}: lower={:?} delivery={:?} client_port={:?} interleaved={:?}",
            spec.lower_transport.as_ref().map(|l| l.name()),
            spec.delivery.as_ref().map(|d| d.name()),
            spec.client_port,
            spec.interleaved
        );
    }
    let first = transport.first().expect("at least one spec");
    assert_eq!(first.client_port, Some((5000, 5001)));
    let back = transport.to_header_value().expect("serialize");
    println!("re-serialized: {back}");
    assert_eq!(Transport::parse(&back).expect("re-parse"), transport);

    // Three frames (RTP, RTCP, RTP) concatenated, then delivered in 7-byte
    // chunks: frames straddle chunk boundaries and must be reassembled.
    let frames = [
        InterleavedFrame::new(0, vec![0x80; 12]),
        InterleavedFrame::new(1, vec![0x81; 8]),
        InterleavedFrame::new(0, vec![0x80; 30]),
    ];
    let mut stream = Vec::new();
    for f in &frames {
        f.serialize_into(&mut stream).expect("frame");
    }
    println!("stream: {} bytes, delivered in 7-byte chunks", stream.len());

    let mut pending: Vec<u8> = Vec::new();
    let mut got: Vec<InterleavedFrame> = Vec::new();
    for chunk in stream.chunks(7) {
        pending.extend_from_slice(chunk);
        let (complete, remainder) = parse_frames(&pending).expect("framing intact");
        got.extend(complete);
        pending.drain(..pending.len() - remainder);
    }
    for f in &got {
        println!(
            "  channel {} ({}): {} payload bytes",
            f.channel,
            if f.channel % 2 == 0 { "RTP" } else { "RTCP" },
            f.payload.len()
        );
    }
    assert_eq!(got, frames);
    assert!(pending.is_empty());
}
