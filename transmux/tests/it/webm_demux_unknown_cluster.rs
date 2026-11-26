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
//! `fixtures/mkv/vp9-opus.webm` (real ffmpeg VP9+Opus WebM, oracle
//! `fixtures/mkv/vp9-opus.packets.csv`, 151 total blocks: 1 `Timestamp` +
//! 150... see below) and re-frames its single real Cluster into two Clusters
//! at a real `SimpleBlock` boundary — every payload byte stays exactly as
//! ffmpeg wrote it; only the Cluster-level EBML framing is edited, and the
//! first Cluster's size is overwritten with RFC 8794 §6.2's "unknown size"
//! reserved VINT encoding (all data bits set) instead of its real size. This
//! is the same construction a live encoder produces, built directly from the
//! spec clause rather than an unavailable tool.
//!
//! The oracle is the fixture's own already-verified total block count and
//! per-block byte lengths (`fixtures/mkv/vp9-opus.packets.csv`, used
//! identically by `transmux/tests/it/webm_demux.rs`): re-framing must not change
//! how many blocks exist or their contents, only whether the demuxer finds
//! all of them.

use isobmff::box_types::parse_box;
use test_fixtures::ffprobe;
use test_harness::collect::Collected as Media;
use test_harness::collect::{CollectMode, media_from_matroska};

fn demux_webm(bytes: &[u8]) -> Result<Media, transmux::Error> {
    media_from_matroska(bytes, CollectMode::Seekable)
}

const FIXTURE: &str = "mkv/vp9-opus.webm";
const ORACLE: &str = "mkv/vp9-opus.packets.csv";

const CLUSTER_ID: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
const SEGMENT_ID: [u8; 4] = [0x18, 0x53, 0x80, 0x67];

/// `SeekHead` (RFC 9559 §20) and the ids inside it, folded with their marker
/// bits so they compare to [`read_id`]'s output.
const SEEK_HEAD_ID: u32 = 0x114D_9B74;
/// A `Seek` element inside `SeekHead`.
const SEEK_ID: u32 = 0x4D_BB;
/// `SeekID` (the element id a `Seek` points at).
const SEEK_SEEK_ID: u32 = 0x53_AB;
/// `SeekPosition` (relative to the Segment payload start).
const SEEK_POSITION_ID: u32 = 0x53_AC;
/// The `Cues` element id these rebuilds move.
const CUES_ID: u32 = 0x1C53_BB6B;

/// Add `extra` bytes to the `SeekHead`'s `Cues` `SeekPosition`, because the
/// rebuild inserts bytes inside the Segment before `Cues` and the recorded
/// position is relative to the Segment payload start.
fn shift_cues_seek_position(bytes: &mut [u8], extra: u64) {
    let seg_pos = bytes
        .windows(4)
        .position(|w| w == SEGMENT_ID)
        .expect("fixture must contain a Segment");
    let (_seg_id, seg_id_len) = read_id(bytes, seg_pos);
    let (_seg_size, seg_size_len) = read_size(bytes, seg_pos + seg_id_len);
    let payload = seg_pos + seg_id_len + seg_size_len;
    let seek_head = bytes
        .windows(4)
        .position(|w| w == SEEK_HEAD_ID.to_be_bytes());
    let Some(seek_head) = seek_head else {
        return;
    };
    let (_id, id_len) = read_id(bytes, seek_head);
    let (head_size, head_size_len) = read_size(bytes, seek_head + id_len);
    let mut p = seek_head + id_len + head_size_len;
    let head_end = p + head_size as usize;
    while p < head_end {
        let (id, id_len) = read_id(bytes, p);
        let (size, size_len) = read_size(bytes, p + id_len);
        let body = p + id_len + size_len;
        let body_end = body + size as usize;
        if id == SEEK_ID {
            let mut q = body;
            let mut is_cues = false;
            let mut position_field = None;
            while q < body_end {
                let (cid, cid_len) = read_id(bytes, q);
                let (csize, csize_len) = read_size(bytes, q + cid_len);
                let cb = q + cid_len + csize_len;
                let csize = csize as usize;
                if cid == SEEK_SEEK_ID && bytes[cb..cb + csize] == CUES_ID.to_be_bytes() {
                    is_cues = true;
                }
                if cid == SEEK_POSITION_ID {
                    position_field = Some((cb, csize));
                }
                q = cb + csize;
            }
            if is_cues && let Some((at, width)) = position_field {
                let mut value = 0u64;
                for &byte in &bytes[at..at + width] {
                    value = (value << 8) | u64::from(byte);
                }
                let shifted = value.saturating_add(extra);
                for index in 0..width {
                    bytes[at + index] = (shifted >> (8 * (width - 1 - index))) as u8;
                }
            }
        }
        p = body_end;
    }
    let _ = payload;
}

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

