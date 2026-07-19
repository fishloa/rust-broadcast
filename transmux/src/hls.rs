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
//!
//! # Parsing (RFC 8216bis, issue #717 slice 1)
//!
//! [`MediaPlaylist::parse`] and [`MasterPlaylist::parse`] are the symmetric
//! *inverse* of `to_m3u8()`: they parse an m3u8 string back into the same
//! structs the renderer consumes, so an LL-HLS **client** (issue #717) can
//! reuse the origin's wire model rather than growing a second one. Recognized
//! tags are the ones listed above plus the client-relevant LL-HLS tags —
//! `#EXT-X-BYTERANGE`, `#EXT-X-MAP`, `#EXT-X-SKIP`, `#EXT-X-RENDITION-REPORT`
//! and the `BYTERANGE`/`GAP`/`CAN-SKIP-UNTIL`/preload-hint-byte-range
//! attributes. Unrecognized tags are preserved verbatim into
//! [`MediaPlaylist::extra_tags`] (never an error — forward-compat); a
//! malformed *known* tag (missing required attribute, unparsable value)
//! returns [`crate::Error::HlsParse`].
//!
//! Known, documented gaps (data the current struct shape cannot yet carry,
//! called out per the project's round-trip-fidelity discipline rather than
//! silently dropped):
//! - `#EXT-X-MEDIA` (Multivariant Playlist alternate audio/subtitle
//!   renditions) is not modeled — `to_m3u8()` doesn't render it either, so
//!   there is nothing to round-trip yet; `MasterPlaylist::parse` skips it as
//!   an unrecognized tag (it has no per-file `extra_tags` field to preserve
//!   it into).
//! - `#EXT-X-MAP` is carried on [`MediaSegment::map`] with carry-forward
//!   parse semantics (a map applies to every following segment until the
//!   next `EXT-X-MAP`, per spec) and dedup-render semantics (re-emitted only
//!   when it changes from the previous segment). A hand-built
//!   [`MediaPlaylist`] whose segments' `map` fields are *not* a valid
//!   carry-forward sequence (e.g. reverting to `None` after a `Some`) cannot
//!   round-trip, since the wire format has no way to say "stop applying the
//!   map" short of `#EXT-X-DISCONTINUITY` + a new `#EXT-X-MAP`.
//! - A per-segment tag outside the recognized set above (e.g.
//!   `#EXT-X-PROGRAM-DATE-TIME`, a segment-scoped `#EXT-X-KEY`) is captured
//!   into the flat, playlist-level [`MediaPlaylist::extra_tags`] — the data
//!   is preserved, not dropped, but re-rendering loses its original
//!   interleaved position (extra tags always render as one block before all
//!   segments, matching `to_m3u8()`'s existing placement).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::cenc::CencScheme;
use crate::error::{Error, Result};

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

/// A byte sub-range into a resource.
///
/// Shared by three tags that all use the same underlying notation:
/// - `#EXT-X-BYTERANGE:<n>[@<o>]` (RFC 8216bis §4.4.4.2) — [`MediaSegment::byte_range`].
/// - `#EXT-X-PART`'s `BYTERANGE="<n>[@<o>]"` attribute (RFC 8216bis
///   §4.4.4.9, "same format as the EXT-X-BYTERANGE tag") — [`PartSpec::byte_range`].
/// - `#EXT-X-MAP`'s `BYTERANGE="<n>@<o>"` attribute (RFC 8216bis §4.4.4.5) —
///   [`MapTag::byte_range`]. Unlike the other two, the spec says the offset
///   `o` is **REQUIRED** here (there is no "previous sub-range" to continue
///   from for an Initialization Section).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// `n` — length of the sub-range in bytes.
    pub length: u64,
    /// `o` — byte offset of the sub-range from the start of the resource.
    /// `None` means "immediately following the previous Media/Partial
    /// Segment's sub-range of the same resource" (only meaningful for
    /// `EXT-X-BYTERANGE`/`EXT-X-PART`'s `BYTERANGE`; `EXT-X-MAP`'s
    /// `BYTERANGE` always carries `Some`).
    pub offset: Option<u64>,
}

impl ByteRange {
    /// Render the `<n>[@<o>]` wire notation (used inside a quoted attribute
    /// value for `PART`/`MAP`, or as the whole `#EXT-X-BYTERANGE` tag value).
    fn render(&self) -> String {
        match self.offset {
            Some(o) => format!("{}@{o}", self.length),
            None => format!("{}", self.length),
        }
    }

    /// Parse the `<n>[@<o>]` wire notation.
    fn parse(s: &str, line_no: usize, line: &str) -> Result<Self> {
        let mut split = s.splitn(2, '@');
        let n = split.next().unwrap_or("");
        let length = parse_decimal::<u64>(n, line_no, line, "BYTERANGE length")?;
        let offset = match split.next() {
            Some(o) => Some(parse_decimal::<u64>(o, line_no, line, "BYTERANGE offset")?),
            None => None,
        };
        Ok(ByteRange { length, offset })
    }
}

/// The Media Initialization Section reference of `#EXT-X-MAP` (RFC 8216bis
/// §4.4.4.5) — see [`MediaSegment::map`] for carry-forward/dedup semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapTag {
    /// `URI` — the resource containing the Media Initialization Section
    /// (REQUIRED).
    pub uri: String,
    /// `BYTERANGE` — a sub-range of `uri` containing just the
    /// Initialization Section. `None` means the entire resource. The
    /// offset is always present when this is `Some` (spec requires it here,
    /// unlike [`MediaSegment::byte_range`]/[`PartSpec::byte_range`]).
    pub byte_range: Option<ByteRange>,
}

/// `TYPE` attribute of `#EXT-X-PRELOAD-HINT` (RFC 8216bis §4.4.5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreloadHintType {
    /// `PART` — the hinted resource is a Partial Segment.
    #[default]
    Part,
    /// `MAP` — the hinted resource is a Media Initialization Section.
    Map,
}

impl PreloadHintType {
    /// The spec token (`"PART"` / `"MAP"`).
    pub fn name(&self) -> &'static str {
        match self {
            PreloadHintType::Part => "PART",
            PreloadHintType::Map => "MAP",
        }
    }
}

broadcast_common::impl_spec_display!(PreloadHintType);

