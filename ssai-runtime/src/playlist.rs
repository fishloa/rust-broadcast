//! Per-session HLS Interstitial playlist rendering.
//!
//! `EXT-X-DATERANGE CLASS="com.apple.hls.interstitial"` — Appendix D §D.2 of
//! draft-pantos-hls-rfc8216bis, transcribed in full at
//! `broadcast-hls/docs/interstitials.md` (§D.2, examples §D.6/§D.7). Renders
//! over `broadcast-hls`: [`render_session_playlist`] clones the primary
//! [`MediaPlaylist`] and appends this session's interstitial tag line to
//! [`MediaPlaylist::extra_tags`] — the injection point `broadcast-hls`
//! documents for exactly this purpose ("Extra tag lines emitted verbatim
//! before segment entries (e.g. `#EXT-X-DATERANGE:...`)").
//!
//! Implements the attribute set issue #929 scoped: `X-ASSET-URI`/
//! `X-ASSET-LIST`, `X-RESUME-OFFSET`, `X-PLAYOUT-LIMIT`, `X-SNAP`,
//! `X-RESTRICT`. `X-CONTENT-MAY-VARY`, `X-TIMELINE-OCCUPIES`,
//! `X-TIMELINE-STYLE`, and the §D.3 skip-button-control attributes are not
//! modeled.
//!
//! This crate does not do wall-clock math: [`InterstitialDateRange::start_date`]
//! is a caller-supplied, already-formatted ISO-8601/RFC3339 string (the same
//! convention `timed-metadata::daterange::DateRange` uses).

use crate::decision::{AdBreakDecision, AssetSource, RestrictMode, SnapMode};
use crate::error::{Error, Result};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use broadcast_hls::MediaPlaylist;

/// `CLASS` value for an Interstitial `EXT-X-DATERANGE` (Appendix D §D.2).
pub const INTERSTITIAL_CLASS: &str = "com.apple.hls.interstitial";

const TAG: &str = "#EXT-X-DATERANGE:";

/// A rendered Interstitial `EXT-X-DATERANGE` tag: an [`AdBreakDecision`]
/// plus the base-tag scheduling fields (`START-DATE`, `DURATION`, RFC 8216bis
/// §4.4.5.1) the decision itself does not carry.
#[derive(Debug, Clone, PartialEq)]
pub struct InterstitialDateRange {
    /// `ID` (quoted).
    pub id: String,
    /// `START-DATE` (quoted, ISO-8601/RFC3339) — caller-supplied; this
    /// module does no wall-clock math.
    pub start_date: String,
    /// `DURATION` in seconds, if known.
    pub duration: Option<f64>,
    /// `X-ASSET-URI` or `X-ASSET-LIST`.
    pub asset: AssetSource,
    /// `X-RESUME-OFFSET` in seconds.
    pub resume_offset: Option<f64>,
    /// `X-PLAYOUT-LIMIT` in seconds.
    pub playout_limit: Option<f64>,
    /// `X-SNAP` identifiers.
    pub snap: Vec<SnapMode>,
    /// `X-RESTRICT` identifiers.
    pub restrict: Vec<RestrictMode>,
}

impl InterstitialDateRange {
    /// Build from an [`AdBreakDecision`] plus the two base-tag fields it
    /// doesn't carry.
    pub fn from_decision(
        decision: &AdBreakDecision,
        start_date: impl Into<String>,
        duration: Option<f64>,
    ) -> Self {
        InterstitialDateRange {
            id: decision.id.clone(),
            start_date: start_date.into(),
            duration,
            asset: decision.asset.clone(),
            resume_offset: decision.resume_offset,
            playout_limit: decision.playout_limit,
            snap: decision.snap.clone(),
            restrict: decision.restrict.clone(),
        }
    }

