//! I-frame-only (trick-play / thumbnail) track derivation from a video track,
//! as a sample stage.
//!
//! [`TrickPlayStage`] produces a sparse, all-keyframe rendition of a video
//! track suitable for timeline scrubbing and thumbnail extraction. It keeps
//! only the source's sync samples (random-access points) and stretches each
//! kept sample's duration to cover the gap to the next kept sample, so the
//! total timeline is conserved. It holds at most one sample (the keyframe whose
//! folded duration is not known until the next one arrives), settling the last
//! at [`Stage::finish`].
//!
//! # Spec basis
//!
//! The "sync sample" concept maps directly to the ISO/IEC 14496-12:2026 `stss`
//! (Sync Sample Box, §8.6.2): a sample not listed in `stss` is a non-sync
//! sample (it depends on others for decoding). The trick-play track carries
//! only the samples that *would* appear in `stss`.
//!
//! ## Future signalling work (not implemented here)
//!
//! - **HLS**: the derived track would be signalled in the master playlist via
//!   `EXT-X-I-FRAME-STREAM-INF` (RFC 8216 §4.3.4.3).
//! - **DASH**: the derived track would be placed in an `AdaptationSet` with a
//!   `SupplementalProperty` of `http://dashif.org/guidelines/trickmode`
//!   (ISO/IEC 23009-1 §5.8.5.8). Signalling is deferred; this module derives
//!   the IR-level track only.
//!
//! # Example
//!
//! ```rust
//! use broadcast_common::{Stage, Timestamp};
//! use transmux::trickplay::TrickPlayStage;
//! use transmux::mux_stage::MuxInput;
//! use transmux::{CodecConfig, Sample, TrackId, TrackSpec};
//! use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
//!
//! let spec = TrackSpec::new(
//!     TrackId::new(1),
//!     broadcast_common::timescale::Timescale::try_new(90000).expect("non-zero timescale"),
//!     CodecConfig::Avc {
//!         config: AVCConfigurationBox {
//!             config: AVCDecoderConfigurationRecord {
//!                 configuration_version: 1,
//!                 profile_indication: 66,
//!                 profile_compatibility: 0,
//!                 level_indication: 30,
//!                 length_size_minus_one: 3,
//!                 sps: vec![],
//!                 pps: vec![],
//!                 chroma_format: None,
//!                 bit_depth_luma_minus8: None,
//!                 bit_depth_chroma_minus8: None,
//!                 sps_ext: vec![],
//!             },
//!         },
//!         width: 1920,
//!         height: 1080,
//!     },
//! );
//!
//! // 8 samples, 100 ticks apart: sync at indices 0 and 4.
//! let mut stage = TrickPlayStage::new(TrackId::new(1), spec);
//! for i in 0u8..8 {
//!     let dts = i as i64 * 100;
//!     let sample = Sample::new(vec![i], Some(dts), Some(dts), Some(100), i == 0 || i == 4)
//!         .expect("valid timing");
//!     stage.feed((TrackId::new(1), sample), Timestamp::ZERO).expect("feed");
//! }
//! stage.finish().expect("finish");
//!
//! let mut kept = 0;
//! loop {
//!     match stage.poll() {
//!         Some(MuxInput::Sample { sample, .. }) => {
//!             assert!(sample.flags.is_sync);
//!             assert_eq!(sample.timing.and_then(|t| t.duration()), Some(400));
//!             kept += 1;
//!         }
//!         Some(_) => {}
//!         None => break,
//!     }
//! }
//! assert_eq!(kept, 2);
//! ```

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use core::num::NonZeroU32;

use broadcast_common::stage::{Stage, Timestamp};

use crate::TrackId;
use crate::error::{Error, Result};
use crate::ir::{Sample, SampleTiming, TrackSpec};
use crate::mux_stage::MuxInput;

/// The fallback span for a keyframe whose folded duration is zero: a span is a
/// positive tick count, so a zero-length trick-play sample cannot state one.
const MINIMUM_FOLDED_DURATION: NonZeroU32 = match NonZeroU32::new(1) {
    Some(one) => one,
    None => NonZeroU32::MIN,
};

/// Derive an I-frame-only track from `source` samples, emitting its
/// announcement and samples as [`MuxInput`].
///
/// Every emitted sample is a sync sample; its duration is the sum of source
/// durations from that keyframe up to (but not including) the next one. Source
/// samples before the first keyframe are dropped with their leading interval.
/// A source sample carrying no timestamp keeps `None` — never fabricated.
#[derive(Debug, Clone)]
pub struct TrickPlayStage {
    source: TrackId,
    spec: TrackSpec,
    pending: Option<Sample>,
    accumulated: u32,
    seen_any: bool,
    announced: bool,
    ready: VecDeque<MuxInput>,
    finished: bool,
}

impl TrickPlayStage {
    /// Derive from `source` samples into a track carrying `spec`.
    #[must_use]
    pub fn new(source: TrackId, spec: TrackSpec) -> Self {
        Self {
            source,
            spec,
            pending: None,
            accumulated: 0,
            seen_any: false,
            announced: false,
            ready: VecDeque::new(),
            finished: false,
        }
    }

    fn announce(&mut self) {
        if !self.announced {
            self.announced = true;
            self.ready
                .push_back(MuxInput::Track(Box::new(self.spec.clone())));
        }
    }

    fn emit_pending(&mut self) {
        let Some(mut sample) = self.pending.take() else {
            return;
        };
        sample.timing = fold_duration(sample.timing, self.accumulated);
        sample.flags = crate::ir::SampleFlags::SYNC;
        sample.protection = None;
        self.announce();
        self.ready.push_back(MuxInput::Sample {
            track: self.spec.track_id,
            sample,
        });
    }
}

fn fold_duration(timing: Option<SampleTiming>, duration: u32) -> Option<SampleTiming> {
    let timing = timing?;
    match (timing.dts(), timing.pts()) {
        (Some(dts), Some(pts)) => Some(SampleTiming::timed_with_duration(dts, pts, duration)),
        _ => Some(SampleTiming::DurationOnly {
            duration: NonZeroU32::new(duration).unwrap_or(MINIMUM_FOLDED_DURATION),
        }),
    }
}

impl Stage for TrickPlayStage {
    type In<'a> = (TrackId, Sample);
    type Out = MuxInput;
    type Error = Error;

    fn feed(&mut self, (track, sample): Self::In<'_>, _now: Timestamp) -> Result<()> {
        if track != self.source {
            return Ok(());
        }
        self.seen_any = true;
        let duration = sample.timing.and_then(|t| t.duration()).unwrap_or(0);
        if sample.flags.is_sync {
            self.emit_pending();
            self.pending = Some(sample);
            self.accumulated = duration;
        } else if self.pending.is_some() {
            self.accumulated = self.accumulated.saturating_add(duration);
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        if !self.seen_any || (!self.announced && self.pending.is_none()) {
            return Err(Error::InvalidInput(
                "source track has no sync samples; cannot derive an I-frame-only track",
            ));
        }
        self.emit_pending();
        Ok(())
    }
}
