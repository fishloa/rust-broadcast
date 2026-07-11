/// Build an RFC 8331 ANC-over-RTP packet from typed fields, serialize it, and
/// dump the wire bytes — the RTP fixed header (`rtp_packet::RtpPacket`, RFC
/// 3550) wrapping the RFC 8331 §2.1 ANC payload (`st291::AncRtpPayload`).
///
/// ```sh
/// cargo run -p st291 --example build_anc_rtp --features rtp
/// ```
use broadcast_common::Serialize;
use rtp_packet::RtpPacket;
use st291::{ANC_RTP_DEFAULT_CLOCK_RATE, AncContent, AncRtpPayload, FieldSense, RtpAncPacket};

fn main() {
    // The RFC 8331 §2.1 ANC payload: two ANC packets, on lines 9 and 10 (as
    // in RFC 8331 Figure 1's own worked example).
    let anc_payload = AncRtpPayload {
        extended_sequence_number: 0,
        field_sense: FieldSense::ProgressiveOrUnspecified,
        anc_packets: vec![
            RtpAncPacket {
                c: false,
                line_number: 9,
                horizontal_offset: 0,
                s: false,
                stream_num: 0,
                content: AncContent {
                    did: 0x161,
                    sdid: 0x101,
                    data_count: 0x002, // low 8 bits => 2 user_data_words
                    user_data_words: vec![0x2CF, 0x101],
                    checksum: 0x233,
                },
            },
            RtpAncPacket {
                c: true,
                line_number: 10,
                horizontal_offset: 0x10,
                s: false,
                stream_num: 0,
                content: AncContent {
                    did: 0x241,
                    sdid: 0x102,
                    data_count: 0x003, // 3 user_data_words
                    user_data_words: vec![0x111, 0x222, 0x333],
                    checksum: 0x1AB,
                },
            },
        ],
    };
    let mut anc_bytes = vec![0u8; anc_payload.serialized_len()];
    anc_payload.serialize_into(&mut anc_bytes).unwrap();

    // Wrap it in an RFC 3550 RTP packet: marker=true (last ANC RTP packet for
    // this frame), dynamic payload type 112 (RFC 8331 §4's own worked SDP
    // example: `a=rtpmap:112 smpte291/90000`).
    let rtp = RtpPacket {
        marker: true,
        payload_type: 112,
        sequence_number: 1,
        timestamp: ANC_RTP_DEFAULT_CLOCK_RATE, // 1.0 s at the default 90 kHz clock rate
        ssrc: 0xCAFE_BABE,
        csrc: vec![],
        extension: None,
        padding: None,
        payload: &anc_bytes,
    };
    let mut bytes = vec![0u8; rtp.serialized_len()];
    rtp.serialize_into(&mut bytes).unwrap();

    println!("ANC-over-RTP packet: {} bytes", bytes.len());
    println!("ANC_Count: {}", anc_payload.anc_count());
    for (i, pkt) in anc_payload.anc_packets.iter().enumerate() {
        println!(
            "  [{i}] line={} h_off={} S={} StreamNum={} DID={:#05X} SDID={:#05X} \
             data_count={:#05X} udw_loop={} checksum={:#05X}",
            pkt.line_number,
            pkt.horizontal_offset,
            pkt.s,
            pkt.stream_num,
            pkt.content.did,
            pkt.content.sdid,
            pkt.content.data_count,
            pkt.content.udw_loop_count(),
            pkt.content.checksum,
        );
    }
    print!("wire bytes:");
    for b in &bytes {
        print!(" {b:02X}");
    }
    println!();

    // Round-trip sanity via the RtpPacket + AncRtpPayload composition helper.
    let (parsed_rtp, parsed_anc) = AncRtpPayload::parse_rtp_packet(&bytes).unwrap();
    assert_eq!(parsed_rtp.payload_type, 112);
    assert_eq!(parsed_anc, anc_payload);
    println!("round-trip: OK");
}
