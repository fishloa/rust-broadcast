//! Gate for **absolute, optional** sample timing — media plane step 2c
//! (`docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4).
//!
//! Before step 2c a `Sample`'s time was a running sum of `duration` anchored on
//! `Track::start_decode_time`, which FLV, WebM, MPEG Program Stream, RTMP and
//! RTP all left at `0`; absolute time lived only in the write-only
//! `SourceTiming`. This file gates the replacement:
//!
//! 1. **33-bit rollover is unwrapped exactly once, at the demux edge.** A
//!    synthesised TS whose PES PTS/DTS cross `2^33` must demux to a strictly
//!    monotonic absolute DTS sequence with uniform steps — no wrap visible in
//!    the IR, and no *second* unroll downstream. `Provenance` still carries the
//!    folded raw wire stamp, proving the unwrap happened once and did not
//!    destroy the original.
//! 2. **Each of the five previously-anchorless demuxers emits real absolute
//!    time** (FLV, WebM, PS, RTMP, RTP), from that container's own clock.
//! 3. **Timestamps are never fabricated**: a section-carried track stays
//!    `None` (gated in `tests/any_stream.rs`), and every timed track's
//!    `Track::start_decode_time` stays in lockstep with its first sample's
//!    `dts`.
//!
//! Every timing assertion is checked against a value derived independently in
//! this file (the PES timestamps this file itself wrote, an FLV tag-timestamp
//! walk, a WebM cluster/block-timecode walk, or the RTP header timestamps),
//! never against the demuxer's own bookkeeping.

use broadcast_common::{Package, Unpackage};

use transmux::pipeline::CodecConfig;
use transmux::rtp::{RtpInput, RtpInputStream, RtpMediaKind};
use transmux::{FlvDemux, Media, PsDemux, RtmpMux, RtpOutput, RtpPacketiser, TsDemux, WebmDemux};

// ═══════════════════════════════════════════════════════════════════════════
// 1. 33-bit rollover, unwrapped once at the demux edge
// ═══════════════════════════════════════════════════════════════════════════

/// The MPEG-2 Systems 33-bit PTS/DTS modulus (ISO/IEC 13818-1 §2.4.3.7).
const TS_WRAP: i64 = 1 << 33;
/// 90 kHz / 30 fps.
const FRAME_DUR: i64 = 3000;

const TS_PACKET_SIZE: usize = 188;
const PAT_PID: u16 = 0x0000;
const PMT_PID: u16 = 0x1000;
const ES_PID: u16 = 0x0100;
const STREAM_TYPE_AVC: u8 = 0x1B;

/// A plausible Baseline SPS + PPS, so `TsDemux`'s single-shot config recovery
/// resolves an `avcC` for the synthesised elementary stream.
const SPS: [u8; 9] = [0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24];
const PPS: [u8; 4] = [0x68, 0xce, 0x3c, 0x80];

/// Encode a 33-bit PTS/DTS into its 5-byte PES field (§2.4.3.7): a 4-bit
/// `prefix`, then the value split 3 / 15 / 15 bits with marker bits set.
fn encode_pes_ts(value: i64, prefix: u8) -> [u8; 5] {
    let v = (value as u64) & ((1u64 << 33) - 1);
    [
        (prefix << 4) | ((((v >> 30) & 0x07) as u8) << 1) | 0x01,
        ((v >> 22) & 0xFF) as u8,
        ((((v >> 15) & 0x7F) as u8) << 1) | 0x01,
        ((v >> 7) & 0xFF) as u8,
        (((v & 0x7F) as u8) << 1) | 0x01,
    ]
}

/// One Annex B access unit: parameter sets on the first AU (so config
/// resolves), then an IDR slice NAL tagged with `tag` so samples are
/// distinguishable.
fn annexb_au(with_param_sets: bool, tag: u8) -> Vec<u8> {
    let mut au = Vec::new();
    let mut push = |nal: &[u8]| {
        au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        au.extend_from_slice(nal);
    };
    if with_param_sets {
        push(&SPS);
        push(&PPS);
    }
    push(&[0x65, 0x88, 0x84, tag]);
    au
}

/// Wrap an access unit in a PES packet carrying both PTS and DTS (§2.4.3.6).
fn pes_packet(au: &[u8], pts: i64, dts: i64) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]); // start code + video stream_id
    let header_data_len = 10u8; // PTS(5) + DTS(5)
    let payload_len = au.len() + 3 + header_data_len as usize;
    out.extend_from_slice(&(payload_len as u16).to_be_bytes());
    out.push(0x80); // '10' marker, no scrambling
    out.push(0xC0); // PTS_DTS_flags = '11'
    out.push(header_data_len);
    out.extend_from_slice(&encode_pes_ts(pts, 0b0011));
    out.extend_from_slice(&encode_pes_ts(dts, 0b0001));
    out.extend_from_slice(au);
    out
}

