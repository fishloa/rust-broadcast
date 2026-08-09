//! Real-fixture test — parses the genuine RIST ARQ traffic captured in
//! `fixtures/rist/rist-simple-loss25pct-loopback.pcap` (issue #741) with
//! `rist_runtime`'s own wire types, and asserts the documented NACK ->
//! retransmission correlation byte-traced in `fixtures/rist/PROVENANCE.md`.
//!
//! The capture is a genuine `librist` (v0.2.20) RIST Simple Profile
//! sender/receiver session running with librist's built-in 25% loss
//! simulator, captured on macOS loopback (`lo0`, `DLT_NULL` link type — a
//! 4-byte BSD address-family prefix instead of Ethernet framing). Unlike
//! `spec_vectors.rs` (hand-transcribed TR-06-1 bytes), this walks real wire
//! bytes a real ARQ implementation put on the wire.
//!
//! Per `PROVENANCE.md`, this build of librist's Simple Profile receiver only
//! emits Range-Based Retransmission Requests (RTCP APP, PT 204, subtype 0) —
//! zero Generic/bitmask NACKs (PT 205) appear anywhere in the capture. So
//! this file exercises [`RangeNack`] (via [`RistReceiverCompound`]), not
//! [`GenericNack`] — asserting bitmask NACKs are present here would be false.
//!
//! The classic-pcap walker is written by hand rather than adding a `pcap`
//! dependency, following `webrtc-runtime/tests/whip_smoke_pcap_stun.rs`'s
//! existing precedent for this exact `DLT_NULL` capture shape.

use broadcast_common::Parse;
use rist_runtime::{PacketRange, RangeNack, RistReceiverCompound, RistSenderCompound};

/// libpcap (classic, not pcapng) global header magic for little-endian,
/// microsecond-resolution captures — the format this fixture was written in.
const PCAP_MAGIC_LE: u32 = 0xa1b2_c3d4;

/// `DLT_NULL` / "BSD loopback" link type: each packet record is prefixed
/// with a 4-byte address-family word (host byte order) instead of an
/// Ethernet header, because the capture was taken on `lo0`.
const DLT_NULL: u32 = 0;

/// The `sa_family_t` value for `AF_INET` on macOS/BSD, as written by the
/// capturing host into the `DLT_NULL` 4-byte prefix.
const AF_INET_BSD: u32 = 2;

const UDP_PROTOCOL: u8 = 17;

/// RIST Simple Profile RTP data port used by this capture (`P`, TR-06-1
/// §5.1.1's unicast port-pairing rule).
const RTP_PORT: u16 = 3234;
/// RIST Simple Profile RTCP feedback port used by this capture (`P+1`).
const RTCP_PORT: u16 = 3235;

/// SSRC of the original (non-retransmitted) RTP flow (LSB=0, TR-06-1
/// §5.3.3).
const SSRC_ORIGINAL: u32 = 0xd561_5604;
/// SSRC of the retransmission flow (same 31 upper bits, LSB=1).
const SSRC_RETRANSMIT: u32 = 0xd561_5605;

fn fixture_path() -> String {
    format!(
        "{}/../fixtures/rist/rist-simple-loss25pct-loopback.pcap",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// One UDP datagram extracted from the capture, in on-wire order.
struct UdpPacket<'a> {
    /// 1-based frame number, matching `tshark`'s `frame.number` (i.e. every
    /// pcap record increments this, whether or not it turns out to be a UDP
    /// datagram we keep) — required to hand-verify against `PROVENANCE.md`,
    /// which cites exact frame numbers (15, 21, 22, 23).
    frame: usize,
    src_port: u16,
    dst_port: u16,
    payload: &'a [u8],
}

/// Minimal classic-pcap walker: yields every `DLT_NULL` / `AF_INET` / UDP
/// datagram in the file, tagged with its 1-based frame number.
fn udp_packets(data: &[u8]) -> Vec<UdpPacket<'_>> {
    assert!(data.len() >= 24, "pcap file too short for global header");
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    assert_eq!(
        magic, PCAP_MAGIC_LE,
        "not a little-endian classic pcap file"
    );
    let linktype = u32::from_le_bytes(data[20..24].try_into().unwrap());
    assert_eq!(linktype, DLT_NULL, "fixture must be a DLT_NULL/lo0 capture");

    let mut out = Vec::new();
    let mut off = 24usize;
    let mut frame = 0usize;
    while off + 16 <= data.len() {
        frame += 1;
        let caplen = u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap()) as usize;
        let rec_start = off + 16;
        assert!(rec_start + caplen <= data.len(), "truncated packet record");
        let rec = &data[rec_start..rec_start + caplen];
        off = rec_start + caplen;

        if rec.len() < 4 {
            continue;
        }
        let family = u32::from_le_bytes(rec[0..4].try_into().unwrap());
        if family != AF_INET_BSD {
            continue;
        }
        let ip = &rec[4..];
        if ip.len() < 20 {
            continue;
        }
        let ihl = ((ip[0] & 0x0F) as usize) * 4;
        if ip.len() < ihl + 8 || ip[9] != UDP_PROTOCOL {
            continue;
        }
        let udp = &ip[ihl..];
        if udp.len() < 8 {
            continue;
        }
        let src_port = u16::from_be_bytes([udp[0], udp[1]]);
        let dst_port = u16::from_be_bytes([udp[2], udp[3]]);
        out.push(UdpPacket {
            frame,
            src_port,
            dst_port,
            payload: &udp[8..],
        });
    }
    out
}