/// `#EXT-X-RENDITION-REPORT` (RFC 8216bis §4.4.5.4) — a pointer to the
/// current state of an associated Rendition's own Media Playlist, so an
/// LL-HLS client following one Rendition can discover how far another has
/// progressed without polling it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenditionReport {
    /// `URI` of the Rendition's Media Playlist, relative to the playlist
    /// carrying this tag (REQUIRED).
    pub uri: String,
    /// `LAST-MSN` — Media Sequence Number of the last segment (or partial
    /// segment, if any) currently in that Rendition (REQUIRED).
    pub last_msn: u64,
    /// `LAST-PART` — Part Index of the last partial segment at `last_msn`,
    /// if that Rendition has partial segments.
    pub last_part: Option<u64>,
}

/// `#EXT-X-SKIP` (RFC 8216bis §4.4.5.2) — present on a Playlist Delta Update
/// response in place of the segments/tags before the Skip Boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkipInfo {
    /// `SKIPPED-SEGMENTS` — count of Media Segments elided (REQUIRED).
    pub skipped_segments: u64,
    /// `RECENTLY-REMOVED-DATERANGES` — `EXT-X-DATERANGE` `ID`s removed from
    /// the playlist recently (tab-delimited on the wire). Empty when the
    /// attribute is absent.
    pub recently_removed_daterange_ids: Vec<String>,
}

/// A single partial segment ("part") of a [`MediaSegment`] — RFC 8216bis
/// §4.4.4.9 (`#EXT-X-PART`).
///
/// A part is an independently addressable CMAF chunk (a `moof`+`mdat` fragment)
/// covering a sub-duration of its parent segment; a client can fetch and play it
/// before the parent segment is complete. Parts are emitted as `#EXT-X-PART`
/// lines immediately before the parent segment's `#EXTINF`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PartSpec {
    /// The part URI (e.g. `"seg0.1.m4s"`).
    pub uri: String,
    /// The part duration in seconds (e.g. `0.334`).
    pub duration: f64,
    /// If `true`, render `,INDEPENDENT=YES` — the part begins with an
    /// independently decodable frame (a sync sample). RFC 8216bis §4.4.4.9.
    pub independent: bool,
    /// `BYTERANGE` attribute (RFC 8216bis §4.4.4.9) — the part is a
    /// sub-range of the resource named by [`Self::uri`], same `<n>[@<o>]`
    /// format as [`MediaSegment::byte_range`]. `None` when the part is the
    /// entire resource.
    pub byte_range: Option<ByteRange>,
    /// `GAP` attribute (RFC 8216bis §4.4.4.9) — `true` if this partial
    /// segment is not actually available (a hole in the part list).
    pub gap: bool,
}

/// An in-progress (open) LL-HLS segment: its parts are known and being served,
/// but the segment is not yet complete, so it carries no `#EXTINF`/URI
/// (RFC 8216bis §4.4.4.9 — an open segment is represented by its trailing
/// `#EXT-X-PART` lines only, until it closes).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OpenSegment {
    /// The parts of the in-progress segment, in order.
    pub parts: Vec<PartSpec>,
}

impl OpenSegment {
    /// Build an open segment from its in-progress parts.
    pub fn new(parts: Vec<PartSpec>) -> Self {
        Self { parts }
    }
}

/// A single media segment in a media playlist.
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// `#EXT-X-BYTERANGE` (RFC 8216bis §4.4.4.2) — this segment is a
    /// sub-range of the resource named by [`Self::uri`]. Rendered
    /// immediately after this segment's `#EXTINF` line, before the URI.
    /// `None` (the default) means the segment is the entire resource.
    pub byte_range: Option<ByteRange>,
    /// `#EXT-X-MAP` (RFC 8216bis §4.4.4.5) applying to this segment. Per
    /// spec a map applies to every segment following it until the next
    /// `#EXT-X-MAP`; `to_m3u8` renders the tag only when it differs from the
    /// previous segment's map (dedup), and [`MediaPlaylist::parse`] carries
    /// the value forward onto every segment it applies to — so this field
    /// is `Some` on every segment covered by a given `#EXT-X-MAP`, not just
    /// the one it was written before.
    pub map: Option<MapTag>,
}

/// A media playlist (`#EXTM3U` / `#EXTINF` / ...).
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// `#EXT-X-RENDITION-REPORT` entries (RFC 8216bis §4.4.5.4) — one per
    /// associated Rendition, pointing an LL-HLS client at that Rendition's
    /// current playlist state. Rendered after the segment list (and any
    /// preload hint), in order.
    pub rendition_reports: Vec<RenditionReport>,
    /// `#EXT-X-SKIP` (RFC 8216bis §4.4.5.2) — present on a Playlist Delta
    /// Update response, replacing the segments/tags before the Skip
    /// Boundary. `None` (the default) means this is a full playlist, not a
    /// delta update.
    pub skip: Option<SkipInfo>,
}

