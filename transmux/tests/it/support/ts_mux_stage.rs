//! Drive the one TS muxer stage from a whole [`Media`], the shape a test that
//! already holds the IR needs.
//!
//! Every call announces the media's tracks, feeds its samples in decode-time
//! order and finishes the stage ([`drive`](test_harness::collect::drive)), then concatenates the
//! [`MuxOutput::Bytes`] chunks into the stream a caller would write.

use broadcast_common::stage::Timestamp;
use core::time::Duration;
use test_harness::collect::Collected as Media;
use test_harness::collect::drive;
use transmux::mux_stage::MuxOutput;
use transmux::{DEFAULT_PSI_INTERVAL, TsMuxStage};

/// Mux `media` with the usual PAT/PMT repetition ([`DEFAULT_PSI_INTERVAL`]).
pub fn mux(media: &Media) -> Vec<u8> {
    mux_with(media, Some(DEFAULT_PSI_INTERVAL))
}

/// Mux `media` repeating the PAT/PMT every `psi_interval` of media time
/// (`None` writes them once only).
pub fn mux_with(media: &Media, psi_interval: Option<Duration>) -> Vec<u8> {
    try_mux_with(media, psi_interval).expect("mux to TS")
}

/// [`mux_with`], returning the muxer's own error instead of panicking.
pub fn try_mux_with(
    media: &Media,
    psi_interval: Option<Duration>,
) -> Result<Vec<u8>, transmux::Error> {
    let mut stage = TsMuxStage::new(psi_interval);
    try_drive(&mut stage, media)
}

/// [`drive`], returning the muxer's own error instead of panicking.
pub fn try_drive(stage: &mut TsMuxStage, media: &Media) -> Result<Vec<u8>, transmux::Error> {
    let outputs = drive(stage, media, Timestamp::ZERO)?;
    Ok(bytes_of(outputs))
}

/// Concatenate the byte chunks of `outputs`, reserving their total size first.
pub fn bytes_of(outputs: Vec<MuxOutput>) -> Vec<u8> {
    let total: usize = outputs
        .iter()
        .map(|output| match output {
            MuxOutput::Bytes(chunk) => chunk.len(),
            _ => 0,
        })
        .sum();
    let mut bytes = Vec::with_capacity(total);
    for output in outputs {
        if let MuxOutput::Bytes(chunk) = output {
            bytes.extend_from_slice(&chunk);
        }
    }
    bytes
}