    /// Render one `#EXT-X-DATERANGE:` line. Attribute order is fixed (`ID`,
    /// `CLASS`, `START-DATE`, `DURATION`, `X-ASSET-URI`/`X-ASSET-LIST`,
    /// `X-RESUME-OFFSET`, `X-PLAYOUT-LIMIT`, `X-SNAP`, `X-RESTRICT`) so
    /// [`Self::parse_tag_line`] round-trips. Built solely from the typed
    /// fields above — no stored source span is echoed.
    pub fn to_tag_line(&self) -> String {
        let mut out = String::from(TAG);
        out.push_str(&format!("ID=\"{}\"", self.id));
        out.push_str(&format!(",CLASS=\"{INTERSTITIAL_CLASS}\""));
        out.push_str(&format!(",START-DATE=\"{}\"", self.start_date));
        if let Some(d) = self.duration {
            out.push_str(&format!(",DURATION={}", fmt_f64(d)));
        }
        match &self.asset {
            AssetSource::Uri(uri) => out.push_str(&format!(",X-ASSET-URI=\"{uri}\"")),
            AssetSource::List(uri) => out.push_str(&format!(",X-ASSET-LIST=\"{uri}\"")),
        }
        if let Some(v) = self.resume_offset {
            out.push_str(&format!(",X-RESUME-OFFSET={}", fmt_f64(v)));
        }
        if let Some(v) = self.playout_limit {
            out.push_str(&format!(",X-PLAYOUT-LIMIT={}", fmt_f64(v)));
        }
        if !self.snap.is_empty() {
            let list = self
                .snap
                .iter()
                .map(SnapMode::name)
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&format!(",X-SNAP=\"{list}\""));
        }
        if !self.restrict.is_empty() {
            let list = self
                .restrict
                .iter()
                .map(RestrictMode::name)
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&format!(",X-RESTRICT=\"{list}\""));
        }
        out
    }

    /// Parse one `#EXT-X-DATERANGE:` line with
    /// `CLASS="com.apple.hls.interstitial"`. Errors with
    /// [`Error::TagParse`] if the `CLASS` doesn't match (or is missing) or a
    /// required attribute is absent, and [`Error::InvalidAssetSource`] if
    /// neither or both of `X-ASSET-URI`/`X-ASSET-LIST` are present.
    pub fn parse_tag_line(s: &str) -> Result<Self> {
        let body = s
            .strip_prefix(TAG)
            .ok_or_else(|| Error::TagParse("missing #EXT-X-DATERANGE: prefix".to_string()))?;

        let mut id = None;
        let mut class_ok = false;
        let mut start_date = None;
        let mut duration = None;
        let mut uri = None;
        let mut list = None;
        let mut resume_offset = None;
        let mut playout_limit = None;
        let mut snap = Vec::new();
        let mut restrict = Vec::new();

        for (k, v) in split_attrs(body) {
            match k {
                "ID" => id = Some(unquote(v)),
                "CLASS" => class_ok = unquote(v) == INTERSTITIAL_CLASS,
                "START-DATE" => start_date = Some(unquote(v)),
                "DURATION" => duration = Some(parse_f64(v)?),
                "X-ASSET-URI" => uri = Some(unquote(v)),
                "X-ASSET-LIST" => list = Some(unquote(v)),
                "X-RESUME-OFFSET" => resume_offset = Some(parse_f64(v)?),
                "X-PLAYOUT-LIMIT" => playout_limit = Some(parse_f64(v)?),
                "X-SNAP" => snap = parse_snap_list(&unquote(v)),
                "X-RESTRICT" => restrict = parse_restrict_list(&unquote(v)),
                _ => {} // extension attributes ignored (spec-extensible)
            }
        }

        if !class_ok {
            return Err(Error::TagParse(
                "not an Interstitial EXT-X-DATERANGE (CLASS missing or mismatched)".to_string(),
            ));
        }
        let asset = match (uri, list) {
            (Some(u), None) => AssetSource::Uri(u),
            (None, Some(l)) => AssetSource::List(l),
            _ => return Err(Error::InvalidAssetSource),
        };

        Ok(InterstitialDateRange {
            id: id.ok_or_else(|| Error::TagParse("missing ID".to_string()))?,
            start_date: start_date
                .ok_or_else(|| Error::TagParse("missing START-DATE".to_string()))?,
            duration,
            asset,
            resume_offset,
            playout_limit,
            snap,
            restrict,
        })
    }
}