/// Rebuild `fixtures/mkv/vp9-opus.webm` with its single real Cluster split
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
    shift_cues_seek_position(&mut out, extra);

    out
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
    let original = std::fs::read(test_fixtures::path(FIXTURE)).expect("read webm fixture");
    let rebuilt = split_into_two_clusters(&original);

    // Sanity: the rebuild really did produce two Cluster occurrences.
    let cluster_count = rebuilt.windows(4).filter(|w| *w == CLUSTER_ID).count();
    assert_eq!(cluster_count, 2, "rebuilt fixture must have 2 Clusters");

    let media: Media = demux_webm(&rebuilt).expect("demux the rebuilt fixture");

    let oracle = ffprobe::packets(ORACLE);
    let vid_oracle = ffprobe::of_type(&oracle, "video");
    let aud_oracle = ffprobe::of_type(&oracle, "audio");

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
    let cmaf = crate::cmaf::package(&media).expect("mux to fmp4");
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
    let original = std::fs::read(test_fixtures::path(FIXTURE)).expect("read webm fixture");
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

/// `Void` element id (RFC 9559 §27): skipped by size, so it carries the decoy
/// bytes without the cluster walker trying to decode them.
const VOID_ID: u8 = 0xEC;

/// Which size the decoy Cluster-id occurrence advertises.
#[derive(Clone, Copy)]
enum DecoySize {
    /// A size that runs past the end of the file: not a boundary.
    OverLarge,
    /// The empty body: a structurally valid sibling header.
    Zero,
}

/// Re-frame the fixture's single Cluster into two (the first unknown-size, the
/// second real) with a decoy `Cluster` id + size VINT inserted inside a `Void`
/// element at a real child boundary of the first Cluster.
///
/// The decoy's `fits` test is what decides it: the body start is
/// `position + id_len + size_len`, so a size chosen as "file length minus
/// position minus 4" is rejected by the correct arithmetic (it ends 8 bytes
/// past the buffer) but accepted if either `+` is weakened to a smaller
/// expression.
fn split_with_decoy(data: &[u8], decoy_size: DecoySize) -> Vec<u8> {
    let cluster_pos = data
        .windows(4)
        .position(|w| w == CLUSTER_ID)
        .expect("fixture must contain a Cluster");
    let (_id, id_len) = read_id(data, cluster_pos);
    let (cluster_size, size_len) = read_size(data, cluster_pos + id_len);
    let body_start = cluster_pos + id_len + size_len;
    let body_end = body_start + cluster_size as usize;

    let mut child_starts = Vec::new();
    let mut p = body_start;
    while p < body_end {
        child_starts.push(p);
        let (_id, idlen) = read_id(data, p);
        let (size, sizelen) = read_size(data, p + idlen);
        p = p + idlen + sizelen + size as usize;
    }
    assert!(
        child_starts.len() >= 3,
        "need several children to place the decoy, got {}",
        child_starts.len()
    );
    // Cluster B starts at the middle child; the decoy sits earlier, so an
    // accepted decoy boundary truncates Cluster A before some of its blocks
    // (which B's later half does not re-supply).
    let split_index = child_starts.len() / 2;
    let split_at = child_starts[split_index];
    let decoy_at = child_starts[split_index / 2];

    let extra_cluster_header = id_len + size_len;
    let void_len = 2 + 4 + 8; // id + one-byte size + (Cluster id + 8-byte size)
    let total = data.len() + extra_cluster_header + void_len;
    // The Void id and its one-byte size sit before the decoy itself.
    let q = decoy_at + 2;
    let decoy_value = match decoy_size {
        DecoySize::OverLarge => (total - q - 4) as u64,
        DecoySize::Zero => 0,
    };
    let mut decoy = CLUSTER_ID.to_vec();
    decoy.extend_from_slice(&encode_size(decoy_value, 8));

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&data[..cluster_pos]);
    out.extend_from_slice(&CLUSTER_ID);
    out.extend_from_slice(&unknown_size(size_len));
    out.extend_from_slice(&data[body_start..decoy_at]);
    out.push(VOID_ID);
    out.push(0x80 | decoy.len() as u8);
    out.extend_from_slice(&decoy);
    out.extend_from_slice(&data[decoy_at..split_at]);

    let b_body_len = (body_end - split_at) as u64;
    out.extend_from_slice(&CLUSTER_ID);
    out.extend_from_slice(&encode_size(b_body_len, size_len));
    out.extend_from_slice(&data[split_at..body_end]);
    out.extend_from_slice(&data[body_end..]);

    let extra = (extra_cluster_header + void_len) as u64;
    let seg_pos = data
        .windows(4)
        .position(|w| w == SEGMENT_ID)
        .expect("fixture must contain a Segment");
    let (_seg_id, seg_id_len) = read_id(data, seg_pos);
    let (seg_size, seg_size_len) = read_size(data, seg_pos + seg_id_len);
    let field = seg_pos + seg_id_len;
    out[field..field + seg_size_len].copy_from_slice(&encode_size(seg_size + extra, seg_size_len));
    shift_cues_seek_position(&mut out, extra);
    out
}

fn total_samples(media: &Media) -> usize {
    media.tracks.iter().map(|t| t.samples.len()).sum()
}

