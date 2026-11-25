//! A sequential stage releases its buffer at every segment end: the init's
//! capacity never becomes the first segment's floor, and a large segment's
//! capacity never survives into the next one.
//!
//! One `#[test]` because `ProcessPeak` is process-global.

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use test_fixtures::read;
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};

#[global_allocator]
static PEAK: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// The init a segment group runs under.
const INIT: &str = "hls/cmaf-fmp4/init.mp4";
/// A large media segment of the same group.
const LARGE_SEGMENT: &str = "hls/cmaf-fmp4/index0.m4s";
/// A large init padded far past any segment, so retaining it would dominate.
const PADDING: usize = 2 * 1024 * 1024;
/// Live bytes the segment demux may add over the segment's own size (the
/// bounded structures it parses plus the samples it hands out).
const PEAK_SLACK: usize = 1024 * 1024;
/// A box header's size in bytes (ISO/IEC 14496-12:2026 §4.2).
const BOX_HEADER: usize = 8;

/// The init bytes followed by one `free` box of `PADDING` zero bytes, so the
/// stage's buffer grows to the padding before the first segment arrives.
fn padded_init(init: &[u8]) -> Vec<u8> {
    let mut bytes = init.to_vec();
    let size = u32::try_from(BOX_HEADER + PADDING).expect("padded init size");
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(b"free");
    bytes.resize(bytes.len() + PADDING, 0);
    bytes
}

#[test]
fn a_segment_end_releases_the_buffer() {
    let init = padded_init(&read(INIT));
    let init_len = init.len();
    let segment = read(LARGE_SEGMENT);

    let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
    demux
        .feed(Mp4Input::Next(&init), Timestamp::ZERO)
        .expect("feed init");
    demux
        .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
        .expect("init segment end");
    assert_eq!(
        demux.buffered(),
        (0, 0),
        "the init's buffer must not survive its segment end"
    );

    drop(init);
    let (peak, ()) = PEAK.peak_during(|| {
        demux
            .feed(Mp4Input::Next(&segment), Timestamp::ZERO)
            .expect("feed segment");
        demux
            .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
            .expect("segment end");
    });
    assert_eq!(
        demux.buffered(),
        (0, 0),
        "a segment's buffer must not survive its segment end"
    );
    assert!(
        peak < init_len + segment.len(),
        "the demux of one segment added {peak} live bytes, a retained {init_len}-byte init plus \
         the segment's {} bytes",
        segment.len()
    );
    assert!(
        peak < segment.len() + PEAK_SLACK,
        "the demux of one segment added {peak} live bytes over its {} bytes",
        segment.len()
    );
}
