//! Split-mux audio followers (issue #1428 slice 2b), the route-facing half.
//!
//! The follower itself, which cuts one audio rendition where the video leader
//! cut and publishes into the rendition's sibling `Trunk`, is
//! [`media_plane::segmenter::audio::AudioFollower`]. This module owns what
//! needs the route: the selectors and name come from the [`RouteHandle`], the
//! sibling `Trunk` and `HlsOrigin` are the route's [`AudioServing`], and the
//! follower's two serving-side effects (its init segment, and the part target
//! its first sample fixes) are applied to that origin here.

use media_plane::segmenter::audio::resolve_sources as resolve_with;
use media_plane::trunk::Trunk;
use transmux::pipeline::TrackSpec;

use crate::route::RouteHandle;

/// Maps each rendition to the source audio track its selector names, from the
/// route's configured selectors; see
/// [`media_plane::segmenter::audio::resolve_sources`].
pub(crate) fn resolve_sources(
    route: &RouteHandle,
    tracks: &[TrackSpec],
) -> Vec<(usize, TrackSpec)> {
    resolve_with(route.name(), route.audio_rendition_sources(), tracks)
}

/// Builds core followers per mapped track over its rendition's sibling origin,
/// applying each init segment to that origin. Part-target updates are applied
/// by the host after pumping (see `RouteHandle::apply_audio_part_targets`).
pub(crate) fn build_core_followers(
    route: &RouteHandle,
    trunk: &Trunk,
    mapped: &[(usize, TrackSpec)],
    target_duration_secs: f64,
    part_target_ms: u32,
) -> Vec<media_plane::segmenter::audio::AudioFollower> {
    if mapped.is_empty() {
        return Vec::new();
    }
    let servings = route.audio_servings_for_trunk(trunk);
    mapped
        .iter()
        .filter_map(|(rendition, spec)| {
            let serving = servings.get(*rendition)?;
            let mut core = media_plane::segmenter::audio::AudioFollower::try_new(
                spec,
                &serving.trunk,
                target_duration_secs,
                part_target_ms,
            )?;
            if let Some(init) = core.take_init() {
                serving.origin.set_init(init);
            }
            Some(core)
        })
        .collect()
}
