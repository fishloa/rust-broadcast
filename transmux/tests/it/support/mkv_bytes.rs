//! Whole-file helpers for the Matroska tests.
//!
//! [`MkvMuxStage`] writes the `Segment` with the RFC 8794 §4 "unknown size"
//! encoding (its length is a whole-stream fact the stage never holds) and
//! patches `Info.Duration` at `finish`. The pre-refactor whole-file writer wrote
//! the same elements with a minimal-width size field, so
//! [`whole_file_equivalent`] re-encodes just that one field: everything else,
//! which is what the goldens pin, is byte-identical.

use broadcast_common::stage::{Stage, Timestamp};
use test_harness::collect::Collected as Media;
use transmux::MkvMuxStage;
use transmux::mux_stage::{MuxInput, MuxOutput};

/// The `Cluster` span the pre-refactor whole-file writer used for a media with
/// a video track (a new `Cluster` also starts at every video keyframe).
const CLUSTER_SPAN_WITH_VIDEO_MS: u64 = 5_000;
/// The same for an audio-only media.
const CLUSTER_SPAN_AUDIO_ONLY_MS: u64 = 1_000;

/// A stage configured the way the whole-file writer configured itself.
pub fn stage(media: &Media) -> MkvMuxStage {
    let span = if media
        .tracks
        .iter()
        .any(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
    {
        CLUSTER_SPAN_WITH_VIDEO_MS
    } else {
        CLUSTER_SPAN_AUDIO_ONLY_MS
    };
    MkvMuxStage::new(core::time::Duration::from_millis(span))
}

/// A `Segment` element ID (RFC 9559 §11).
const SEGMENT_ID: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
/// The EBML "unknown size" marker (RFC 8794 §4).
const UNKNOWN_SIZE: [u8; 8] = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
/// Milliseconds per second: the `TimestampScale` this writer emits, in ticks.
const MS_PER_SECOND: i128 = 1_000;
/// One EBML VINT byte contributes seven data bits (RFC 8794 §4).
const VINT_DATA_BITS: u32 = 7;

/// Rewrite the stage's `Segment` size field as the minimal-width VINT of the
/// payload that follows it — the encoding the whole-file writer produced.
pub fn whole_file_equivalent(bytes: &[u8]) -> Vec<u8> {
    let at = bytes
        .windows(SEGMENT_ID.len())
        .position(|window| window == SEGMENT_ID)
        .expect("a Segment element");
    let marker = at + SEGMENT_ID.len();
    assert_eq!(
        &bytes[marker..marker + UNKNOWN_SIZE.len()],
        &UNKNOWN_SIZE,
        "the stage writes the unknown-size marker"
    );
    let payload = &bytes[marker + UNKNOWN_SIZE.len()..];
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..marker]);
    out.extend_from_slice(&minimal_vint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

/// The minimal-width EBML size VINT for `value` (RFC 8794 §4).
fn minimal_vint(value: u64) -> Vec<u8> {
    let mut width = 1usize;
    while width < 8 && value > ((1u64 << (width as u32 * VINT_DATA_BITS)) - 1) {
        width += 1;
    }
    let marker = 1u64 << (width as u32 * VINT_DATA_BITS);
    (marker | value).to_be_bytes()[8 - width..].to_vec()
}

/// Drive `stage` with `media`'s samples merged by **presentation** time across
/// tracks — the order the pre-refactor whole-file writer placed `SimpleBlock`s
/// in — and collect the byte stream.
///
/// [`drive`](test_harness::collect::drive) merges by decode time; a
/// B-frame track's presentation order is not its decode order, so the two
/// orders interleave tracks differently and the resulting files are not
/// byte-comparable to the pre-refactor goldens. The merge here is the old
/// writer's own rule: repeatedly take the track whose *next* sample has the
/// smallest presentation time, ties to the earlier track, never reordering a
/// track's own samples.
pub fn mux_pts_ordered(stage: &mut MkvMuxStage, media: &Media) -> Vec<u8> {
    for track in &media.tracks {
        stage
            .feed(
                MuxInput::Track(Box::new(track.spec.clone())),
                Timestamp::ZERO,
            )
            .expect("announce track");
    }
    let mut cursors = vec![0usize; media.tracks.len()];
    loop {
        let mut best: Option<(usize, i64)> = None;
        for (index, track) in media.tracks.iter().enumerate() {
            let Some(sample) = track.samples.get(cursors[index]) else {
                continue;
            };
            let ticks = sample
                .timing
                .and_then(|t| t.pts())
                .or(sample.timing.and_then(|t| t.dts()))
                .unwrap_or(0);
            // Every track's own timescale rescaled to the Matroska tick this
            // writer emits (1 ms), so the comparison is across tracks.
            let pts = i128::from(ticks) * MS_PER_SECOND / i128::from(track.spec.timescale.get());
            let pts = i64::try_from(pts).unwrap_or(i64::MAX);
            if best.is_none_or(|(_, best_pts)| pts < best_pts) {
                best = Some((index, pts));
            }
        }
        let Some((index, _)) = best else {
            break;
        };
        let sample = media.tracks[index].samples[cursors[index]].clone();
        cursors[index] += 1;
        stage
            .feed(
                MuxInput::Sample {
                    track: media.tracks[index].spec.track_id,
                    sample,
                },
                Timestamp::ZERO,
            )
            .expect("feed sample");
    }
    stage.finish().expect("finish");
    let mut outputs = Vec::new();
    while let Some(output) = stage.poll() {
        outputs.push(output);
    }
    let mut bytes: Vec<u8> = Vec::new();
    for output in outputs {
        match output {
            MuxOutput::Bytes(chunk) | MuxOutput::Init(chunk) => bytes.extend_from_slice(&chunk),
            MuxOutput::Segment(segment) => bytes.extend_from_slice(&segment.bytes),
            MuxOutput::BytesAt {
                offset,
                bytes: patch,
            } => {
                let start = usize::try_from(offset).expect("offset fits usize");
                let end = start + patch.len();
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[start..end].copy_from_slice(&patch);
            }
            _ => {}
        }
    }
    bytes
}
