//! HLS playlist generation — RFC 8216.
//!
//! Produces `#EXTM3U`-formatted media and master playlists from structured
//! data, suitable for VOD and live CMAF workflows.
//!
//! # Trick-play (I-frame-only) signalling
//!
//! HLS supports two complementary tags for trick-play (timeline scrubbing /
//! thumbnail extraction) renditions; both are strictly opt-in so existing
//! playlists are byte-for-byte unchanged:
//!
//! - **`#EXT-X-I-FRAME-STREAM-INF`** (RFC 8216 §4.3.4.2) — a master-playlist
//!   tag declaring an I-frame-only rendition.  Unlike `#EXT-X-STREAM-INF` the
//!   URI is an *attribute* on the tag line itself, not a following line.  Add
//!   one [`IFrameVariant`] per rendition to [`MasterPlaylist::iframe_variants`];
//!   `to_m3u8` renders each as
//!   `#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=<n>[,CODECS="<c>"][,RESOLUTION=<w>x<h>],URI="<uri>"`.
//!
//! - **`#EXT-X-I-FRAMES-ONLY`** (RFC 8216 §4.3.3.6) — a media-playlist tag
//!   declaring that every segment carries a single I-frame.  Set
//!   [`MediaPlaylist::iframes_only`] to `true`; `to_m3u8` emits the tag in
//!   the header block (after the version line).  RFC 8216 §4.3.3.6 requires
//!   protocol version ≥ 4 when this tag is present; the renderer enforces
//!   this by emitting `max(self.version, 4)`.
//!
//! # Discontinuity support
//!
//! The playlist model supports RFC 8216 discontinuity signalling:
//!
//! - **`#EXT-X-DISCONTINUITY`** (RFC 8216 §4.3.4.3) — a marker emitted
//!   immediately before the `#EXTINF` of a discontinuous [`MediaSegment`]
//!   (one whose [`MediaSegment::discontinuous`] flag is `true`). It signals
//!   a break in the media timeline between the preceding segment and the one
//!   that follows it (change in encoding, timestamps, tracks, or format).
//!
//! - **`#EXT-X-DISCONTINUITY-SEQUENCE`** (RFC 8216 §4.3.3.3) — a header
//!   tag equal to the count of discontinuities that have already rolled off
//!   the front of a live/sliding-window playlist. Emitted as
//!   `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` when `n > 0`; absent (defaulting
//!   to 0) otherwise.
//!
//! Use [`Segmenter::mark_discontinuity`](crate::Segmenter::mark_discontinuity)
//! to mark the next cut as discontinuous; the segmenter also auto-detects
//! init-segment changes and marks those cuts automatically.
//!
//! # Low-Latency HLS (RFC 8216bis)
//!
//! Low-Latency HLS (LL-HLS — the HLS 2nd edition draft, *RFC 8216bis*) drives
//! end-to-end latency below one segment duration by publishing each segment's
//! **partial segments** ("parts", RFC 8216bis §4.4.4.9) as they are produced,
//! before the parent segment is complete. This model adds four opt-in playlist
//! directives, all rendered only when [`MediaPlaylist::low_latency`] is set (so a
//! plain playlist is byte-for-byte unchanged):
//!
//! - **`#EXT-X-SERVER-CONTROL`** (RFC 8216bis §4.4.3.8) — the header carries
//!   `CAN-BLOCK-RELOAD=YES` (the server supports blocking playlist reload) and
//!   `PART-HOLD-BACK=<sec>` (how far from the live edge a client may play parts).
//!   Per the spec, `PART-HOLD-BACK` MUST be at least **three times** the
//!   part-target duration.
//! - **`#EXT-X-PART-INF:PART-TARGET=<sec>`** (RFC 8216bis §4.4.3.7) — the header
//!   declaring the part-target duration.
//! - **`#EXT-X-PART:DURATION=<sec>,URI="<uri>"[,INDEPENDENT=YES]`**
//!   (RFC 8216bis §4.4.4.9) — one line per part, emitted before the parent
//!   segment's `#EXTINF`. `INDEPENDENT=YES` marks a part that begins with an
//!   independently decodable frame (a sync sample).
//! - **`#EXT-X-PRELOAD-HINT:TYPE=PART,URI="<next-part-uri>"`**
//!   (RFC 8216bis §4.4.5.3) — hints the URI of the next, not-yet-available part
//!   so a client can request it ahead of time.
//!
//! A live origin's trailing segment is often still *open* — being filled in by
//! new parts as they are produced, not yet closed with a duration and URI.
//! [`MediaPlaylist::open_segment`] carries that in-progress
//! [`OpenSegment`]'s known parts; `to_m3u8` renders them as trailing
//! `#EXT-X-PART` lines with **no** `#EXTINF`/URI (RFC 8216bis §4.4.4.9), same
//! opt-in gating as the closed segments' parts above.
//!
//! # CENC/CBCS DRM signalling (ISO/IEC 23001-7, issue #564)
//!
//! [`cenc_ext_x_key`] renders the `#EXT-X-KEY` tag line for a `cbcs`
//! (AES-128 pattern CBC)-protected CMAF track — the CMAF-HLS case Apple's
//! HLS authoring guidance carries as `METHOD=SAMPLE-AES`. Push the returned
//! line into [`MediaPlaylist::extra_tags`] (before the segments it
//! protects). `cenc` (AES-128 full-block CTR) has **no** valid HLS `METHOD`
//! — CTR is not one of HLS's two encryption methods (`SAMPLE-AES`/
//! `AES-128`, both CBC) — so `cenc`-protected CMAF is signalling-only on the
//! DASH side (`crate::dash`); `cenc_ext_x_key` returns `None` rather than
//! emit an invalid tag.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cenc::CencScheme;