/// Packetise a payload into 188-byte TS packets on `pid`, PUSI on the first,
/// stuffing the tail of the final packet via an adaptation field.
fn packetise(pid: u16, payload: &[u8], cc: &mut u8, out: &mut Vec<u8>) {
    let mut first = true;
    let mut off = 0usize;
    while off < payload.len() {
        let remaining = payload.len() - off;
        let mut pkt = Vec::with_capacity(TS_PACKET_SIZE);
        pkt.push(0x47);
        let pusi = if first { 0x40 } else { 0x00 };
        pkt.push(pusi | ((pid >> 8) as u8 & 0x1F));
        pkt.push((pid & 0xFF) as u8);
        let body_cap = TS_PACKET_SIZE - 4;
        if remaining >= body_cap {
            pkt.push(0x10 | (*cc & 0x0F)); // payload only
            pkt.extend_from_slice(&payload[off..off + body_cap]);
            off += body_cap;
        } else {
            // Adaptation field carrying stuffing so the packet stays 188 bytes.
            pkt.push(0x30 | (*cc & 0x0F));
            let af_len = body_cap - 1 - remaining;
            pkt.push(af_len as u8);
            if af_len > 0 {
                pkt.push(0x00); // flags
                pkt.extend(core::iter::repeat_n(0xFF, af_len - 1));
            }
            pkt.extend_from_slice(&payload[off..]);
            off = payload.len();
        }
        assert_eq!(pkt.len(), TS_PACKET_SIZE, "TS packet must be 188 bytes");
        *cc = cc.wrapping_add(1);
        out.extend_from_slice(&pkt);
        first = false;
    }
}

