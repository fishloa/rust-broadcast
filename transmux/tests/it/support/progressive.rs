//! Two-pass progressive packaging over a whole [`Media`], for tests that hold
//! one: [`PlanPass`] records the plan, then [`ProgressiveMuxStage`] writes it.
//!
//! The chunk length is the one the CLI uses, so an output here is the CLI's.

use broadcast_common::stage::Timestamp;
use isobmff::writer::{Interleave, Layout, MovieHeader, Sink};
use transmux::media::Media;
use transmux::mux_stage::{MuxOutput, drive_media};
use transmux::plan_pass::PlanPass;
use transmux::progressive::ProgressiveMuxStage;

/// The wall-clock length of one interleaved chunk, as the CLI chooses it.
pub const INTERLEAVE_MILLIS: u32 = 500;

/// Mux `media` into one progressive MP4, `faststart` placing `moov` first.
pub fn package(media: &Media, faststart: bool) -> Vec<u8> {
    package_with_sink(media, faststart, Sink::Seekable)
}

/// Mux `media` into one progressive MP4 for a caller whose sink is `sink`.
pub fn package_with_sink(media: &Media, faststart: bool, sink: Sink) -> Vec<u8> {
    let plan = PlanPass::for_media(media).expect("plan progressive");
    let layout = Layout::new(
        faststart,
        Interleave::ByDuration {
            millis: INTERLEAVE_MILLIS,
        },
        sink,
    );
    let movie = MovieHeader::new(media.movie_timescale);
    let mut stage = ProgressiveMuxStage::new(plan, layout, movie);
    let out = drive_media(&mut stage, media, Timestamp::ZERO).expect("package progressive");
    place(&out)
}

/// The output length `out` describes: the running end of the assembled file.
pub fn output_len(out: &[MuxOutput]) -> usize {
    let mut end = 0usize;
    for item in out {
        match item {
            MuxOutput::Bytes(chunk) | MuxOutput::Init(chunk) => {
                end = end.saturating_add(chunk.len());
            }
            MuxOutput::Segment(segment) => {
                end = end.saturating_add(segment.bytes.len());
            }
            MuxOutput::BytesAt {
                offset,
                bytes: chunk,
            } => {
                let offset = usize::try_from(*offset).expect("offset fits the address space");
                end = end.max(offset.saturating_add(chunk.len()));
            }
            _ => {}
        }
    }
    end
}

/// Assemble a muxer's output: appends in order, offsets placed in a buffer.
pub fn place(out: &[MuxOutput]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(output_len(out));
    for item in out {
        match item {
            MuxOutput::Bytes(chunk) => bytes.extend_from_slice(chunk),
            MuxOutput::BytesAt {
                offset,
                bytes: chunk,
            } => {
                let start = usize::try_from(*offset).expect("offset fits the address space");
                let end = start.checked_add(chunk.len()).expect("write fits");
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[start..end].copy_from_slice(chunk);
            }
            MuxOutput::Init(chunk) => bytes.extend_from_slice(chunk),
            MuxOutput::Segment(segment) => bytes.extend_from_slice(&segment.bytes),
            MuxOutput::Text(_) => {}
            _ => {}
        }
    }
    bytes
}