/// Clone `base` and append `active`'s rendered tag line (if any) to
/// [`MediaPlaylist::extra_tags`] — the per-session playlist for one viewer.
/// `base` is otherwise untouched: SSAI needs no per-viewer copy of the media
/// itself (issue #929 design decision), only of this one tag line.
pub fn render_session_playlist(
    base: &MediaPlaylist,
    active: Option<&InterstitialDateRange>,
) -> MediaPlaylist {
    let mut out = base.clone();
    if let Some(dr) = active {
        out.extra_tags.push(dr.to_tag_line());
    }
    out
}

fn fmt_f64(v: f64) -> String {
    // Integer-valued numbers render without a trailing ".0", matching the
    // spec examples (`X-RESUME-OFFSET=0`). Avoid f64::fract() (std-only
    // intrinsic in no_std); use a cast comparison instead.
    let trunc = v as i64;
    if v == trunc as f64 {
        format!("{trunc}")
    } else {
        format!("{v}")
    }
}

fn unquote(v: &str) -> String {
    v.trim_matches('"').to_string()
}

fn parse_f64(v: &str) -> Result<f64> {
    v.parse::<f64>()
        .map_err(|_| Error::TagParse(format!("bad number: {v}")))
}

fn parse_snap_list(v: &str) -> Vec<SnapMode> {
    v.split(',')
        .filter_map(|tok| match tok.trim() {
            "OUT" => Some(SnapMode::Out),
            "IN" => Some(SnapMode::In),
            _ => None,
        })
        .collect()
}

fn parse_restrict_list(v: &str) -> Vec<RestrictMode> {
    v.split(',')
        .filter_map(|tok| match tok.trim() {
            "SKIP" => Some(RestrictMode::Skip),
            "JUMP" => Some(RestrictMode::Jump),
            _ => None,
        })
        .collect()
}