// ---------------------------------------------------------------------------
// CENC/CBCS DRM signalling — ISO/IEC 23001-7 `cbcs` over CMAF-HLS (issue #564).
// ---------------------------------------------------------------------------

/// `KEYFORMAT` for the generic CENC identification (mirrors DASH's
/// `ContentProtection@schemeIdUri` for the "common" scheme —
/// ISO/IEC 23001-7 / `urn:mpeg:dash:mp4protection:2011`).
pub const CENC_KEYFORMAT: &str = "urn:mpeg:dash:mp4protection:2011";

/// `KEYFORMATVERSIONS` for [`CENC_KEYFORMAT`] (there is only version `"1"`).
pub const CENC_KEYFORMATVERSIONS: &str = "1";

/// Build the `#EXT-X-KEY` tag line for a `cbcs`-protected CMAF track
/// (RFC 8216 §4.3.2.4 `METHOD=SAMPLE-AES`, `KEYFORMAT`/`KEYFORMATVERSIONS`
/// per [`CENC_KEYFORMAT`]/[`CENC_KEYFORMATVERSIONS`], plus the `KEYID`
/// attribute Apple's HLS CMAF/fMP4 authoring guidance uses to identify the
/// CENC key ID).
///
/// Returns `None` for [`CencScheme::Cenc`] (AES-128 full-block CTR): CTR is
/// not a valid HLS `METHOD` (HLS only speaks `SAMPLE-AES`/`AES-128`, both
/// CBC), so `cenc`-protected CMAF has no HLS key tag — it is DASH-only (see
/// the module docs).
///
/// `key_uri` is caller-supplied (a key-server URL, `skd://`, or `data:`
/// URI — no DRM logic lives here) and `kid` is the track's
/// [`crate::cenc::TrackEncryptionBox::default_kid`]
/// (`crate::media::TrackEncryption::tenc::default_kid`).
pub fn cenc_ext_x_key(scheme: CencScheme, kid: &[u8; 16], key_uri: &str) -> Option<String> {
    if scheme != CencScheme::Cbcs {
        return None;
    }
    Some(format!(
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"{key_uri}\",KEYFORMAT=\"{CENC_KEYFORMAT}\",\
         KEYFORMATVERSIONS=\"{CENC_KEYFORMATVERSIONS}\",KEYID=0x{}",
        crate::rtp::hex_encode(kid)
    ))
}

/// A single partial segment ("part") of a [`MediaSegment`] — RFC 8216bis
/// §4.4.4.9 (`#EXT-X-PART`).
///
/// A part is an independently addressable CMAF chunk (a `moof`+`mdat` fragment)
/// covering a sub-duration of its parent segment; a client can fetch and play it
/// before the parent segment is complete. Parts are emitted as `#EXT-X-PART`
/// lines immediately before the parent segment's `#EXTINF`.
#[derive(Debug, Clone, PartialEq)]
pub struct PartSpec {
    /// The part URI (e.g. `"seg0.1.m4s"`).
    pub uri: String,
    /// The part duration in seconds (e.g. `0.334`).
    pub duration: f64,
    /// If `true`, render `,INDEPENDENT=YES` — the part begins with an
    /// independently decodable frame (a sync sample). RFC 8216bis §4.4.4.9.
    pub independent: bool,
}

