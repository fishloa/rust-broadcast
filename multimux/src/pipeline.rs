//! Per-route pipeline: pull samples from a [`SampleSource`], feed a
//! [`transmux::ll_hls::LlHlsSegmenter`], and publish the init/parts/segments
//! it produces into a [`crate::route::RouteHandle`] (which forwards them into
//! its `Trunk` — see that type's own docs).
//!
//! One `run_pipeline` future is spawned per configured route; it runs until the
//! source reports end-of-stream (`Ok(None)`) or a hard error.

use std::sync::Arc;

use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{Sample, TrackSpec};

use crate::Result;
use crate::route::RouteHandle;

/// A pull source of depayloaded, timed samples for one or more tracks — the
/// pipeline's input side.
///
/// `#[allow(async_fn_in_trait)]`: this trait is internal to the crate (not a
/// public API contract consumed by unrelated callers), so the usual
/// `async_fn_in_trait` lint concern — that the trait can't spell out `Send`
/// bounds on the returned future for callers who need it — doesn't apply here.
#[allow(async_fn_in_trait)]
pub trait SampleSource {
    /// The track specs to build the init segment from. Called once, before
    /// the first sample is pulled.
    fn track_specs(&self) -> Vec<TrackSpec>;

    /// Pull the next batch of samples, paired with their track id. Returns
    /// `Ok(None)` at end-of-stream; a batch may be empty (e.g. a non-media
    /// event was consumed) without signaling end-of-stream.
    async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>>;
}

// `rtsp`, `rtp_udp`, `ts_udp`, `ts_http`, `srt` (step 5a round 2), `hls_pull`,
// `dash_pull`, `smooth_pull` (step 5a round 3), and `rtmp` (issue #805 task
// 4 — the last of the nine) no longer implement `SampleSource` — they were
// ported onto `media_plane::ingress::{Dialer, Listener, IngestSession}` (see
// their own modules' `run_rtsp`/`run_rtp_udp`/`run_ts_udp`/`run_ts_http`/
// `run_srt_caller`/`run_hls_pull`/`run_dash_pull`/`run_smooth_pull`/
// `run_rtmp`), which publish straight into a `media_plane::Trunk` rather
// than through this trait. `crate::origin::supervisor`/
// `crate::origin::serve_with_registry` still reference the old
// `SourceConnector`/`run_pipeline` shape for `InputSpec::Custom` — see this
// crate's CHANGELOG.

