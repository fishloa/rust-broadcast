//! Thin glue over `media_plane::segmenter::program::ProgramSegmenter`.

use std::collections::HashMap;

use media_plane::ingress::{IngestDriver, IngestSession, ProgramId};
use media_plane::segmenter::program::{
    ProgramSegmenter, SegmenterConfig, SegmenterEvent, SiblingTrunks,
};
use media_plane::trunk::Trunk;

use crate::route::{RouteHandle, TsHlsIdrStatus};

impl SiblingTrunks for RouteHandle {
    fn report_output_failed(&self, message: String) {
        let _ = self.fail_output_message(message);
    }
    fn build_audio_followers(
        &self,
        trunk: &Trunk,
        mapped: &[(usize, transmux::pipeline::TrackSpec)],
        target_duration_secs: f64,
        part_target_ms: u32,
    ) -> Vec<media_plane::segmenter::audio::AudioFollower> {
        // Delegate to split_audio's core builder (applies init to origin).
        crate::source::split_audio::build_core_followers(
            self,
            trunk,
            mapped,
            target_duration_secs,
            part_target_ms,
        )
    }

    fn build_subtitle_followers(
        &self,
        trunk: &Trunk,
        tracks: &[transmux::pipeline::TrackSpec],
    ) -> Vec<media_plane::segmenter::subtitle::SubtitleFollower> {
        crate::source::split_subtitle::build_followers(self, trunk, tracks)
    }
}

impl RouteHandle {
    pub fn segmenter_config(&self) -> SegmenterConfig {
        SegmenterConfig::default()
            .with_container(self.container())
            .with_ts_cut_policy(self.ts_cut_policy())
            .with_ts_idr_fallback(match self.ts_idr_fallback() {
                crate::config::TsHlsIdrFallback::Auto => {
                    media_plane::segmenter::program::TsHlsIdrFallback::Auto
                }
                crate::config::TsHlsIdrFallback::Targets(n) => {
                    media_plane::segmenter::program::TsHlsIdrFallback::Targets(n)
                }
                crate::config::TsHlsIdrFallback::Never => {
                    media_plane::segmenter::program::TsHlsIdrFallback::Never
                }
            })
            .with_target_duration_secs(self.target_duration_secs())
            .with_part_target_ms(self.part_target_ms())
            .with_window_segments_cap(self.window_segments_cap())
            .with_route_name(self.name().to_string())
            .with_audio_rendition_sources(self.audio_rendition_sources().to_vec())
            .with_audio_profiles(self.audio_profiles_for_segmenter())
            .with_subtitles(self.subtitles_enabled())
            .with_trunk_config(*self.trunk_config())
    }
}

pub fn apply_segmenter_events(
    route: &RouteHandle,
    program: ProgramId,
    events: Vec<SegmenterEvent>,
) {
    for e in events {
        match e {
            SegmenterEvent::TrackSpecs(specs) => route.set_track_specs(program, specs),
            SegmenterEvent::TracksActive(active) => route.set_tracks_active(program, active),
            SegmenterEvent::BeginTrackSpecs {
                first_segment,
                specs,
            } => route.begin_track_specs(program, specs, first_segment),
            SegmenterEvent::Init(bytes) => route.set_init(program, bytes),
            SegmenterEvent::InitFrom {
                init,
                first_segment,
            } => route.set_init_from(program, init, first_segment),
            SegmenterEvent::PartTargetMs(ms) => {
                route.set_part_target_ms(program, ms);
            }
            SegmenterEvent::ClosedCaptions {
                detected,
                languages,
            } => {
                route.note_closed_caption_languages(program, languages);
                route.note_closed_captions(program, detected);
            }
            SegmenterEvent::TsIdrMissing(status) => {
                let mm_status = match status {
                    media_plane::segmenter::program::TsHlsIdrStatus::FellBack => {
                        TsHlsIdrStatus::FellBack
                    }
                    media_plane::segmenter::program::TsHlsIdrStatus::Awaiting => {
                        TsHlsIdrStatus::Awaiting
                    }
                    _ => TsHlsIdrStatus::Awaiting,
                };
                if route.note_ts_idr_missing(mm_status) {
                    // increment prometheus counter
                    metrics::counter!(
                        crate::prometheus::TS_HLS_NON_IDR_TOTAL,
                        "route" => route.name().to_string(),
                        "mode" => mm_status.name()
                    )
                    .increment(1);
                }
            }
            SegmenterEvent::OutputFailed(msg) => {
                let _ = route.fail_output_message(msg);
            }
            _ => {}
        }
    }
}

