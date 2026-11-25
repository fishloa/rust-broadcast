#![cfg(feature = "std")]
//! A 10 000-segment VOD renders its MPD with memory independent of the segment
//! count: the summary log run-length-codes the constant-rate segments into one
//! entry per track, so neither the log nor the renderer grows with the count.
//!
//! Budget: [`ThreadCapped`] caps this test thread at `2 << 20` bytes (2 MiB)
//! of live allocations for the whole test. The measurement is that the log
//! retains one run for all 10 000 segments (a per-segment log would hold
//! 10 000 entries) and the rendered MPD measures 879 bytes (asserted under
//! 2048); the cap is a constant, never scaled by `SEGMENTS`.

use broadcast_common::timescale::Timescale;
use bytes::Bytes;
use test_alloc::ThreadCapped;
use transmux::dash::DashPackager;
use transmux::ir::{CodecConfig, TrackId, TrackSpec};
use transmux::mux_stage::{SegmentOut, SegmentSummary, TrackPart};
use transmux::segment_log::SegmentSummaryLog;

#[global_allocator]
static ALLOC: ThreadCapped<{ 2 << 20 }> = ThreadCapped::new();

const SEGMENTS: u64 = 10_000;
const SEGMENT_DURATION: u64 = 100;
const TIMESCALE: u32 = 1000;
const MAX_MPD_BYTES: usize = 2048;

fn aac_spec() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(1),
        Timescale::try_new(TIMESCALE).expect("a non-zero timescale"),
        CodecConfig::Aac {
            esds: isobmff::mp4esds::EsdsBox::new(isobmff::mp4esds::ESDescriptor::new(
                1,
                0,
                Some(isobmff::mp4esds::DecoderConfigDescriptor::new(
                    0x40,
                    0x05,
                    false,
                    0,
                    0,
                    0,
                    Some(isobmff::mp4esds::DecoderSpecificInfo::new(vec![0x12, 0x10])),
                )),
                Some(isobmff::mp4esds::SLConfigDescriptor::predefined_two()),
            )),
            channel_count: 2,
            sample_rate: TIMESCALE,
            sample_size: 16,
        },
    )
}

fn segment(track: TrackId, sequence: u64) -> SegmentOut {
    let start = sequence * SEGMENT_DURATION;
    SegmentOut {
        bytes: Bytes::new(),
        summary: SegmentSummary {
            track,
            sequence,
            start,
            duration: SEGMENT_DURATION,
            timescale: TIMESCALE,
            bytes: 10,
            independent: true,
            discontinuous: false,
        },
        tracks: vec![TrackPart {
            track,
            start,
            duration: SEGMENT_DURATION,
            timescale: TIMESCALE,
            bytes: 10,
            samples: 1,
        }],
    }
}

#[test]
fn ten_thousand_segment_vod_mpd_is_constant_memory() {
    let spec = aac_spec();
    let mut log = SegmentSummaryLog::new();
    for sequence in 0..SEGMENTS {
        let out = segment(spec.track_id, sequence);
        log.push(&out).expect("push one segment summary");
    }

    let summary = log.track(spec.track_id).expect("the track has summaries");
    assert_eq!(
        summary.segments, SEGMENTS,
        "every segment is recorded in the totals"
    );
    assert_eq!(
        summary.runs().count(),
        1,
        "constant-rate contiguous segments collapse into one run"
    );

    let mpd = DashPackager::default()
        .render(core::slice::from_ref(&spec), &log)
        .expect("render the long VOD MPD");
    assert!(
        mpd.len() < MAX_MPD_BYTES,
        "the MPD stays {} bytes for {} segments, under the {MAX_MPD_BYTES}-byte bound",
        mpd.len(),
        SEGMENTS
    );
}
