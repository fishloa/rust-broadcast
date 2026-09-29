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
use broadcast_hls::{AttrValue, MediaPlaylist, parse_attribute_list, render_attribute_list};

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
    ///
    /// Every attribute value goes through [`AttrValue::quoted`]/[`bare`]
    /// (issue #1140 / audit r14-SSAI-W1) before
    /// [`broadcast_hls::render_attribute_list`] renders it: `id`,
    /// `start_date` and the asset URI/list are frequently supplied (directly
    /// or indirectly) by an [`crate::decision::AdDecisionProvider`] — a
    /// third-party ad-decision service — so a `"`, CR or LF in any of them
    /// is rejected here rather than terminating the attribute list and
    /// injecting arbitrary tag lines into the session playlist. `duration`/
    /// `resume_offset`/`playout_limit` must also be finite and non-negative
    /// (audit r14-SSAI-W4) — `Error::InvalidDuration` otherwise.
    ///
    /// [`bare`]: AttrValue::bare
    pub fn to_tag_line(&self) -> Result<String> {
        let mut attrs = alloc::vec![
            (String::from("ID"), AttrValue::quoted(self.id.clone())?),
            (
                String::from("CLASS"),
                AttrValue::quoted(INTERSTITIAL_CLASS)?,
            ),
            (
                String::from("START-DATE"),
                AttrValue::quoted(self.start_date.clone())?,
            ),
        ];
        if let Some(d) = self.duration {
            attrs.push((
                String::from("DURATION"),
                AttrValue::bare(fmt_checked_secs("DURATION", d)?)?,
            ));
        }
        match &self.asset {
            AssetSource::Uri(uri) => {
                attrs.push((String::from("X-ASSET-URI"), AttrValue::quoted(uri.clone())?));
            }
            AssetSource::List(uri) => {
                attrs.push((
                    String::from("X-ASSET-LIST"),
                    AttrValue::quoted(uri.clone())?,
                ));
            }
        }
        if let Some(v) = self.resume_offset {
            attrs.push((
                String::from("X-RESUME-OFFSET"),
                AttrValue::bare(fmt_checked_secs("X-RESUME-OFFSET", v)?)?,
            ));
        }
        if let Some(v) = self.playout_limit {
            attrs.push((
                String::from("X-PLAYOUT-LIMIT"),
                AttrValue::bare(fmt_checked_secs("X-PLAYOUT-LIMIT", v)?)?,
            ));
        }
        if !self.snap.is_empty() {
            let list = self
                .snap
                .iter()
                .map(SnapMode::name)
                .collect::<Vec<_>>()
                .join(",");
            attrs.push((String::from("X-SNAP"), AttrValue::quoted(list)?));
        }
        if !self.restrict.is_empty() {
            let list = self
                .restrict
                .iter()
                .map(RestrictMode::name)
                .collect::<Vec<_>>()
                .join(",");
            attrs.push((String::from("X-RESTRICT"), AttrValue::quoted(list)?));
        }
        // `render_attribute_list` always prefixes each entry with `,`
        // (broadcast-hls's convention for appending to an already-started
        // attribute list); build the whole list through it and drop the
        // single leading comma rather than special-casing the first entry.
        let mut body = String::new();
        render_attribute_list(&mut body, &attrs);
        let mut out = String::from(TAG);
        out.push_str(body.trim_start_matches(','));
        Ok(out)
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

        // The shared workspace tokenizer (issue #1140 T12): `attrs` values
        // already have their surrounding `"` stripped for quoted attributes.
        let (attrs, _quoted) = parse_attribute_list(body);

        let id = attrs.get("ID").cloned();
        let class_ok = attrs.get("CLASS").map(String::as_str) == Some(INTERSTITIAL_CLASS);
        let start_date = attrs.get("START-DATE").cloned();
        let duration = attrs
            .get("DURATION")
            .map(|v| parse_checked_secs("DURATION", v))
            .transpose()?;
        let uri = attrs.get("X-ASSET-URI").cloned();
        let list = attrs.get("X-ASSET-LIST").cloned();
        let resume_offset = attrs
            .get("X-RESUME-OFFSET")
            .map(|v| parse_checked_secs("X-RESUME-OFFSET", v))
            .transpose()?;
        let playout_limit = attrs
            .get("X-PLAYOUT-LIMIT")
            .map(|v| parse_checked_secs("X-PLAYOUT-LIMIT", v))
            .transpose()?;
        let snap = attrs
            .get("X-SNAP")
            .map(|v| parse_snap_list(v))
            .unwrap_or_default();
        let restrict = attrs
            .get("X-RESTRICT")
            .map(|v| parse_restrict_list(v))
            .unwrap_or_default();

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
///
/// Fallible (issue #1140 / audit r14-SSAI-W1): an ad-decision-supplied value
/// that fails [`InterstitialDateRange::to_tag_line`]'s validation is an
/// error from this entry point, not a line silently dropped or injected.
pub fn render_session_playlist(
    base: &MediaPlaylist,
    active: Option<&InterstitialDateRange>,
) -> Result<MediaPlaylist> {
    let mut out = base.clone();
    if let Some(dr) = active {
        out.extra_tags.push(dr.to_tag_line()?);
    }
    Ok(out)
}

/// Format a non-negative, finite seconds value without a trailing `.0`,
/// matching the spec examples (`X-RESUME-OFFSET=0`). Avoid `f64::fract()`
/// (std-only intrinsic in `no_std`); use a cast comparison instead.
///
/// Rejects NaN/infinite/negative `v` (issue #1140 / audit r14-SSAI-W4):
/// `NaN as i64 == 0` and `NaN != 0.0`, so an unchecked version of this
/// function rendered `DURATION=NaN` — syntax the §4.2 grammar has no
/// notation for.
fn fmt_checked_secs(what: &'static str, v: f64) -> Result<String> {
    if !v.is_finite() || v < 0.0 {
        return Err(Error::InvalidDuration { what, value: v });
    }
    let trunc = v as i64;
    Ok(if v == trunc as f64 {
        format!("{trunc}")
    } else {
        format!("{v}")
    })
}

/// Parse a decimal-floating-point seconds attribute, rejecting a
/// NaN/infinite/negative value at parse time (issue #1140 / audit
/// r14-SSAI-W4) rather than letting it reach [`fmt_checked_secs`] on the
/// next render.
fn parse_checked_secs(what: &'static str, v: &str) -> Result<f64> {
    let parsed: f64 = v
        .parse()
        .map_err(|_| Error::TagParse(format!("bad number: {v}")))?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(Error::InvalidDuration {
            what,
            value: parsed,
        });
    }
    Ok(parsed)
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use broadcast_hls::{DecimalSeconds, MediaSegment};

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
        let line = sample().to_tag_line().unwrap();
        assert!(line.starts_with(TAG));
        assert!(line.contains(r#"CLASS="com.apple.hls.interstitial""#));
        assert!(line.contains(r#"X-ASSET-URI="http://example.com/ad1.m3u8""#));
        assert!(line.contains("X-RESUME-OFFSET=0"));
        assert!(line.contains(r#"X-RESTRICT="SKIP,JUMP""#));
    }

    #[test]
    fn tag_line_round_trips() {
        let dr = sample();
        let line = dr.to_tag_line().unwrap();
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
        let line = dr.to_tag_line().unwrap();
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
        let base_line = base.to_tag_line().unwrap();

        let mut mutated = base.clone();
        mutated.id = "different-id".to_string();
        assert_ne!(mutated.to_tag_line().unwrap(), base_line);

        let mut mutated = base.clone();
        mutated.resume_offset = Some(5.0);
        assert_ne!(mutated.to_tag_line().unwrap(), base_line);

        let mut mutated = base.clone();
        mutated.restrict = vec![RestrictMode::Skip];
        assert_ne!(mutated.to_tag_line().unwrap(), base_line);

        let mut mutated = base;
        mutated.asset = AssetSource::Uri("http://example.com/different.m3u8".to_string());
        assert_ne!(mutated.to_tag_line().unwrap(), base_line);
    }

    /// Audit r14-SSAI-W1: a `"`, CR or LF in an ad-decision-supplied value
    /// (here `X-ASSET-URI`) must be rejected, not injected into the
    /// playlist as a broken attribute list or an extra tag line.
    #[test]
    fn to_tag_line_rejects_injection_in_asset_uri() {
        let mut dr = sample();
        dr.asset = AssetSource::Uri("http://example.com/ad1.m3u8\"\r\n#EXT-X-ENDLIST".to_string());
        assert!(matches!(
            dr.to_tag_line().unwrap_err(),
            Error::HlsAttrValue(_)
        ));
    }

    #[test]
    fn to_tag_line_rejects_injection_in_id() {
        let mut dr = sample();
        dr.id = "ad\"1".to_string();
        assert!(matches!(
            dr.to_tag_line().unwrap_err(),
            Error::HlsAttrValue(_)
        ));
    }

    /// Audit r14-SSAI-W4: NaN/infinite/negative durations must be rejected,
    /// never rendered as the bare token `NaN`/`inf`/a negative number.
    #[test]
    fn to_tag_line_rejects_non_finite_and_negative_durations() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let mut dr = sample();
            dr.duration = Some(bad);
            let err = dr.to_tag_line().unwrap_err();
            assert!(
                matches!(err, Error::InvalidDuration { .. }),
                "duration {bad} should be rejected, got {err:?}"
            );

            let mut dr = sample();
            dr.resume_offset = Some(bad);
            assert!(matches!(
                dr.to_tag_line().unwrap_err(),
                Error::InvalidDuration { .. }
            ));

            let mut dr = sample();
            dr.playout_limit = Some(bad);
            assert!(matches!(
                dr.to_tag_line().unwrap_err(),
                Error::InvalidDuration { .. }
            ));
        }
    }

    #[test]
    fn parse_rejects_non_finite_duration() {
        let line = "#EXT-X-DATERANGE:ID=\"x\",CLASS=\"com.apple.hls.interstitial\",\
                     START-DATE=\"2020-01-01T00:00:00Z\",X-ASSET-URI=\"http://x/a.m3u8\",\
                     DURATION=nan";
        assert!(matches!(
            InterstitialDateRange::parse_tag_line(line).unwrap_err(),
            Error::InvalidDuration { .. }
        ));
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
            duration: DecimalSeconds::new(6.0).unwrap(),
            uri: "main.ts".to_string(),
            ..Default::default()
        });

        let dr = sample();
        let with_break = render_session_playlist(&base, Some(&dr)).unwrap();
        let without_break = render_session_playlist(&base, None).unwrap();

        assert!(with_break.to_m3u8().unwrap().contains("X-ASSET-URI"));
        assert!(!without_break.to_m3u8().unwrap().contains("X-ASSET-URI"));
        // The base playlist itself (what every other viewer renders from)
        // is untouched.
        assert!(base.extra_tags.is_empty());
        // Only the tag line differs; the segment list is byte-identical.
        assert_eq!(with_break.segments, without_break.segments);
    }
}
