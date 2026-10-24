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