/// Split `K=V,K=V` honouring quoted values (commas inside quotes are not
/// separators) — the same algorithm `timed-metadata::daterange` uses for the
/// base `EXT-X-DATERANGE` tag; duplicated here rather than shared because
/// this module parses a different attribute set (the interstitial `X-`
/// attributes, not `SCTE35-*`) and pulling in `timed-metadata` only for this
/// ~20-line helper would be a heavier dependency than keeping it local.
fn split_attrs(body: &str) -> Vec<(&str, &str)> {
    let mut pairs = Vec::new();
    let bytes = body.as_bytes();
    let (mut start, mut in_q) = (0usize, false);
    let mut i = 0;
    while i <= bytes.len() {
        let at_end = i == bytes.len();
        let c = if at_end { b',' } else { bytes[i] };
        match c {
            b'"' => in_q = !in_q,
            b',' if !in_q => {
                let field = &body[start..i];
                if let Some(eq) = field.find('=') {
                    pairs.push((&field[..eq], &field[eq + 1..]));
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use broadcast_hls::MediaSegment;

    fn sample() -> InterstitialDateRange {
        InterstitialDateRange {
            id: "ad1".to_string(),
            start_date: "2020-01-02T21:55:44.000Z".to_string(),
            duration: Some(15.0),
            asset: AssetSource::Uri("http://example.com/ad1.m3u8".to_string()),
            resume_offset: Some(0.0),
            playout_limit: None,
            snap: Vec::new(),
            restrict: vec![RestrictMode::Skip, RestrictMode::Jump],
        }
    }

    #[test]
    fn tag_line_matches_the_spec_example_shape() {
        // Appendix D §D.6 example, reproduced with this crate's own types.
        let line = sample().to_tag_line();
        assert!(line.starts_with(TAG));
        assert!(line.contains(r#"CLASS="com.apple.hls.interstitial""#));
        assert!(line.contains(r#"X-ASSET-URI="http://example.com/ad1.m3u8""#));
        assert!(line.contains("X-RESUME-OFFSET=0"));
        assert!(line.contains(r#"X-RESTRICT="SKIP,JUMP""#));
    }

    #[test]
    fn tag_line_round_trips() {
        let dr = sample();
        let line = dr.to_tag_line();
        let back = InterstitialDateRange::parse_tag_line(&line).unwrap();
        assert_eq!(back, dr);
    }

    #[test]
    fn asset_list_variant_round_trips() {
        let dr = InterstitialDateRange {
            id: "ad2".to_string(),
            start_date: "2020-01-02T21:55:44.000Z".to_string(),
            duration: Some(30.0),
            asset: AssetSource::List("http://example.com/adv.json".to_string()),
            resume_offset: None,
            playout_limit: Some(20.0),
            snap: vec![SnapMode::Out, SnapMode::In],
            restrict: Vec::new(),
        };
        let line = dr.to_tag_line();
        assert!(line.contains(r#"X-ASSET-LIST="http://example.com/adv.json""#));
        assert!(line.contains(r#"X-SNAP="OUT,IN""#));
        let back = InterstitialDateRange::parse_tag_line(&line).unwrap();
        assert_eq!(back, dr);
    }

    /// Mutating any field must change the rendered output — the anti-cheat
    /// property a raw-passthrough (source-span-echoing) serializer cannot
    /// satisfy, since an echo can't reflect a mutation it didn't store.
    #[test]
    fn mutating_a_field_changes_the_output() {
        let base = sample();
        let base_line = base.to_tag_line();

        let mut mutated = base.clone();
        mutated.id = "different-id".to_string();
        assert_ne!(mutated.to_tag_line(), base_line);

        let mut mutated = base.clone();
        mutated.resume_offset = Some(5.0);
        assert_ne!(mutated.to_tag_line(), base_line);

        let mut mutated = base.clone();
        mutated.restrict = vec![RestrictMode::Skip];
        assert_ne!(mutated.to_tag_line(), base_line);

        let mut mutated = base;
        mutated.asset = AssetSource::Uri("http://example.com/different.m3u8".to_string());
        assert_ne!(mutated.to_tag_line(), base_line);
    }

    #[test]
    fn parse_rejects_wrong_or_missing_class() {
        let line = "#EXT-X-DATERANGE:ID=\"x\",START-DATE=\"2020-01-01T00:00:00Z\",\
                     X-ASSET-URI=\"http://x/a.m3u8\"";
        let err = InterstitialDateRange::parse_tag_line(line).unwrap_err();
        assert!(matches!(err, Error::TagParse(_)));
    }

    #[test]
    fn parse_rejects_both_or_neither_asset_source() {
        let both = "#EXT-X-DATERANGE:ID=\"x\",CLASS=\"com.apple.hls.interstitial\",\
                     START-DATE=\"2020-01-01T00:00:00Z\",X-ASSET-URI=\"http://x/a.m3u8\",\
                     X-ASSET-LIST=\"http://x/list.json\"";
        assert!(matches!(
            InterstitialDateRange::parse_tag_line(both).unwrap_err(),
            Error::InvalidAssetSource
        ));

        let neither = "#EXT-X-DATERANGE:ID=\"x\",CLASS=\"com.apple.hls.interstitial\",\
                        START-DATE=\"2020-01-01T00:00:00Z\"";
        assert!(matches!(
            InterstitialDateRange::parse_tag_line(neither).unwrap_err(),
            Error::InvalidAssetSource
        ));
    }

    #[test]
    fn render_session_playlist_appends_only_for_the_active_session() {
        let mut base = MediaPlaylist {
            target_duration: 6,
            ..Default::default()
        };
        base.segments.push(MediaSegment {
            duration: 6.0,
            uri: "main.ts".to_string(),
            ..Default::default()
        });

        let dr = sample();
        let with_break = render_session_playlist(&base, Some(&dr));
        let without_break = render_session_playlist(&base, None);

        assert!(with_break.to_m3u8().contains("X-ASSET-URI"));
        assert!(!without_break.to_m3u8().contains("X-ASSET-URI"));
        // The base playlist itself (what every other viewer renders from)
        // is untouched.
        assert!(base.extra_tags.is_empty());
        // Only the tag line differs; the segment list is byte-identical.
        assert_eq!(with_break.segments, without_break.segments);
    }
}