/// Low-Latency HLS playlist configuration — RFC 8216bis.
///
/// Presence of this config on a [`MediaPlaylist`] switches on the LL-HLS
/// directives (`#EXT-X-SERVER-CONTROL`, `#EXT-X-PART-INF`, `#EXT-X-PART`,
/// `#EXT-X-PRELOAD-HINT`); see the module docs for each tag's spec section.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LowLatencyConfig {
    /// Part-target duration in seconds — the `PART-TARGET` of `#EXT-X-PART-INF`
    /// (RFC 8216bis §4.4.3.7). Typically 0.2–0.5 s.
    pub part_target: f64,
    /// `PART-HOLD-BACK` in seconds — the `#EXT-X-SERVER-CONTROL` attribute
    /// (RFC 8216bis §4.4.3.8). MUST be at least `3 × part_target`; the renderer
    /// raises it to that floor if a smaller value is supplied.
    pub part_hold_back: f64,
    /// URI of the next, not-yet-available part or map — rendered as
    /// `#EXT-X-PRELOAD-HINT:TYPE=<...>,URI="<uri>"` (RFC 8216bis §4.4.5.3). When
    /// `None`, no preload hint is emitted (e.g. an ended playlist).
    pub preload_hint_part: Option<String>,
    /// `TYPE` of [`Self::preload_hint_part`]'s hinted resource (RFC 8216bis
    /// §4.4.5.3): `PART` (a Partial Segment) or `MAP` (a Media
    /// Initialization Section). Only meaningful when `preload_hint_part` is
    /// `Some`; defaults to [`PreloadHintType::Part`] (the overwhelmingly
    /// common case).
    pub preload_hint_type: PreloadHintType,
    /// `BYTERANGE-START` of the `#EXT-X-PRELOAD-HINT` tag (RFC 8216bis
    /// §4.4.5.3) — byte offset of the hinted resource. `None` implies 0.
    pub preload_hint_byte_range_start: Option<u64>,
    /// `BYTERANGE-LENGTH` of the `#EXT-X-PRELOAD-HINT` tag (RFC 8216bis
    /// §4.4.5.3) — length in bytes. `None` means "to the end of the
    /// resource".
    pub preload_hint_byte_range_length: Option<u64>,
    /// `CAN-SKIP-UNTIL` attribute of `#EXT-X-SERVER-CONTROL` (RFC 8216bis
    /// §4.4.3.8) — the Skip Boundary in seconds, advertising support for
    /// Playlist Delta Updates (`#EXT-X-SKIP`). `None` omits the attribute.
    pub can_skip_until: Option<f64>,
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
            // part-target, enforced by effective_part_hold_back) + optional
            // CAN-SKIP-UNTIL (RFC 8216bis §4.4.3.8).
            s.push_str(&format!(
                "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK={}",
                format_secs(ll.effective_part_hold_back()),
            ));
            if let Some(csu) = ll.can_skip_until {
                s.push_str(&format!(",CAN-SKIP-UNTIL={}", format_secs(csu)));
            }
            s.push('\n');
            // #EXT-X-PART-INF — the part-target duration.
            s.push_str(&format!(
                "#EXT-X-PART-INF:PART-TARGET={}\n",
                format_secs(ll.part_target),
            ));
        }

        // #EXT-X-SKIP (RFC 8216bis §4.4.5.2) — a Playlist Delta Update marker
        // standing in for the segments/tags before the Skip Boundary.
        if let Some(skip) = &self.skip {
            s.push_str(&format!(
                "#EXT-X-SKIP:SKIPPED-SEGMENTS={}",
                skip.skipped_segments
            ));
            if !skip.recently_removed_daterange_ids.is_empty() {
                s.push_str(&format!(
                    ",RECENTLY-REMOVED-DATERANGES=\"{}\"",
                    skip.recently_removed_daterange_ids.join("\t")
                ));
            }
            s.push('\n');
        }

        for tag in &self.extra_tags {
            s.push_str(tag);
            s.push('\n');
        }

        for (i, seg) in self.segments.iter().enumerate() {
            if seg.discontinuous {
                s.push_str("#EXT-X-DISCONTINUITY\n");
            }
            // #EXT-X-MAP (RFC 8216bis §4.4.4.5) — emitted only when it
            // changes from the previous segment's map, since the tag
            // applies "until the next EXT-X-MAP or the end of the Playlist".
            let prev_map = if i == 0 {
                None
            } else {
                self.segments[i - 1].map.as_ref()
            };
            if seg.map.as_ref() != prev_map {
                if let Some(map) = &seg.map {
                    push_map_line(&mut s, map);
                }
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
            // #EXT-X-BYTERANGE (RFC 8216bis §4.4.4.2) — after EXTINF, before
            // the URI it applies to.
            if let Some(br) = &seg.byte_range {
                s.push_str(&format!("#EXT-X-BYTERANGE:{}\n", br.render()));
            }
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

        // LL-HLS preload hint for the next not-yet-available part or map
        // (RFC 8216bis §4.4.5.3) — after the segment list, before ENDLIST.
        if let Some(ll) = &self.low_latency {
            if let Some(uri) = &ll.preload_hint_part {
                s.push_str(&format!(
                    "#EXT-X-PRELOAD-HINT:TYPE={},URI=\"{uri}\"",
                    ll.preload_hint_type.name(),
                ));
                if let Some(start) = ll.preload_hint_byte_range_start {
                    s.push_str(&format!(",BYTERANGE-START={start}"));
                }
                if let Some(len) = ll.preload_hint_byte_range_length {
                    s.push_str(&format!(",BYTERANGE-LENGTH={len}"));
                }
                s.push('\n');
            }
        }

        // #EXT-X-RENDITION-REPORT entries (RFC 8216bis §4.4.5.4).
        for rr in &self.rendition_reports {
            s.push_str(&format!(
                "#EXT-X-RENDITION-REPORT:URI=\"{}\",LAST-MSN={}",
                rr.uri, rr.last_msn
            ));
            if let Some(lp) = rr.last_part {
                s.push_str(&format!(",LAST-PART={lp}"));
            }
            s.push('\n');
        }

        if self.endlist {
            s.push_str("#EXT-X-ENDLIST\n");
        }

        s
    }

    /// Parse an RFC 8216bis `#EXTM3U` Media Playlist — the symmetric inverse
    /// of [`Self::to_m3u8`]. See the module docs for the recognized-tag list
    /// and the documented modeling gaps.
    ///
    /// Unrecognized `#EXT-...` tags are preserved verbatim into
    /// [`Self::extra_tags`] rather than erroring (forward-compat); a
    /// non-`#EXT` comment line (RFC 8216 §4.1) is silently ignored. A known
    /// tag with a missing required attribute or an unparsable value returns
    /// [`crate::Error::HlsParse`].
    pub fn parse(input: &str) -> Result<Self> {
        let mut version: u8 = 1;
        let mut target_duration: Option<u32> = None;
        let mut media_sequence: u64 = 0;
        let mut discontinuity_sequence: u64 = 0;
        let mut iframes_only = false;
        let mut endlist = false;
        let mut extra_tags: Vec<String> = Vec::new();
        let mut segments: Vec<MediaSegment> = Vec::new();
        let mut rendition_reports: Vec<RenditionReport> = Vec::new();
        let mut skip: Option<SkipInfo> = None;
        let mut saw_extm3u = false;

        // Low-Latency HLS accumulators.
        let mut part_target: Option<f64> = None;
        let mut part_hold_back: Option<f64> = None;
        let mut can_skip_until: Option<f64> = None;
        let mut preload_hint_part: Option<String> = None;
        let mut preload_hint_type = PreloadHintType::Part;
        let mut preload_hint_byte_range_start: Option<u64> = None;
        let mut preload_hint_byte_range_length: Option<u64> = None;
        let mut saw_ll_tag = false;

        // Per-segment pending state, reset each time a bare URI line closes
        // a segment.
        let mut current_map: Option<MapTag> = None;
        let mut pending_discontinuous = false;
        let mut pending_byte_range: Option<ByteRange> = None;
        let mut pending_parts: Vec<PartSpec> = Vec::new();
        let mut pending_duration: Option<f64> = None;

        for (idx, raw_line) in input.lines().enumerate() {
            let line_no = idx + 1;
            let mut line = raw_line.trim_end_matches('\r');
            if line_no == 1 {
                line = line.strip_prefix('\u{feff}').unwrap_or(line);
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if line == "#EXTM3U" {
                saw_extm3u = true;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-VERSION:") {
                version = parse_decimal(rest, line_no, line, "EXT-X-VERSION")?;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
                target_duration = Some(parse_decimal(rest, line_no, line, "EXT-X-TARGETDURATION")?);
            } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                media_sequence = parse_decimal(rest, line_no, line, "EXT-X-MEDIA-SEQUENCE")?;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
                discontinuity_sequence =
                    parse_decimal(rest, line_no, line, "EXT-X-DISCONTINUITY-SEQUENCE")?;
            } else if line == "#EXT-X-I-FRAMES-ONLY" {
                iframes_only = true;
            } else if line == "#EXT-X-ENDLIST" {
                endlist = true;
            } else if line == "#EXT-X-DISCONTINUITY" {
                pending_discontinuous = true;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
                pending_byte_range = Some(ByteRange::parse(rest, line_no, line)?);
            } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
                let attrs = parse_attr_list(rest);
                let uri = require_attr(&attrs, "URI", line_no, line, "EXT-X-MAP")?;
                let byte_range = match attrs.get("BYTERANGE") {
                    Some(v) => Some(ByteRange::parse(v, line_no, line)?),
                    None => None,
                };
                current_map = Some(MapTag { uri, byte_range });
            } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
                let dur_str = rest.split(',').next().unwrap_or(rest);
                pending_duration = Some(parse_decimal(dur_str, line_no, line, "EXTINF duration")?);
            } else if let Some(rest) = line.strip_prefix("#EXT-X-PART-INF:") {
                let attrs = parse_attr_list(rest);
                if let Some(v) = attrs.get("PART-TARGET") {
                    part_target = Some(parse_decimal(v, line_no, line, "PART-TARGET")?);
                }
                saw_ll_tag = true;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-SERVER-CONTROL:") {
                let attrs = parse_attr_list(rest);
                if let Some(v) = attrs.get("PART-HOLD-BACK") {
                    part_hold_back = Some(parse_decimal(v, line_no, line, "PART-HOLD-BACK")?);
                }
                if let Some(v) = attrs.get("CAN-SKIP-UNTIL") {
                    can_skip_until = Some(parse_decimal(v, line_no, line, "CAN-SKIP-UNTIL")?);
                }
                saw_ll_tag = true;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-PART:") {
                let attrs = parse_attr_list(rest);
                let uri = require_attr(&attrs, "URI", line_no, line, "EXT-X-PART")?;
                let duration_str = attrs.get("DURATION").ok_or_else(|| Error::HlsParse {
                    line_no,
                    line: line.to_string(),
                    reason: "EXT-X-PART missing required DURATION attribute".to_string(),
                })?;
                let duration = parse_decimal(duration_str, line_no, line, "EXT-X-PART DURATION")?;
                let independent = attrs.get("INDEPENDENT").map(String::as_str) == Some("YES");
                let gap = attrs.get("GAP").map(String::as_str) == Some("YES");
                let byte_range = match attrs.get("BYTERANGE") {
                    Some(v) => Some(ByteRange::parse(v, line_no, line)?),
                    None => None,
                };
                pending_parts.push(PartSpec {
                    uri,
                    duration,
                    independent,
                    byte_range,
                    gap,
                });
                saw_ll_tag = true;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-PRELOAD-HINT:") {
                let attrs = parse_attr_list(rest);
                preload_hint_type = match attrs.get("TYPE").map(String::as_str) {
                    Some("MAP") => PreloadHintType::Map,
                    _ => PreloadHintType::Part,
                };
                preload_hint_part = Some(require_attr(
                    &attrs,
                    "URI",
                    line_no,
                    line,
                    "EXT-X-PRELOAD-HINT",
                )?);
                if let Some(v) = attrs.get("BYTERANGE-START") {
                    preload_hint_byte_range_start =
                        Some(parse_decimal(v, line_no, line, "BYTERANGE-START")?);
                }
                if let Some(v) = attrs.get("BYTERANGE-LENGTH") {
                    preload_hint_byte_range_length =
                        Some(parse_decimal(v, line_no, line, "BYTERANGE-LENGTH")?);
                }
                saw_ll_tag = true;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-RENDITION-REPORT:") {
                let attrs = parse_attr_list(rest);
                let uri = require_attr(&attrs, "URI", line_no, line, "EXT-X-RENDITION-REPORT")?;
                let last_msn = match attrs.get("LAST-MSN") {
                    Some(v) => parse_decimal(v, line_no, line, "LAST-MSN")?,
                    None => 0,
                };
                let last_part = match attrs.get("LAST-PART") {
                    Some(v) => Some(parse_decimal(v, line_no, line, "LAST-PART")?),
                    None => None,
                };
                rendition_reports.push(RenditionReport {
                    uri,
                    last_msn,
                    last_part,
                });
            } else if let Some(rest) = line.strip_prefix("#EXT-X-SKIP:") {
                let attrs = parse_attr_list(rest);
                let skipped_segments_str =
                    require_attr(&attrs, "SKIPPED-SEGMENTS", line_no, line, "EXT-X-SKIP")?;
                let skipped_segments =
                    parse_decimal(&skipped_segments_str, line_no, line, "SKIPPED-SEGMENTS")?;
                let recently_removed_daterange_ids = attrs
                    .get("RECENTLY-REMOVED-DATERANGES")
                    .map(|v| {
                        v.split('\t')
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                skip = Some(SkipInfo {
                    skipped_segments,
                    recently_removed_daterange_ids,
                });
            } else if let Some(rest) = line.strip_prefix("#EXT") {
                let _ = rest;
                // A well-formed but unrecognized tag: preserve verbatim
                // (forward-compat) rather than error or drop.
                extra_tags.push(line.to_string());
            } else if line.starts_with('#') {
                // RFC 8216 §4.1: a non-"#EXT" '#' line is a comment — ignore.
            } else {
                // A bare (non-'#') line is always a Media Segment URI; parts
                // have no URI line of their own (their URI is an attribute).
                let duration = pending_duration.take().ok_or_else(|| Error::HlsParse {
                    line_no,
                    line: line.to_string(),
                    reason: "media segment URI with no preceding #EXTINF".to_string(),
                })?;
                segments.push(MediaSegment {
                    uri: line.to_string(),
                    duration,
                    discontinuous: core::mem::take(&mut pending_discontinuous),
                    parts: core::mem::take(&mut pending_parts),
                    byte_range: pending_byte_range.take(),
                    map: current_map.clone(),
                });
            }
        }

        if !saw_extm3u {
            return Err(Error::HlsParse {
                line_no: 1,
                line: String::new(),
                reason: "missing #EXTM3U header".to_string(),
            });
        }
        let target_duration = target_duration.ok_or_else(|| Error::HlsParse {
            line_no: 0,
            line: String::new(),
            reason: "missing required #EXT-X-TARGETDURATION".to_string(),
        })?;

        // Any parts accumulated but never closed by a following #EXTINF/URI
        // are the in-progress (open) segment at the live edge
        // (RFC 8216bis §4.4.4.9).
        let open_segment = if pending_parts.is_empty() {
            None
        } else {
            Some(OpenSegment::new(pending_parts))
        };

        let low_latency = if saw_ll_tag {
            Some(LowLatencyConfig {
                part_target: part_target.unwrap_or(0.0),
                part_hold_back: part_hold_back.unwrap_or(0.0),
                preload_hint_part,
                preload_hint_type,
                preload_hint_byte_range_start,
                preload_hint_byte_range_length,
                can_skip_until,
            })
        } else {
            None
        };

        Ok(MediaPlaylist {
            version,
            target_duration,
            media_sequence,
            discontinuity_sequence,
            segments,
            open_segment,
            endlist,
            extra_tags,
            low_latency,
            iframes_only,
            rendition_reports,
            skip,
        })
    }
}

