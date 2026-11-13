//! Split-mux audio followers (issue #1428 slice 2b).
//!
//! Each audio rendition is its own fMP4 timeline: an audio-only
//! [`LlHlsSegmenter`] in follower mode cuts where the video leader cut, so
//! both playlists share media sequence numbers, and writes into the
//! rendition's sibling `Trunk`. All renditions share one timeline origin, the
//! start of the leader's first segment, so the `tfdt` of audio and video
//! segments with the same sequence number agree to within one audio frame.
//!
//! A leader segment in which the audio track has no sample starting is
//! published as an empty segment, which the origin renders as `EXT-X-GAP`
//! (RFC 8216bis §4.4.4.7: the segment's media is unavailable and clients
//! must not request it), so the sequence numbers of both playlists stay
//! contiguous and aligned.
//!
//! The leader hands out each part boundary as it closes, and a follower cuts
//! the matching audio part as soon as its audio reaches the next boundary, so
//! an audio playlist's live edge trails the video's by at most one part
//! (RFC 8216bis §4.4.4.9: renditions carry matching media sequence numbers and
//! parts).

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use broadcast_common::Timestamp;
use broadcast_common::timescale::cmp_ticks;
use transmux::audio_profile::AudioTrackSelector;
use transmux::ll_hls::{LlHlsSegmenter, PartCutReady, PartInfo, SegmentInfo};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{SegmentMeta, TrackId};

use crate::ids::SegmentSeq;
use crate::publish_queue::{PendingPublish, drain_publish_queue, part_entry};
use crate::trunk::{SegmentEntry, SegmentWriter, Trunk};

const MILLIS_PER_SEC: u128 = 1_000;

const NANOS_PER_SEC_F64: f64 = 1_000_000_000.0;

/// Leader part boundaries a follower may hold back waiting for its audio to
/// catch up with the boundary before it applies them regardless (an audio
/// track that went silent must not stall its playlist forever).
const MAX_QUEUED_CUTS: usize = 16;

/// Audio samples buffered before the first leader cut fixes the shared
/// origin; older ones are dropped.
const MAX_HELD_SAMPLES: usize = 8192;

/// Leader segment durations remembered to size gap placeholders.
pub const LEADER_DURATION_HISTORY: usize = 512;

pub type LeaderDurations = BTreeMap<u32, f64>;

/// Maps each rendition to the source audio track its selector names.
/// Returns `(rendition index, source spec)` pairs; renditions whose selector
/// matches nothing (yet) are skipped with a warning, as is a track already
/// claimed by an earlier rendition. An empty result also when mapping every
/// muxable track would leave the leader with nothing to anchor on.
pub fn resolve_sources(
    route: &str,
    selectors: &[AudioTrackSelector],
    tracks: &[TrackSpec],
) -> Vec<(usize, TrackSpec)> {
    if selectors.is_empty() {
        return Vec::new();
    }
    let audio: Vec<&TrackSpec> = tracks
        .iter()
        .filter(|t| !t.config.is_video() && t.config.is_muxable_in_bmff())
        .collect();
    let mut mapped: Vec<(usize, TrackSpec)> = Vec::new();
    for (rendition, selector) in selectors.iter().enumerate() {
        let found = selector
            .position(audio.iter().map(|t| t.track_id))
            .map(|n| audio[n]);
        match found {
            Some(spec) if mapped.iter().any(|(_, m)| m.track_id == spec.track_id) => {
                tracing::warn!(
                    route,
                    rendition,
                    "audio rendition selector names a track another rendition already uses; ignoring"
                );
            }
            Some(spec) => mapped.push((rendition, spec.clone())),
            None => tracing::warn!(
                route,
                rendition,
                selector = %selector,
                "no audio track matches this rendition's selector; ignoring until one appears"
            ),
        }
    }
    let muxable = tracks
        .iter()
        .filter(|t| t.config.is_muxable_in_bmff())
        .count();
    if mapped.len() >= muxable {
        tracing::warn!(
            route,
            "every muxable track is mapped to an audio rendition, leaving the primary playlist \
             nothing to anchor on; serving the program muxed"
        );
        return Vec::new();
    }
    mapped
}