/// A `Cluster` id inside Cluster A's coded payload whose advertised size runs
/// past the buffer is a false positive, not the sibling boundary: the first
/// Cluster must keep every block and Cluster B must still be read.
#[test]
fn decoy_cluster_id_with_an_over_large_size_is_not_a_boundary() {
    let original = std::fs::read(test_fixtures::path(FIXTURE)).expect("read webm fixture");
    let clean = demux_webm(&original).expect("clean fixture");
    let rebuilt = split_with_decoy(&original, DecoySize::OverLarge);

    let media = demux_webm(&rebuilt).expect("a decoy Cluster id must not break the demux");
    assert_eq!(
        total_samples(&media),
        total_samples(&clean),
        "every block of both Clusters must be recovered"
    );
    assert!(
        media.degraded.is_empty(),
        "the decoy must be skipped, not reported: {:?}",
        media.degraded
    );
    for t in 0..clean.tracks.len() {
        assert_eq!(
            media.tracks[t]
                .samples
                .iter()
                .map(|s| s.data.clone())
                .collect::<Vec<_>>(),
            clean.tracks[t]
                .samples
                .iter()
                .map(|s| s.data.clone())
                .collect::<Vec<_>>(),
            "track {t} frames must be identical to the clean fixture"
        );
    }
}

/// Control: the same decoy with an empty body is a structurally valid sibling
/// header, so the boundary scan takes it and the first Cluster ends there. The
/// blocks after it are carried by the following Cluster, so none is lost.
#[test]
fn decoy_cluster_id_with_a_fitting_size_keeps_every_block() {
    let original = std::fs::read(test_fixtures::path(FIXTURE)).expect("read webm fixture");
    let clean = demux_webm(&original).expect("clean fixture");
    let rebuilt = split_with_decoy(&original, DecoySize::Zero);

    let media = demux_webm(&rebuilt).expect("the rebuilt file still demuxes");
    assert_eq!(
        total_samples(&media),
        total_samples(&clean),
        "the blocks after the accepted boundary still belong to a Cluster"
    );
    for t in 0..clean.tracks.len() {
        assert_eq!(
            media.tracks[t].samples.last().map(|s| s.data.clone()),
            clean.tracks[t].samples.last().map(|s| s.data.clone()),
            "track {t}: Cluster B after the boundary must still be read"
        );
    }
}

/// A decoy whose own size VINT is one byte wide pins the
/// `id_len + size_len` sum: weakening the second `+` to `*` evaluates to
/// `id_len * size_len` (4 here, not 5), so a size chosen to end one byte past
/// the buffer is accepted.
///
/// The decoy is near the end of an unknown-size Cluster with one real block
/// after it, so accepting it truncates the Cluster and loses that block.
#[test]
fn a_one_byte_decoy_size_pins_the_element_header_sum() {
    use crate::webm_build::{
        CLUSTER_TIMESTAMP, EBML_HEADER, SEGMENT, SIMPLE_BLOCK, TRACKS, element, encode_size,
        encode_size_width, track_entry,
    };

    let block = |byte: u8| {
        let mut payload = encode_size(1); // track-number VINT
        payload.extend_from_slice(&0i16.to_be_bytes());
        payload.push(0x80);
        payload.push(byte);
        element(SIMPLE_BLOCK, &payload)
    };
    let tracks = element(TRACKS, &track_entry(1, 1, b"V_VP9", &[]));
    let timestamp = element(CLUSTER_TIMESTAMP, &[0]);
    let before = block(0xAA);
    let after = block(0xBB);

    // The decoy's position within the Segment body (the reader's buffer):
    // Tracks, then the Cluster id + its one-byte unknown-size marker, then
    // Timestamp and the first block, then the Void id and its one-byte size.
    let q = tracks.len() + 4 + 1 + timestamp.len() + before.len() + 2;
    // After the decoy itself (its remaining 5 bytes) comes the second block.
    let l = q + 5 + after.len();
    let decoy_value = (l - q - 4) as u64;
    assert_eq!(decoy_value, 8, "a one-byte size VINT value");

    let mut decoy = CLUSTER_ID.to_vec();
    decoy.extend_from_slice(&encode_size_width(decoy_value, 1));
    let void = element(0xEC, &decoy); // EBML `Void`

    let mut cluster_body = timestamp;
    cluster_body.extend_from_slice(&before);
    cluster_body.extend_from_slice(&void);
    cluster_body.extend_from_slice(&after);
    let mut segment_body = tracks;
    segment_body.extend_from_slice(&CLUSTER_ID);
    segment_body.push(0xFF); // RFC 8794 §6.2 unknown size
    segment_body.extend_from_slice(&cluster_body);
    let mut file = element(EBML_HEADER, &[]);
    file.extend(element(SEGMENT, &segment_body));

    let media = demux_webm(&file).expect("the one-byte decoy must not break the demux");
    assert!(media.degraded.is_empty(), "{:?}", media.degraded);
    assert_eq!(media.tracks.len(), 1);
    assert_eq!(
        media.tracks[0].samples.len(),
        2,
        "a decoy ending one byte past the buffer is not a boundary"
    );
}