/// Render one `#EXT-X-PART:DURATION=<sec>,URI="<uri>"[,BYTERANGE="<n>[@<o>]"]
/// [,INDEPENDENT=YES][,GAP=YES]` line (RFC 8216bis §4.4.4.9) into `s`, shared
/// by both a closed segment's parts and an open (in-progress) segment's parts
/// so the two can never drift in format.
fn push_part_line(s: &mut String, part: &PartSpec) {
    s.push_str(&format!(
        "#EXT-X-PART:DURATION={},URI=\"{}\"",
        format_secs(part.duration),
        part.uri,
    ));
    if let Some(br) = &part.byte_range {
        s.push_str(&format!(",BYTERANGE=\"{}\"", br.render()));
    }
    if part.independent {
        s.push_str(",INDEPENDENT=YES");
    }
    if part.gap {
        s.push_str(",GAP=YES");
    }
    s.push('\n');
}

/// Render one `#EXT-X-MAP:URI="<uri>"[,BYTERANGE="<n>@<o>"]` line
/// (RFC 8216bis §4.4.4.5).
fn push_map_line(s: &mut String, map: &MapTag) {
    s.push_str(&format!("#EXT-X-MAP:URI=\"{}\"", map.uri));
    if let Some(br) = &map.byte_range {
        s.push_str(&format!(",BYTERANGE=\"{}\"", br.render()));
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

/// Parse a decimal-integer or decimal-floating-point attribute/tag value
/// (RFC 8216bis §4.2), returning a structured, contextual
/// [`crate::Error::HlsParse`] on failure rather than panicking.
fn parse_decimal<T: core::str::FromStr>(
    s: &str,
    line_no: usize,
    line: &str,
    what: &str,
) -> Result<T> {
    s.trim().parse::<T>().map_err(|_| Error::HlsParse {
        line_no,
        line: line.to_string(),
        reason: format!("{what} value {s:?} is not a valid number"),
    })
}

/// Split an HLS `<attribute-list>` (RFC 8216 §4.2: comma-separated
/// `AttributeName=AttributeValue` pairs, where a quoted-string value may
/// itself contain commas) into a name → value map. Quoted values are
/// returned with their surrounding `"` stripped; unquoted (enumerated-string
/// / decimal) values are returned as-is.
fn parse_attr_list(s: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        while i < len && (bytes[i] == b',' || bytes[i].is_ascii_whitespace()) {
            i += 1;
        }
        if i >= len {
            break;
        }
        let key_start = i;
        while i < len && bytes[i] != b'=' {
            i += 1;
        }
        if i >= len {
            // Trailing key with no '=': nothing sane to record, stop.
            break;
        }
        let key = &s[key_start..i];
        i += 1; // skip '='
        if i < len && bytes[i] == b'"' {
            i += 1;
            let value_start = i;
            while i < len && bytes[i] != b'"' {
                i += 1;
            }
            let value = &s[value_start..i];
            if i < len {
                i += 1; // skip closing '"'
            }
            map.insert(key.to_string(), value.to_string());
        } else {
            let value_start = i;
            while i < len && bytes[i] != b',' {
                i += 1;
            }
            map.insert(key.to_string(), s[value_start..i].to_string());
        }
    }
    map
}

/// Fetch a required attribute from an already-parsed attribute map, or
/// return a contextual [`crate::Error::HlsParse`] naming the missing
/// attribute and the owning tag.
fn require_attr(
    attrs: &BTreeMap<String, String>,
    key: &str,
    line_no: usize,
    line: &str,
    tag: &str,
) -> Result<String> {
    attrs.get(key).cloned().ok_or_else(|| Error::HlsParse {
        line_no,
        line: line.to_string(),
        reason: format!("{tag} missing required {key} attribute"),
    })
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

/// A parsed but not-yet-closed `#EXT-X-STREAM-INF` — `(bandwidth, codecs,
/// resolution)` — awaiting the URI line that turns it into a [`Variant`].
type PendingStreamInf = (u32, String, Option<(u32, u32)>);

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

    /// Parse an RFC 8216 `#EXTM3U` Multivariant (Master) Playlist — the
    /// symmetric inverse of [`Self::to_m3u8`].
    ///
    /// Recognizes `#EXT-X-VERSION`, `#EXT-X-STREAM-INF` + its following URI
    /// line, and `#EXT-X-I-FRAME-STREAM-INF`. `#EXT-X-MEDIA` (alternate
    /// audio/subtitle renditions) and any other tag are not modeled by
    /// [`MasterPlaylist`] (`to_m3u8` doesn't render them either) and are
    /// silently skipped — there is no per-playlist `extra_tags` field here to
    /// preserve them into (unlike [`MediaPlaylist`]). A malformed
    /// `#EXT-X-STREAM-INF`/`#EXT-X-I-FRAME-STREAM-INF` (missing required
    /// attribute, unparsable value) or a variant URI with no preceding
    /// `#EXT-X-STREAM-INF` returns [`crate::Error::HlsParse`].
    pub fn parse(input: &str) -> Result<Self> {
        let mut version: u8 = 1;
        let mut variants: Vec<Variant> = Vec::new();
        let mut iframe_variants: Vec<IFrameVariant> = Vec::new();
        let mut saw_extm3u = false;
        let mut pending_stream_inf: Option<PendingStreamInf> = None;

        for (idx, raw_line) in input.lines().enumerate() {
            let line_no = idx + 1;
            let mut line = raw_line.trim_end_matches('\r');
            if line_no == 1 {
                line = line.strip_prefix('\u{feff}').unwrap_or(line);
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if line == "#EXTM3U" {
                saw_extm3u = true;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-VERSION:") {
                version = parse_decimal(rest, line_no, line, "EXT-X-VERSION")?;
            } else if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
                let attrs = parse_attr_list(rest);
                let bandwidth_str =
                    require_attr(&attrs, "BANDWIDTH", line_no, line, "EXT-X-STREAM-INF")?;
                let bandwidth = parse_decimal(&bandwidth_str, line_no, line, "BANDWIDTH")?;
                let codecs = attrs.get("CODECS").cloned().unwrap_or_default();
                let resolution = match attrs.get("RESOLUTION") {
                    Some(v) => Some(parse_resolution(v, line_no, line)?),
                    None => None,
                };
                pending_stream_inf = Some((bandwidth, codecs, resolution));
            } else if let Some(rest) = line.strip_prefix("#EXT-X-I-FRAME-STREAM-INF:") {
                let attrs = parse_attr_list(rest);
                let bandwidth_str = require_attr(
                    &attrs,
                    "BANDWIDTH",
                    line_no,
                    line,
                    "EXT-X-I-FRAME-STREAM-INF",
                )?;
                let bandwidth = parse_decimal(&bandwidth_str, line_no, line, "BANDWIDTH")?;
                let codecs = attrs.get("CODECS").cloned();
                let resolution = match attrs.get("RESOLUTION") {
                    Some(v) => Some(parse_resolution(v, line_no, line)?),
                    None => None,
                };
                let uri = require_attr(&attrs, "URI", line_no, line, "EXT-X-I-FRAME-STREAM-INF")?;
                iframe_variants.push(IFrameVariant {
                    bandwidth,
                    codecs,
                    resolution,
                    uri,
                });
            } else if line.starts_with('#') {
                // Unrecognized tag (e.g. #EXT-X-MEDIA) or a comment: this
                // struct has no escape hatch to preserve it into, and
                // `to_m3u8` doesn't render it either — skip gracefully.
            } else {
                let (bandwidth, codecs, resolution) =
                    pending_stream_inf.take().ok_or_else(|| Error::HlsParse {
                        line_no,
                        line: line.to_string(),
                        reason: "variant URI with no preceding #EXT-X-STREAM-INF".to_string(),
                    })?;
                variants.push(Variant {
                    bandwidth,
                    codecs,
                    resolution,
                    uri: line.to_string(),
                });
            }
        }

        if !saw_extm3u {
            return Err(Error::HlsParse {
                line_no: 1,
                line: String::new(),
                reason: "missing #EXTM3U header".to_string(),
            });
        }

        Ok(MasterPlaylist {
            version,
            variants,
            iframe_variants,
        })
    }
}

/// Parse a `RESOLUTION=<w>x<h>` attribute value.
fn parse_resolution(v: &str, line_no: usize, line: &str) -> Result<(u32, u32)> {
    let mut split = v.splitn(2, 'x');
    let w = split.next().unwrap_or("");
    let h = split.next().ok_or_else(|| Error::HlsParse {
        line_no,
        line: line.to_string(),
        reason: format!("RESOLUTION value {v:?} is not of the form <width>x<height>"),
    })?;
    let width = parse_decimal(w, line_no, line, "RESOLUTION width")?;
    let height = parse_decimal(h, line_no, line, "RESOLUTION height")?;
    Ok((width, height))
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
/// let mut seg0 = MediaSegment { uri: "s0.m4s".into(), duration: 5.0, discontinuous: false, parts: vec![], ..Default::default() };
/// let mut seg1 = MediaSegment { uri: "s1.m4s".into(), duration: 5.0, discontinuous: false, parts: vec![], ..Default::default() };
/// let mut seg2 = MediaSegment { uri: "s2.m4s".into(), duration: 5.0, discontinuous: false, parts: vec![], ..Default::default() };
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
            ..Default::default()
        }
    }

    fn seg_disc(uri: &str, duration: f64) -> MediaSegment {
        MediaSegment {
            uri: uri.into(),
            duration,
            discontinuous: true,
            parts: vec![],
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
                    ..Default::default()
                }],
                ..Default::default()
            }],
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll_config()),
            iframes_only: false,
            open_segment: None,
            ..Default::default()
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
                ..Default::default()
            }],
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll_config()),
            iframes_only: false,
            open_segment: Some(OpenSegment::new(vec![PartSpec {
                uri: "part-1-5.0.m4s".into(),
                duration: 0.5,
                independent: true,
                ..Default::default()
            }])),
            ..Default::default()
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
        // Exact count: only 1 closed segment, so exactly 1 #EXTINF occurrence.
        assert_eq!(
            out.matches("#EXTINF:").count(),
            1,
            "only closed segments should have #EXTINF lines; open segment must not:\n{out}"
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
            open_segment: Some(OpenSegment::new(vec![PartSpec {
                uri: "part-1-5.0.m4s".into(),
                duration: 0.5,
                independent: true,
                ..Default::default()
            }])),
            ..Default::default()
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
            ..Default::default()
        };
        let out = pl.to_m3u8();
        assert!(out.contains("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-5.1.m4s\""));
    }

    #[test]
    fn open_segment_parts_precede_preload_hint() {
        let mut ll = ll_config();
        ll.preload_hint_part = Some("part-1-5.1.m4s".into());
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
                ..Default::default()
            }],
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll),
            iframes_only: false,
            open_segment: Some(OpenSegment::new(vec![PartSpec {
                uri: "part-1-5.0.m4s".into(),
                duration: 0.5,
                independent: true,
                ..Default::default()
            }])),
            ..Default::default()
        };
        let out = pl.to_m3u8();
        // Both the open-segment part and preload-hint must be present.
        assert!(
            out.contains("#EXT-X-PART:DURATION=0.5,URI=\"part-1-5.0.m4s\",INDEPENDENT=YES"),
            "open-segment part must be present:\n{out}"
        );
        assert!(
            out.contains("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-5.1.m4s\""),
            "preload-hint must be present:\n{out}"
        );
        // The open-segment #EXT-X-PART line must appear BEFORE the #EXT-X-PRELOAD-HINT line.
        let part_pos = out
            .find("#EXT-X-PART:DURATION=0.5,URI=\"part-1-5.0.m4s\",INDEPENDENT=YES")
            .expect("open-segment part line not found");
        let preload_pos = out
            .find("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-5.1.m4s\"")
            .expect("preload-hint line not found");
        assert!(
            part_pos < preload_pos,
            "open-segment #EXT-X-PART must precede #EXT-X-PRELOAD-HINT:\npart at {}, preload at {}\noutput:\n{out}",
            part_pos,
            preload_pos
        );
    }

    // --- parsing (issue #717 slice 1): round-trip + real-world-sample tests ---

    fn ll_config_full() -> LowLatencyConfig {
        LowLatencyConfig {
            part_target: 0.5,
            part_hold_back: 1.5, // already at the 3x floor: idempotent through render.
            preload_hint_part: Some("part-9.2.m4s".into()),
            preload_hint_type: PreloadHintType::Part,
            preload_hint_byte_range_start: Some(0),
            preload_hint_byte_range_length: Some(1000),
            can_skip_until: Some(24.0),
        }
    }

    #[test]
    fn round_trip_live_ll_playlist_with_parts_preload_and_server_control() {
        let map = MapTag {
            uri: "init.mp4".into(),
            byte_range: Some(ByteRange {
                length: 800,
                offset: Some(0),
            }),
        };
        let pl = MediaPlaylist {
            version: 9,
            target_duration: 4,
            media_sequence: 100,
            discontinuity_sequence: 0,
            segments: vec![MediaSegment {
                uri: "seg-9.m4s".into(),
                duration: 4.0,
                discontinuous: false,
                parts: vec![
                    PartSpec {
                        uri: "part-9.0.m4s".into(),
                        duration: 0.5,
                        independent: true,
                        byte_range: None,
                        gap: false,
                    },
                    PartSpec {
                        uri: "part-9.1.m4s".into(),
                        duration: 0.5,
                        independent: false,
                        byte_range: Some(ByteRange {
                            length: 500,
                            offset: Some(1000),
                        }),
                        gap: false,
                    },
                ],
                byte_range: None,
                map: Some(map.clone()),
            }],
            open_segment: Some(OpenSegment::new(vec![PartSpec {
                uri: "part-10.0.m4s".into(),
                duration: 0.5,
                independent: true,
                byte_range: None,
                gap: true,
            }])),
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll_config_full()),
            iframes_only: false,
            rendition_reports: vec![RenditionReport {
                uri: "../audio/playlist.m3u8".into(),
                last_msn: 100,
                last_part: Some(1),
            }],
            skip: None,
        };
        let text = pl.to_m3u8();
        let parsed = MediaPlaylist::parse(&text).expect("parse must succeed");
        assert_eq!(parsed, pl, "round trip must be lossless:\n{text}");
    }

    #[test]
    fn round_trip_vod_playlist_with_byteranges_map_and_endlist() {
        let map = MapTag {
            uri: "init.mp4".into(),
            byte_range: None,
        };
        let pl = MediaPlaylist {
            version: 6,
            target_duration: 10,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![
                MediaSegment {
                    uri: "media.ts".into(),
                    duration: 10.0,
                    discontinuous: false,
                    parts: vec![],
                    byte_range: Some(ByteRange {
                        length: 500_000,
                        offset: Some(0),
                    }),
                    map: Some(map.clone()),
                },
                MediaSegment {
                    uri: "media.ts".into(),
                    duration: 10.0,
                    discontinuous: false,
                    parts: vec![],
                    // No offset: continues immediately after the previous
                    // sub-range of the same resource (RFC 8216bis §4.4.4.2).
                    byte_range: Some(ByteRange {
                        length: 500_000,
                        offset: None,
                    }),
                    // Same map as the previous segment — to_m3u8 must dedup
                    // (emit the tag only once) and parse must carry it forward.
                    map: Some(map.clone()),
                },
            ],
            open_segment: None,
            endlist: true,
            extra_tags: vec![
                "#EXT-X-DATERANGE:ID=\"ad-1\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=15.0"
                    .into(),
            ],
            low_latency: None,
            iframes_only: false,
            rendition_reports: vec![],
            skip: None,
        };
        let text = pl.to_m3u8();
        // The map is only emitted once (dedup), not once per segment.
        assert_eq!(
            text.matches("#EXT-X-MAP:").count(),
            1,
            "identical map on consecutive segments must render once:\n{text}"
        );
        let parsed = MediaPlaylist::parse(&text).expect("parse must succeed");
        assert_eq!(parsed, pl, "round trip must be lossless:\n{text}");
    }

    #[test]
    fn round_trip_multivariant_playlist() {
        let pl = MasterPlaylist {
            version: 7,
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
            iframe_variants: vec![IFrameVariant {
                bandwidth: 50_000,
                codecs: Some("avc1.64001e".into()),
                resolution: Some((640, 360)),
                uri: "v300/iframe.m3u8".into(),
            }],
        };
        let text = pl.to_m3u8();
        let parsed = MasterPlaylist::parse(&text).expect("parse must succeed");
        assert_eq!(parsed, pl, "round trip must be lossless:\n{text}");
    }

    /// Real-world sample: RFC 8216bis §9.11 "Low-Latency Playlist" appendix
    /// example, verbatim for the segment/part/discontinuity/preload-hint/
    /// rendition-report lines (only the elided `...` header lines were filled
    /// in with plausible values, since the spec elides them for brevity).
    #[test]
    fn real_world_sample_ll_playlist_from_rfc8216bis_appendix() {
        let text = "\
#EXTM3U
#EXT-X-VERSION:9
#EXT-X-TARGETDURATION:4
#EXT-X-MEDIA-SEQUENCE:266
#EXT-X-PART-INF:PART-TARGET=2.00002
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=6.00006
#EXTINF:4.00008,
fileSequence268.mp4
#EXTINF:4.00008,
fileSequence269.mp4
#EXTINF:4.00008,
fileSequence270.mp4
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI=\"filePart271.0.mp4\"
#EXT-X-PART:DURATION=2.00004,URI=\"filePart271.1.mp4\"
#EXTINF:4.00008,
fileSequence271.mp4
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI=\"filePart272.0.mp4\"
#EXT-X-PART:DURATION=0.50001,URI=\"filePart272.1.mp4\"
#EXTINF:2.50005,
fileSequence272.mp4
#EXT-X-DISCONTINUITY
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI=\"midRoll273.0.mp4\"
#EXT-X-PART:DURATION=2.00004,URI=\"midRoll273.1.mp4\"
#EXTINF:4.00008,
midRoll273.mp4
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI=\"midRoll274.0.mp4\"
#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"midRoll274.1.mp4\"
#EXT-X-RENDITION-REPORT:URI=\"/1M/LL-HLS.m3u8\",LAST-MSN=274,LAST-PART=1
";
        let pl = MediaPlaylist::parse(text).expect("real-world LL sample must parse");
        assert_eq!(pl.version, 9);
        assert_eq!(pl.target_duration, 4);
        assert_eq!(pl.media_sequence, 266);
        // 5 closed segments: 268, 269, 270, 271, 272 + the discontinuous
        // midRoll273 = 6; midRoll274 has parts but never closes with an
        // EXTINF/URI, so it becomes the open (in-progress) segment.
        assert_eq!(pl.segments.len(), 6, "{:?}", pl.segments);
        assert_eq!(pl.segments[4].uri, "fileSequence272.mp4");
        assert_eq!(pl.segments[4].parts.len(), 2);
        assert!(pl.segments[4].parts[0].independent);
        assert!(!pl.segments[4].parts[1].independent);
        assert_eq!(pl.segments[5].uri, "midRoll273.mp4");
        assert!(
            pl.segments[5].discontinuous,
            "midRoll273 follows #EXT-X-DISCONTINUITY"
        );
        let open = pl.open_segment.as_ref().expect("midRoll274 is open");
        assert_eq!(open.parts.len(), 1);
        assert_eq!(open.parts[0].uri, "midRoll274.0.mp4");
        let ll = pl.low_latency.as_ref().expect("LL config must be present");
        assert_eq!(ll.preload_hint_part.as_deref(), Some("midRoll274.1.mp4"));
        assert_eq!(pl.rendition_reports.len(), 1);
        assert_eq!(pl.rendition_reports[0].uri, "/1M/LL-HLS.m3u8");
        assert_eq!(pl.rendition_reports[0].last_msn, 274);
        assert_eq!(pl.rendition_reports[0].last_part, Some(1));
    }

    /// Real-world-shaped sample: a Playlist Delta Update (`#EXT-X-SKIP`),
    /// hand-written per RFC 8216bis §4.4.5.2's confirmed attribute grammar
    /// (no full numeric example is given in the spec appendix for this tag).
    #[test]
    fn real_world_sample_delta_update_with_skip() {
        let text = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-MEDIA-SEQUENCE:1000\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,CAN-SKIP-UNTIL=24.0,PART-HOLD-BACK=1.5\n\
#EXT-X-SKIP:SKIPPED-SEGMENTS=996,RECENTLY-REMOVED-DATERANGES=\"ad-1\tad-2\"\n\
#EXTINF:4.00000,\n\
fileSequence1996.mp4\n\
#EXTINF:4.00000,\n\
fileSequence1997.mp4\n";
        let pl = MediaPlaylist::parse(text).expect("delta update sample must parse");
        assert_eq!(pl.media_sequence, 1000);
        assert_eq!(pl.segments.len(), 2);
        assert_eq!(pl.segments[0].uri, "fileSequence1996.mp4");
        let skip = pl.skip.as_ref().expect("EXT-X-SKIP must be captured");
        assert_eq!(skip.skipped_segments, 996);
        assert_eq!(skip.recently_removed_daterange_ids, vec!["ad-1", "ad-2"]);
        let ll = pl.low_latency.as_ref().expect("LL config must be present");
        assert_eq!(ll.can_skip_until, Some(24.0));
        assert!(!pl.endlist);
    }

    #[test]
    fn parse_ignores_unrecognized_tag_by_preserving_it_into_extra_tags() {
        let text = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:00.000Z\n\
#EXTINF:6.000,\n\
s0.m4s\n\
#EXT-X-ENDLIST\n";
        let pl = MediaPlaylist::parse(text).expect("unrecognized tag must not error");
        assert!(
            pl.extra_tags
                .iter()
                .any(|t| t.starts_with("#EXT-X-PROGRAM-DATE-TIME:")),
            "unrecognized tag must be preserved verbatim, not dropped: {:?}",
            pl.extra_tags
        );
    }

    #[test]
    fn parse_rejects_missing_targetduration() {
        let text = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:6.000,\ns0.m4s\n";
        let err = MediaPlaylist::parse(text).expect_err("missing TARGETDURATION must error");
        assert!(matches!(err, Error::HlsParse { .. }));
    }

    #[test]
    fn parse_rejects_malformed_part_missing_duration() {
        let text = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART:URI=\"part.m4s\"\n\
#EXTINF:4.000,\n\
seg.m4s\n";
        let err = MediaPlaylist::parse(text).expect_err("EXT-X-PART without DURATION must error");
        match err {
            Error::HlsParse { reason, .. } => {
                assert!(reason.contains("DURATION"), "{reason}");
            }
            other => panic!("expected HlsParse, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_variant_uri_with_no_preceding_stream_inf() {
        let text = "#EXTM3U\n#EXT-X-VERSION:6\nv300/index.m3u8\n";
        let err = MasterPlaylist::parse(text).expect_err("orphan variant URI must error");
        assert!(matches!(err, Error::HlsParse { .. }));
    }

    #[test]
    fn parse_master_playlist_ignores_ext_x_media() {
        // #EXT-X-MEDIA is not modeled (to_m3u8 doesn't render it either) but
        // must not cause a parse error.
        let text = "#EXTM3U\n\
#EXT-X-VERSION:7\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"English\",DEFAULT=YES,URI=\"eng.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=300000,CODECS=\"avc1.64001e,mp4a.40.2\"\n\
v300/index.m3u8\n";
        let pl = MasterPlaylist::parse(text).expect("EXT-X-MEDIA must be ignored, not error");
        assert_eq!(pl.variants.len(), 1);
        assert_eq!(pl.variants[0].uri, "v300/index.m3u8");
    }
}