/// The tracks the primary (leader) playlist keeps: every fMP4-muxable track
/// except those mapped to a separate rendition.
pub fn leader_tracks(tracks: &[TrackSpec], mapped: &[(usize, TrackSpec)]) -> Vec<TrackSpec> {
    tracks
        .iter()
        .filter(|t| t.config.is_muxable_in_bmff())
        .filter(|t| !mapped.iter().any(|(_, m)| m.track_id == t.track_id))
        .cloned()
        .collect()
}

/// One separate audio rendition being segmented.
pub struct AudioFollower {
    pub source_track_id: TrackId,
    seg: LlHlsSegmenter,
    writer: SegmentWriter,
    pending: VecDeque<PendingPublish>,
    pending_since: Option<Instant>,
    held: VecDeque<Sample>,
    origin: Option<(i64, u32)>,
    timescale: u32,
    cuts: VecDeque<PartCutReady>,
    newest_dts: Option<i64>,
    next_seq: u32,
    next_timeline_ns: u64,
    first_discontinuous: bool,
    target_duration_secs: f64,
    part_target_ms: u32,
    part_target_sized: bool,
    init: Option<Vec<u8>>,
    part_target_event: Option<u32>,
}

fn part_target_for_frame_ms(part_target_ms: u32, frame_ticks: u32, timescale: u32) -> Option<u32> {
    if frame_ticks == 0 || timescale == 0 {
        return None;
    }
    let ticks = u128::from(frame_ticks);
    let scale = u128::from(timescale);
    let target_ticks = u128::from(part_target_ms) * scale;
    let frames = target_ticks.div_ceil(ticks * MILLIS_PER_SEC).max(1);
    let millis = (frames * ticks * MILLIS_PER_SEC).div_ceil(scale);
    u32::try_from(millis).ok()
}

impl AudioFollower {
    /// Build a follower for `spec` that publishes into the sibling `trunk`; `None`
    /// when the trunk's segment writer is already taken or the segmenter cannot be built.
    pub fn try_new(
        spec: &TrackSpec,
        trunk: &Arc<Trunk>,
        target_duration_secs: f64,
        part_target_ms: u32,
    ) -> Option<Self> {
        let writer = trunk.segment_writer()?;
        let next_seq = writer.next_sequence_number().value();
        let seg = match LlHlsSegmenter::with_part_target_at(
            vec![spec.clone()],
            transmux::VIDEO_CLOCK_RATE,
            target_duration_secs,
            part_target_ms,
            next_seq,
            next_seq,
        ) {
            Ok(seg) => seg,
            Err(e) => {
                tracing::warn!(error = %e, "audio follower segmenter build failed");
                return None;
            }
        };
        let init = match seg.init_segment() {
            Ok(init) => init,
            Err(e) => {
                tracing::warn!(error = %e, "audio follower init segment build failed");
                return None;
            }
        };
        Some(Self {
            source_track_id: spec.track_id,
            seg,
            writer,
            pending: VecDeque::new(),
            pending_since: None,
            held: VecDeque::new(),
            origin: None,
            timescale: spec.timescale,
            cuts: VecDeque::new(),
            newest_dts: None,
            next_seq,
            next_timeline_ns: 0,
            first_discontinuous: false,
            target_duration_secs,
            part_target_ms,
            part_target_sized: false,
            init: Some(init),
            part_target_event: None,
        })
    }

    /// The follower's init segment, once: the host serves it for the rendition.
    pub fn take_init(&mut self) -> Option<Vec<u8>> {
        self.init.take()
    }

    /// The part target (milliseconds) the rendition's origin should advertise,
    /// once, after the first sample fixed the frame duration.
    pub fn take_part_target_ms(&mut self) -> Option<u32> {
        self.part_target_event.take()
    }