/// A long-form PSI section (§2.4.4.1): header + body + trailing CRC-32.
fn psi_section(table_id: u8, id_extension: u16, body: &[u8]) -> Vec<u8> {
    let mut s = Vec::new();
    s.push(table_id);
    let section_length = body.len() + 5 + 4; // the 5 bytes below + CRC
    s.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
    s.push((section_length & 0xFF) as u8);
    s.extend_from_slice(&id_extension.to_be_bytes());
    s.push(0xC1); // version 0, current_next = 1
    s.push(0x00); // section_number
    s.push(0x00); // last_section_number
    s.extend_from_slice(body);
    let crc = broadcast_common::crc32_mpeg2::compute(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    // PSI packets carry a pointer_field before the section.
    let mut with_pointer = alloc_vec_with_pointer();
    with_pointer.extend_from_slice(&s);
    with_pointer
}

fn alloc_vec_with_pointer() -> Vec<u8> {
    vec![0x00]
}

/// Build a single-program TS carrying one H.264 elementary stream whose PES
/// PTS/DTS start at `first_dts` and step by [`FRAME_DUR`], so a `first_dts`
/// just below `2^33` makes the stream cross the wrap.
fn synth_ts_crossing_wrap(first_dts: i64, n_frames: usize) -> Vec<u8> {
    let mut out = Vec::new();

    // PAT: program 1 → PMT_PID.
    let mut pat_body = Vec::new();
    pat_body.extend_from_slice(&1u16.to_be_bytes());
    pat_body.extend_from_slice(&(0xE000 | PMT_PID).to_be_bytes());
    let pat = psi_section(0x00, 1, &pat_body);
    let mut cc_pat = 0u8;
    packetise(PAT_PID, &pat, &mut cc_pat, &mut out);

    // PMT: PCR on the ES PID, one AVC stream.
    let mut pmt_body = Vec::new();
    pmt_body.extend_from_slice(&(0xE000 | ES_PID).to_be_bytes()); // PCR_PID
    pmt_body.extend_from_slice(&0xF000u16.to_be_bytes()); // program_info_length = 0
    pmt_body.push(STREAM_TYPE_AVC);
    pmt_body.extend_from_slice(&(0xE000 | ES_PID).to_be_bytes());
    pmt_body.extend_from_slice(&0xF000u16.to_be_bytes()); // ES_info_length = 0
    let pmt = psi_section(0x02, 1, &pmt_body);
    let mut cc_pmt = 0u8;
    packetise(PMT_PID, &pmt, &mut cc_pmt, &mut out);

    // The elementary stream.
    let mut cc_es = 0u8;
    for i in 0..n_frames {
        let dts = first_dts + i as i64 * FRAME_DUR;
        let au = annexb_au(i == 0, i as u8);
        // PTS == DTS (no reordering) so composition_offset stays 0; the wire
        // value is the 33-bit-folded one.
        let pes = pes_packet(&au, dts, dts);
        packetise(ES_PID, &pes, &mut cc_es, &mut out);
    }
    out
}

/// The expected unwrapped (absolute) DTS sequence for `synth_ts_crossing_wrap`.
fn expected_absolute_dts(first_dts: i64, n_frames: usize) -> Vec<i64> {
    (0..n_frames)
        .map(|i| first_dts + i as i64 * FRAME_DUR)
        .collect()
}

/// **The rollover gate.** A stream whose wire PTS/DTS cross `2^33` must demux
/// to a strictly monotonic ABSOLUTE dts sequence, unwrapped exactly once at the
/// demux edge — the wrap must be invisible in the IR.
///
/// Mutation-checked: reverting `ts_demux`'s `WrapState::push` to return the raw
/// folded value makes the monotonicity and equality assertions below fail.
#[test]
fn ts_33bit_rollover_is_unwrapped_once_at_the_demux_edge() {
    // Start 2 frames below 2^33 so frame 2 lands exactly ON the boundary
    // (wire value 0) and frames 3.. fold to small positive values — the
    // classic wrap.
    const N: usize = 6;
    let first_dts = TS_WRAP - 2 * FRAME_DUR;
    let ts = synth_ts_crossing_wrap(first_dts, N);

    let media = TsDemux::new().unpackage(&ts).expect("demux synthesised TS");
    let track = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("the synthesised AVC track must be recovered");
    assert_eq!(
        track.samples.len(),
        N,
        "every synthesised access unit must survive demux"
    );

    // Sanity: the fixture really does cross the wrap on the wire (otherwise
    // this test would pass without exercising the unroll at all).
    let wire: Vec<u64> = expected_absolute_dts(first_dts, N)
        .iter()
        .map(|&d| (d as u64) % (TS_WRAP as u64))
        .collect();
    assert!(
        wire.windows(2).any(|w| w[1] < w[0]),
        "the synthesised wire DTS must step BACKWARD across 2^33 (else no wrap to unroll): {wire:?}"
    );

    // The IR must be absolute and monotonic, with the exact expected values.
    let got: Vec<i64> = track
        .samples
        .iter()
        .map(|s| {
            s.dts
                .expect("a timed video sample must carry an absolute dts")
        })
        .collect();
    assert_eq!(
        got,
        expected_absolute_dts(first_dts, N),
        "absolute dts must be the unwrapped PES clock, crossing 2^33 without folding"
    );
    for w in got.windows(2) {
        assert!(
            w[1] > w[0],
            "absolute dts must be strictly increasing across the 2^33 wrap: {got:?}"
        );
        assert_eq!(
            w[1] - w[0],
            FRAME_DUR,
            "each step must be exactly one frame — a missed OR double unroll shows up here"
        );
    }

    // Unwrapped exactly ONCE: `Provenance` still carries the *folded* wire
    // stamp, so the original is preserved rather than overwritten, and the
    // absolute value is congruent to it modulo 2^33.
    for (i, s) in track.samples.iter().enumerate() {
        let p = s
            .provenance
            .expect("a TS-demuxed timed sample carries debug Provenance");
        let abs = s.dts.unwrap();
        assert_eq!(
            p.wire_dts,
            Some((abs as u64) % (TS_WRAP as u64)),
            "sample {i}: Provenance must carry the folded wire dts"
        );
        assert_eq!(
            wire[i],
            (abs as u64) % (TS_WRAP as u64),
            "sample {i}: absolute dts must stay congruent to the wire value mod 2^33"
        );
    }

    // The anchor stays in lockstep with the first sample (step-2c invariant),
    // and is itself the UNWRAPPED value.
    assert_eq!(
        track.start_decode_time, first_dts as u64,
        "start_decode_time must equal the first sample's absolute dts"
    );
}

/// The wrap-crossing stream must also survive TS → IR → TS: the re-demuxed
/// absolute DTS deltas are preserved (the muxer re-folds to 33 bits on the
/// wire, per §2.4.3.7, and the demuxer unrolls again — exactly once each way).
#[test]
fn ts_rollover_survives_ts_ir_ts_round_trip() {
    const N: usize = 6;
    let first_dts = TS_WRAP - 2 * FRAME_DUR;
    let ts = synth_ts_crossing_wrap(first_dts, N);
    let ir = TsDemux::new().unpackage(&ts).expect("demux");
    let ts2 = transmux::TsMux::new().package(&ir).expect("re-mux to TS");
    let ir2 = TsDemux::new().unpackage(&ts2).expect("re-demux");

    let deltas = |m: &Media| -> Vec<i64> {
        let t = m
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("AVC track");
        t.samples
            .iter()
            .filter_map(|s| s.dts)
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| w[1] - w[0])
            .collect()
    };
    assert_eq!(
        deltas(&ir2),
        deltas(&ir),
        "TS → IR → TS must preserve the absolute inter-sample decode deltas across a 2^33 wrap"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. The five previously-anchorless demuxers now emit real absolute time
// ═══════════════════════════════════════════════════════════════════════════

/// Assert a track's timing is genuinely absolute and self-consistent: every
/// sample carries `dts`/`pts`, the sequence is non-decreasing, the anchor
/// matches the first sample, and at least one sample sits at a non-zero time
/// (so a demuxer that silently left everything at 0 fails).
fn assert_absolute_timeline(media: &Media, what: &str) {
    assert!(
        !media.tracks.is_empty(),
        "{what}: expected at least 1 track"
    );
    let mut saw_nonzero = false;
    for (ti, t) in media.tracks.iter().enumerate() {
        if t.samples.is_empty() {
            continue;
        }
        let dts: Vec<i64> = t
            .samples
            .iter()
            .map(|s| {
                s.dts
                    .unwrap_or_else(|| panic!("{what}: track {ti} sample has no absolute dts"))
            })
            .collect();
        for s in &t.samples {
            assert!(
                s.pts.is_some(),
                "{what}: track {ti} sample has no absolute pts"
            );
        }
        for w in dts.windows(2) {
            assert!(
                w[1] >= w[0],
                "{what}: track {ti} absolute dts must be non-decreasing, got {dts:?}"
            );
        }
        assert_eq!(
            t.start_decode_time,
            dts[0].max(0) as u64,
            "{what}: track {ti} start_decode_time must equal its first sample's absolute dts"
        );
        if dts.iter().any(|&d| d != 0) {
            saw_nonzero = true;
        }
    }
    assert!(
        saw_nonzero,
        "{what}: every sample sat at time 0 — the demuxer is not recovering real absolute time"
    );
}

/// FLV: absolute time comes from the tag timestamps (§E.4.1, milliseconds),
/// independently re-walked here from the raw file.
#[test]
fn flv_demux_emits_absolute_time_from_tag_timestamps() {
    let flv = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/flv/av.flv"
    ))
    .expect("fixtures/flv/av.flv");
    let media = FlvDemux::new().unpackage(&flv[..]).expect("FLV → IR");
    assert_absolute_timeline(&media, "FLV");

    // Independent oracle: the video tag timestamps in file order.
    let mut want: Vec<i64> = Vec::new();
    let mut off = 9usize + 4; // header + PreviousTagSize0
    while off + 11 <= flv.len() {
        let tag_type = flv[off];
        let data_size = ((flv[off + 1] as usize) << 16)
            | ((flv[off + 2] as usize) << 8)
            | flv[off + 3] as usize;
        let ts_lo =
            ((flv[off + 4] as u32) << 16) | ((flv[off + 5] as u32) << 8) | flv[off + 6] as u32;
        let ts_ext = flv[off + 7] as u32;
        let timestamp = ((ts_ext << 24) | ts_lo) as i64;
        let body = &flv[off + 11..(off + 11 + data_size).min(flv.len())];
        // Video AVC NALU tags only (AVCPacketType 1), matching FlvDemux's scope.
        if tag_type == 9 && body.len() > 1 && (body[0] & 0x0F) == 7 && body[1] == 1 {
            want.push(timestamp);
        }
        off += 11 + data_size + 4;
    }
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("FLV AVC track");
    let got: Vec<i64> = video.samples.iter().filter_map(|s| s.dts).collect();
    assert_eq!(
        got, want,
        "FLV sample dts must be the tag timestamps verbatim (absolute ms)"
    );
}

