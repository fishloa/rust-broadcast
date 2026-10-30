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
//! Follower segments are produced one leader segment behind the leader, since
//! a leader cut is only known once the leader has finished that segment.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use broadcast_common::Timestamp;
use media_plane::ids::{PartIndex, SegmentSeq};
use media_plane::trunk::{PartEntry, SegmentEntry, SegmentWriter, Trunk};
use transmux::SegmentMeta;
use transmux::TrackId;
use transmux::ll_hls::{CutPlan, LlHlsSegmenter, PartInfo, SegmentInfo};
use transmux::pipeline::{Sample, TrackSpec};

use super::segment::{PendingPublish, drain_publish_queue};
use crate::config::AudioTrackSelector;
use crate::route::{AudioServing, RouteHandle};

const NANOS_PER_SEC_F64: f64 = 1_000_000_000.0;

/// Leader cut plans a follower may hold back waiting for its audio to catch
/// up with the cut before it closes the segment regardless (an audio track
/// that went silent must not stall its playlist forever).
const MAX_QUEUED_PLANS: usize = 2;

/// Audio samples buffered before the first leader cut fixes the shared
/// origin; older ones are dropped.
const MAX_HELD_SAMPLES: usize = 8192;

/// Leader segment durations remembered to size gap placeholders.
pub(super) const LEADER_DURATION_HISTORY: usize = 512;

pub(super) type LeaderDurations = BTreeMap<u32, f64>;

