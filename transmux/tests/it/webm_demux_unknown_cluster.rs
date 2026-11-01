//! `WebmDemux` unknown-size Cluster gate (C10, #1011).
//!
//! ffmpeg's matroska muxer (`-f webm -live 1`, `-cluster_size_limit`, piped
//! stdout) was tried and always buffers a whole Cluster before writing it, so
//! it never emits an unknown-size *Cluster* locally (only the enclosing
//! `Segment` comes out unknown-size, which this bug does not affect — nothing
//! follows a top-level Segment in these fixtures). No unknown-size-Cluster
//! real-tool fixture could be produced (per the fixture-first rule, this is
//! flagged rather than silently skipped).
//!
//! Instead this test takes the already-committed **real** fixture
//! `fixtures/webm/vp9_opus.webm` (real ffmpeg VP9+Opus WebM, oracle
//! `fixtures/webm/vp9_opus.packets.csv`, 151 total blocks: 1 `Timestamp` +
//! 150... see below) and re-frames its single real Cluster into two Clusters
//! at a real `SimpleBlock` boundary — every payload byte stays exactly as
//! ffmpeg wrote it; only the Cluster-level EBML framing is edited, and the
//! first Cluster's size is overwritten with RFC 8794 §6.2's "unknown size"
//! reserved VINT encoding (all data bits set) instead of its real size. This
//! is the same construction a live encoder produces, built directly from the
//! spec clause rather than an unavailable tool.
//!
//! The oracle is the fixture's own already-verified total block count and
//! per-block byte lengths (`fixtures/webm/vp9_opus.packets.csv`, used
//! identically by `transmux/tests/webm_demux.rs`): re-framing must not change
//! how many blocks exist or their contents, only whether the demuxer finds
//! all of them.

use broadcast_common::Package;
use transmux::webm_demux::WebmDemux;
use transmux::{CmafMux, Media, parse_box};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp9_opus.webm"
);
const ORACLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/webm/vp9_opus.packets.csv"
);

const CLUSTER_ID: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
const SEGMENT_ID: [u8; 4] = [0x18, 0x53, 0x80, 0x67];

/// Minimal EBML VINT reader, independent of (and much simpler than) the
/// demuxer under test: used only to locate element boundaries so this test
/// can slice the real fixture, not to validate parsing semantics.
fn read_id(buf: &[u8], p: usize) -> (u32, usize) {
    let first = buf[p];
    let len = first.leading_zeros() as usize + 1;
    let mut val = 0u32;
    for &b in &buf[p..p + len] {
        val = (val << 8) | b as u32;
    }
    (val, len)
}

fn read_size(buf: &[u8], p: usize) -> (u64, usize) {
    let first = buf[p];
    let len = first.leading_zeros() as usize + 1;
    let mask: u8 = if len < 8 { 0xFF >> len } else { 0 };
    let mut val = (first & mask) as u64;
    for &b in &buf[p + 1..p + len] {
        val = (val << 8) | b as u64;
    }
    (val, len)
}

/// Encode `value` as an EBML size VINT of exactly `width` bytes.
fn encode_size(value: u64, width: usize) -> Vec<u8> {
    let marker: u8 = 1 << (8 - width);
    let mut out = vec![0u8; width];
    let mut v = value;
    for i in (0..width).rev() {
        out[i] = (v & 0xFF) as u8;
        v >>= 8;
    }
    out[0] |= marker;
    out
}

