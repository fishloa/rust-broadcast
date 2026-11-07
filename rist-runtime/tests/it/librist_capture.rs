//! The librist 0.2.18 Simple Profile capture
//! (`private/fixtures/rist/rist-simple-profile-librist.bin`, see its
//! PROVENANCE.md): 1 200 length-prefixed RTP datagrams, MPEG-2 TS payload type
//! 33, no RTCP (so no NACK or RTT classification applies). An independent
//! producer, so the media framing is checked against something other than this
//! workspace.

use broadcast_common::{Parse, Serialize};
use rtp_packet::RtpPacket;

const FIXTURE: &str = "rist/rist-simple-profile-librist.bin";
const DATAGRAMS: usize = 1_200;
const TS_PAYLOAD_LEN: usize = 7 * 188;
const TS_SYNC_BYTE: u8 = 0x47;
const PT_MP2T: u8 = 33;
const LEN_PREFIX: usize = 4;

fn datagrams(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + LEN_PREFIX].try_into().unwrap()) as usize;
        at += LEN_PREFIX;
        out.push(&bytes[at..at + len]);
        at += len;
    }
    out
}

#[test]
fn every_librist_datagram_round_trips_as_rtp() {
    let Some(path) = test_fixtures::require(FIXTURE, "librist_capture") else {
        return;
    };
    let file = std::fs::read(path).unwrap();
    let all = datagrams(&file);
    assert_eq!(all.len(), DATAGRAMS);

    let mut prev_seq: Option<u16> = None;
    for (i, datagram) in all.iter().enumerate() {
        let pkt = RtpPacket::parse(datagram).unwrap_or_else(|e| panic!("datagram {i}: {e}"));
        assert_eq!(pkt.payload_type, PT_MP2T, "datagram {i}");
        assert_eq!(pkt.payload.len(), TS_PAYLOAD_LEN, "datagram {i}");
        assert_eq!(pkt.payload[0], TS_SYNC_BYTE, "datagram {i}");
        if let Some(prev) = prev_seq {
            assert_eq!(pkt.sequence_number, prev.wrapping_add(1), "datagram {i}");
        }
        prev_seq = Some(pkt.sequence_number);
        assert_eq!(
            pkt.try_to_bytes().unwrap().as_slice(),
            *datagram,
            "datagram {i} must re-serialize byte-exactly"
        );
    }
}