/// An in-progress (open) LL-HLS segment: its parts are known and being served,
/// but the segment is not yet complete, so it carries no `#EXTINF`/URI
/// (RFC 8216bis §4.4.4.9 — an open segment is represented by its trailing
/// `#EXT-X-PART` lines only, until it closes).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenSegment {
    /// The parts of the in-progress segment, in order.
    pub parts: Vec<PartSpec>,
}

/// A single media segment in a media playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaSegment {
    /// The segment URI (e.g. `"seg0.m4s"`).
    pub uri: String,
    /// The segment duration in seconds (e.g. `9.009`).
    pub duration: f64,
    /// If `true`, emit `#EXT-X-DISCONTINUITY` immediately before this
    /// segment's `#EXTINF` line — RFC 8216 §4.3.4.3.
    pub discontinuous: bool,
    /// Low-Latency HLS partial segments of this segment (RFC 8216bis §4.4.4.9).
    /// Rendered as `#EXT-X-PART` lines *before* this segment's `#EXTINF`, but
    /// only when the playlist is low-latency (see [`MediaPlaylist::low_latency`]).
    /// Empty for a non-low-latency playlist or a segment whose parts have already
    /// been coalesced into the full `#EXTINF`.
    pub parts: Vec<PartSpec>,
}

/// A media playlist (`#EXTM3U` / `#EXTINF` / ...).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    /// `#EXT-X-VERSION`
    pub version: u8,
    /// `#EXT-X-TARGETDURATION` — must be >= the max rounded segment duration.
    pub target_duration: u32,
    /// `#EXT-X-MEDIA-SEQUENCE`
    pub media_sequence: u64,
    /// `#EXT-X-DISCONTINUITY-SEQUENCE` (RFC 8216 §4.3.3.3) — the count of
    /// discontinuities that have already rolled off the front of a live
    /// sliding-window playlist and are no longer represented by any in-window
    /// `#EXT-X-DISCONTINUITY` tag. Emitted as
    /// `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` when `n > 0`; omitted when `0`
    /// (which is the implicit default per the spec).
    pub discontinuity_sequence: u64,
    /// Ordered list of segments.
    pub segments: Vec<MediaSegment>,
    /// The in-progress (open) segment, if any — rendered as trailing
    /// `#EXT-X-PART` lines with no `#EXTINF` (LL-HLS live edge).
    pub open_segment: Option<OpenSegment>,
    /// If `true`, append `#EXT-X-ENDLIST`.
    pub endlist: bool,
    /// Extra tag lines emitted verbatim before segment entries
    /// (e.g. `#EXT-X-DATERANGE:...`).
    pub extra_tags: Vec<String>,
    /// Low-Latency HLS configuration (RFC 8216bis). When `Some`, `to_m3u8`
    /// renders the LL-HLS directives — `#EXT-X-SERVER-CONTROL`,
    /// `#EXT-X-PART-INF`, each segment's `#EXT-X-PART` lines, and (if set) the
    /// `#EXT-X-PRELOAD-HINT`. When `None` (the default), none of these appear —
    /// LL-HLS is strictly opt-in and a plain playlist is unchanged.
    pub low_latency: Option<LowLatencyConfig>,
    /// If `true`, emit `#EXT-X-I-FRAMES-ONLY` (RFC 8216 §4.3.3.6) in the
    /// header block, declaring that every segment in this playlist carries a
    /// single I-frame (a trick-play / thumbnail rendition).  When `true` the
    /// rendered version is at least 4 (RFC 8216 §4.3.3.6 requirement).
    pub iframes_only: bool,
}

/// Low-Latency HLS playlist configuration — RFC 8216bis.
///
/// Presence of this config on a [`MediaPlaylist`] switches on the LL-HLS
/// directives (`#EXT-X-SERVER-CONTROL`, `#EXT-X-PART-INF`, `#EXT-X-PART`,
/// `#EXT-X-PRELOAD-HINT`); see the module docs for each tag's spec section.
#[derive(Debug, Clone, PartialEq)]
pub struct LowLatencyConfig {
    /// Part-target duration in seconds — the `PART-TARGET` of `#EXT-X-PART-INF`
    /// (RFC 8216bis §4.4.3.7). Typically 0.2–0.5 s.
    pub part_target: f64,
    /// `PART-HOLD-BACK` in seconds — the `#EXT-X-SERVER-CONTROL` attribute
    /// (RFC 8216bis §4.4.3.8). MUST be at least `3 × part_target`; the renderer
    /// raises it to that floor if a smaller value is supplied.
    pub part_hold_back: f64,
    /// URI of the next, not-yet-available part — rendered as
    /// `#EXT-X-PRELOAD-HINT:TYPE=PART,URI="<uri>"` (RFC 8216bis §4.4.5.3). When
    /// `None`, no preload hint is emitted (e.g. an ended playlist).
    pub preload_hint_part: Option<String>,
}