/// Maps each rendition to the source audio track its selector names.
/// Returns `(rendition index, source spec)` pairs; renditions whose selector
/// matches nothing (yet) are skipped with a warning, as is a track already
/// claimed by an earlier rendition. An empty result also when mapping every
/// muxable track would leave the leader with nothing to anchor on.
pub(crate) fn resolve_sources(
    route: &RouteHandle,
    tracks: &[TrackSpec],
) -> Vec<(usize, TrackSpec)> {
    let selectors = route.audio_rendition_sources();
    if selectors.is_empty() {
        return Vec::new();
    }
    let audio: Vec<&TrackSpec> = tracks
        .iter()
        .filter(|t| !crate::push::egress::is_video(&t.config) && t.config.is_muxable_in_bmff())
        .collect();
    let mut mapped: Vec<(usize, TrackSpec)> = Vec::new();
    for (rendition, selector) in selectors.iter().enumerate() {
        let found = match selector {
            AudioTrackSelector::Index(i) => audio.get(*i as usize).copied(),
            AudioTrackSelector::TrackId(id) => audio
                .iter()
                .copied()
                .find(|t| t.track_id == TrackId::new(*id)),
        };
        match found {
            Some(spec) if mapped.iter().any(|(_, m)| m.track_id == spec.track_id) => {
                tracing::warn!(
                    route = route.name(),
                    rendition,
                    "audio rendition selector names a track another rendition already uses; ignoring"
                );
            }
            Some(spec) => mapped.push((rendition, spec.clone())),
            None => tracing::warn!(
                route = route.name(),
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
            route = route.name(),
            "every muxable track is mapped to an audio rendition, leaving the primary playlist \
             nothing to anchor on; serving the program muxed"
        );
        return Vec::new();
    }
    mapped
}

/// The tracks the primary (leader) playlist keeps: every fMP4-muxable track
/// except those mapped to a separate rendition.
pub(crate) fn leader_tracks(tracks: &[TrackSpec], mapped: &[(usize, TrackSpec)]) -> Vec<TrackSpec> {
    tracks
        .iter()
        .filter(|t| t.config.is_muxable_in_bmff())
        .filter(|t| !mapped.iter().any(|(_, m)| m.track_id == t.track_id))
        .cloned()
        .collect()
}

/// Builds one follower per mapped track over its rendition's sibling origin.
pub(super) fn build_followers(
    trunk: &Trunk,
    route: &RouteHandle,
    mapped: &[(usize, TrackSpec)],
    target_duration_secs: f64,
    part_target_ms: u32,
) -> Vec<AudioFollower> {
    if mapped.is_empty() {
        return Vec::new();
    }
    let servings = route.audio_servings_for_trunk(trunk);
    mapped
        .iter()
        .filter_map(|(rendition, spec)| {
            let serving = servings.get(*rendition)?;
            AudioFollower::try_new(spec, serving, target_duration_secs, part_target_ms)
        })
        .collect()
}

/// One separate audio rendition being segmented.
pub(super) struct AudioFollower {
    pub(super) source_track_id: TrackId,
    seg: LlHlsSegmenter,
    writer: SegmentWriter,
    pending: VecDeque<PendingPublish>,
    pending_since: Option<Instant>,
    held: VecDeque<Sample>,
    origin: Option<(i64, u32)>,
    timescale: u32,
    plans: VecDeque<CutPlan>,
    registered_any: bool,
    newest_dts: Option<i64>,
    next_seq: u32,
    next_timeline_ns: u64,
    target_duration_secs: f64,
}

impl AudioFollower {
    pub(super) fn try_new(
        spec: &TrackSpec,
        serving: &AudioServing,
        target_duration_secs: f64,
        part_target_ms: u32,
    ) -> Option<Self> {
        let writer = serving.trunk.segment_writer()?;
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
        match seg.init_segment() {
            Ok(init) => serving.origin.set_init(init),
            Err(e) => {
                tracing::warn!(error = %e, "audio follower init segment build failed");
                return None;
            }
        }
        Some(Self {
            source_track_id: spec.track_id,
            seg,
            writer,
            pending: VecDeque::new(),
            pending_since: None,
            held: VecDeque::new(),
            origin: None,
            timescale: spec.timescale,
            plans: VecDeque::new(),
            registered_any: false,
            newest_dts: None,
            next_seq,
            next_timeline_ns: 0,
            target_duration_secs,
        })
    }

    pub(super) fn push(&mut self, sample: Sample, leaders: &LeaderDurations) {
        if self.origin.is_none() {
            if self.held.len() >= MAX_HELD_SAMPLES {
                self.held.pop_front();
            }
            self.held.push_back(sample);
            return;
        }
        self.push_now(sample);
        self.release_plans(false);
        self.publish_ready(leaders);
    }

    fn push_now(&mut self, sample: Sample) {
        if let Some(dts) = sample.dts {
            self.newest_dts = Some(self.newest_dts.map_or(dts, |n| n.max(dts)));
        }
        if let Err(e) = self.seg.push(self.source_track_id, sample) {
            tracing::warn!(error = %e, "audio follower push failed");
        }
    }

    pub(super) fn on_plans(&mut self, plans: Vec<CutPlan>, leaders: &LeaderDurations) {
        for plan in plans {
            if self.origin.is_none() {
                self.start_timeline(plan);
            } else {
                self.plans.push_back(plan);
            }
        }
        self.release_plans(false);
        self.publish_ready(leaders);
    }

    /// The first leader cut fixes the shared origin and puts the segmenter in
    /// follower mode before any sample reaches it, so it never cuts on its own.
    fn start_timeline(&mut self, plan: CutPlan) {
        let (start, timescale) = (plan.segment_start, plan.timescale);
        self.next_seq = plan.segment_seq;
        self.origin = Some((start, timescale));
        self.seg.set_timeline_origin(start, timescale);
        self.follow(plan);
        for sample in std::mem::take(&mut self.held) {
            let at_or_after = sample
                .dts
                .is_some_and(|d| cmp_time(d, self.timescale, start, timescale).is_ge());
            if at_or_after {
                self.push_now(sample);
            }
        }
    }

    fn follow(&mut self, plan: CutPlan) {
        self.registered_any = true;
        if let Err(e) = self.seg.follow(plan) {
            tracing::warn!(error = %e, "audio follower follow failed");
        }
    }

    fn caught_up(&self, plan: &CutPlan) -> bool {
        self.newest_dts.is_some_and(|d| {
            cmp_time(d, self.timescale, plan.segment_start, plan.timescale).is_ge()
        })
    }

    fn release_plans(&mut self, force: bool) {
        while let Some(front) = self.plans.front() {
            let ready = force
                || !self.registered_any
                || self.caught_up(front)
                || self.plans.len() > MAX_QUEUED_PLANS;
            if !ready {
                return;
            }
            let Some(plan) = self.plans.pop_front() else {
                return;
            };
            self.follow(plan);
        }
    }

    /// Close the trailing segment at end of stream and pad the playlist with
    /// gaps up to the leader's last segment.
    pub(super) fn finish(&mut self, leaders: &LeaderDurations) {
        self.release_plans(true);
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
        self.pending.push_back(PendingPublish::Part(PartEntry::new(
            part.bytes,
            SegmentSeq::new(part.segment_seq),
            PartIndex::new(part.part_index),
            Duration::from_secs_f64(part.duration),
            part.independent,
        )));
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
                    discontinuous: false,
                },
            )));
    }

    fn drain(&mut self) {
        drain_publish_queue(&mut self.writer, &mut self.pending, &mut self.pending_since);
    }
}

fn cmp_time(a: i64, a_ts: u32, b: i64, b_ts: u32) -> std::cmp::Ordering {
    (i128::from(a) * i128::from(b_ts)).cmp(&(i128::from(b) * i128::from(a_ts)))
}