/// The documented frame-15 Range-Based Retransmission Request, byte-traced
/// in `PROVENANCE.md`: `RR + SDES(CNAME) + APP(RangeNack)`, 72 bytes total,
/// requesting seq 1349/1538/1542 (each a single-packet request,
/// `additional=0`).
const FRAME_15_HEX: &str = "80c90001d561560481ca0009d5615604011d416c65782d53747564696f2d4d32403132372e302e302e313a333233350080cc0005d561560452495354054500000602000006060000";

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Parses the exact frame-15 NACK->retransmission pair documented in
/// `PROVENANCE.md`: a genuine Range-Based Retransmission Request naming
/// three lost sequence numbers, immediately followed (frames 21/22/23) by
/// retransmitted RTP packets carrying exactly those sequence numbers on the
/// SSRC-LSB-flipped retransmission flow.
#[test]
fn frame15_range_nack_matches_documented_retransmission_pair() {
    let data = std::fs::read(fixture_path()).expect("read rist-simple-loss25pct-loopback.pcap");
    let packets = udp_packets(&data);

    let frame15 = packets
        .iter()
        .find(|p| p.frame == 15)
        .expect("frame 15 present in fixture");
    assert_eq!(frame15.src_port, RTCP_PORT);
    assert_eq!(
        frame15.payload,
        hex_decode(FRAME_15_HEX).as_slice(),
        "frame 15's raw bytes must match PROVENANCE.md's documented hex dump exactly"
    );

    // Parse the real compound RTCP packet with the crate's own production
    // parser (RR + SDES(CNAME) + RangeNack), not a hand-rolled sub-decode.
    let compound = RistReceiverCompound::parse(frame15.payload)
        .expect("parse the real RR+SDES+RangeNack compound packet");
    assert_eq!(compound.rr.ssrc, SSRC_ORIGINAL);
    assert_eq!(
        compound.nacks.len(),
        0,
        "no Generic (bitmask) NACKs in this fixture"
    );
    assert_eq!(compound.range_nacks.len(), 1);

    let range_nack: &RangeNack = &compound.range_nacks[0];
    assert_eq!(range_nack.ssrc_media, SSRC_ORIGINAL);
    assert_eq!(
        range_nack.ranges,
        vec![
            PacketRange {
                start: 1349,
                additional: 0
            },
            PacketRange {
                start: 1538,
                additional: 0
            },
            PacketRange {
                start: 1542,
                additional: 0
            },
        ],
        "frame 15 must request exactly seq 1349, 1538, 1542 (each a single-packet request)"
    );

    // The very next RTP-data packets on the retransmission SSRC are frames
    // 21/22/23, carrying exactly the requested sequence numbers in the
    // requested order (PROVENANCE.md's worked example).
    let retransmit_frames: Vec<(usize, u16, u32)> = packets
        .iter()
        .filter(|p| p.frame >= 21 && p.frame <= 23 && p.dst_port == RTP_PORT)
        .map(|p| {
            let seq = u16::from_be_bytes([p.payload[2], p.payload[3]]);
            let ssrc =
                u32::from_be_bytes([p.payload[8], p.payload[9], p.payload[10], p.payload[11]]);
            (p.frame, seq, ssrc)
        })
        .collect();
    assert_eq!(
        retransmit_frames,
        vec![
            (21, 1349, SSRC_RETRANSMIT),
            (22, 1538, SSRC_RETRANSMIT),
            (23, 1542, SSRC_RETRANSMIT),
        ],
        "frames 21/22/23 must retransmit exactly the requested sequence numbers, in order, \
         on the SSRC-LSB-flipped retransmission flow"
    );
}