// ── A standalone EBML/Matroska walk (T6): an independent oracle for WebM ────
//
// The module doc (above) has long claimed the WebM test compares against "a
// WebM cluster/block-timecode walk", but until this file's `absolute_timing`
// test-integrity pass, no such walk existed — the test below only called
// `assert_absolute_timeline` (monotonic + anchor + "saw at least one nonzero
// dts"), which a 1000x wrong `TimestampScale` satisfies just as easily as a
// correct one. This section is a real walk: it reads EBML element IDs/sizes
// and Matroska's `Segment > {Info, Cluster > {Timestamp, SimpleBlock}}`
// structure directly off the fixture's raw bytes (RFC 8794 §4/§5 for the EBML
// varint/ID encoding; RFC 9559 §12/§27 for Cluster/Timestamp/SimpleBlock and
// `TimestampScale`) — entirely independently of `transmux::webm_demux`, which
// this test exists to check.

/// Read an EBML variable-size-integer **element ID** (RFC 8794 §5): unlike a
/// size/value vint, the ID's leading length-marker bits are kept as part of
/// the ID. Returns `(id, bytes_consumed)`.
fn ebml_read_id(buf: &[u8]) -> Option<(u32, usize)> {
    let first = *buf.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 4 || buf.len() < len {
        return None;
    }
    let mut id: u32 = 0;
    for &b in &buf[..len] {
        id = (id << 8) | b as u32;
    }
    Some((id, len))
}

