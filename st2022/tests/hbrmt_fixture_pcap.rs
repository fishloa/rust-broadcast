//! Real-fixture test — parses the genuine ST 2022-6 HBRMT/RTP/UDP capture in
//! `fixtures/st2022/st2022-6-hbrmt-1080i5994-single-frame-loopback.pcap`
//! (issue #926/#943) with `st2022::PayloadHeader`, and byte-exact
//! round-trips every payload header in the capture. Until now every
//! `st2022` test used hand-invented bytes — this is the crate's first
//! real-fixture test.
//!
//! The capture is a genuine `cisco/herisson` (`ip2vf`) HBRMT sender, one
//! full 1080i59.94 frame (4,497 RTP packets), captured on Linux loopback
//! (`lo`, `EN10MB`/Ethernet link type with zeroed MACs — unlike the RIST
//! fixture's macOS `DLT_NULL` capture). See `fixtures/st2022/PROVENANCE.md`
//! for the byte-level hand-decode this test's expected values come from,
//! independently re-derived here against the crate's own parser.
//!
//! This is also the fixture `st2022/docs/st2022-6-framing.md` asked a real
//! capture to confirm or refute: whether `RESERVE` (`PayloadHeader::reserve`)
//! is genuinely 5 bits wide, since that width was derived by arithmetic
//! from the surrounding fields rather than read verbatim off spec prose. See
//! `reserve_field_width_is_confirmed_not_contradicted_by_real_capture` below.

use broadcast_common::{Parse, Serialize};
use st2022::{
    ClockFrequency, FecUsage, FrameStructure, MapStructure, PayloadHeader, SampleStructure,
    Scrambling, TimestampRef, VideoSourceFormat, VideoSourceId,
};

/// libpcap (classic, not pcapng) global header magic for little-endian,
/// microsecond-resolution captures.
const PCAP_MAGIC_LE: u32 = 0xa1b2_c3d4;

/// `DLT_EN10MB` (Ethernet) link type — this capture was taken on Linux
/// `lo`, which (unlike macOS `lo0`) frames loopback traffic as Ethernet
/// with zeroed source/destination MAC addresses.
const DLT_EN10MB: u32 = 1;

const ETHERNET_HEADER_LEN: usize = 14;
const UDP_PROTOCOL: u8 = 17;

/// RTP fixed header length (RFC 3550 §5.1), no CSRC/extension in this
/// capture (`CC=0`, `X=0`).
const RTP_HEADER_LEN: usize = 12;

/// Expected byte length of the HBRMT payload preceding the video/embedded
/// data, for every non-final-of-frame packet -- and, per this capture's own
/// packetizer behaviour (PROVENANCE.md), for the final packet too (which is
/// padded up to this same fixed size rather than emitting a runt datagram).
const HBRMT_MEDIA_PAYLOAD_LEN: usize = 1376;

