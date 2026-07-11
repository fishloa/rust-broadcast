//! Parse the committed real-ish fixture (`fixtures/st291/anc_rtp.bin`) and
//! print its decoded RFC 8331 ANC-over-RTP fields, then round-trip it back to
//! bytes and confirm the output is byte-identical.
//!
//! ```sh
//! cargo run -p st291 --example parse_anc_rtp --features rtp
//! ```
use broadcast_common::Serialize;
use st291::AncRtpPayload;

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/st291/anc_rtp.bin");
    let bytes = std::fs::read(path).expect("fixture must exist");

    let (rtp, anc) = AncRtpPayload::parse_rtp_packet(&bytes).expect("parse ANC-over-RTP packet");

    println!("RTP marker:       {}", rtp.marker);
    println!("RTP payload_type: {}", rtp.payload_type);
    println!("RTP timestamp:    {}", rtp.timestamp);
    println!("RTP ssrc:         {:#010x}", rtp.ssrc);
    println!("Extended Sequence Number: {}", anc.extended_sequence_number);
    println!("F (field sense):  {}", anc.field_sense);
    println!("ANC_Count:        {}", anc.anc_count());
    for (i, pkt) in anc.anc_packets.iter().enumerate() {
        println!(
            "  [{i}] C={} line={} h_off={} S={} StreamNum={} DID={:#05X} SDID={:#05X} \
             checksum={:#05X}",
            pkt.c,
            pkt.line_number,
            pkt.horizontal_offset,
            pkt.s,
            pkt.stream_num,
            pkt.content.did,
            pkt.content.sdid,
            pkt.content.checksum,
        );
    }

    let mut out = vec![0u8; rtp.serialized_len()];
    rtp.serialize_into(&mut out).expect("serialize RTP packet");
    assert_eq!(out, bytes, "byte-identical RTP round trip");

    let mut anc_out = vec![0u8; anc.serialized_len()];
    anc.serialize_into(&mut anc_out)
        .expect("serialize ANC payload");
    assert_eq!(
        anc_out, rtp.payload,
        "byte-identical ANC payload round trip"
    );

    println!("round trip byte-identical: OK ({} bytes)", out.len());
}