impl LowLatencyConfig {
    /// The `PART-HOLD-BACK` value actually rendered: at least `3 × part_target`
    /// per RFC 8216bis §4.4.3.8, even if [`Self::part_hold_back`] is smaller.
    pub fn effective_part_hold_back(&self) -> f64 {
        let floor = 3.0 * self.part_target;
        if self.part_hold_back < floor {
            floor
        } else {
            self.part_hold_back
        }
    }
}

impl MediaPlaylist {
    /// Render this media playlist as an RFC 8216 `#EXTM3U` string.
    ///
    /// Emits `#EXT-X-DISCONTINUITY-SEQUENCE:<n>` after the media-sequence
    /// header when `discontinuity_sequence > 0` (RFC 8216 §4.3.3.3), and
    /// `#EXT-X-DISCONTINUITY` immediately before the `#EXTINF` of every
    /// segment whose [`MediaSegment::discontinuous`] flag is `true`
    /// (RFC 8216 §4.3.4.3).
    ///
    /// When [`Self::iframes_only`] is `true`, emits `#EXT-X-I-FRAMES-ONLY`
    /// (RFC 8216 §4.3.3.6) in the header block and renders version ≥ 4 as
    /// required by that section.
    pub fn to_m3u8(&self) -> String {
        let mut s = String::new();
        s.push_str("#EXTM3U\n");
        // RFC 8216 §4.3.3.6: EXT-X-I-FRAMES-ONLY requires protocol version >= 4.
        let version = if self.iframes_only {
            self.version.max(4)
        } else {
            self.version
        };
        s.push_str(&format!("#EXT-X-VERSION:{version}\n"));
        if self.iframes_only {
            s.push_str("#EXT-X-I-FRAMES-ONLY\n");
        }
        s.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", self.target_duration));
        s.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", self.media_sequence));
        if self.discontinuity_sequence > 0 {
            s.push_str(&format!(
                "#EXT-X-DISCONTINUITY-SEQUENCE:{}\n",
                self.discontinuity_sequence
            ));
        }

        // Low-Latency HLS header directives (RFC 8216bis §4.4.3.7/§4.4.3.8),
        // opt-in via `low_latency`.
        if let Some(ll) = &self.low_latency {
            // #EXT-X-SERVER-CONTROL — CAN-BLOCK-RELOAD + PART-HOLD-BACK (>= 3×
            // part-target, enforced by effective_part_hold_back).
            s.push_str(&format!(
                "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK={}\n",
                format_secs(ll.effective_part_hold_back()),
            ));
            // #EXT-X-PART-INF — the part-target duration.
            s.push_str(&format!(
                "#EXT-X-PART-INF:PART-TARGET={}\n",
                format_secs(ll.part_target),
            ));
        }

        for tag in &self.extra_tags {
            s.push_str(tag);
            s.push('\n');
        }

        for seg in &self.segments {
            if seg.discontinuous {
                s.push_str("#EXT-X-DISCONTINUITY\n");
            }
            // LL-HLS partial segments precede the parent's #EXTINF
            // (RFC 8216bis §4.4.4.9), rendered only for a low-latency playlist.
            if self.low_latency.is_some() {
                for part in &seg.parts {
                    push_part_line(&mut s, part);
                }
            }
            // Format with exactly 3 decimal places per RFC 8216 examples.
            s.push_str(&format!("#EXTINF:{:.3},\n", seg.duration));
            s.push_str(&seg.uri);
            s.push('\n');
        }

        // The in-progress (open) segment at the live edge — its parts are
        // known but it has not yet closed, so it carries no #EXTINF/URI
        // (RFC 8216bis §4.4.4.9). Rendered only for a low-latency playlist,
        // same opt-in gating as the closed segments' parts above.
        if self.low_latency.is_some() {
            if let Some(open) = &self.open_segment {
                for part in &open.parts {
                    push_part_line(&mut s, part);
                }
            }
        }

        // LL-HLS preload hint for the next not-yet-available part
        // (RFC 8216bis §4.4.5.3) — after the segment list, before ENDLIST.
        if let Some(ll) = &self.low_latency {
            if let Some(uri) = &ll.preload_hint_part {
                s.push_str(&format!("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"{uri}\"\n"));
            }
        }

        if self.endlist {
            s.push_str("#EXT-X-ENDLIST\n");
        }

        s
    }
}

