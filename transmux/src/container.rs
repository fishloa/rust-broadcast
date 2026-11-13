//! The container a segment is cut as.

/// Which container a segmenter cuts and an origin serves segments/parts as,
/// orthogonal to whether LL-HLS is enabled; issue #873.
///
/// RFC 8216bis §3.1.1 / §3.1.2 give the two containers different
/// `#EXT-X-MAP` obligations:
///
/// - fMP4 (§3.1.2): "Each fMP4 Segment in a Media Playlist MUST have an
///   `EXT-X-MAP` tag applied to it" — unconditional, so [`Container::Fmp4`]
///   always emits one.
/// - MPEG-2 TS (§3.1.1): "Each Transport Stream Segment MUST contain a PAT
///   and a PMT, **or** have an `EXT-X-MAP` tag applied to it" — a
///   disjunction, not a container restriction. `EXT-X-MAP` is legal for TS;
///   it is not required when the segments carry their own PAT/PMT.
///
/// [`Container::MpegTs`] omits `#EXT-X-MAP` **by default**, on the
/// assumption that segments come from a self-initialising source (e.g.
/// `transmux`'s TS segmenter, which re-emits PAT+PMT at the head of every
/// segment) — it does not *forbid* the tag; a future caller feeding
/// pre-segmented TS without in-band PSI would need a way to opt back in,
/// which is not implemented here (out of scope for issue #873; the current
/// wiring never calls `set_init` from a `MpegTs`-configured pipeline, so the
/// gap has no live caller yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Container {
    /// Fragmented MP4 / CMAF segments (`.m4s`), with an fMP4 init segment
    /// (`.mp4`) referenced by an always-present `#EXT-X-MAP`.
    Fmp4,
    /// Whole MPEG-2 Transport Stream segments (`.ts`), self-initialising
    /// (in-band PAT/PMT) — no `#EXT-X-MAP`, no init segment served.
    MpegTs,
    /// WebVTT subtitle segments (`.vtt`), each starting with its own WebVTT
    /// header (RFC 8216 §3.5), so no `#EXT-X-MAP` and no init segment. With a
    /// `low_latency` setting a WebVTT origin serves Partial Segments exactly
    /// like the other containers: each part is itself a complete WebVTT
    /// document (RFC 8216bis §3.1.4).
    WebVtt,
}

impl Container {
    /// The spec token for this container — `"fmp4"` / `"mpeg-ts"` / `"webvtt"`.
    pub fn name(&self) -> &'static str {
        match self {
            Container::Fmp4 => "fmp4",
            Container::MpegTs => "mpeg-ts",
            Container::WebVtt => "webvtt",
        }
    }

    /// The dynamic-filename extension (without the leading `.`) this
    /// container's segments/parts are named with.
    pub fn segment_extension(self) -> &'static str {
        match self {
            Container::Fmp4 => "m4s",
            Container::MpegTs => "ts",
            Container::WebVtt => "vtt",
        }
    }
}

broadcast_common::impl_spec_display!(Container);

impl Default for Container {
    /// [`Container::Fmp4`] — preserves every pre-#873 caller's behaviour.
    fn default() -> Self {
        Container::Fmp4
    }
}
