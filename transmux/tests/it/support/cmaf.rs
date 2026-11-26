//! Whole-`Media` CMAF packaging for tests: drives [`CmafMuxStage`] through
//! [`drive`](test_harness::collect::drive) and concatenates what it emits — the test-side bridge for a
//! caller that still holds a whole `Media`.

use broadcast_common::Timestamp;
use test_harness::collect::Collected as Media;
use test_harness::collect::drive;
use transmux::media::{CmafMuxStage, FragmentCut};
use transmux::mux_stage::MuxOutput;

/// One CMAF init segment followed by one media segment.
pub fn package(media: &Media) -> transmux::Result<Vec<u8>> {
    package_seq(media, 1)
}

/// One CMAF init segment followed by one media segment, numbered from
/// `sequence_number`.
pub fn package_seq(media: &Media, sequence_number: u32) -> transmux::Result<Vec<u8>> {
    let mut stage = CmafMuxStage::new(media.movie_timescale, FragmentCut::OnFlush, sequence_number);
    let outputs = drive(&mut stage, media, Timestamp::ZERO)?;
    let mut bytes = Vec::new();
    for output in outputs {
        match output {
            MuxOutput::Init(part) | MuxOutput::Bytes(part) => bytes.extend_from_slice(&part),
            MuxOutput::Segment(segment) => bytes.extend_from_slice(&segment.bytes),
            _ => {}
        }
    }
    Ok(bytes)
}