/// Render one `#EXT-X-PART:DURATION=<sec>,URI="<uri>"[,INDEPENDENT=YES]` line
/// (RFC 8216bis §4.4.4.9) into `s`, shared by both a closed segment's parts and
/// an open (in-progress) segment's parts so the two can never drift in format.
fn push_part_line(s: &mut String, part: &PartSpec) {
    s.push_str(&format!(
        "#EXT-X-PART:DURATION={},URI=\"{}\"",
        format_secs(part.duration),
        part.uri,
    ));
    if part.independent {
        s.push_str(",INDEPENDENT=YES");
    }
    s.push('\n');
}

/// Format a non-negative seconds value with up to three decimal places, trailing
/// zeros trimmed (`0.334`, `1.5`, `6`) — the HLS decimal-floating-point form
/// (RFC 8216bis §4.2). Integer millisecond math (no `std` float-format intrinsic
/// beyond core `Display`, so it holds under `no_std`+`alloc`).
fn format_secs(v: f64) -> String {
    let millis = (v * 1000.0 + 0.5) as u64;
    let whole = millis / 1000;
    let frac = millis % 1000;
    if frac == 0 {
        return format!("{whole}");
    }
    let mut f = format!("{frac:03}");
    while f.ends_with('0') {
        f.pop();
    }
    format!("{whole}.{f}")
}

/// A variant stream entry in a master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// `BANDWIDTH` in bits per second.
    pub bandwidth: u32,
    /// `CODECS` string (e.g. `"avc1.64001e,mp4a.40.2"`).
    pub codecs: String,
    /// `RESOLUTION` as `(width, height)`, if present.
    pub resolution: Option<(u32, u32)>,
    /// URI of the media playlist for this variant.
    pub uri: String,
}

/// An I-frame-only rendition entry for a master playlist — RFC 8216 §4.3.4.2
/// (`#EXT-X-I-FRAME-STREAM-INF`).
///
/// Unlike [`Variant`] / `#EXT-X-STREAM-INF`, the URI is an *attribute* on the
/// tag line itself (not on a following line).  Rendered as:
/// ```text
/// #EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=<n>[,CODECS="<c>"][,RESOLUTION=<w>x<h>],URI="<uri>"
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct IFrameVariant {
    /// `BANDWIDTH` in bits per second (required).
    pub bandwidth: u32,
    /// `CODECS` RFC 6381 string (e.g. `"hvc1.1.6.L93.B0"`).  `None` to omit.
    pub codecs: Option<String>,
    /// `RESOLUTION` as `(width, height)`.  `None` to omit.
    pub resolution: Option<(u32, u32)>,
    /// URI of the I-frame-only media playlist.
    pub uri: String,
}

/// A master playlist (`#EXTM3U` / `#EXT-X-STREAM-INF` / ...).
#[derive(Debug, Clone, PartialEq)]
pub struct MasterPlaylist {
    /// `#EXT-X-VERSION`
    pub version: u8,
    /// Ordered list of variant streams.
    pub variants: Vec<Variant>,
    /// Ordered list of I-frame-only renditions (RFC 8216 §4.3.4.2).
    ///
    /// Each entry is rendered as an `#EXT-X-I-FRAME-STREAM-INF` line with the
    /// URI as an attribute (not a following line).  An empty `Vec` (the
    /// default) produces no such lines.
    pub iframe_variants: Vec<IFrameVariant>,
}

impl MasterPlaylist {
    /// Render this master playlist as an RFC 8216 `#EXTM3U` string.
    ///
    /// After the regular `#EXT-X-STREAM-INF` variant lines, emits one
    /// `#EXT-X-I-FRAME-STREAM-INF` line per entry in
    /// [`Self::iframe_variants`] (RFC 8216 §4.3.4.2).  The URI is rendered
    /// as an attribute on the tag line itself — *not* on a following line.
    pub fn to_m3u8(&self) -> String {
        let mut s = String::new();
        s.push_str("#EXTM3U\n");
        s.push_str(&format!("#EXT-X-VERSION:{}\n", self.version));

        for var in &self.variants {
            s.push_str(&format!(
                "#EXT-X-STREAM-INF:BANDWIDTH={},CODECS=\"{}\"",
                var.bandwidth, var.codecs,
            ));
            if let Some((w, h)) = var.resolution {
                s.push_str(&format!(",RESOLUTION={w}x{h}"));
            }
            s.push('\n');
            s.push_str(&var.uri);
            s.push('\n');
        }

        // I-frame-only renditions — RFC 8216 §4.3.4.2.
        // URI is an attribute on the tag line, not a following URI line.
        for iv in &self.iframe_variants {
            s.push_str(&format!(
                "#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH={}",
                iv.bandwidth
            ));
            if let Some(c) = &iv.codecs {
                s.push_str(&format!(",CODECS=\"{c}\""));
            }
            if let Some((w, h)) = iv.resolution {
                s.push_str(&format!(",RESOLUTION={w}x{h}"));
            }
            s.push_str(&format!(",URI=\"{}\"\n", iv.uri));
        }

        s
    }
}

