//! Drives `arq::Receiver` with a real loss pattern taken from the genuine
//! librist capture (`fixtures/rist/rist-simple-loss25pct-loopback.pcap`,
//! issue #741) and asserts the engine's own, independently-computed NACK
//! output matches the real RTCP bytes librist actually put on the wire —
//! the "must bite against the real capture" requirement for the ARQ engine
//! (as opposed to `tests/rist_fixture_range_nack_pcap.rs`, which only
//! exercises the wire *codec*, not the engine that decides when to build
//! one).
//!
//! # Why this test does not replay the fixture's own arrival timeline
//!
//! Frame 15's Range-Based Retransmission Request (`RR + SDES + APP`, byte
//! traced in `fixtures/rist/PROVENANCE.md`) requests exactly seq
//! 1349/1538/1542. Per `PROVENANCE.md`'s own methodology, this fixture is a
//! 400 ms slice taken from the *middle* of a much longer librist session
//! (`editcap -A/-B`); verified below (and independently, with `tshark`,
//! during this test's development) the window's own RTP traffic on port
//! 3234 starts at seq **1576** — i.e. all three losses this NACK reports
//! were detected *before* the window even begins. There is no way to
//! independently re-derive *when* librist detected them from data this
//! fixture contains; replaying only the window's own arrivals would not
//! reproduce frame 15's NACK by causal simulation, and pretending otherwise
//! would be exactly the kind of fabricated-provenance claim this crate's
//! docs have already had to correct once (see `docs/tr-06-1-simple-profile.md`
//! Appendix B's correction note).
//!
//! What *is* independently verifiable from the real bytes: each of the
//! three requested ranges has `additional = 0` — i.e. librist's own NACK
//! encodes each as an isolated single-packet loss, not a burst. So this
//! test reproduces that exact, verified loss *shape* (three isolated
//! single-packet gaps at those exact sequence numbers, everything else in
//! the local neighbourhood received) as input to our own engine, and checks
//! the engine's own coalesced NACK output is field-for-field equal (typed
//! equality, not a wire-byte comparison) to the `RangeNack` decoded from
//! what librist really sent — a genuine independent computation compared
//! against genuine wire bytes, honestly scoped to what the fixture can
//! actually support.

use std::time::Duration;

use broadcast_common::Parse;
use rist_runtime::arq::{ArqConfig, Receiver};
use rist_runtime::{RangeNack, RistReceiverCompound};

/// `DLT_NULL` classic-pcap walker, deliberately re-implemented (not shared
/// via a test-support module) so this test independently re-verifies
/// against the committed fixture file rather than trusting a copy of a hex
/// literal — mirrors `tests/rist_fixture_range_nack_pcap.rs`'s walker.
struct UdpPacket<'a> {
    frame: usize,
    src_port: u16,
    dst_port: u16,
    payload: &'a [u8],
}

fn udp_packets(data: &[u8]) -> Vec<UdpPacket<'_>> {
    const PCAP_MAGIC_LE: u32 = 0xa1b2_c3d4;
    const AF_INET_BSD: u32 = 2;
    const UDP_PROTOCOL: u8 = 17;

    let mut out = Vec::new();
    assert_eq!(
        u32::from_le_bytes(data[0..4].try_into().unwrap()),
        PCAP_MAGIC_LE
    );
    let mut off = 24usize;
    let mut frame = 0usize;
    while off + 16 <= data.len() {
        frame += 1;
        let caplen = u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap()) as usize;
        let rec_start = off + 16;
        let rec = &data[rec_start..rec_start + caplen];
        off = rec_start + caplen;
        if rec.len() < 4 {
            continue;
        }
        if u32::from_le_bytes(rec[0..4].try_into().unwrap()) != AF_INET_BSD {
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
        out.push(UdpPacket {
            frame,
            src_port: u16::from_be_bytes([udp[0], udp[1]]),
            dst_port: u16::from_be_bytes([udp[2], udp[3]]),
            payload: &udp[8..],
        });
    }
    out
}

fn fixture_path() -> String {
    format!(
        "{}/../fixtures/rist/rist-simple-loss25pct-loopback.pcap",
        env!("CARGO_MANIFEST_DIR")
    )
}

const RTCP_PORT: u16 = 3235;
const RTP_PORT: u16 = 3234;
const SSRC_ORIGINAL: u32 = 0xd561_5604;