/// Whole-fixture cross-check against every count `PROVENANCE.md` documents:
/// walks every RTCP compound packet in the capture (dispatching to
/// [`RistSenderCompound`] or [`RistReceiverCompound`] by RTCP packet type,
/// exactly as a real RIST peer would) and every RTP data packet, and adds up
/// to the exact totals the fixture's own provenance record claims. This is
/// the "must bite" test: it fails the instant the real capture's structure
/// disagrees with any documented count, byte-count, or SSRC split.
#[test]
fn fixture_totals_match_documented_provenance() {
    let data = std::fs::read(fixture_path()).expect("read rist-simple-loss25pct-loopback.pcap");
    let packets = udp_packets(&data);
    assert_eq!(packets.len(), 685, "total UDP packet count");

    let mut rtp_original = 0usize;
    let mut rtp_retransmit = 0usize;
    let mut range_nack_total = 0usize;
    let mut rtt_req_total = 0usize;
    let mut rtt_resp_total = 0usize;
    let mut sr_compounds = 0usize;
    let mut rr_compounds = 0usize;

    // RTCP common-header `PT` byte (RFC 3550 §6.1) identifies which compound
    // shape a given RIST peer sent: PT=200 (SR) is the sender's compound,
    // PT=201 (RR) is the receiver's.
    const PT_SR: u8 = 200;
    const PT_RR: u8 = 201;

    for pkt in &packets {
        if pkt.dst_port == RTCP_PORT || pkt.src_port == RTCP_PORT {
            assert!(pkt.payload.len() >= 2, "RTCP payload too short");
            match pkt.payload[1] {
                PT_SR => {
                    sr_compounds += 1;
                    let compound = RistSenderCompound::parse(pkt.payload)
                        .unwrap_or_else(|e| panic!("frame {}: parse SR compound: {e}", pkt.frame));
                    if compound.rtt_echo.is_some() {
                        // This build's SR compounds all carry an RTT Echo
                        // Request (subtype 2) per PROVENANCE.md.
                        rtt_req_total += 1;
                    }
                }
                PT_RR => {
                    rr_compounds += 1;
                    let compound = RistReceiverCompound::parse(pkt.payload)
                        .unwrap_or_else(|e| panic!("frame {}: parse RR compound: {e}", pkt.frame));
                    range_nack_total += compound.range_nacks.len();
                    if let Some(echo) = compound.rtt_echo {
                        match echo.kind {
                            rist_runtime::RttEchoKind::Request => rtt_req_total += 1,
                            rist_runtime::RttEchoKind::Response => rtt_resp_total += 1,
                            other => {
                                panic!("frame {}: unexpected RttEchoKind {other:?}", pkt.frame)
                            }
                        }
                    }
                    // Every range NACK observed in this fixture is against
                    // the original flow's SSRC.
                    for rn in &compound.range_nacks {
                        assert_eq!(rn.ssrc_media, SSRC_ORIGINAL);
                    }
                }
                other => panic!("frame {}: unexpected RTCP PT {other}", pkt.frame),
            }
        } else if pkt.dst_port == RTP_PORT || pkt.src_port == RTP_PORT {
            assert!(pkt.payload.len() >= 12, "RTP payload too short");
            let ssrc = u32::from_be_bytes([
                pkt.payload[8],
                pkt.payload[9],
                pkt.payload[10],
                pkt.payload[11],
            ]);
            match ssrc {
                SSRC_ORIGINAL => rtp_original += 1,
                SSRC_RETRANSMIT => rtp_retransmit += 1,
                other => panic!("frame {}: unexpected RTP SSRC 0x{other:08X}", pkt.frame),
            }
        }
    }

    assert_eq!(sr_compounds, 3, "Sender Report compound count");
    assert_eq!(rr_compounds, 65, "Receiver Report compound count");
    assert_eq!(
        range_nack_total, 55,
        "Range-Based Retransmission Request count"
    );
    assert_eq!(rtt_req_total, 7, "RTT Echo Request count");
    assert_eq!(rtt_resp_total, 6, "RTT Echo Response count");
    assert_eq!(rtp_original, 458, "original-flow RTP packet count");
    assert_eq!(rtp_retransmit, 159, "retransmission-flow RTP packet count");
}

/// Bite-proof: corrupting the RIST APP name field of the real frame-15
/// packet (in memory — the committed fixture file is never touched) must
/// break parsing, proving `RangeNack`'s validation is load-bearing against
/// genuine wire bytes and not vacuously true.
#[test]
fn corrupting_frame15_app_name_is_rejected() {
    let data = std::fs::read(fixture_path()).expect("read fixture");
    let packets = udp_packets(&data);
    let frame15 = packets.iter().find(|p| p.frame == 15).unwrap();

    // Sanity: the pristine bytes parse.
    RistReceiverCompound::parse(frame15.payload).expect("pristine frame 15 parses");

    // The "RIST" APP name starts at byte offset 8 of the APP sub-packet,
    // which itself starts at offset 48 of the compound payload (RR:8 +
    // SDES:40) -- verified structurally by the previous test's byte-exact
    // hex match. Flip one byte of it.
    let mut corrupted = frame15.payload.to_vec();
    let app_name_offset = 48 + 8;
    assert_eq!(&corrupted[app_name_offset..app_name_offset + 4], b"RIST");
    corrupted[app_name_offset] ^= 0xFF;

    let err = RistReceiverCompound::parse(&corrupted)
        .expect_err("corrupted RIST APP name must be rejected");
    assert!(
        matches!(err, rist_runtime::Error::InvalidAppName(_)),
        "expected InvalidAppName, got {err:?}"
    );
}
