//! Per-route pipeline: pull samples from a [`SampleSource`], feed a
//! [`transmux::ll_hls::LlHlsSegmenter`], and publish the init/parts/segments it
//! produces into a [`crate::store::MediaStore`].
//!
//! One `run_pipeline` future is spawned per configured route; it runs until the
//! source reports end-of-stream (`Ok(None)`) or a hard error.

use std::sync::Arc;

use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{Sample, TrackSpec};

use crate::Result;
use crate::store::MediaStore;

/// CMAF movie timescale used for every route's fragmented `moov`/`moof`
/// (matches [`transmux::pipeline::build_init_segment`]'s convention of a
/// video-rate movie timescale; 90 kHz is the standard MPEG-2/CMAF video clock).
const MOVIE_TIMESCALE: u32 = 90_000;

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

impl SampleSource for crate::source::rtsp::RtspSession {
    fn track_specs(&self) -> Vec<TrackSpec> {
        crate::source::rtsp::RtspSession::track_specs(self)
    }

    async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        crate::source::rtsp::RtspSession::next_samples(self).await
    }
}

impl SampleSource for crate::source::rtp_udp::RtpUdpSession {
    fn track_specs(&self) -> Vec<TrackSpec> {
        crate::source::rtp_udp::RtpUdpSession::track_specs(self)
    }

    async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        crate::source::rtp_udp::RtpUdpSession::next_samples(self).await
    }
}

impl SampleSource for crate::source::ts_udp::TsUdpSession {
    fn track_specs(&self) -> Vec<TrackSpec> {
        crate::source::ts_udp::TsUdpSession::track_specs(self)
    }

    async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        crate::source::ts_udp::TsUdpSession::next_samples(self).await
    }
}

impl SampleSource for crate::source::ts_http::TsHttpSession {
    fn track_specs(&self) -> Vec<TrackSpec> {
        crate::source::ts_http::TsHttpSession::track_specs(self)
    }

    async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        crate::source::ts_http::TsHttpSession::next_samples(self).await
    }
}

impl SampleSource for crate::source::hls_pull::HlsPullSession {
    fn track_specs(&self) -> Vec<TrackSpec> {
        crate::source::hls_pull::HlsPullSession::track_specs(self)
    }

    async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        crate::source::hls_pull::HlsPullSession::next_samples(self).await
    }
}

/// Drive `source` into an [`LlHlsSegmenter`], publishing every init segment,
/// ready part, and ready segment into `store`, until the source reports
/// end-of-stream.
///
/// `route` is used only to label the `multimux_parts_produced_total`/
/// `multimux_segments_produced_total` counters (`crate::prometheus`) bumped
/// as parts/segments land in `store` — it carries no other behaviour.
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
    store: Arc<MediaStore>,
    target_duration_secs: f64,
    part_target_ms: u32,
    mut source: S,
    route: &str,
) -> Result<()> {
    let specs = source.track_specs();
    let mut seg = LlHlsSegmenter::with_part_target(
        specs,
        MOVIE_TIMESCALE,
        target_duration_secs,
        part_target_ms,
    )?;
    store.set_init(seg.init_segment()?);

    while let Some(batch) = source.next_samples().await? {
        for (track_id, sample) in batch {
            seg.push(track_id, sample)?;
        }
        for part in seg.take_ready_parts() {
            store.add_part(part);
            metrics::counter!(crate::prometheus::PARTS_PRODUCED_TOTAL, "route" => route.to_string())
                .increment(1);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(segment);
            metrics::counter!(crate::prometheus::SEGMENTS_PRODUCED_TOTAL, "route" => route.to_string())
                .increment(1);
        }
    }

    seg.flush()?;
    for part in seg.take_ready_parts() {
        store.add_part(part);
        metrics::counter!(crate::prometheus::PARTS_PRODUCED_TOTAL, "route" => route.to_string())
            .increment(1);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(segment);
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
    use crate::store::MediaStore;
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

    #[tokio::test]
    async fn drives_source_through_segmenter_into_store() {
        let store = Arc::new(MediaStore::new(1.0, 500, 8));
        let specs = vec![video_track_spec()];

        // 90 samples @ 3000 ticks/30fps = 3 s of video, comfortably over the
        // 1 s target duration / 500 ms part target — enough to close at least
        // one full segment and several parts before end-of-stream.
        let mut batches = Vec::new();
        for i in 0..90u32 {
            let is_sync = i == 0 || i == 45;
            let data = vec![0xAAu8.wrapping_add(i as u8); 32];
            let sample = Sample::new(data, FRAME_DUR, is_sync, 0);
            batches.push(vec![(1u32, sample)]);
        }

        let source = MockSource::new(specs, batches);
        run_pipeline(store.clone(), 1.0, 500, source, "test-route")
            .await
            .expect("pipeline runs to completion");

        assert!(store.init_bytes().is_some(), "init segment stored");
        let playlist = crate::output::llhls::media_playlist_m3u8(&store, 1);
        assert!(
            playlist.contains("seg-") || playlist.contains("#EXT-X-PART"),
            "playlist has landed media: {playlist}"
        );
    }

    #[tokio::test]
    async fn empty_batches_are_a_no_op() {
        let store = Arc::new(MediaStore::new(1.0, 500, 8));
        let specs = vec![video_track_spec()];
        let source = MockSource::new(specs, vec![Vec::new(), Vec::new()]);
        run_pipeline(store.clone(), 1.0, 500, source, "test-route")
            .await
            .expect("pipeline tolerates empty batches");
        assert!(store.init_bytes().is_some());
    }

    #[tokio::test]
    async fn eos_flush_emits_buffered_tail_segment() {
        // Regression test for the EOS flush path: ensure that samples buffered
        // after the last auto-closed segment are actually emitted via seg.flush().
        let store = Arc::new(MediaStore::new(1.0, 500, 8));
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
            let sample = Sample::new(data, FRAME_DUR, is_sync, 0);
            batches.push(vec![(1u32, sample)]);
        }

        let source = MockSource::new(specs, batches);
        run_pipeline(store.clone(), 1.0, 500, source, "test-route")
            .await
            .expect("pipeline runs to completion");

        assert!(store.init_bytes().is_some(), "init segment stored");
        let playlist = crate::output::llhls::media_playlist_m3u8(&store, 1);

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