/// Read an EBML variable-size-integer **value** (a size or SimpleBlock track
/// number, RFC 8794 §4): the leading length-marker bits are masked off.
/// Returns `(value, bytes_consumed)`.
fn ebml_read_vint(buf: &[u8]) -> Option<(u64, usize)> {
    let first = *buf.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len == 0 || len > 8 || buf.len() < len {
        return None;
    }
    let mask: u8 = if len >= 8 { 0 } else { 0xFFu8 >> len };
    let mut value: u64 = (first & mask) as u64;
    for &b in &buf[1..len] {
        value = (value << 8) | b as u64;
    }
    Some((value, len))
}

/// One SimpleBlock's independently-computed absolute presentation time (IR
/// milliseconds) and keyframe flag, keyed by the raw Matroska track number.
struct WebmOracle {
    /// `track_number -> [(pts_ms, is_sync), ...]` in file (Cluster) order.
    tracks: std::collections::BTreeMap<u64, Vec<(i64, bool)>>,
}

/// Walk a WebM/Matroska file's `EBML header`, `Segment > Info` (for
/// `TimestampScale`), and every `Segment > Cluster > {Timestamp, SimpleBlock}`
/// (RFC 9559 §12), computing each SimpleBlock's absolute presentation time in
/// IR milliseconds exactly as documented in `webm_demux.rs`'s module doc
/// (`(cluster_ts + rel_ts) * timestamp_scale_ns / 1_000_000`) — but by
/// re-deriving it from the raw bytes here, not by calling into that module.
fn walk_webm(data: &[u8]) -> WebmOracle {
    const EBML_HEADER: u32 = 0x1A45_DFA3;
    const SEGMENT: u32 = 0x1853_8067;
    const INFO: u32 = 0x1549_A966;
    const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
    const CLUSTER: u32 = 0x1F43_B675;
    const CLUSTER_TIMESTAMP: u32 = 0xE7;
    const SIMPLE_BLOCK: u32 = 0xA3;
    const BLOCK_GROUP: u32 = 0xA0;
    const BLOCK: u32 = 0xA1;
    const REFERENCE_BLOCK: u32 = 0xFB;
    const KEYFRAME_FLAG: u8 = 0x80;
    const DEFAULT_TIMESTAMP_SCALE_NS: u64 = 1_000_000;
    const NS_PER_MS: i64 = 1_000_000;

    // Decode a SimpleBlock/Block payload's `track_number` vint + `i16`
    // relative timecode (both use the identical layout, RFC 9559 §12.5/§12.9),
    // returning `(track_number, rel_ts, flags_byte_offset)`.
    fn read_block_header(data: &[u8], p: usize) -> (u64, i64, usize) {
        let (track_number, tn_len) =
            ebml_read_vint(&data[p..]).expect("Block/SimpleBlock track number vint");
        let rel_off = p + tn_len;
        let rel_ts = i16::from_be_bytes([data[rel_off], data[rel_off + 1]]) as i64;
        (track_number, rel_ts, rel_off + 2)
    }

    let (id, idlen) = ebml_read_id(data).expect("EBML header element id");
    assert_eq!(id, EBML_HEADER, "file must start with the EBML header");
    let (hdr_size, hdr_sizelen) = ebml_read_vint(&data[idlen..]).expect("EBML header size");
    let mut pos = idlen + hdr_sizelen + hdr_size as usize;

    let (id, idlen) = ebml_read_id(&data[pos..]).expect("Segment element id");
    assert_eq!(id, SEGMENT, "EBML header must be followed by a Segment");
    pos += idlen;
    let (_seg_size, sizelen) = ebml_read_vint(&data[pos..]).expect("Segment size");
    pos += sizelen;

    let mut timestamp_scale_ns = DEFAULT_TIMESTAMP_SCALE_NS;
    let mut tracks: std::collections::BTreeMap<u64, Vec<(i64, bool)>> = Default::default();

    while pos < data.len() {
        let Some((id, idlen)) = ebml_read_id(&data[pos..]) else {
            break;
        };
        pos += idlen;
        let Some((size, sizelen)) = ebml_read_vint(&data[pos..]) else {
            break;
        };
        pos += sizelen;
        let body_start = pos;
        let body_end = (body_start + size as usize).min(data.len());

        if id == INFO {
            let mut p = body_start;
            while p < body_end {
                let Some((cid, cidlen)) = ebml_read_id(&data[p..]) else {
                    break;
                };
                p += cidlen;
                let Some((csize, csizelen)) = ebml_read_vint(&data[p..]) else {
                    break;
                };
                p += csizelen;
                if cid == TIMESTAMP_SCALE {
                    let mut v = 0u64;
                    for &b in &data[p..p + csize as usize] {
                        v = (v << 8) | b as u64;
                    }
                    timestamp_scale_ns = v;
                }
                p += csize as usize;
            }
        } else if id == CLUSTER {
            let mut cluster_ts: i64 = 0;
            let mut p = body_start;
            while p < body_end {
                let Some((cid, cidlen)) = ebml_read_id(&data[p..]) else {
                    break;
                };
                p += cidlen;
                let Some((csize, csizelen)) = ebml_read_vint(&data[p..]) else {
                    break;
                };
                p += csizelen;
                if cid == CLUSTER_TIMESTAMP {
                    let mut v = 0i64;
                    for &b in &data[p..p + csize as usize] {
                        v = (v << 8) | b as i64;
                    }
                    cluster_ts = v;
                } else if cid == SIMPLE_BLOCK {
                    let (track_number, rel_ts, flags_off) = read_block_header(data, p);
                    let flags = data[flags_off];
                    let is_sync = flags & KEYFRAME_FLAG != 0;
                    let raw_ticks = cluster_ts + rel_ts;
                    let ns = raw_ticks.saturating_mul(timestamp_scale_ns as i64);
                    let pts_ms = ns / NS_PER_MS;
                    tracks
                        .entry(track_number)
                        .or_default()
                        .push((pts_ms, is_sync));
                } else if cid == BLOCK_GROUP {
                    // A `Block` wrapped in a `BlockGroup` (RFC 9559 §12.4) —
                    // ffmpeg emits this instead of a `SimpleBlock` at least
                    // for a track's final frame (observed: this fixture's
                    // very last audio frame, alongside a `DiscardPadding`
                    // sibling). `Block`'s own flags byte carries no keyframe
                    // bit (unlike `SimpleBlock`'s); a `BlockGroup` is a
                    // keyframe iff it carries no `ReferenceBlock` child (a
                    // frame with no reference is, by definition, not
                    // predicted from another frame).
                    let mut q = p;
                    let qend = p + csize as usize;
                    let mut block: Option<(u64, i64, usize)> = None;
                    let mut has_reference_block = false;
                    while q < qend {
                        let Some((qid, qidlen)) = ebml_read_id(&data[q..]) else {
                            break;
                        };
                        q += qidlen;
                        let Some((qsize, qsizelen)) = ebml_read_vint(&data[q..]) else {
                            break;
                        };
                        q += qsizelen;
                        if qid == BLOCK {
                            block = Some(read_block_header(data, q));
                        } else if qid == REFERENCE_BLOCK {
                            has_reference_block = true;
                        }
                        q += qsize as usize;
                    }
                    let (track_number, rel_ts, _flags_off) =
                        block.expect("BlockGroup must carry exactly one Block child");
                    let raw_ticks = cluster_ts + rel_ts;
                    let ns = raw_ticks.saturating_mul(timestamp_scale_ns as i64);
                    let pts_ms = ns / NS_PER_MS;
                    tracks
                        .entry(track_number)
                        .or_default()
                        .push((pts_ms, !has_reference_block));
                }
                p += csize as usize;
            }
        }
        pos = body_end;
    }

    WebmOracle { tracks }
}