/// Auto-detect init-segment changes across a sequence of segments and mark the
/// first segment that follows an init change as discontinuous (RFC 8216 §4.3.4.3).
///
/// `entries` is an ordered list of `(init_bytes, segment)` pairs — one per
/// media segment in playlist order. For each segment after the first, if its
/// init bytes differ from the preceding segment's, `segment.discontinuous` is
/// set to `true`. The first segment is never marked (no preceding context).
///
/// This is the building block for playlist assemblers that splice content from
/// multiple sources with different `EXT-X-MAP` init segments: detect changes
/// once, then pass the updated `MediaSegment` list to [`MediaPlaylist`].
///
/// # Example
/// ```
/// use transmux::hls::{mark_init_discontinuities, MediaSegment};
/// let init_a = b"moov_a" as &[u8];
/// let init_b = b"moov_b" as &[u8];
/// let mut seg0 = MediaSegment { uri: "s0.m4s".into(), duration: 5.0, discontinuous: false, parts: vec![] };
/// let mut seg1 = MediaSegment { uri: "s1.m4s".into(), duration: 5.0, discontinuous: false, parts: vec![] };
/// let mut seg2 = MediaSegment { uri: "s2.m4s".into(), duration: 5.0, discontinuous: false, parts: vec![] };
/// let mut entries: Vec<(&[u8], &mut MediaSegment)> = vec![
///     (init_a, &mut seg0),
///     (init_b, &mut seg1),
///     (init_b, &mut seg2),
/// ];
/// mark_init_discontinuities(&mut entries);
/// assert!(!entries[0].1.discontinuous);
/// assert!(entries[1].1.discontinuous);   // init changed: a → b
/// assert!(!entries[2].1.discontinuous);  // same init
/// ```
pub fn mark_init_discontinuities(entries: &mut [(&[u8], &mut MediaSegment)]) {
    if entries.len() < 2 {
        return;
    }
    // Walk the slice as a sliding window: [prev | cur..].
    // `split_at_mut` gives two non-overlapping sub-slices so we can hold an
    // immutable read of `prev.0` while mutating `cur.1.discontinuous`.
    for i in 1..entries.len() {
        let (head, tail) = entries.split_at_mut(i);
        let prev_init: &[u8] = head[i - 1].0;
        let cur = &mut tail[0];
        if cur.0 != prev_init {
            cur.1.discontinuous = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(uri: &str, duration: f64) -> MediaSegment {
        MediaSegment {
            uri: uri.into(),
            duration,
            discontinuous: false,
            parts: vec![],
        }
    }

    fn seg_disc(uri: &str, duration: f64) -> MediaSegment {
        MediaSegment {
            uri: uri.into(),
            duration,
            discontinuous: true,
            parts: vec![],
        }
    }

    fn playlist(segments: Vec<MediaSegment>) -> MediaPlaylist {
        MediaPlaylist {
            version: 3,
            target_duration: 10,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments,
            endlist: true,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            open_segment: None,
        }
    }

    #[test]
    fn media_playlist_basic() {
        let pl = MediaPlaylist {
            version: 3,
            target_duration: 10,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![
                seg("seg0.m4s", 9.009),
                seg("seg1.m4s", 9.009),
                seg("seg2.m4s", 3.003),
            ],
            endlist: true,
            extra_tags: vec![
                "#EXT-X-DATERANGE:ID=\"ad-1\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=15.0"
                    .into(),
            ],
            low_latency: None,
            iframes_only: false,
            open_segment: None,
        };
        let out = pl.to_m3u8();
        assert!(out.starts_with("#EXTM3U\n"));
        assert!(out.contains("#EXT-X-TARGETDURATION:10\n"));
        assert!(out.contains("#EXT-X-MEDIA-SEQUENCE:0\n"));
        assert_eq!(out.matches("#EXTINF:").count(), 3);
        assert!(out.ends_with("#EXT-X-ENDLIST\n"));
        // Check extra tag is present before segments.
        assert!(out.contains("#EXT-X-DATERANGE:ID=\"ad-1\""));
        // No discontinuity sequence when 0.
        assert!(!out.contains("#EXT-X-DISCONTINUITY-SEQUENCE"));
    }

    #[test]
    fn media_playlist_no_endlist() {
        let pl = MediaPlaylist {
            version: 7,
            target_duration: 6,
            media_sequence: 42,
            discontinuity_sequence: 0,
            segments: vec![seg("seg.m4s", 6.000)],
            endlist: false,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            open_segment: None,
        };
        let out = pl.to_m3u8();
        assert!(out.starts_with("#EXTM3U\n"));
        assert!(out.contains("#EXT-X-VERSION:7\n"));
        assert!(!out.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn master_playlist_basic() {
        let pl = MasterPlaylist {
            version: 6,
            variants: vec![
                Variant {
                    bandwidth: 300_000,
                    codecs: "avc1.64001e,mp4a.40.2".into(),
                    resolution: Some((640, 360)),
                    uri: "v300/index.m3u8".into(),
                },
                Variant {
                    bandwidth: 800_000,
                    codecs: "avc1.640028,mp4a.40.2".into(),
                    resolution: Some((1280, 720)),
                    uri: "v800/index.m3u8".into(),
                },
            ],
            iframe_variants: vec![],
        };
        let out = pl.to_m3u8();
        assert!(out.starts_with("#EXTM3U\n"));
        assert_eq!(out.matches("#EXT-X-STREAM-INF:").count(), 2);
        assert!(out.contains("v300/index.m3u8"));
        assert!(out.contains("v800/index.m3u8"));
        assert!(out.contains("RESOLUTION=640x360"));
        assert!(out.contains("RESOLUTION=1280x720"));
    }

    #[test]
    fn master_playlist_no_resolution() {
        let pl = MasterPlaylist {
            version: 6,
            variants: vec![Variant {
                bandwidth: 1_000_000,
                codecs: "avc1.640028".into(),
                resolution: None,
                uri: "v1k/index.m3u8".into(),
            }],
            iframe_variants: vec![],
        };
        let out = pl.to_m3u8();
        assert!(!out.contains("RESOLUTION"));
        assert!(out.contains("#EXT-X-STREAM-INF:BANDWIDTH=1000000,CODECS=\"avc1.640028\""));
    }

    #[test]
    fn extinf_three_decimals() {
        let pl = playlist(vec![seg("s.m4s", 9.0)]);
        let out = pl.to_m3u8();
        assert!(out.contains("#EXTINF:9.000,\n"));
    }

    // --- discontinuity tag tests ---

    #[test]
    fn discontinuity_tag_emitted_before_extinf() {
        // seg1 is discontinuous; the tag must appear before its #EXTINF.
        let pl = playlist(vec![
            seg("s0.m4s", 5.0),
            seg_disc("s1.m4s", 5.0),
            seg("s2.m4s", 5.0),
        ]);
        let out = pl.to_m3u8();
        assert_eq!(out.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
        // The tag must immediately precede the #EXTINF for s1.
        let disc_pos = out.find("#EXT-X-DISCONTINUITY\n").unwrap();
        let extinf_pos = out.find("#EXTINF:5.000,\n#s1.m4s\n").unwrap_or_else(|| {
            // Find the position of "s1.m4s" in the output and trace back to its #EXTINF.
            let s1_pos = out.find("s1.m4s\n").unwrap();
            // The #EXTINF line starts 11 chars before "5.000,\n" — find it preceding s1.
            out[..s1_pos].rfind("#EXTINF:").unwrap()
        });
        assert!(
            disc_pos < extinf_pos,
            "#EXT-X-DISCONTINUITY must appear before #EXTINF of s1"
        );
        // The discontinuity tag must be the line immediately before #EXTINF:
        let tag_end = disc_pos + "#EXT-X-DISCONTINUITY\n".len();
        assert!(
            out[tag_end..].starts_with("#EXTINF:"),
            "#EXT-X-DISCONTINUITY must be immediately before #EXTINF, got: {:?}",
            &out[tag_end..tag_end + 20]
        );
    }

    #[test]
    fn no_discontinuity_tag_when_all_continuous() {
        let pl = playlist(vec![
            seg("s0.m4s", 5.0),
            seg("s1.m4s", 5.0),
            seg("s2.m4s", 5.0),
        ]);
        let out = pl.to_m3u8();
        assert!(
            !out.contains("#EXT-X-DISCONTINUITY\n"),
            "no tag when all segments are continuous"
        );
    }

    #[test]
    fn discontinuity_sequence_emitted_when_nonzero() {
        let pl = MediaPlaylist {
            version: 3,
            target_duration: 6,
            media_sequence: 5,
            discontinuity_sequence: 2,
            segments: vec![seg("s5.m4s", 6.0)],
            endlist: false,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            open_segment: None,
        };
        let out = pl.to_m3u8();
        assert!(
            out.contains("#EXT-X-DISCONTINUITY-SEQUENCE:2\n"),
            "header must be present when n>0"
        );
    }

    #[test]
    fn discontinuity_sequence_absent_when_zero() {
        let pl = playlist(vec![seg("s0.m4s", 6.0)]);
        let out = pl.to_m3u8();
        assert!(
            !out.contains("#EXT-X-DISCONTINUITY-SEQUENCE"),
            "header must be absent when n==0"
        );
    }

    // --- LL-HLS render tests (issue #702: OpenSegment) ---

    fn ll_config() -> LowLatencyConfig {
        LowLatencyConfig {
            part_target: 0.5,
            part_hold_back: 1.5,
            preload_hint_part: None,
        }
    }

    #[test]
    fn ll_hls_renders_server_control_part_inf_and_parts() {
        let pl = MediaPlaylist {
            version: 9,
            target_duration: 4,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![MediaSegment {
                uri: "seg-1-4.m4s".into(),
                duration: 4.0,
                discontinuous: false,
                parts: vec![PartSpec {
                    uri: "part-1-1.m4s".into(),
                    duration: 0.5,
                    independent: true,
                }],
            }],
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll_config()),
            iframes_only: false,
            open_segment: None,
        };
        let out = pl.to_m3u8();
        assert!(out.contains("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES"));
        assert!(out.contains("#EXT-X-PART-INF:PART-TARGET="));
        assert!(out.contains("#EXT-X-PART:DURATION=0.5,URI=\"part-1-1.m4s\""));
    }

    #[test]
    fn open_segment_renders_parts_without_extinf() {
        let pl = MediaPlaylist {
            version: 9,
            target_duration: 4,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![MediaSegment {
                uri: "seg-1-4.m4s".into(),
                duration: 4.0,
                discontinuous: false,
                parts: vec![],
            }],
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll_config()),
            iframes_only: false,
            open_segment: Some(OpenSegment {
                parts: vec![PartSpec {
                    uri: "part-1-5.0.m4s".into(),
                    duration: 0.5,
                    independent: true,
                }],
            }),
        };
        let out = pl.to_m3u8();
        // The open part is rendered as an #EXT-X-PART line.
        assert!(
            out.contains("#EXT-X-PART:DURATION=0.5,URI=\"part-1-5.0.m4s\",INDEPENDENT=YES"),
            "open segment's part must render:\n{out}"
        );
        // The closed segment is still rendered with its #EXTINF.
        assert!(out.contains("#EXTINF:4.000,\n"));
        assert!(out.contains("seg-1-4.m4s"));
        // The open part's URI never appears on an #EXTINF/plain-URI line — only
        // inside its #EXT-X-PART line (there is no #EXTINF for an open segment).
        assert!(
            !out.contains("#EXTINF:0.500,\npart-1-5.0.m4s"),
            "open segment must not be rendered as a closed #EXTINF segment:\n{out}"
        );
        let lines: Vec<&str> = out.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if *line == "part-1-5.0.m4s" {
                panic!("open part URI must not appear on its own URI line: {out}");
            }
            if line.starts_with("#EXTINF") && i + 1 < lines.len() {
                assert_ne!(
                    lines[i + 1],
                    "part-1-5.0.m4s",
                    "open part URI must not follow an #EXTINF line:\n{out}"
                );
            }
        }
    }

    #[test]
    fn open_segment_not_rendered_without_low_latency() {
        let pl = MediaPlaylist {
            version: 9,
            target_duration: 4,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![seg("seg-1-4.m4s", 4.0)],
            endlist: false,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            open_segment: Some(OpenSegment {
                parts: vec![PartSpec {
                    uri: "part-1-5.0.m4s".into(),
                    duration: 0.5,
                    independent: true,
                }],
            }),
        };
        let out = pl.to_m3u8();
        assert!(
            !out.contains("part-1-5.0.m4s"),
            "open segment parts must not render without low_latency:\n{out}"
        );
        assert!(!out.contains("#EXT-X-PART:"));
    }

    #[test]
    fn preload_hint_rendered_from_low_latency() {
        let mut ll = ll_config();
        ll.preload_hint_part = Some("part-1-5.1.m4s".into());
        let pl = MediaPlaylist {
            version: 9,
            target_duration: 4,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![seg("seg-1-4.m4s", 4.0)],
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll),
            iframes_only: false,
            open_segment: None,
        };
        let out = pl.to_m3u8();
        assert!(out.contains("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-5.1.m4s\""));
    }
}
