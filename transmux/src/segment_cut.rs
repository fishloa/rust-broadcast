//! Segment cut policy for the classic TS-HLS segmenter.
//!
//! Apple HLS Authoring Specification (docs/hls-apple-conformance, audit A,
//! requirement 7.4): every segment must begin with an IDR picture.
//! [`SegmentCutPolicy::IdrOnly`] enforces that; [`SegmentCutPolicy::AnyRap`] keeps
//! the broader random-access-point rule of [`crate::access_unit_is_rap`]
//! (recovery-point SEI, SPS + I-slice, CRA/BLA), which DASH and CMAF accept.

/// Which video pictures a segmenter may open a segment on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum SegmentCutPolicy {
    /// Any random-access point the demuxer flagged as a sync sample.
    #[default]
    AnyRap,
    /// Only an IDR picture: H.264 IDR slice, HEVC `IDR_W_RADL` / `IDR_N_LP`.
    /// CRA, BLA, recovery-point SEI and SPS-only pictures are not cut points.
    IdrOnly,
}

impl SegmentCutPolicy {
    /// The short spec token for this policy.
    pub fn name(&self) -> &'static str {
        match self {
            SegmentCutPolicy::AnyRap => "any_rap",
            SegmentCutPolicy::IdrOnly => "idr_only",
        }
    }
}

broadcast_common::impl_spec_display!(SegmentCutPolicy);

const NANOS_PER_SEC: u128 = 1_000_000_000;

/// Media-time bound on how long [`SegmentCutPolicy::IdrOnly`] waits for the
/// first IDR before the caller falls back to any random-access point.
///
/// Open-GOP H.264 (recovery-point SEI, I-slices, no type-5 NAL) never carries
/// an IDR; a segmenter that waits for one forever serves nothing. Feed every
/// dropped video sample to [`IdrWait::observe_dropped`]; it reports `true`
/// once, on the sample at which the dropped media reached the bound.
#[derive(Debug, Clone, Copy)]
pub struct IdrWait {
    bound: core::time::Duration,
    clock: crate::segmenter::MediaClock,
    waited_ticks: u64,
    tripped: bool,
}

impl IdrWait {
    /// A wait that trips after `bound` of dropped media.
    pub const fn new(bound: core::time::Duration) -> Self {
        Self {
            bound,
            clock: crate::segmenter::MediaClock::new(),
            waited_ticks: 0,
            tripped: false,
        }
    }

    /// Account one dropped video `sample` (clock rate `timescale` ticks per
    /// second). Returns `true` exactly once: when the bound is first reached.
    pub fn observe_dropped(&mut self, sample: &crate::Sample, timescale: u32) -> bool {
        self.waited_ticks = self.waited_ticks.saturating_add(self.clock.tick(sample));
        if self.tripped || timescale == 0 {
            return false;
        }
        let waited_ns = u128::from(self.waited_ticks) * NANOS_PER_SEC / u128::from(timescale);
        self.tripped = waited_ns >= self.bound.as_nanos();
        self.tripped
    }

    /// Whether the bound has been reached.
    pub const fn tripped(&self) -> bool {
        self.tripped
    }

    /// The configured bound.
    pub const fn bound(&self) -> core::time::Duration {
        self.bound
    }
}