/// Fetch the real, verified [`RangeNack`] carried in the fixture's frame 15
/// — parsed fresh from the committed file, not a copy of a hex literal.
fn real_frame15_range_nack(data: &[u8]) -> RangeNack {
    let packets = udp_packets(data);
    let frame15 = packets
        .iter()
        .find(|p| p.frame == 15)
        .expect("frame 15 present in fixture");
    assert_eq!(frame15.src_port, RTCP_PORT);
    let compound = RistReceiverCompound::parse(frame15.payload)
        .expect("parse the real RR+SDES+RangeNack compound packet");
    assert_eq!(compound.rr.ssrc, SSRC_ORIGINAL);
    assert_eq!(compound.range_nacks.len(), 1);
    compound.range_nacks[0].clone()
}

/// Independently confirms (re-derived from the real capture, not assumed)
/// that the window's own original-flow RTP traffic starts well past all
/// three of frame 15's requested sequence numbers — i.e. those losses
/// predate this 400 ms window, per `PROVENANCE.md`.
fn earliest_original_flow_seq_in_window(data: &[u8]) -> u16 {
    const SSRC_ORIGINAL_BYTES_OFFSET: usize = 8;
    let packets = udp_packets(data);
    packets
        .iter()
        .filter(|p| p.src_port == RTP_PORT || p.dst_port == RTP_PORT)
        .filter_map(|p| {
            if p.payload.len() < 12 {
                return None;
            }
            let ssrc = u32::from_be_bytes([
                p.payload[SSRC_ORIGINAL_BYTES_OFFSET],
                p.payload[SSRC_ORIGINAL_BYTES_OFFSET + 1],
                p.payload[SSRC_ORIGINAL_BYTES_OFFSET + 2],
                p.payload[SSRC_ORIGINAL_BYTES_OFFSET + 3],
            ]);
            (ssrc == SSRC_ORIGINAL).then(|| u16::from_be_bytes([p.payload[2], p.payload[3]]))
        })
        .min()
        .expect("at least one original-flow RTP packet in the window")
}

#[test]
fn frame15_losses_predate_the_fixture_window() {
    let data = std::fs::read(fixture_path()).expect("read fixture");
    let earliest = earliest_original_flow_seq_in_window(&data);
    let real = real_frame15_range_nack(&data);
    for range in &real.ranges {
        assert!(
            rist_runtime::arq::seq::seq_lt(range.start, earliest),
            "expected frame 15's requested seq {} to precede the window's own \
             earliest original-flow seq {earliest} (confirming it predates the \
             window, per PROVENANCE.md)",
            range.start
        );
    }
}

/// The bite: feed `arq::Receiver` the verified isolated-loss shape (three
/// single-packet gaps at the real capture's exact sequence numbers) and
/// assert its own coalesced NACK output is byte-identical, as a
/// [`RangeNack`], to what librist genuinely sent.
#[test]
fn engine_reproduces_frame15_range_nack_from_the_verified_loss_shape() {
    let data = std::fs::read(fixture_path()).expect("read fixture");
    let real = real_frame15_range_nack(&data);

    // Every requested range in the real capture is a single isolated
    // packet — confirms the loss shape this test reproduces is not an
    // assumption but a direct reading of the real wire bytes.
    for range in &real.ranges {
        assert_eq!(
            range.additional, 0,
            "expected every real frame-15 range entry to be a single-packet \
             loss (additional=0) per this test's premise"
        );
    }
    let lost: Vec<u16> = real.ranges.iter().map(|r| r.start).collect();
    assert_eq!(lost, vec![1349, 1538, 1542]);

    let mut cfg = ArqConfig::default();
    cfg.reorder_section = Duration::ZERO; // isolate NACK-content from timing
    let mut receiver = Receiver::new(cfg);

    // Feed every sequence number in the local neighbourhood as received,
    // except the three genuinely-lost ones — reproducing the verified loss
    // shape without inventing any additional gaps.
    for s in 1348u16..=1543 {
        if lost.contains(&s) {
            continue;
        }
        receiver.feed(s, Duration::ZERO);
    }
    assert_eq!(receiver.missing_count(), 3);

    receiver.tick(Duration::ZERO); // promotes all three (reorder_section=0)
    let due = receiver.tick(cfg.fallback_retransmission_interval()).due;

    let engine_nack = RangeNack {
        ssrc_media: SSRC_ORIGINAL,
        ranges: due,
    };
    assert_eq!(
        engine_nack, real,
        "engine-generated RangeNack must be field-for-field equal to librist's \
         real frame-15 payload (typed equality — this test compares decoded \
         `RangeNack` values, not wire bytes; the wire-level byte-exact \
         round-trip lives in tests/rist_fixture_range_nack_pcap.rs)"
    );
}
