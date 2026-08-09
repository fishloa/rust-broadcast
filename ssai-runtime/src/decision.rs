//! Ad-break decision types.
//!
//! Models the subset of the HLS Interstitials `EXT-X-DATERANGE` attribute
//! set (`CLASS="com.apple.hls.interstitial"`, draft-pantos-hls-rfc8216bis
//! Appendix D §D.2, transcribed in full at
//! `broadcast-hls/docs/interstitials.md`) issue #929 scoped:
//! `X-ASSET-URI`/`X-ASSET-LIST`, `X-RESUME-OFFSET`, `X-PLAYOUT-LIMIT`,
//! `X-SNAP`, `X-RESTRICT`. `X-CONTENT-MAY-VARY`, `X-TIMELINE-OCCUPIES`,
//! `X-TIMELINE-STYLE`, and the §D.3 skip-button-control attributes are not
//! modeled yet — a later addition, not a correction of what is here.
//!
//! The ad-decision server itself (VAST/VMAP, or anything else) is a caller
//! concern per the issue's design decision: [`AdDecisionProvider`] is the
//! only extension point this crate defines, and it performs no I/O — this
//! crate has no HTTP client.

use crate::error::Result;
use alloc::string::String;
use alloc::vec::Vec;

/// `X-SNAP` identifier (Appendix D §D.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SnapMode {
    /// `OUT` — the client should locate the primary-content segment
    /// boundary nearest the interstitial's `START-DATE` and transition
    /// there.
    Out,
    /// `IN` — the client should locate the primary-content segment boundary
    /// nearest the scheduled resumption point and resume there.
    In,
}

impl SnapMode {
    /// Stable label — the literal `X-SNAP` wire token.
    pub fn name(&self) -> &'static str {
        match self {
            SnapMode::Out => "OUT",
            SnapMode::In => "IN",
        }
    }
}
broadcast_common::impl_spec_display!(SnapMode);

/// `X-RESTRICT` identifier (Appendix D §D.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RestrictMode {
    /// `SKIP` — the client must not allow seeking forward or fast playback
    /// while the interstitial plays.
    Skip,
    /// `JUMP` — the client must not allow seeking across the interstitial's
    /// `START-DATE` without first playing it.
    Jump,
}

impl RestrictMode {
    /// Stable label — the literal `X-RESTRICT` wire token.
    pub fn name(&self) -> &'static str {
        match self {
            RestrictMode::Skip => "SKIP",
            RestrictMode::Jump => "JUMP",
        }
    }
}
broadcast_common::impl_spec_display!(RestrictMode);

/// Which of the mutually-exclusive `X-ASSET-URI` / `X-ASSET-LIST` attributes
/// identifies the interstitial content. Appendix D §D.2: "An Interstitial
/// `EXT-X-DATERANGE` tag MUST have either `X-ASSET-URI` or `X-ASSET-LIST`; it
/// MUST NOT have both."
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AssetSource {
    /// `X-ASSET-URI` — a single interstitial asset's absolute URI.
    Uri(String),
    /// `X-ASSET-LIST` — URI of a JSON asset-list resource (the §D.2
    /// `X-ASSET-LIST` JSON schema). This crate carries the URI only; it does
    /// not fetch or parse that JSON — doing so is HTTP, and this crate is
    /// sans-IO.
    List(String),
}

/// The decision for one ad break: which interstitial content to schedule and
/// how the client should treat it, per Appendix D §D.2.
// `resume_offset`/`playout_limit` are `f64`, so this is `PartialEq` only (no
// `Eq`) — the same reason `broadcast_hls::MediaPlaylist` and
// `timed_metadata::DateRange` are `PartialEq`-only.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AdBreakDecision {
    /// `ID` of the rendered `EXT-X-DATERANGE` tag.
    pub id: String,
    /// `X-ASSET-URI` or `X-ASSET-LIST`.
    pub asset: AssetSource,
    /// `X-RESUME-OFFSET` in seconds; `None` = attribute omitted (the client
    /// then treats it as the interstitial's duration, per §D.2).
    pub resume_offset: Option<f64>,
    /// `X-PLAYOUT-LIMIT` in seconds; `None` = attribute omitted.
    pub playout_limit: Option<f64>,
    /// `X-SNAP` identifiers, in the order they should be rendered.
    pub snap: Vec<SnapMode>,
    /// `X-RESTRICT` identifiers, in the order they should be rendered.
    pub restrict: Vec<RestrictMode>,
}