/// RFC 8794 §6.2's reserved "unknown size" VINT encoding for the given byte
/// width: the length-marker bit AND every data bit set to 1. The first byte
/// is NOT plain `0xFF` except at width 1 — the marker bit for a width-`w`
/// VINT sits at bit `8-w` (0-indexed from the LSB), so the first byte's set
/// bits are only positions `0..=8-w` (e.g. width 3 -> `0x3F`, width 8 ->
/// `0x01`, matching the "`01 FF FF FF FF FF FF FF`" pattern real 8-byte
/// unknown sizes use); every following byte is a full `0xFF`. Getting this
/// wrong (plain `0xFF` at every width) makes the first byte decode as an
/// unrelated *shorter* VINT instead of the intended width.
fn unknown_size(width: usize) -> Vec<u8> {
    let first = (((1u32 << (9 - width)) - 1) & 0xFF) as u8;
    let mut out = vec![0xFFu8; width];
    out[0] = first;
    out
}

/// Rebuild `fixtures/webm/vp9_opus.webm` with its single real Cluster split
/// into two Clusters at a real `SimpleBlock` boundary, the first marked
/// unknown-size. Returns the rebuilt bytes.
fn split_into_two_clusters(data: &[u8]) -> Vec<u8> {
    // Locate the (only) top-level Cluster.
    let cluster_pos = data
        .windows(4)
        .position(|w| w == CLUSTER_ID)
        .expect("fixture must contain a Cluster");
    let (_id, id_len) = read_id(data, cluster_pos);
    let (cluster_size, size_len, ..) = {
        let (v, l) = read_size(data, cluster_pos + id_len);
        (v, l)
    };
    let body_start = cluster_pos + id_len + size_len;
    let body_end = body_start + cluster_size as usize;

    // Walk the Cluster body's direct children (Timestamp + SimpleBlocks) to
    // find element boundaries, and split at the middle one.
    let mut child_starts = Vec::new();
    let mut p = body_start;
    while p < body_end {
        child_starts.push(p);
        let (_id, idlen) = read_id(data, p);
        let (size, sizelen) = read_size(data, p + idlen);
        p = p + idlen + sizelen + size as usize;
    }
    assert!(
        child_starts.len() >= 4,
        "need several children to split meaningfully, got {}",
        child_starts.len()
    );
    let split_at = child_starts[child_starts.len() / 2];

    let mut out = Vec::with_capacity(data.len() + 8);
    out.extend_from_slice(&data[..cluster_pos]);

    // Cluster A: unknown-size (width matches the original size field's
    // width, so the rest of the file's offsets need no other adjustment).
    out.extend_from_slice(&CLUSTER_ID);
    out.extend_from_slice(&unknown_size(size_len));
    out.extend_from_slice(&data[body_start..split_at]);

    // Cluster B: real, computed size covering the remaining original blocks.
    let b_body_len = (body_end - split_at) as u64;
    out.extend_from_slice(&CLUSTER_ID);
    out.extend_from_slice(&encode_size(b_body_len, size_len));
    out.extend_from_slice(&data[split_at..body_end]);

    out.extend_from_slice(&data[body_end..]);

    // The rebuild inserts one extra Cluster header (id + size VINT) that
    // wasn't in the original file, so the enclosing Segment's own (real,
    // known-size) size field must grow by that many bytes too, or its
    // declared body would no longer reach the file's actual end.
    let extra = (id_len + size_len) as u64;
    let seg_pos = data
        .windows(4)
        .position(|w| w == SEGMENT_ID)
        .expect("fixture must contain a Segment");
    let (_seg_id, seg_id_len) = read_id(data, seg_pos);
    let (seg_size, seg_size_len) = read_size(data, seg_pos + seg_id_len);
    let seg_size_field_start = seg_pos + seg_id_len;
    out[seg_size_field_start..seg_size_field_start + seg_size_len]
        .copy_from_slice(&encode_size(seg_size + extra, seg_size_len));

    out
}

#[derive(Debug)]
struct OracleRow {
    codec_type: String,
    size: usize,
}

fn load_oracle() -> Vec<OracleRow> {
    let text = std::fs::read_to_string(ORACLE).expect("read oracle csv");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            OracleRow {
                codec_type: f[0].to_string(),
                size: f[5].trim().parse().unwrap(),
            }
        })
        .collect()
}