/// Drive `source` into an [`LlHlsSegmenter`], publishing every init segment,
/// ready part, and ready segment into `route_handle`, until the source
/// reports end-of-stream.
///
/// `route` is used only to label the `multimux_parts_produced_total`/
/// `multimux_segments_produced_total` counters (`crate::prometheus`) bumped
/// as parts/segments land in `route_handle` — it carries no other
/// behaviour.
///
/// # Errors
/// Propagates a source read error or a segmenter build failure.
///
/// # Send Bound Footgun
/// The `SampleSource` trait's async method has no explicit `+ Send` bound, which
/// is sound *only* because `run_pipeline<S>` is instantiated at concrete `Send`
/// types (`MockSource`, `RtspSession`). Adding another generic layer over
/// `run_pipeline` could hide Send-ness from `tokio::spawn` and would then require
/// an explicit `+ Send` bound on the inner type.
pub async fn run_pipeline<S: SampleSource>(
    route_handle: Arc<RouteHandle>,
    target_duration_secs: f64,
    part_target_ms: u32,
    mut source: S,
    route: &str,
) -> Result<()> {
    // Egress resolves everything it serves through the route's program
    // registry (issue #805), so a producer that writes into the handle's own
    // `Trunk` must index that `Trunk` there or every request for this route
    // blocks forever waiting for a program that is already present. This is
    // the same call `origin::supervisor::supervise` makes for RTMP; without
    // it, `run_pipeline` ingests correctly and serves nothing.
    //
    // The whole owned-`Trunk`-plus-explicit-publish arrangement is
    // transitional and disappears when the legacy field does.
    route_handle.publish_owned_trunk();
    let specs = source.track_specs();
    // Recorded so a DASH `Output` (issue #663 P4, `crate::output::dash`) can
    // build a real RFC 6381 `codecs` string for its `Representation` — the
    // one thing this route needs beyond the bytes+timing LL-HLS's playlist
    // rendering already covers.
    route_handle.set_track_specs(specs.clone());
    let mut seg = LlHlsSegmenter::with_part_target(
        specs,
        transmux::VIDEO_CLOCK_RATE,
        target_duration_secs,
        part_target_ms,
    )?;
    route_handle.set_init(seg.init_segment()?);

    while let Some(batch) = source.next_samples().await? {
        for (track_id, sample) in batch {
            seg.push(track_id, sample)?;
        }
        for part in seg.take_ready_parts() {
            route_handle.add_part(part);
            metrics::counter!(crate::prometheus::PARTS_PRODUCED_TOTAL, "route" => route.to_string())
                .increment(1);
        }
        for segment in seg.take_ready_segments() {
            route_handle.add_segment(segment);
            metrics::counter!(crate::prometheus::SEGMENTS_PRODUCED_TOTAL, "route" => route.to_string())
                .increment(1);
        }
    }

    seg.flush()?;
    for part in seg.take_ready_parts() {
        route_handle.add_part(part);
        metrics::counter!(crate::prometheus::PARTS_PRODUCED_TOTAL, "route" => route.to_string())
            .increment(1);
    }
    for segment in seg.take_ready_segments() {
        route_handle.add_segment(segment);
        metrics::counter!(crate::prometheus::SEGMENTS_PRODUCED_TOTAL, "route" => route.to_string())
            .increment(1);
    }
    Ok(())
}

/// A [`SampleSource`] driven from a fixed script of pre-built batches, for
/// tests: yields one batch per [`next_samples`](SampleSource::next_samples)
/// call, then `Ok(None)`.
///
/// Feature-gated behind `testsupport` (plus the crate's own `#[cfg(test)]`
/// unit tests): it's test/example scaffolding, not part of the crate's
/// published API contract, so it's compiled out of a normal production build.
/// External integration tests/examples that need it must build with
/// `--features testsupport`.
#[doc(hidden)]
#[cfg(any(test, feature = "testsupport"))]
pub struct MockSource {
    specs: Vec<TrackSpec>,
    batches: std::vec::IntoIter<Vec<(u32, Sample)>>,
}

#[doc(hidden)]
#[cfg(any(test, feature = "testsupport"))]
impl MockSource {
    /// Build a mock yielding each of `batches` in order, one per
    /// `next_samples` call, then ending the stream.
    pub fn new(specs: Vec<TrackSpec>, batches: Vec<Vec<(u32, Sample)>>) -> Self {
        MockSource {
            specs,
            batches: batches.into_iter(),
        }
    }
}