/// WebM: absolute time comes from Cluster + SimpleBlock timecodes (RFC 9559
/// §12), scaled by `TimestampScale` — WebM carries a presentation clock only,
/// so `dts == pts`. Checked against `walk_webm`'s independent re-derivation
/// of the same value straight from the file's raw EBML bytes (see above) —
/// not against `assert_absolute_timeline` alone, which a 1000x wrong
/// `TimestampScale` would satisfy just as easily as a correct one.
#[test]
fn webm_demux_emits_absolute_time_from_cluster_timecodes() {
    let webm = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/webm/vp9_opus.webm"
    ))
    .expect("fixtures/webm/vp9_opus.webm");
    let media = WebmDemux::new().unpackage(&webm[..]).expect("WebM → IR");
    assert_absolute_timeline(&media, "WebM");
    for t in &media.tracks {
        for s in &t.samples {
            assert_eq!(
                s.dts, s.pts,
                "WebM carries only a presentation clock, so dts must equal pts"
            );
        }
    }

    let oracle = walk_webm(&webm);
    assert_eq!(
        oracle.tracks.len(),
        media.tracks.len(),
        "walk_webm must find the same track count as the IR"
    );
    // Matroska track numbers are 1-based and (for this fixture, like the IR's
    // own track_id) assigned in ascending Tracks-element order, so zipping
    // the two ascending sequences pairs up the same elementary streams.
    for (t, (_track_number, oracle_samples)) in media.tracks.iter().zip(oracle.tracks.iter()) {
        assert_eq!(
            t.samples.len(),
            oracle_samples.len(),
            "sample count must match the independent EBML walk"
        );
        for (i, (s, &(oracle_pts_ms, oracle_sync))) in
            t.samples.iter().zip(oracle_samples.iter()).enumerate()
        {
            let dts = s
                .dts
                .unwrap_or_else(|| panic!("WebM sample {i} has no dts"));
            assert_eq!(
                dts, oracle_pts_ms,
                "WebM sample {i} dts must match the independent EBML \
                 Cluster+SimpleBlock walk (this is what a 1000x wrong \
                 TimestampScale would break)"
            );
            assert_eq!(
                s.flags.is_sync, oracle_sync,
                "WebM sample {i} keyframe flag must match the independent EBML walk"
            );
        }
    }
}