/// Before the C10 fix: `Codec::Video`... no — `next_element` on the unknown
/// -size Cluster A ran to the end of the buffer, so `walk_cluster` was
/// handed Cluster A's real blocks PLUS Cluster B's raw (id, size, body)
/// bytes appended after them; Cluster B's `CLUSTER` id doesn't match any
/// arm `walk_cluster` recognizes, so its `_ => {}` catch-all silently
/// skipped the whole thing as one opaque unrecognized child — Cluster B's
/// blocks never became samples. Only the first half of the real 151 blocks
/// (see `webm_demux.rs`'s oracle) would demux.
#[test]
fn unknown_size_cluster_does_not_swallow_the_next_cluster() {
    let original = std::fs::read(FIXTURE).expect("read webm fixture");
    let rebuilt = split_into_two_clusters(&original);

    // Sanity: the rebuild really did produce two Cluster occurrences.
    let cluster_count = rebuilt.windows(4).filter(|w| *w == CLUSTER_ID).count();
    assert_eq!(cluster_count, 2, "rebuilt fixture must have 2 Clusters");

    let mut d = WebmDemux::new();
    let media: Media = d.demux(&rebuilt).expect("demux the rebuilt fixture");

    let oracle = load_oracle();
    let vid_oracle: Vec<&OracleRow> = oracle.iter().filter(|r| r.codec_type == "video").collect();
    let aud_oracle: Vec<&OracleRow> = oracle.iter().filter(|r| r.codec_type == "audio").collect();

    let vid = &media.tracks[0];
    let aud = &media.tracks[1];
    assert_eq!(
        vid.samples.len(),
        vid_oracle.len(),
        "every video block across both Clusters must be recovered"
    );
    assert_eq!(
        aud.samples.len(),
        aud_oracle.len(),
        "every audio block across both Clusters must be recovered"
    );

    // Byte lengths must still match the real per-block oracle, in order —
    // proves the split didn't corrupt any block, only re-framed them.
    for (i, (s, o)) in vid.samples.iter().zip(vid_oracle.iter()).enumerate() {
        assert_eq!(s.data.len(), o.size, "video block {i} byte length");
    }
    for (i, (s, o)) in aud.samples.iter().zip(aud_oracle.iter()).enumerate() {
        assert_eq!(s.data.len(), o.size, "audio block {i} byte length");
    }

    // The re-framed file must still mux cleanly end to end (no leftover
    // stray bytes from the second Cluster corrupting a later sample).
    let mut mux = CmafMux::default();
    let cmaf = mux.package(&media).expect("mux to fmp4");
    let mut off = 0usize;
    let mut saw_moov = false;
    while off < cmaf.len() {
        let (b, consumed) = parse_box(&cmaf[off..]).expect("parse top-level box");
        if b.header.box_type.is(b"moov") {
            saw_moov = true;
        }
        off += consumed;
    }
    assert!(saw_moov, "muxed output must contain a moov box");
}

/// Sanity check on the test's own construction: the mutation is not a no-op
/// (the rebuilt bytes differ from the original, and specifically carry the
/// unknown-size marker where the original had a real size).
#[test]
fn rebuild_helper_actually_changes_the_size_field() {
    let original = std::fs::read(FIXTURE).expect("read webm fixture");
    let rebuilt = split_into_two_clusters(&original);
    assert_ne!(
        original.len(),
        rebuilt.len(),
        "rebuild must change file length"
    );

    let cluster_pos = original.windows(4).position(|w| w == CLUSTER_ID).unwrap();
    let (_id, id_len) = read_id(&original, cluster_pos);
    let (_size, size_len) = read_size(&original, cluster_pos + id_len);
    let rebuilt_size_field = &rebuilt[cluster_pos + id_len..cluster_pos + id_len + size_len];
    assert_eq!(
        rebuilt_size_field,
        unknown_size(size_len),
        "first Cluster's size field must be the unknown-size marker"
    );
}
