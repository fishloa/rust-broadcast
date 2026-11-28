//! fMP4 demux of one Representation's init and media segments through the
//! streaming MP4 stage (ISO/IEC 14496-12:2026): the init is fed once and each
//! segment is fed to the same [`Mp4Demux`], never concatenated with it.
//!
//! A segment the stage refuses leaves the stage unusable, so the caller drops
//! it ([`SegmentStage`] is recreated from the init) and the next segment of the
//! same Representation demuxes as usual. A segment the reader merely tolerates
//! costs only itself: its fault arrives as the reader's one
//! [`DemuxEvent::Damage`] and the following segment continues under the same
//! init.

use std::collections::BTreeSet;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::{AbandonReason, DemuxEvent, InputDegradation, Sample, SkippedTrack, TrackSpec};

/// The samples of one segment, by the `tkhd.track_ID` of the init they were
/// demuxed with.
#[derive(Debug)]
pub(crate) struct Staged {
    pub(crate) tracks: Vec<(u32, Vec<Sample>)>,
    pub(crate) damaged: usize,
    pub(crate) skipped: Vec<SkippedTrack>,
    /// Every damage report the reader published for this segment, in order.
    pub(crate) damage: Vec<broadcast_core::damage::Damage>,
}

/// The modelled tracks of an Initialization Segment, and what the demuxer
/// left out.
pub(crate) struct InitInfo {
    pub(crate) specs: Vec<TrackSpec>,
    pub(crate) skipped: Vec<SkippedTrack>,
}

/// What one drain of the stage's events yields.
#[derive(Default)]
struct Collected {
    tracks: Vec<(u32, Vec<Sample>)>,
    skipped: Vec<SkippedTrack>,
    damaged: usize,
    damage: Vec<broadcast_core::damage::Damage>,
}

impl Collected {
    fn drain(&mut self, demux: &mut Mp4Demux, skip: &BTreeSet<u32>) {
        while let Some(event) = demux.poll() {
            self.on_event(event, skip);
        }
    }

    fn on_event(&mut self, event: DemuxEvent, skip: &BTreeSet<u32>) {
        match event {
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                let id = track_id.value();
                if skip.contains(&id) {
                    return;
                }
                match self.tracks.iter_mut().find(|(track, _)| *track == id) {
                    Some((_, samples)) => samples.push(sample),
                    None => self.tracks.push((id, vec![sample])),
                }
            }
            DemuxEvent::TrackAbandoned {
                track_id, reason, ..
            } => self.skipped.push(skipped_from_reason(reason, track_id)),
            DemuxEvent::InputDegraded { kind, .. } => {
                if matches!(
                    kind,
                    InputDegradation::MalformedUnit { .. } | InputDegradation::TruncatedUnit { .. }
                ) {
                    self.damaged = self.damaged.saturating_add(1);
                }
            }
            // The one report the reader publishes; forwarded as it arrived so
            // the player sees the location, track and cause the reader knew.
            DemuxEvent::Damage(damage) => self.damage.push(damage),
            _ => {}
        }
    }
}

/// Split an init's tracks into those this client can carry and the CENC/CBCS
/// ones it cannot. A track with `TrackSpec::protection` is sample-encrypted;
/// this client decrypts AES-128 HLS/DASH segments only, so the track is
/// reported and left out.
fn split_encrypted(specs: Vec<TrackSpec>) -> (Vec<TrackSpec>, Vec<SkippedTrack>) {
    let mut kept = Vec::with_capacity(specs.len());
    let mut encrypted = Vec::new();
    for spec in specs {
        match encrypted_data_track(&spec) {
            Some(track) => encrypted.push(track),
            None => kept.push(spec),
        }
    }
    (kept, encrypted)
}

fn encrypted_data_track(spec: &TrackSpec) -> Option<SkippedTrack> {
    spec.protection.as_ref()?;
    let fourcc = if spec.config.is_audio() {
        "enca"
    } else {
        "encv"
    };
    let mut skipped = SkippedTrack::new(
        fourcc.to_string(),
        String::from("CENC/CBCS sample encryption is not supported by this client"),
    );
    skipped.track_id = Some(spec.track_id);
    Some(skipped)
}

/// A new stage over `init`, with the init's bytes fed.
fn stage_over(init: &[u8]) -> Result<Mp4Demux, transmux::Error> {
    let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
    demux.feed(Mp4Input::Next(init), Timestamp::ZERO)?;
    Ok(demux)
}

/// The track specs of an Initialization Segment, read from the stage as soon
/// as its `moov` is parsed (no `finish`).
pub(crate) fn init_info(init: &[u8]) -> Result<InitInfo, transmux::Error> {
    let mut demux = stage_over(init)?;
    let mut collected = Collected::default();
    collected.drain(&mut demux, &BTreeSet::new());
    let (specs, encrypted) = split_encrypted(demux.init_tracks().cloned().collect());
    collected.skipped.extend(encrypted);
    Ok(InitInfo {
        specs,
        skipped: collected.skipped,
    })
}