/// MPEG Program Stream: absolute time comes from the PES PTS/DTS carried in the
/// packs (ISO/IEC 13818-1 §2.5), 33-bit-unwrapped once at the demux edge.
#[test]
fn ps_demux_emits_absolute_time_from_pes_timestamps() {
    let ps = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ps/h264_ac3.ps"
    ))
    .expect("fixtures/ps/h264_ac3.ps");
    let media = PsDemux::new().unpackage(&ps[..]).expect("PS → IR");
    assert_absolute_timeline(&media, "MPEG Program Stream");
}

/// RTMP: absolute time comes from the RTMP message timestamps, which are the
/// FLV tag timestamps on the wire — so an RTMP round-trip of an FLV-derived IR
/// must recover the same absolute timeline.
#[test]
fn rtmp_demux_emits_absolute_time_from_message_timestamps() {
    let flv = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/flv/av.flv"
    ))
    .expect("fixtures/flv/av.flv");
    let ir = FlvDemux::new().unpackage(&flv[..]).expect("FLV → IR");
    let wire = RtmpMux::default().package(&ir).expect("IR → RTMP wire");
    let back = transmux::RtmpDemux::new()
        .unpackage(&wire[..])
        .expect("RTMP wire → IR");
    assert_absolute_timeline(&back, "RTMP");

    // The recovered inter-sample deltas must match the FLV source's.
    let deltas = |m: &Media| -> Vec<Vec<i64>> {
        m.tracks
            .iter()
            .map(|t| {
                t.samples
                    .iter()
                    .filter_map(|s| s.dts)
                    .collect::<Vec<_>>()
                    .windows(2)
                    .map(|w| w[1] - w[0])
                    .collect()
            })
            .collect()
    };
    assert_eq!(
        deltas(&back),
        deltas(&ir),
        "RTMP must preserve the absolute inter-sample timing of its FLV source"
    );
}