    fn size_part_target(&mut self, sample: &Sample) {
        if self.part_target_sized {
            return;
        }
        let Some(frame_ticks) = sample.duration else {
            return;
        };
        self.part_target_sized = true;
        if let Some(ms) = part_target_for_frame_ms(self.part_target_ms, frame_ticks, self.timescale)
        {
            self.part_target_event = Some(ms);
        }
    }

    pub fn set_start_time_ns(&mut self, start_ns: u64) {
        self.seg.set_start_time_ns(start_ns);
        self.next_timeline_ns = start_ns;
        self.first_discontinuous = true;
    }

    pub fn push(&mut self, sample: Sample, leaders: &LeaderDurations) {
        self.size_part_target(&sample);
        if self.precedes_origin(&sample) {
            return;
        }
        if self.origin.is_none() {
            if self.held.len() >= MAX_HELD_SAMPLES {
                self.held.pop_front();
            }
            self.held.push_back(sample);
            return;
        }
        self.push_now(sample);
        self.release_cuts(false);
        self.publish_ready(leaders);
    }

    fn precedes_origin(&self, sample: &Sample) -> bool {
        match (self.origin, sample.dts) {
            (Some((origin, origin_ts)), Some(dts)) => {
                cmp_ticks(dts, self.timescale, origin, origin_ts).is_lt()
            }
            _ => false,
        }
    }

    fn push_now(&mut self, sample: Sample) {
        if let Some(dts) = sample.dts {
            self.newest_dts = Some(self.newest_dts.map_or(dts, |n| n.max(dts)));
        }
        if let Err(e) = self.seg.push(self.source_track_id, sample) {
            tracing::warn!(error = %e, "audio follower push failed");
        }
    }

    pub fn on_part_cuts(&mut self, cuts: Vec<PartCutReady>, leaders: &LeaderDurations) {
        self.cuts.extend(cuts);
        self.release_cuts(false);
        self.publish_ready(leaders);
    }

    fn caught_up(&self, cut: &PartCutReady) -> bool {
        self.newest_dts
            .is_some_and(|d| cmp_ticks(d, self.timescale, cut.start_dts, cut.timescale).is_ge())
    }

    /// Applies queued leader boundaries once the audio has reached them, so a
    /// segment is never closed before the audio it will contain has arrived.
    fn release_cuts(&mut self, force: bool) {
        while let Some(cut) = self.cuts.front().copied() {
            let ready = force
                || self.origin.is_none()
                || self.caught_up(&cut)
                || self.cuts.len() > MAX_QUEUED_CUTS;
            if !ready {
                return;
            }
            self.cuts.pop_front();
            self.follow(&cut);
        }
    }

    /// The first leader boundary fixes the shared origin and puts the
    /// segmenter in follower mode before any sample reaches it, so it never
    /// cuts on its own.
    fn follow(&mut self, cut: &PartCutReady) {
        let first = self.origin.is_none();
        if first {
            self.next_seq = cut.msn;
            self.origin = Some((cut.start_dts, cut.timescale));
            self.seg.set_timeline_origin(cut.start_dts, cut.timescale);
        }
        if let Err(e) = self
            .seg
            .follow_part(cut.msn, cut.part_index, cut.start_dts, cut.timescale)
        {
            tracing::warn!(error = %e, "audio follower follow_part failed");
        }
        if first {
            for sample in std::mem::take(&mut self.held) {
                let at_or_after = sample.dts.is_some_and(|d| {
                    cmp_ticks(d, self.timescale, cut.start_dts, cut.timescale).is_ge()
                });
                if at_or_after {
                    self.push_now(sample);
                }
            }
        }
    }

    /// Close the trailing segment at end of stream and pad the playlist with
    /// gaps up to the leader's last segment.
    pub fn finish(&mut self, leaders: &LeaderDurations) {
        self.release_cuts(true);
        if let Err(e) = self.seg.flush() {
            tracing::warn!(error = %e, "audio follower flush failed");
        }
        self.publish_ready(leaders);
        if let Some(last) = leaders.keys().next_back().copied() {
            self.fill_gaps(last.saturating_add(1), leaders);
        }
        self.drain();
    }

