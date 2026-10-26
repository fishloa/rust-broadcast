//! fMP4 demux of `init ++ segment` (ISO/IEC 14496-12), the `hls-runtime` and
//! multimux `dash_pull` pattern: a fresh [`Fmp4Demux`] per segment, so no
//! state leaks between segments and any segment can be retried alone.

use broadcast_common::Unpackage;
use transmux::{Fmp4Demux, InputDegradation, Sample, SkippedTrack, TrackSpec};

/// The samples of one segment, by the `tkhd.track_ID` of the init they were
/// demuxed with.
#[derive(Debug)]
pub(crate) struct Staged {
    pub(crate) tracks: Vec<(u32, Vec<Sample>)>,
    pub(crate) damaged: usize,
    pub(crate) skipped: Vec<SkippedTrack>,
}

/// The modelled tracks of an Initialization Segment, and what the demuxer
/// left out.
pub(crate) struct InitInfo {
    pub(crate) specs: Vec<TrackSpec>,
    pub(crate) skipped: Vec<SkippedTrack>,
}

pub(crate) fn init_info(init: &[u8]) -> Result<InitInfo, transmux::Error> {
    let media = Fmp4Demux::new().unpackage(init)?;
    Ok(InitInfo {
        specs: media.tracks.into_iter().map(|t| t.spec).collect(),
        skipped: media.skipped,
    })
}

pub(crate) fn demux_segment(init: &[u8], segment: &[u8]) -> Result<Staged, transmux::Error> {
    let mut combined = Vec::with_capacity(init.len().saturating_add(segment.len()));
    combined.extend_from_slice(init);
    combined.extend_from_slice(segment);
    let media = Fmp4Demux::new().unpackage(combined.as_slice())?;
    let damaged = media
        .degraded
        .iter()
        .filter(|d| matches!(d, InputDegradation::MalformedUnit { .. }))
        .count();
    Ok(Staged {
        tracks: media
            .tracks
            .into_iter()
            .map(|t| (t.spec.track_id.value(), t.samples))
            .collect(),
        damaged,
        skipped: media.skipped,
    })
}