impl SiblingTrunks for &RouteHandle {
    fn report_output_failed(&self, message: String) {
        (**self).report_output_failed(message)
    }
    fn build_audio_followers(
        &self,
        trunk: &Trunk,
        mapped: &[(usize, transmux::pipeline::TrackSpec)],
        target_duration_secs: f64,
        part_target_ms: u32,
    ) -> Vec<media_plane::segmenter::audio::AudioFollower> {
        (**self).build_audio_followers(trunk, mapped, target_duration_secs, part_target_ms)
    }
    fn build_subtitle_followers(
        &self,
        trunk: &Trunk,
        tracks: &[transmux::pipeline::TrackSpec],
    ) -> Vec<media_plane::segmenter::subtitle::SubtitleFollower> {
        (**self).build_subtitle_followers(trunk, tracks)
    }
}

pub fn pump_segmenter(
    segmenter: &mut ProgramSegmenter,
    program: ProgramId,
    trunk: &Trunk,
    route: &RouteHandle,
) -> (usize, usize) {
    let mut events: Vec<SegmenterEvent> = Vec::new();
    let counts = segmenter.pump(program, trunk, &mut events, route);
    let servings = route.audio_servings_for_trunk(trunk);
    for (follower, serving) in segmenter.followers.iter_mut().zip(servings.iter()) {
        if let Some(ms) = follower.take_part_target_ms() {
            serving.origin.set_part_target_ms(ms);
        }
    }
    apply_segmenter_events(route, program, events);
    counts
}

pub fn drive_program_segmenters<S: IngestSession>(
    driver: &IngestDriver<S>,
    route_handle: &RouteHandle,
    segmenters: &mut HashMap<ProgramId, ProgramSegmenter>,
) -> bool {
    let config = route_handle.segmenter_config();
    for program in driver.programs() {
        if segmenters.contains_key(&program) {
            continue;
        }
        let Some(trunk) = driver.trunk(program) else {
            continue;
        };
        let Some(mut segmenter) =
            ProgramSegmenter::try_new_with_siblings(trunk, config.clone(), route_handle)
        else {
            continue;
        };
        segmenter.continue_after_reconnect(
            route_handle
                .epoch(program)
                .map(|e| e.media_start_ns)
                .unwrap_or(0),
        );
        let mut init_events = Vec::new();
        // set track specs
        init_events.push(SegmenterEvent::TrackSpecs(
            segmenter.published_tracks.to_vec(),
        ));
        if let Some(init) = segmenter.init_segment() {
            init_events.push(SegmenterEvent::Init(init));
        }
        apply_segmenter_events(route_handle, program, init_events);
        segmenters.insert(program, segmenter);
    }

    let mut parts_total = 0usize;
    let mut segments_total = 0usize;
    for (&program, segmenter) in segmenters.iter_mut() {
        let Some(trunk) = driver.trunk(program) else {
            continue;
        };
        let mut events: Vec<SegmenterEvent> = Vec::new();
        let (parts, segments) = segmenter.pump(program, trunk, &mut events, route_handle);
        parts_total += parts;
        segments_total += segments;
        let servings = route_handle.audio_servings_for_trunk(trunk);
        for (follower, serving) in segmenter.followers.iter_mut().zip(servings.iter()) {
            if let Some(ms) = follower.take_part_target_ms() {
                serving.origin.set_part_target_ms(ms);
            }
        }
        apply_segmenter_events(route_handle, program, events);
    }

    let more = segmenters.values().any(|s| s.backlogged);
    if !more && !driver.health().is_running() {
        for segmenter in segmenters.values_mut() {
            // need program for events; but flush has no program context here. We'll skip apply?
            let (parts, segments) = segmenter.flush();
            parts_total += parts;
            segments_total += segments;
        }
    }

    if parts_total > 0 {
        metrics::counter!(
            crate::prometheus::PARTS_PRODUCED_TOTAL,
            "route" => route_handle.name().to_string(),
        )
        .increment(parts_total as u64);
    }
    if segments_total > 0 {
        metrics::counter!(
            crate::prometheus::SEGMENTS_PRODUCED_TOTAL,
            "route" => route_handle.name().to_string(),
        )
        .increment(segments_total as u64);
    }
    more
}