/// RTP: absolute time comes from the 32-bit RTP timestamp (RFC 3550 §5.1),
/// unwrapped once at the demux edge. Its origin is a random offset, so the
/// recovered timeline is an absolute *media clock* — the deltas are what must
/// match the source, and the header timestamps are the independent oracle.
#[test]
fn rtp_depacketiser_emits_absolute_time_from_rtp_timestamps() {
    let ts = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac.ts"
    ))
    .expect("h264_aac.ts fixture");
    let ir = TsDemux::new().unpackage(&ts[..]).expect("TS → IR");
    let out: RtpOutput = RtpPacketiser {
        mtu: 1400,
        ssrc: 0x1234_5678,
        ..RtpPacketiser::default()
    }
    .package(&ir)
    .expect("IR → RTP");

    let recovered = transmux::RtpDepacketiser::new()
        .unpackage(RtpInput {
            streams: out
                .streams
                .iter()
                .map(|s| RtpInputStream {
                    kind: s.kind,
                    packets: s.packets.clone(),
                })
                .collect(),
        })
        .expect("RTP → IR");
    assert_absolute_timeline(&recovered, "RTP");

    // Independent oracle: the distinct RTP header timestamps of the video
    // stream, in wire order — one per access unit.
    let video = out
        .streams
        .iter()
        .find(|s| matches!(s.kind, RtpMediaKind::H264))
        .expect("an H.264 RTP stream");
    let mut want: Vec<i64> = Vec::new();
    for pkt in &video.packets {
        let t = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]) as i64;
        if want.last() != Some(&t) {
            want.push(t);
        }
    }
    let got: Vec<i64> = recovered.tracks[0]
        .samples
        .iter()
        .filter_map(|s| s.dts)
        .collect();
    assert_eq!(
        got, want,
        "RTP sample dts must be the unwrapped RTP header timestamps, one per AU"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. fMP4 -> IR -> fMP4 byte-identical: tfdt / trun / ctts rebuilt from
//    absolute dts+pts alone
// ═══════════════════════════════════════════════════════════════════════════

/// The hardest gate in media plane step 2c. Absolute `dts`/`pts` carry strictly
/// *more* information than the old running-sum model, so re-muxing an fMP4 the
/// crate itself wrote must still reproduce it **byte for byte** — which means
/// reconstructing, from the absolute pair alone:
///
/// - `tfdt.baseMediaDecodeTime` (§8.8.12) — from the track anchor,
/// - each `trun` `sample_duration` (§8.8.8),
/// - each `trun` `sample_composition_time_offset` **and the `trun` version**
///   (v1 signed, emitted only when some sample has `pts != dts`) — now derived
///   via `Sample::composition_offset()` rather than read from a stored field.
///
/// Deliberately uses a **non-zero `tfdt`** and **real B-frame reordering**
/// (negative and positive composition offsets, out-of-order pts), because a
/// zero anchor and zero offsets would let a broken implementation pass.
#[test]
fn fmp4_ir_fmp4_is_byte_identical_with_nonzero_tfdt_and_ctts() {
    use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    use transmux::media::{CmafMux, Fmp4Demux, Track};
    use transmux::nalu_types::{AvcPps, AvcSps};
    use transmux::pipeline::{Sample, TrackSpec};

    const ANCHOR: u64 = 1_234_567;
    const DUR: i64 = 3000;

    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(SPS.to_vec())],
        pps: vec![AvcPps(PPS.to_vec())],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    let spec = TrackSpec::new(
        1,
        90_000,
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    );

    // A classic IBBP-ish reorder: decode order 0,1,2,3 with composition offsets
    // +2 frames, -1, -1, 0 — so pts is genuinely out of decode order and both
    // signs appear (forcing trun version 1 with signed offsets).
    let offsets: [i64; 4] = [2 * DUR, -DUR, -DUR, 0];
    let samples: Vec<Sample> = offsets
        .iter()
        .enumerate()
        .map(|(i, &co)| {
            let dts = ANCHOR as i64 + i as i64 * DUR;
            let nal = [0x65u8, 0x88, 0x84, i as u8];
            let mut data = (nal.len() as u32).to_be_bytes().to_vec();
            data.extend_from_slice(&nal);
            Sample::new(data, Some(dts), Some(dts + co), Some(DUR as u32), i == 0)
        })
        .collect();

    let media = Media::new(vec![Track::new_at(spec, samples, ANCHOR)], 90_000);

    // fMP4 #1, written by this crate.
    let fmp4_1 = CmafMux::default().package(&media).expect("mux #1");
    // -> IR (absolute dts/pts recovered from tfdt + trun durations + ctts) ...
    let ir = Fmp4Demux::new().unpackage(&fmp4_1).expect("demux");
    // ... -> fMP4 #2.
    let fmp4_2 = CmafMux::default().package(&ir).expect("mux #2");

    // The round-tripped IR must carry the absolute timing, not a rebased copy.
    assert_eq!(
        ir.tracks[0].start_decode_time, ANCHOR,
        "the tfdt anchor must survive into the IR"
    );
    let got: Vec<(i64, i64)> = ir.tracks[0]
        .samples
        .iter()
        .map(|s| (s.dts.expect("dts"), s.pts.expect("pts")))
        .collect();
    let want: Vec<(i64, i64)> = offsets
        .iter()
        .enumerate()
        .map(|(i, &co)| {
            let dts = ANCHOR as i64 + i as i64 * DUR;
            (dts, dts + co)
        })
        .collect();
    assert_eq!(
        got, want,
        "absolute dts/pts (incl. B-frame reordering) must round-trip exactly"
    );
    // And the composition offsets must be recoverable from the pair alone.
    let got_co: Vec<i32> = ir.tracks[0]
        .samples
        .iter()
        .map(|s| s.composition_offset())
        .collect();
    let want_co: Vec<i32> = offsets.iter().map(|&c| c as i32).collect();
    assert_eq!(
        got_co, want_co,
        "composition offsets must be implied by pts - dts"
    );

    assert_eq!(
        fmp4_2.len(),
        fmp4_1.len(),
        "fMP4 -> IR -> fMP4 must not change the file length"
    );
    assert_eq!(
        fmp4_2, fmp4_1,
        "fMP4 -> IR -> fMP4 must be BYTE-IDENTICAL: tfdt/trun/ctts rebuilt from \
         absolute dts+pts alone"
    );
}