/// One Representation's long-lived demux stage: the init is fed once and each
/// media segment follows it, closed by [`Mp4Input::SegmentEnd`].
pub(crate) struct SegmentStage {
    demux: Mp4Demux,
    init_events: Collected,
    skipped_tracks: BTreeSet<u32>,
}

impl SegmentStage {
    /// A stage with `init` fed, ready for its first media segment.
    pub(crate) fn new(init: &[u8]) -> Result<Self, transmux::Error> {
        let mut demux = stage_over(init)?;
        let (_specs, encrypted) = split_encrypted(demux.init_tracks().cloned().collect());
        let skipped_tracks: BTreeSet<u32> = encrypted
            .iter()
            .filter_map(|track| track.track_id.map(transmux::TrackId::value))
            .collect();
        let mut init_events = Collected::default();
        init_events.drain(&mut demux, &skipped_tracks);
        init_events.skipped.extend(encrypted);
        Ok(Self {
            demux,
            init_events,
            skipped_tracks,
        })
    }

    /// Feed one media segment and return its samples.
    pub(crate) fn demux(&mut self, segment: &[u8]) -> Result<Staged, transmux::Error> {
        self.demux.feed(Mp4Input::Next(segment), Timestamp::ZERO)?;
        self.demux.feed(Mp4Input::SegmentEnd, Timestamp::ZERO)?;
        let mut collected = Collected::default();
        collected.drain(&mut self.demux, &self.skipped_tracks);
        let mut damage = self.init_events.damage.clone();
        damage.extend(collected.damage);
        Ok(Staged {
            tracks: collected.tracks,
            damaged: self.init_events.damaged.saturating_add(collected.damaged),
            skipped: self.init_events.skipped.clone(),
            damage,
        })
    }
}

fn skipped_from_reason(reason: AbandonReason, track_id: Option<transmux::TrackId>) -> SkippedTrack {
    let (fourcc, text) = match reason {
        AbandonReason::UnsupportedSampleEntry { fourcc, detail } => {
            let fourcc = String::from_utf8_lossy(&fourcc).into_owned();
            let text = match detail {
                Some(detail) => format!("sample entry '{fourcc}' rejected: {detail}"),
                None => format!(
                    "sample entry '{fourcc}' has no CodecConfig reconstruction in this crate"
                ),
            };
            (fourcc, text)
        }
        AbandonReason::UnsupportedCodec => (
            String::from("unknown"),
            "sample entry 'unknown' has no CodecConfig reconstruction in this crate".into(),
        ),
        AbandonReason::MalformedTrack { expected } => (
            String::from("unknown"),
            format!("unexpected box: expected {expected}"),
        ),
        other => (String::from("unknown"), other.name().into()),
    };
    let mut skipped = SkippedTrack::new(fourcc, text);
    skipped.track_id = track_id;
    skipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::damage::{Damage, DamageKind, Severity};
    use transmux::ir::EventProvenance;

    fn degraded(kind: InputDegradation) -> DemuxEvent {
        DemuxEvent::input_degraded(None, kind, EventProvenance::default())
    }

    #[test]
    fn a_degraded_unit_marks_the_segment_and_reports_nothing_itself() {
        let mut collected = Collected::default();
        collected.on_event(
            degraded(InputDegradation::TruncatedUnit { position: 640 }),
            &BTreeSet::new(),
        );
        assert_eq!(collected.damaged, 1);
        assert!(
            collected.damage.is_empty(),
            "the report is the reader's own, never derived from the signal"
        );
    }

    #[test]
    fn a_run_of_the_same_damage_is_counted_once_per_unit_and_never_reported() {
        let mut collected = Collected::default();
        for position in 0..8u64 {
            collected.on_event(
                degraded(InputDegradation::MalformedUnit { position }),
                &BTreeSet::new(),
            );
        }
        assert_eq!(collected.damaged, 8, "every damaged unit is counted");
        assert!(collected.damage.is_empty(), "{:?}", collected.damage);
    }

    #[test]
    fn one_reader_report_beside_its_signal_is_one_report() {
        let mut collected = Collected::default();
        collected.on_event(
            degraded(InputDegradation::MalformedUnit { position: 640 }),
            &BTreeSet::new(),
        );
        let report = Damage::new(
            DamageKind::Resynchronised,
            Severity::Degraded,
            "malformed-fragment",
        )
        .at_byte_range(broadcast_core::damage::ByteRange::new(640, 0));
        collected.on_event(DemuxEvent::damage(report.clone()), &BTreeSet::new());
        assert_eq!(collected.damage, vec![report]);
    }

    #[test]
    fn the_readers_own_report_is_forwarded_verbatim() {
        let report = Damage::new(DamageKind::Resynchronised, Severity::Degraded, "box")
            .at_byte_range(broadcast_core::damage::ByteRange::new(16, 4));
        let mut collected = Collected::default();
        collected.on_event(DemuxEvent::damage(report.clone()), &BTreeSet::new());
        assert_eq!(collected.damage, vec![report]);
    }

    #[test]
    fn a_clean_segment_reports_nothing() {
        let collected = Collected::default();
        assert!(collected.damage.is_empty());
        assert_eq!(collected.damaged, 0);
    }
}
