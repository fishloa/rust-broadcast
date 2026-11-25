//! Whole-input TS fold for this crate's own unit tests.
//!
//! A unit test in `src/` cannot reuse `test_harness::collect::media_from_ts`:
//! that helper links the library as an external crate, so the `Media` it
//! returns is a different type from `crate::Media`. The integration tests
//! (`tests/it/**`) use the harness copy; this module is the same fold over the
//! one streaming stage for the tests that live inside the crate.

use alloc::collections::BTreeMap;

use crate::ir::{DegradationRecord, DemuxEvent, Media, PcrSample, Track, TrackId};
use crate::rtp::VIDEO_CLOCK_TIMESCALE;
use crate::{AfdPersistence, InputDegradation, StreamingTsDemux};

/// Demux `bytes` (a whole MPEG-2 TS image) through [`StreamingTsDemux`] with
/// strict PMT track order and fold its events into one [`Media`].
pub(crate) fn media_from_ts(bytes: &[u8]) -> crate::Result<Media> {
    media_from_ts_with_afd(bytes, AfdPersistence::UntilReplaced)
}

/// [`media_from_ts`] with an explicit AFD persistence policy.
pub(crate) fn media_from_ts_with_afd(bytes: &[u8], afd: AfdPersistence) -> crate::Result<Media> {
    let mut demux = StreamingTsDemux::new()
        .with_afd_persistence(afd)
        .with_strict_pmt_order();
    demux.feed(bytes)?;
    demux.finish()?;

    let mut tracks: alloc::vec::Vec<Track> = alloc::vec::Vec::new();
    let mut index_by_id: BTreeMap<TrackId, usize> = BTreeMap::new();
    let mut pcr: alloc::vec::Vec<PcrSample> = alloc::vec::Vec::new();
    let mut degraded: alloc::vec::Vec<InputDegradation> = alloc::vec::Vec::new();
    let mut degraded_at: alloc::vec::Vec<DegradationRecord> = alloc::vec::Vec::new();
    while let Some(event) = demux.poll_event() {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                index_by_id.insert(spec.track_id, tracks.len());
                tracks.push(Track::new(spec, alloc::vec::Vec::new()));
            }
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(&index) = index_by_id.get(&spec.track_id) {
                    tracks[index].spec = spec;
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&index) = index_by_id.get(&track_id) {
                    let track = &mut tracks[index];
                    if track.samples.is_empty()
                        && let Some(dts) = sample.timing.and_then(|timing| timing.dts())
                    {
                        track.start_decode_time = u64::try_from(dts).unwrap_or(0);
                    }
                    track.samples.push(sample);
                }
            }
            DemuxEvent::ClockReference {
                pcr: wire,
                discontinuous,
                provenance,
                ..
            } => {
                if let (Some(wire), Some(pid), Some(packet_index)) =
                    (wire, provenance.pid, provenance.packet_index)
                {
                    pcr.push(PcrSample::new(wire, pid, packet_index, discontinuous));
                }
            }
            DemuxEvent::InputDegraded {
                track,
                kind,
                provenance,
                ..
            } => {
                degraded.push(kind);
                degraded_at.push(DegradationRecord::new(kind, track, provenance));
            }
            _ => {}
        }
    }
    for record in &mut degraded_at {
        if record.track.is_none()
            && let Some(pid) = record.provenance.pid
            && let Some(track) = tracks
                .iter()
                .find(|track| track.spec.source_pid == Some(pid))
        {
            record.track = Some(track.spec.track_id);
        }
    }
    let mut media = Media::new(tracks, VIDEO_CLOCK_TIMESCALE).with_pcr(pcr);
    media.degraded = degraded;
    media.degraded_at = degraded_at;
    Ok(media)
}

/// Demux `bytes` (a whole FLV image) through
/// [`StreamingFlvDemux`](crate::StreamingFlvDemux) and fold its events into one
/// [`Media`] on FLV's 1 kHz clock.
pub(crate) fn media_from_flv(bytes: &[u8]) -> core::result::Result<Media, crate::flv::FlvError> {
    let mut demux = crate::StreamingFlvDemux::new();
    demux.feed(bytes)?;
    demux.finish()?;
    let mut tracks: alloc::vec::Vec<Track> = alloc::vec::Vec::new();
    let mut index_by_id: BTreeMap<TrackId, usize> = BTreeMap::new();
    while let Some(event) = demux.poll_event() {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                index_by_id.insert(spec.track_id, tracks.len());
                tracks.push(Track::new(spec, alloc::vec::Vec::new()));
            }
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(&index) = index_by_id.get(&spec.track_id) {
                    tracks[index].spec = spec;
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&index) = index_by_id.get(&track_id) {
                    let track = &mut tracks[index];
                    if track.samples.is_empty()
                        && let Some(dts) = sample.timing.and_then(|timing| timing.dts())
                    {
                        track.start_decode_time = u64::try_from(dts).unwrap_or(0);
                    }
                    track.samples.push(sample);
                }
            }
            _ => {}
        }
    }
    let mut media = Media::new(tracks, crate::rtp::VIDEO_CLOCK_TIMESCALE);
    if let Some(track) = media.tracks.first() {
        media.movie_timescale = track.spec.timescale;
    }
    Ok(media)
}
