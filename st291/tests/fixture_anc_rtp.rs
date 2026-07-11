//! Real-ish fixture test: parse the committed `fixtures/st291/anc_rtp.bin`
//! (an RFC 8331 ANC-over-RTP packet built from the same real, already-audited
//! ANC content bytes as `fixtures/st291/anc.bin` — see
//! `fixtures/st291/anc_rtp-PROVENANCE.md` — wrapped fresh in RFC-8331-correct
//! RTP + payload-header framing), assert the decoded fields against
//! independently-known expected values, and verify a byte-exact round-trip.

use std::fs;

use broadcast_common::Serialize;
use st291::{AncContent, AncRtpPayload, FieldSense, RtpAncPacket};

fn fixture() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/st291/anc_rtp.bin");
    fs::read(path).expect("fixture anc_rtp.bin must be committed")
}

#[test]
fn parses_expected_fields() {
    let data = fixture();
    let (rtp, anc) = AncRtpPayload::parse_rtp_packet(&data).unwrap();

    // RTP fixed header expectations.
    assert!(rtp.marker);
    assert_eq!(rtp.payload_type, 112);
    assert_eq!(rtp.sequence_number, 1);
    assert_eq!(rtp.timestamp, 90_000);
    assert_eq!(rtp.ssrc, 0x5354_3239);
    assert_eq!(rtp.csrc_count(), 0);
    assert!(rtp.extension.is_none());
    assert!(rtp.padding.is_none());

    // ANC RTP payload header expectations.
    assert_eq!(anc.extended_sequence_number, 0);
    assert_eq!(anc.field_sense, FieldSense::ProgressiveOrUnspecified);
    assert_eq!(anc.anc_count(), 2);

    let expected = AncRtpPayload {
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
                    data_count: 0x002,
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
                    data_count: 0x003,
                    user_data_words: vec![0x111, 0x222, 0x333],
                    checksum: 0x1AB,
                },
            },
        ],
    };
    assert_eq!(anc, expected);
}

#[test]
fn byte_exact_round_trip() {
    let data = fixture();
    let (rtp, anc) = AncRtpPayload::parse_rtp_packet(&data).unwrap();

    let mut rtp_out = vec![0u8; rtp.serialized_len()];
    let n = rtp.serialize_into(&mut rtp_out).unwrap();
    assert_eq!(n, data.len());
    assert_eq!(
        rtp_out, data,
        "RTP serialize must be byte-identical to the fixture"
    );

    let mut anc_out = vec![0u8; anc.serialized_len()];
    anc.serialize_into(&mut anc_out).unwrap();
    assert_eq!(
        anc_out, rtp.payload,
        "ANC payload serialize must be byte-identical to the RTP payload"
    );

    // And serialize -> parse -> equal.
    let (rtp2, anc2) = AncRtpPayload::parse_rtp_packet(&rtp_out).unwrap();
    assert_eq!(rtp2.payload_type, rtp.payload_type);
    assert_eq!(anc2, anc);
}
