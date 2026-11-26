//! Drive the classic TS-HLS segmenter, and the muxer vocabulary it speaks, from
//! tests.

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use test_harness::collect::Collected as Media;
use transmux::mux_stage::{MuxInput, MuxOutput, SegmentOut};
use transmux::{Sample, StreamingTsHlsSegmenter, TrackId};

/// Feed one coded sample to `seg`.
pub fn feed(seg: &mut StreamingTsHlsSegmenter, track: TrackId, sample: Sample) {
    Stage::feed(seg, MuxInput::Sample { track, sample }, Timestamp::ZERO).expect("feed sample");
}

/// Finish `seg`, cutting its trailing partial segment.
pub fn finish(seg: &mut StreamingTsHlsSegmenter) {
    Stage::finish(seg).expect("finish segmenter");
}

/// Drain every segment `seg` has cut so far, in cut order.
pub fn drain(seg: &mut StreamingTsHlsSegmenter) -> Vec<SegmentOut> {
    let mut out = Vec::new();
    while let Some(item) = Stage::poll(seg) {
        if let MuxOutput::Segment(segment) = item {
            out.push(*segment);
        }
    }
    out
}

/// A segment's duration, from its summary's own timescale.
pub fn duration(segment: &SegmentOut) -> core::time::Duration {
    const NANOS_PER_SEC: u128 = 1_000_000_000;
    let nanos = u128::from(segment.summary.duration).saturating_mul(NANOS_PER_SEC)
        / u128::from(segment.summary.timescale.max(1));
    core::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// The classic-HLS output of one whole [`Media`]: every segment at
/// `target_secs`-second targets, and the VOD playlist naming all of them.
pub struct HlsOutput {
    /// The `.ts` media segments, in playlist order.
    pub segments: Vec<Vec<u8>>,
    /// The RFC 8216 media playlist (`#EXTM3U` … `#EXT-X-ENDLIST`).
    pub playlist: String,
}

/// Segment `media` into classic TS-HLS with a VOD playlist.
pub fn hls(media: &Media, target_secs: u32) -> HlsOutput {
    let specs: Vec<transmux::TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut seg = StreamingTsHlsSegmenter::new(specs, target_secs, usize::MAX).expect("segmenter");
    let outputs =
        test_harness::collect::drive(&mut seg, media, Timestamp::ZERO).expect("segment media");
    let mut segments = Vec::new();
    for output in outputs {
        if let MuxOutput::Segment(segment) = output {
            segments.push(segment.bytes.to_vec());
        }
    }
    HlsOutput {
        segments,
        playlist: seg.playlist().expect("playlist"),
    }
}