impl AdBreakDecision {
    /// Build a decision for a single-asset (`X-ASSET-URI`) interstitial,
    /// with no resume offset, playout limit, snap, or restrict attributes —
    /// set those fields afterwards as needed.
    pub fn single_asset(id: impl Into<String>, uri: impl Into<String>) -> Self {
        AdBreakDecision {
            id: id.into(),
            asset: AssetSource::Uri(uri.into()),
            resume_offset: None,
            playout_limit: None,
            snap: Vec::new(),
            restrict: Vec::new(),
        }
    }

    /// Build a decision for an asset-list (`X-ASSET-LIST`) interstitial.
    pub fn asset_list(id: impl Into<String>, list_uri: impl Into<String>) -> Self {
        AdBreakDecision {
            id: id.into(),
            asset: AssetSource::List(list_uri.into()),
            resume_offset: None,
            playout_limit: None,
            snap: Vec::new(),
            restrict: Vec::new(),
        }
    }
}

/// Context handed to [`AdDecisionProvider::decide`]: what the SCTE-35 cue
/// told us about the upcoming break, plus which session is asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakContext {
    /// The session requesting a decision.
    pub session_id: String,
    /// `splice_event_id` from the triggering `splice_insert()`, if known.
    pub splice_event_id: Option<u32>,
    /// `out_of_network_indicator` — `true` for break start, `false` for
    /// return-to-network.
    pub out_of_network: bool,
    /// `break_duration().duration`, in 90 kHz ticks, if the cue carried one.
    pub break_duration_ticks: Option<u64>,
    /// The (conditioned) media-timeline instant this break is scheduled to
    /// start at, in the caller's clock unit (typically 90 kHz ticks).
    pub splice_point_pts: u64,
}

/// Pluggable ad-decision hook (issue #929 design decision): VAST/VMAP or any
/// other decisioning scheme is entirely the caller's concern. This trait's
/// only job is to hand back what to render — it performs no I/O itself, and
/// this crate adds no HTTP client to reach one.
pub trait AdDecisionProvider {
    /// Decide what interstitial content to schedule for `ctx`.
    fn decide(&self, ctx: &BreakContext) -> Result<AdBreakDecision>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysSameAd;
    impl AdDecisionProvider for AlwaysSameAd {
        fn decide(&self, ctx: &BreakContext) -> Result<AdBreakDecision> {
            Ok(AdBreakDecision::single_asset(
                alloc::format!("break-{}", ctx.splice_event_id.unwrap_or(0)),
                "https://ads.example.com/creative.m3u8",
            ))
        }
    }

    #[test]
    fn provider_trait_is_object_safe_and_callable() {
        let provider: &dyn AdDecisionProvider = &AlwaysSameAd;
        let ctx = BreakContext {
            session_id: "s1".into(),
            splice_event_id: Some(7),
            out_of_network: true,
            break_duration_ticks: Some(1_800_000),
            splice_point_pts: 12_345,
        };
        let decision = provider.decide(&ctx).unwrap();
        assert_eq!(decision.id, "break-7");
        assert_eq!(
            decision.asset,
            AssetSource::Uri("https://ads.example.com/creative.m3u8".into())
        );
    }

    #[test]
    fn single_asset_and_asset_list_builders() {
        let single = AdBreakDecision::single_asset("a", "https://x/a.m3u8");
        assert_eq!(single.asset, AssetSource::Uri("https://x/a.m3u8".into()));

        let list = AdBreakDecision::asset_list("b", "https://x/list.json");
        assert_eq!(list.asset, AssetSource::List("https://x/list.json".into()));
    }

    #[test]
    fn label_convention() {
        assert_eq!(SnapMode::Out.name(), "OUT");
        assert_eq!(alloc::format!("{}", SnapMode::In), "IN");
        assert_eq!(RestrictMode::Skip.name(), "SKIP");
        assert_eq!(alloc::format!("{}", RestrictMode::Jump), "JUMP");
    }
}
