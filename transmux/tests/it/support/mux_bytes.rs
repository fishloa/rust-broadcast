//! Drive a muxer stage over a whole [`Media`] and collect its byte stream.
//!
//! [`drive_media`] is the bridge from a whole `Media` to a muxer stage, whose
//! output is a stream of [`MuxOutput`] units, some of them `BytesAt` patches. These tests compare
//! bytes, so they apply the units to a buffer the way a seekable output file
//! would.

use broadcast_common::stage::Timestamp;
use transmux::Media;
use transmux::mux_stage::{MuxInput, MuxOutput, drive_media};

/// The byte stream `stage` produces for `media`, with every `BytesAt` patch
/// applied in place.
pub fn mux_bytes<S>(stage: &mut S, media: &Media) -> Vec<u8>
where
    S: for<'a> broadcast_common::stage::Stage<In<'a> = MuxInput, Out = MuxOutput>,
    S::Error: core::fmt::Debug,
{
    mux_result(stage, media).expect("mux stage drive")
}

/// [`mux_bytes`] leaving the stage's error to the caller.
pub fn mux_result<S>(stage: &mut S, media: &Media) -> Result<Vec<u8>, S::Error>
where
    S: for<'a> broadcast_common::stage::Stage<In<'a> = MuxInput, Out = MuxOutput>,
{
    drive_media(stage, media, Timestamp::ZERO).map(apply)
}

/// Apply `outputs` to a byte buffer, in order, as a seekable sink would.
pub fn apply(outputs: Vec<MuxOutput>) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    for output in outputs {
        match output {
            MuxOutput::Bytes(chunk) => bytes.extend_from_slice(&chunk),
            MuxOutput::Init(chunk) => bytes.extend_from_slice(&chunk),
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