    fn publish_ready(&mut self, leaders: &LeaderDurations) {
        let mut parts: VecDeque<PartInfo> = self.seg.take_ready_parts().into();
        let segments = self.seg.take_ready_segments();
        for SegmentInfo {
            bytes,
            duration,
            segment_seq,
            ..
        } in segments
        {
            self.fill_gaps(segment_seq, leaders);
            while parts.front().is_some_and(|p| p.segment_seq <= segment_seq) {
                if let Some(part) = parts.pop_front() {
                    self.queue_part(part);
                }
            }
            self.queue_segment(bytes, segment_seq, duration);
            self.next_seq = segment_seq.saturating_add(1);
        }
        for part in parts {
            self.queue_part(part);
        }
        self.drain();
    }

    fn fill_gaps(&mut self, upto: u32, leaders: &LeaderDurations) {
        for seq in self.next_seq..upto {
            let duration = leaders
                .get(&seq)
                .copied()
                .unwrap_or(self.target_duration_secs);
            self.queue_segment(Vec::new(), seq, duration);
        }
        self.next_seq = self.next_seq.max(upto);
    }

    fn queue_part(&mut self, part: PartInfo) {
        self.pending
            .push_back(PendingPublish::Part(part_entry(part)));
    }

    fn queue_segment(&mut self, bytes: Vec<u8>, seq: u32, duration_secs: f64) {
        let duration = Duration::from_secs_f64(duration_secs);
        let start_ns = self.next_timeline_ns;
        self.next_timeline_ns = self
            .next_timeline_ns
            .saturating_add((duration_secs * NANOS_PER_SEC_F64) as u64);
        self.pending
            .push_back(PendingPublish::Segment(SegmentEntry::new(
                bytes,
                SegmentSeq::new(seq),
                duration,
                Timestamp::from_nanos(start_ns),
                SegmentMeta {
                    discontinuous: std::mem::take(&mut self.first_discontinuous),
                },
            )));
    }

    fn drain(&mut self) {
        drain_publish_queue(&mut self.writer, &mut self.pending, &mut self.pending_since);
    }
}

#[cfg(test)]
mod part_target_tests {
    use super::part_target_for_frame_ms;

    #[test]
    fn rounds_up_to_whole_frames() {
        assert_eq!(part_target_for_frame_ms(200, 1536, 48_000), Some(224));
        assert_eq!(part_target_for_frame_ms(200, 1024, 48_000), Some(214));
        assert_eq!(part_target_for_frame_ms(200, 960, 48_000), Some(200));
        assert_eq!(part_target_for_frame_ms(10, 1536, 48_000), Some(32));
        assert_eq!(part_target_for_frame_ms(200, 0, 48_000), None);
    }
}

#[cfg(test)]
mod follower_tests {
    use super::*;
    use crate::trunk::TrunkConfig;
    use crate::ts_session::test_support::{track_spec, trunk_config};

    fn trunk() -> Arc<Trunk> {
        Trunk::new(trunk_config())
    }

    /// The init segment is handed out once, for the host to serve, and a
    /// second follower over the same sibling trunk is refused because the
    /// trunk's segment writer is already claimed.
    #[test]
    fn the_init_is_handed_out_once_and_a_trunk_has_one_follower() {
        let trunk = trunk();
        let spec = track_spec(TrackId::new(2));
        let mut follower = AudioFollower::try_new(&spec, &trunk, 4.0, 500).expect("follower");
        assert!(follower.take_init().is_some_and(|init| !init.is_empty()));
        assert!(follower.take_init().is_none());
        assert!(AudioFollower::try_new(&spec, &trunk, 4.0, 500).is_none());
    }

    /// The follower writes into the trunk it was given, whatever capacities the
    /// host chose for it.
    #[test]
    fn the_follower_publishes_into_the_given_trunk() {
        let config: TrunkConfig = trunk_config();
        let trunk = Trunk::new(config);
        let spec = track_spec(TrackId::new(2));
        let follower = AudioFollower::try_new(&spec, &trunk, 4.0, 500);
        assert!(follower.is_some());
        assert_eq!(trunk.segment_len(), 0);
    }
}