#[cfg(any(test, feature = "testsupport"))]
impl SampleSource for MockSource {
    fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        Ok(self.batches.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Timestamp;
    use ll_hls_runtime::server::{DEFAULT_TRACK_ID, LlHlsBody, LlHlsRequest};
    use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
    use transmux::avc_config_from_sprop;
    use transmux::pipeline::CodecConfig;

    /// A real-ish sprop-parameter-sets pair (SPS+PPS), reused from
    /// `multimux::source::rtsp`'s own tests, decoded into an `avcC` config.
    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";

    /// 90 kHz video timescale — 1/30 s per access unit at 30 fps.
    const VIDEO_TIMESCALE: u32 = 90_000;
    const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;

    fn video_track_spec() -> TrackSpec {
        let config = avc_config_from_sprop(SPROP).expect("valid sprop");
        TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 0,
                height: 0,
            },
        )
    }

    /// Render `route`'s current LL-HLS media playlist synchronously — the
    /// direct `ServedEgress::resolve` call these tests need in place of the
    /// deleted `crate::output::llhls::media_playlist_m3u8` re-export (Step 4
    /// moved playlist rendering behind `LlHlsOrigin`/`ServedEgress`).
    fn render_playlist(route: &RouteHandle) -> String {
        match route.ll_hls().resolve(
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: Default::default(),
            },
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(0)),
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn drives_source_through_segmenter_into_store() {
        let route = Arc::new(RouteHandle::new(1.0, 500, 8));
        let specs = vec![video_track_spec()];

        // 90 samples @ 3000 ticks/30fps = 3 s of video, comfortably over the
        // 1 s target duration / 500 ms part target — enough to close at least
        // one full segment and several parts before end-of-stream.
        let mut batches = Vec::new();
        for i in 0..90u32 {
            let is_sync = i == 0 || i == 45;
            let data = vec![0xAAu8.wrapping_add(i as u8); 32];
            let sample = Sample::new(
                data,
                Some(i64::from(i) * i64::from(FRAME_DUR)),
                Some(i64::from(i) * i64::from(FRAME_DUR)),
                Some(FRAME_DUR),
                is_sync,
            );
            batches.push(vec![(1u32, sample)]);
        }

        let source = MockSource::new(specs, batches);
        run_pipeline(route.clone(), 1.0, 500, source, "test-route")
            .await
            .expect("pipeline runs to completion");

        assert!(route.init_bytes().is_some(), "init segment stored");
        let playlist = render_playlist(&route);
        assert!(
            playlist.contains("seg-") || playlist.contains("#EXT-X-PART"),
            "playlist has landed media: {playlist}"
        );
    }

    #[tokio::test]
    async fn empty_batches_are_a_no_op() {
        let route = Arc::new(RouteHandle::new(1.0, 500, 8));
        let specs = vec![video_track_spec()];
        let source = MockSource::new(specs, vec![Vec::new(), Vec::new()]);
        run_pipeline(route.clone(), 1.0, 500, source, "test-route")
            .await
            .expect("pipeline tolerates empty batches");
        assert!(route.init_bytes().is_some());
    }

    #[tokio::test]
    async fn eos_flush_emits_buffered_tail_segment() {
        // Regression test for the EOS flush path: ensure that samples buffered
        // after the last auto-closed segment are actually emitted via seg.flush().
        let route = Arc::new(RouteHandle::new(1.0, 500, 8));
        let specs = vec![video_track_spec()];

        // 60 frames @ 30fps = 2s total:
        // - Frame 0 (sync, t=0): segment start
        // - Frame 45 (sync, t=1.5s): exceeds 1s target, triggers auto-close
        // - Frames 46-59 (non-sync, t=1.5s..2s): buffered tail, only emitted via flush()
        //
        // Without the seg.flush() + drain block, frames 46-59 would be discarded,
        // resulting in only seg-1-1. With flush(), a second segment (seg-1-2) is
        // emitted from the buffered samples.
        let mut batches = Vec::new();
        for i in 0..60u32 {
            let is_sync = i == 0 || i == 45;
            let data = vec![0xCCu8.wrapping_add(i as u8); 32];
            let sample = Sample::new(
                data,
                Some(i64::from(i) * i64::from(FRAME_DUR)),
                Some(i64::from(i) * i64::from(FRAME_DUR)),
                Some(FRAME_DUR),
                is_sync,
            );
            batches.push(vec![(1u32, sample)]);
        }

        let source = MockSource::new(specs, batches);
        run_pipeline(route.clone(), 1.0, 500, source, "test-route")
            .await
            .expect("pipeline runs to completion");

        assert!(route.init_bytes().is_some(), "init segment stored");
        let playlist = render_playlist(&route);

        // Assertion bites the flush path: playlist MUST contain seg-1-2.
        // This proves the buffered tail after frame 45 was flushed and emitted as
        // a second segment. Without seg.flush(), only seg-1-1 would exist.
        assert!(
            playlist.contains("seg-1-2"),
            "seg.flush() must emit buffered tail as seg-1-2, got playlist: {}",
            playlist
        );
    }
}
