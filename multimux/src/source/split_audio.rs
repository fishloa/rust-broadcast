//! Split-mux audio followers (issue #1428 slice 2b), the route-facing half.
//!
//! The follower itself, which cuts one audio rendition where the video leader
//! cut and publishes into the rendition's sibling `Trunk`, is
//! [`media_plane::segmenter::audio::AudioFollower`]. This module owns what
//! needs the route: the selectors and name come from the [`RouteHandle`], the
//! sibling `Trunk` and `HlsOrigin` are the route's [`AudioServing`], and the
//! follower's two serving-side effects (its init segment, and the part target
//! its first sample fixes) are applied to that origin here.

use std::sync::Arc;

use media_plane::segmenter::audio::{
    AudioFollower as Follower, LeaderDurations, resolve_sources as resolve_with,
};
use media_plane::trunk::Trunk;
use transmux::TrackId;
use transmux::ll_hls::PartCutReady;
use transmux::pipeline::{Sample, TrackSpec};

use crate::route::RouteHandle;
use hls_runtime::server::HlsOrigin;

/// Maps each rendition to the source audio track its selector names, from the
/// route's configured selectors; see
/// [`media_plane::segmenter::audio::resolve_sources`].
pub(crate) fn resolve_sources(
    route: &RouteHandle,
    tracks: &[TrackSpec],
) -> Vec<(usize, TrackSpec)> {
    resolve_with(route.name(), route.audio_rendition_sources(), tracks)
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
            let mut core =
                Follower::try_new(spec, &serving.trunk, target_duration_secs, part_target_ms)?;
            if let Some(init) = core.take_init() {
                serving.origin.set_init(init);
            }
            Some(AudioFollower {
                source_track_id: spec.track_id,
                core,
                origin: Arc::clone(&serving.origin),
            })
        })
        .collect()
}

/// One separate audio rendition being segmented, with the origin its serving
/// side is applied to.
pub(super) struct AudioFollower {
    pub(super) source_track_id: TrackId,
    core: Follower,
    origin: Arc<HlsOrigin>,
}

impl AudioFollower {
    pub(super) fn set_start_time_ns(&mut self, start_ns: u64) {
        self.core.set_start_time_ns(start_ns);
    }

    pub(super) fn push(&mut self, sample: Sample, leaders: &LeaderDurations) {
        self.core.push(sample, leaders);
        if let Some(ms) = self.core.take_part_target_ms() {
            self.origin.set_part_target_ms(ms);
        }
    }

    pub(super) fn on_part_cuts(&mut self, cuts: Vec<PartCutReady>, leaders: &LeaderDurations) {
        self.core.on_part_cuts(cuts, leaders);
    }

    pub(super) fn finish(&mut self, leaders: &LeaderDurations) {
        self.core.finish(leaders);
    }
}