fn fixture_path() -> String {
    format!(
        "{}/../fixtures/st2022/st2022-6-hbrmt-1080i5994-single-frame-loopback.pcap",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Minimal classic-pcap walker: yields the UDP payload of every
/// `DLT_EN10MB`/IPv4/UDP packet in the file, in on-wire order. Written by
/// hand rather than adding a `pcap` dependency, following
/// `webrtc-runtime/tests/whip_smoke_pcap_stun.rs`'s existing precedent (that
/// one walks `DLT_NULL`; this one walks Ethernet framing instead).
fn udp_payloads(data: &[u8]) -> Vec<&[u8]> {
    assert!(data.len() >= 24, "pcap file too short for global header");
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    assert_eq!(
        magic, PCAP_MAGIC_LE,
        "not a little-endian classic pcap file"
    );
    let linktype = u32::from_le_bytes(data[20..24].try_into().unwrap());
    assert_eq!(
        linktype, DLT_EN10MB,
        "fixture must be an EN10MB/Ethernet capture"
    );

    let mut out = Vec::new();
    let mut off = 24usize;
    while off + 16 <= data.len() {
        let caplen = u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap()) as usize;
        let rec_start = off + 16;
        assert!(rec_start + caplen <= data.len(), "truncated packet record");
        let rec = &data[rec_start..rec_start + caplen];
        off = rec_start + caplen;

        if rec.len() < ETHERNET_HEADER_LEN {
            continue;
        }
        let ethertype = u16::from_be_bytes([rec[12], rec[13]]);
        if ethertype != 0x0800 {
            continue; // not IPv4
        }
        let ip = &rec[ETHERNET_HEADER_LEN..];
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
        out.push(&udp[8..]);
    }
    out
}

/// Splits one UDP payload into its RTP header and HBRMT-payload-header +
/// media bytes.
fn rtp_and_hbrmt(udp_payload: &[u8]) -> (&[u8], &[u8]) {
    assert!(
        udp_payload.len() > RTP_HEADER_LEN,
        "UDP payload too short for RTP header"
    );
    udp_payload.split_at(RTP_HEADER_LEN)
}

/// Parses the first packet of the capture and asserts every HBRMT header
/// field against the hand-decode in `PROVENANCE.md`, independently
/// re-derived directly from the committed file above.
#[test]
fn first_packet_hbrmt_header_matches_documented_decode() {
    let data = std::fs::read(fixture_path()).expect("read the real HBRMT capture");
    let payloads = udp_payloads(&data);
    assert_eq!(
        payloads.len(),
        4497,
        "one 1080i59.94 frame at 1376 B/packet"
    );

    let (rtp, hbrmt_bytes) = rtp_and_hbrmt(payloads[0]);
    // RTP: V=2,P=0,X=0,CC=0,M=0,PT=98 (dynamic), seq=1, SSRC=0.
    assert_eq!(rtp[0], 0x80);
    assert_eq!(rtp[1], 0x62);
    assert_eq!(u16::from_be_bytes([rtp[2], rtp[3]]), 1);
    assert_eq!(u32::from_be_bytes([rtp[8], rtp[9], rtp[10], rtp[11]]), 0);

    let header = PayloadHeader::parse(hbrmt_bytes).expect("parse real HBRMT payload header");
    assert_eq!(header.vsid, VideoSourceId::Primary);
    assert_eq!(header.fr_count, 0);
    assert_eq!(header.timestamp_ref, TimestampRef::NotLocked);
    assert_eq!(header.scrambling, Scrambling::NotScrambled);
    assert_eq!(header.fec_usage, FecUsage::None);
    assert_eq!(header.clock_frequency, ClockFrequency::Mhz148_5Div1001);
    assert_eq!(header.reserve, 0);
    assert_eq!(
        header.video_source_format,
        Some(VideoSourceFormat {
            map: MapStructure::Direct,
            frame: FrameStructure::Hd1080i,
            frate: st2022::FrameRate::Hz30Div1001,
            sample: SampleStructure::Yuv422At10Bit,
            fmt_reserve: 0,
        })
    );
    assert_eq!(header.video_timestamp, Some(0));
    assert_eq!(header.extension, None);

    // 4 (fixed) + 4 (VSF) + 4 (timestamp) = 12-byte HBRMT header, then the
    // fixed 1376-byte media payload -- matching PROVENANCE.md's own
    // "12 (RTP) + 12 (HBRMT hdr incl. timestamp) = 24 header bytes" tally.
    assert_eq!(header.serialized_len(), 12);
    assert_eq!(hbrmt_bytes.len() - 12, HBRMT_MEDIA_PAYLOAD_LEN);

    // Byte-exact round trip against the real capture bytes (kills a lossy
    // parser or a raw-passthrough serializer).
    let mut out = vec![0u8; header.serialized_len()];
    let written = header.serialize_into(&mut out).unwrap();
    assert_eq!(written, 12);
    assert_eq!(out, &hbrmt_bytes[..12]);
}

/// Parses and byte-exact round-trips the HBRMT payload header of *every*
/// packet in the capture (not just the first), and cross-checks the
/// whole-frame invariants `PROVENANCE.md` documents: constant `VSID`,
/// `FRCount` (single frame), `CF`, `RESERVE`; contiguous RTP sequence
/// numbers 1..=4497; and a fixed 1376-byte media payload on every packet
/// including the final, packetizer-padded one.
#[test]
fn every_packet_hbrmt_header_round_trips_byte_exact() {
    let data = std::fs::read(fixture_path()).expect("read the real HBRMT capture");
    let payloads = udp_payloads(&data);
    assert_eq!(payloads.len(), 4497);

    let mut seqs = Vec::with_capacity(payloads.len());
    for (i, udp_payload) in payloads.iter().enumerate() {
        let (rtp, hbrmt_bytes) = rtp_and_hbrmt(udp_payload);
        seqs.push(u16::from_be_bytes([rtp[2], rtp[3]]));

        let header = PayloadHeader::parse(hbrmt_bytes)
            .unwrap_or_else(|e| panic!("packet {i}: parse real HBRMT header: {e}"));

        // Whole-frame invariants: same video source, same frame (FRCount
        // doesn't advance because this capture is exactly one frame), same
        // clock reference, and RESERVE reads 0 (spec: "shall be set to 0 by
        // the sender") on every single real packet, not just the first.
        assert_eq!(header.vsid, VideoSourceId::Primary, "packet {i}");
        assert_eq!(header.fr_count, 0, "packet {i}");
        assert_eq!(
            header.clock_frequency,
            ClockFrequency::Mhz148_5Div1001,
            "packet {i}"
        );
        assert_eq!(header.reserve, 0, "packet {i}");

        let header_len = header.serialized_len();
        assert_eq!(
            hbrmt_bytes.len() - header_len,
            HBRMT_MEDIA_PAYLOAD_LEN,
            "packet {i}"
        );

        let mut out = vec![0u8; header_len];
        let written = header.serialize_into(&mut out).unwrap();
        assert_eq!(written, header_len, "packet {i}");
        assert_eq!(
            out,
            &hbrmt_bytes[..header_len],
            "packet {i}: byte-exact round trip"
        );

        let reparsed = PayloadHeader::parse(&out).unwrap();
        assert_eq!(reparsed, header, "packet {i}: re-parse equality");
    }

    let expected_seqs: Vec<u16> = (1..=4497).collect();
    assert_eq!(
        seqs, expected_seqs,
        "RTP sequence numbers must be contiguous 1..=4497"
    );

    // Marker bit: RFC 3550 leaves M's meaning to the payload format; this
    // sender sets it only on the last packet of the frame.
    let (last_rtp, _) = rtp_and_hbrmt(payloads[4496]);
    assert_eq!(
        last_rtp[1] & 0x80,
        0x80,
        "M bit set on the frame's final packet"
    );
}

/// The specific finding `st2022/docs/st2022-6-framing.md` asked a real
/// capture to settle: is `RESERVE` really 5 bits wide at bit position
/// `[4:0]` of the `R|S|FEC|CF|RESERVE` word, as derived by arithmetic
/// (2+2+3+4+5=16), or is that derivation wrong?
///
/// This capture **confirms** the derivation:
///
/// - Every one of the 4,497 real packets decodes `CF` (bits `[8:5]`) as `3`
///   (148.5/1.001 MHz) -- cross-corroborated two independent ways in
///   `PROVENANCE.md` (herisson's own `g_FRATE`/profile tables, and this
///   crate's spec transcription both independently landing on the same
///   clock). If `RESERVE` were actually a different width (shifting where
///   `CF`'s 4 bits sit), that cross-corroboration would not hold -- `CF`
///   would decode to a nonsensical or inconsistent value instead of a
///   real-world clock frequency every single real sender emitted.
/// - `RESERVE` itself reads exactly `0` on every real packet, matching the
///   spec's "shall be set to 0 by the sender" -- a genuine implementation's
///   actual output, not an assumption.
///
/// So the doc's own flagged uncertainty is resolved as CONFIRMED, not
/// contradicted -- reported loudly here rather than silently treated as
/// settled, per this task's instructions.
#[test]
fn reserve_field_width_is_confirmed_not_contradicted_by_real_capture() {
    let data = std::fs::read(fixture_path()).expect("read the real HBRMT capture");
    let payloads = udp_payloads(&data);

    for (i, udp_payload) in payloads.iter().enumerate() {
        let (_, hbrmt_bytes) = rtp_and_hbrmt(udp_payload);
        let header = PayloadHeader::parse(hbrmt_bytes).unwrap();
        assert_eq!(header.reserve, 0, "packet {i}: RESERVE must decode to 0");
        assert!(
            header.reserve <= st2022::MAX_RESERVE,
            "packet {i}: RESERVE must fit 5 bits"
        );
        assert_eq!(
            header.clock_frequency,
            ClockFrequency::Mhz148_5Div1001,
            "packet {i}: CF must decode to the real, cross-corroborated clock \
             (a wrong RESERVE width would desync this field)"
        );
    }
}

/// Bite-proof: corrupting the real capture's fixed-header byte carrying
/// `CF`/`RESERVE` (in memory -- the committed fixture file is never
/// touched) changes the decoded clock frequency and/or reserve bits,
/// proving the assertions above are actually reading real bytes rather
/// than passing vacuously.
#[test]
fn corrupting_the_real_cf_reserve_byte_changes_the_decode() {
    let data = std::fs::read(fixture_path()).expect("read fixture");
    let payloads = udp_payloads(&data);
    let (_, hbrmt_bytes) = rtp_and_hbrmt(payloads[0]);

    let pristine = PayloadHeader::parse(hbrmt_bytes).unwrap();
    assert_eq!(pristine.clock_frequency, ClockFrequency::Mhz148_5Div1001);
    assert_eq!(pristine.reserve, 0);

    // Byte 3 of the HBRMT fixed header (index 3 within `hbrmt_bytes`) holds
    // the low byte of the R|S|FEC|CF|RESERVE word: `0x60` = CF=3, RESERVE=0.
    // Flip the RESERVE bits (low 5 bits) to a nonzero value.
    let mut corrupted = hbrmt_bytes.to_vec();
    assert_eq!(corrupted[3], 0x60);
    corrupted[3] |= 0x1F;

    let after = PayloadHeader::parse(&corrupted).unwrap();
    assert_eq!(
        after.reserve, 0x1F,
        "corrupting the real byte must change the decoded RESERVE value"
    );
    assert_ne!(
        after, pristine,
        "corrupted header must no longer equal the pristine real-capture decode"
    );
}
