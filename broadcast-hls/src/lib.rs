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
//!   protocol version ≥ 4 when this tag is present; the renderer computes
//!   this as one input to the general `#EXT-X-VERSION` derivation (see
//!   "Protocol version derivation" below), not as a special case.
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
//! A caller assembling segments (e.g. the `transmux` crate's `Segmenter`,
//! via its `mark_discontinuity` method) marks the next cut as discontinuous;
//! a segmenter also typically auto-detects init-segment changes and marks
//! those cuts automatically (see [`mark_init_discontinuities`] below).
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
//! # Protocol version derivation (RFC 8216bis §8, issue #871)
//!
//! `#EXT-X-VERSION` is never chosen ahead of time — it is *computed* as the
//! `max()` of the minimums the playlist's actual content triggers, per the
//! feature → minimum-version table transcribed at
//! `docs/version-compatibility.md` (twice-verified against
//! draft-pantos-hls-rfc8216bis-22 §8). [`MediaPlaylist::computed_version`]
//! and [`MasterPlaylist::computed_version`] expose this directly; `to_m3u8`
//! uses it internally. A Playlist that triggers nothing (fully compatible
//! with version 1) carries **no** `#EXT-X-VERSION` tag at all, per §8's
//! opening rule.
//!
//! [`MediaPlaylist::version`]/[`MasterPlaylist::version`] stay a *settable*
//! floor rather than becoming computed-only: `0` (the field's `Default`)
//! means "no explicit floor" (render exactly the computed value, or nothing);
//! a nonzero value is raised — never lowered — to the computed minimum, so a
//! caller can still deliberately over-declare (e.g. the backward-compatible
//! `EXT-X-MEDIA`/`AUDIO`/`VIDEO`/`SUBTITLES` MAY-rule in §8) but can never
//! silently under-declare an invalid playlist.
//!
//! This replaces a real bug (issue #871): an LL-HLS origin previously baked
//! in a hardcoded `EXT-X-VERSION:9` unconditionally, even though none of the
//! low-latency tags it emits (`EXT-X-PART`/`EXT-X-PART-INF`/
//! `EXT-X-PRELOAD-HINT`/`EXT-X-SERVER-CONTROL`) carry any version
//! requirement at all — only `EXT-X-SKIP` does. RFC 8216 §7: "A client MUST
//! NOT attempt playback if it does not support the protocol version
//! specified by the EXT-X-VERSION tag" — so over-declaring silently locks
//! out every client that supports the playlist's true (lower) requirement.
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
//! DASH side (the `transmux` crate's `dash` module); `cenc_ext_x_key` returns
//! `None` rather than emit an invalid tag.
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
//!   renditions) is not modeled with typed fields — but `MasterPlaylist`
//!   now has its own `extra_tags` (mirroring [`MediaPlaylist::extra_tags`]):
//!   `MasterPlaylist::parse` preserves an unrecognized `#EXT-...` tag like
//!   this one verbatim, and `to_m3u8` re-renders it, so it round-trips (just
//!   without structured field access) rather than being silently dropped.
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
//!
//! Depends only on `broadcast-common`. `#![no_std]` (+ `alloc`) when the
//! `std` feature is disabled.
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p broadcast-hls --example <name>`.\n"]
#![doc = "\n### `build_playlist`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_playlist.rs")]
#![doc = "```\n\n### `parse_playlist`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_playlist.rs")]
#![doc = "```"]

extern crate alloc;

mod error;

pub use error::{Error, Result};

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use broadcast_common::hex::hex_encode;

/// A CENC protection scheme (`schm.scheme_type`) — ISO/IEC 23001-7 §4.
///
/// Re-exported from [`broadcast_common::cenc`], which holds the single
/// definition. CENC is *Common* Encryption — a container-independent scheme
/// identity — so it sits below both this crate and `transmux` (which owns the
/// ISOBMFF `schm`/`tenc`/`senc` boxes carrying it), rather than being defined
/// twice and converted at the boundary (issues #564, #878).
pub use broadcast_common::CencScheme;

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
/// URI — no DRM logic lives here) and `kid` is the track's Track Encryption
/// Box default KID (`tenc.default_kid`, ISO/IEC 14496-12 §8.12.1 — the
/// `transmux` crate's `cenc::TrackEncryptionBox::default_kid` /
/// `media::TrackEncryption::tenc::default_kid`).
pub fn cenc_ext_x_key(scheme: CencScheme, kid: &[u8; 16], key_uri: &str) -> Option<String> {
    if scheme != CencScheme::Cbcs {
        return None;
    }
    Some(format!(
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"{key_uri}\",KEYFORMAT=\"{CENC_KEYFORMAT}\",\
         KEYFORMATVERSIONS=\"{CENC_KEYFORMATVERSIONS}\",KEYID=0x{}",
        hex_encode(kid)
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
#[non_exhaustive]
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
    /// The Media Initialization Section in effect for this segment (RFC
    /// 8216bis §4.4.4.5) — `#EXT-X-MAP` applies "until the next `EXT-X-MAP`
    /// tag or the end of the Playlist", so this carries forward from the
    /// last `#EXT-X-MAP` seen, whether or not any segment has *closed* yet.
    /// `None` if no `#EXT-X-MAP` has appeared at all (rare in practice — a
    /// live LL-HLS playlist's very first segment normally needs one).
    pub map: Option<MapTag>,
}

impl OpenSegment {
    /// Build an open segment from its in-progress parts, with no map (see
    /// [`Self::with_map`] to attach one).
    pub fn new(parts: Vec<PartSpec>) -> Self {
        Self { parts, map: None }
    }

    /// Attach the Media Initialization Section in effect for this segment.
    pub fn with_map(mut self, map: MapTag) -> Self {
        self.map = Some(map);
        self
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
    /// `#EXT-X-VERSION` — an explicit *floor*, not the rendered value. `0`
    /// (this type's `Default`) means "no explicit floor": [`Self::to_m3u8`]
    /// renders exactly [`Self::computed_version`], or no tag at all when
    /// nothing triggers one. A nonzero value is raised — never lowered — to
    /// the computed minimum. See the module's "Protocol version derivation"
    /// docs (issue #871).
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
#[derive(Debug, Clone, PartialEq)]
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
    /// `CAN-BLOCK-RELOAD` attribute of `#EXT-X-SERVER-CONTROL` (RFC 8216bis
    /// §4.4.3.8) — whether the server supports Blocking Playlist Reload
    /// (RFC 8216bis §6.2.5.2). `to_m3u8` renders the actual value
    /// (`YES`/`NO`) rather than assuming `YES`; `MediaPlaylist::parse`
    /// derives it from the attribute's real wire value, defaulting to
    /// `false` (per RFC 8216bis) when the attribute — or the whole
    /// `#EXT-X-SERVER-CONTROL` tag — is absent. A client MUST NOT infer
    /// blocking-reload support merely from [`MediaPlaylist::low_latency`]
    /// being `Some`; it must check this field (issue #717 slice 1 gap).
    pub can_block_reload: bool,
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

impl Default for LowLatencyConfig {
    /// Defaults `can_block_reload` to `true` — this crate's own LL-HLS
    /// origin (and every in-repo test/fixture that builds a
    /// [`LowLatencyConfig`] from scratch via `..Default::default()`) always
    /// supports blocking reload, matching `to_m3u8()`'s historical
    /// `CAN-BLOCK-RELOAD=YES` output. This default is never consulted for a
    /// *parsed* playlist: [`MediaPlaylist::parse`] always derives
    /// `can_block_reload` from the wire attribute (RFC 8216bis default:
    /// `false`/absent-means-NO), independent of this `Default` impl.
    fn default() -> Self {
        Self {
            part_target: 0.0,
            part_hold_back: 0.0,
            preload_hint_part: None,
            preload_hint_type: PreloadHintType::default(),
            preload_hint_byte_range_start: None,
            preload_hint_byte_range_length: None,
            can_skip_until: None,
            can_block_reload: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Protocol Version Compatibility — RFC 8216bis §8 (issue #871).
//
// Source of truth: `docs/version-compatibility.md`, a twice-verified
// transcription of §8 from draft-pantos-hls-rfc8216bis-22. One named
// constant per row of that table, cited by row number. `#EXT-X-VERSION` is
// always `max()` over the minimums the playlist's actual content triggers —
// never a value picked ahead of time and baked in (the bug this issue
// fixes: a low-latency origin unconditionally declaring 9 when nothing it
// emits requires more than 6, forcing every client on 6/7/8 to refuse a
// stream it could have played).
// ---------------------------------------------------------------------------

/// §8 row 2 (Media Playlist): the `IV` attribute of `EXT-X-KEY`.
const VERSION_KEY_IV: u8 = 2;
/// §8 row 3 (Media Playlist): a floating-point `EXTINF` duration.
const VERSION_FLOAT_EXTINF: u8 = 3;
/// §8 row 4 (Media Playlist): `EXT-X-BYTERANGE`, or `EXT-X-I-FRAMES-ONLY`.
const VERSION_BYTERANGE_OR_IFRAMES_ONLY: u8 = 4;
/// §8 row 5 (Media Playlist): `EXT-X-KEY` with `METHOD=SAMPLE-AES`, or its
/// `KEYFORMAT`/`KEYFORMATVERSIONS` attributes, or `EXT-X-MAP` **together
/// with** `EXT-X-I-FRAMES-ONLY`. (`EXT-X-MAP` alone, without
/// `EXT-X-I-FRAMES-ONLY`, needs [`VERSION_MAP_WITHOUT_IFRAMES_ONLY`] = 6
/// instead — the row-6 note in `docs/version-compatibility.md`.)
const VERSION_SAMPLE_AES_OR_KEYFORMAT_OR_MAP_WITH_IFRAMES_ONLY: u8 = 5;
/// §8 row 6 (Media Playlist): `EXT-X-MAP` in a playlist that does *not* also
/// carry `EXT-X-I-FRAMES-ONLY`.
const VERSION_MAP_WITHOUT_IFRAMES_ONLY: u8 = 6;
/// §8 row 7 (Multivariant Playlist): a `"SERVICE"` value for the
/// `INSTREAM-ID` attribute of `EXT-X-MEDIA`.
const VERSION_MEDIA_SERVICE_INSTREAM_ID: u8 = 7;
/// §8 row 8 (any Playlist): variable substitution.
const VERSION_VARIABLE_SUBSTITUTION: u8 = 8;
/// §8 row 9 (any Playlist): `EXT-X-SKIP`.
const VERSION_SKIP: u8 = 9;
/// §8 row 10 (any Playlist): an `EXT-X-SKIP` that replaces
/// `EXT-X-DATERANGE` tags in a Playlist Delta Update — its
/// `RECENTLY-REMOVED-DATERANGES` attribute is non-empty.
const VERSION_SKIP_REPLACES_DATERANGE: u8 = 10;
/// §8 row 11 (any Playlist): `EXT-X-DEFINE` with a `QUERYPARAM` attribute.
const VERSION_DEFINE_QUERYPARAM: u8 = 11;
/// §8 row 12 (any Playlist): an attribute whose name starts with `"REQ-"`.
const VERSION_REQ_ATTRIBUTE: u8 = 12;
/// §8 row 13 (Multivariant Playlist): `EXT-X-MEDIA` with an `INSTREAM-ID`
/// attribute for a non-`CLOSED-CAPTIONS` `TYPE`.
const VERSION_MEDIA_INSTREAM_ID_NON_CC: u8 = 13;

/// RFC 8216bis §4.2.2's variable-substitution marker (`{$name}`, inside a
/// quoted-string attribute value or a URI) — §8 row 8's trigger. This is
/// *not* part of the version-compatibility transcription (that section
/// states only the rule, not the substitution grammar, which lives in a
/// different part of the spec) — it is a textual heuristic over the opaque
/// strings this crate already carries (`extra_tags`, segment/variant URIs),
/// not a modeled `EXT-X-DEFINE` parser.
const VARIABLE_SUBSTITUTION_MARKER: &str = "{$";

/// Fold `n` into the running maximum `v` — §8's rule is that the highest
/// version required by any single triggered feature governs the whole
/// Playlist.
fn bump_version(v: &mut Option<u8>, n: u8) {
    *v = Some(match *v {
        Some(m) => m.max(n),
        None => n,
    });
}

/// `true` if `duration` is not a whole number of seconds, to the same
/// integer-millisecond precision `to_m3u8` itself renders at (§8 row 3).
/// Uses integer millisecond math rather than `f64::fract()`/`round()`,
/// which are `std`-only and unavailable to this `no_std`+`alloc` crate (see
/// `format_secs` below for the same pattern).
fn is_fractional_duration(duration: f64) -> bool {
    let millis = (duration * 1000.0 + 0.5) as u64;
    millis % 1000 != 0
}

/// `true` if `s` carries a variable-substitution reference (§8 row 8).
fn contains_variable_substitution(s: &str) -> bool {
    s.contains(VARIABLE_SUBSTITUTION_MARKER)
}

/// Scan a set of opaque, verbatim tag lines (a playlist's `extra_tags`) for
/// the §8 triggers that are attributes of tags this crate does not model as
/// struct fields: `EXT-X-KEY`'s `IV`/`METHOD`/`KEYFORMAT*` attributes (rows
/// 2 and 5), `EXT-X-DEFINE`'s `QUERYPARAM` (row 11), `EXT-X-MEDIA`'s
/// `INSTREAM-ID` (rows 7 and 13 — Multivariant-Playlist-only; harmless to
/// scan on a Media Playlist's `extra_tags` since that tag never legitimately
/// appears there), and any attribute name starting `REQ-` (row 12). Returns
/// the max triggered version, or `None`.
fn scan_tag_lines_for_version(tags: &[String]) -> Option<u8> {
    let mut v: Option<u8> = None;
    for tag in tags {
        if let Some(rest) = tag.strip_prefix("#EXT-X-KEY:") {
            let attrs = parse_attr_list(rest);
            if attrs.contains_key("IV") {
                bump_version(&mut v, VERSION_KEY_IV);
            }
            if attrs.get("METHOD").map(String::as_str) == Some("SAMPLE-AES")
                || attrs.contains_key("KEYFORMAT")
                || attrs.contains_key("KEYFORMATVERSIONS")
            {
                bump_version(
                    &mut v,
                    VERSION_SAMPLE_AES_OR_KEYFORMAT_OR_MAP_WITH_IFRAMES_ONLY,
                );
            }
        } else if let Some(rest) = tag.strip_prefix("#EXT-X-DEFINE:") {
            if parse_attr_list(rest).contains_key("QUERYPARAM") {
                bump_version(&mut v, VERSION_DEFINE_QUERYPARAM);
            }
        } else if let Some(rest) = tag.strip_prefix("#EXT-X-MEDIA:") {
            let attrs = parse_attr_list(rest);
            if let Some(instream_id) = attrs.get("INSTREAM-ID") {
                if instream_id.starts_with("SERVICE") {
                    bump_version(&mut v, VERSION_MEDIA_SERVICE_INSTREAM_ID);
                }
                let is_closed_captions =
                    attrs.get("TYPE").map(String::as_str) == Some("CLOSED-CAPTIONS");
                if !is_closed_captions {
                    bump_version(&mut v, VERSION_MEDIA_INSTREAM_ID_NON_CC);
                }
            }
        }
        // Row 12 applies to ANY attribute-bearing tag, not just the three
        // handled above — scan every tag's attribute keys uniformly.
        if let Some(colon) = tag.find(':') {
            if parse_attr_list(&tag[colon + 1..])
                .keys()
                .any(|k| k.starts_with("REQ-"))
            {
                bump_version(&mut v, VERSION_REQ_ATTRIBUTE);
            }
        }
    }
    v
}

/// Shared floor/clamp logic behind `MediaPlaylist`'s and `MasterPlaylist`'s
/// private `effective_version` methods: `explicit` (the public `version`
/// field) acts as a floor over `computed` (the derived minimum), raised —
/// never lowered — to it. `0` means "no explicit floor".
fn effective_version(explicit: u8, computed: Option<u8>) -> Option<u8> {
    match (explicit, computed) {
        (0, None) => None,
        (0, Some(m)) => Some(m),
        (e, None) => Some(e),
        (e, Some(m)) => Some(e.max(m)),
    }
}

impl MediaPlaylist {
    /// Compute the minimum `#EXT-X-VERSION` this playlist's actual content
    /// requires, per RFC 8216bis §8 (`docs/version-compatibility.md`).
    /// `None` means the playlist is fully compatible with version 1, which
    /// per §8's opening rule need not carry the tag at all.
    ///
    /// This is `max()` over every triggered rule — see the `VERSION_*`
    /// constants above for the rule → row mapping. [`Self::to_m3u8`] uses
    /// this (via the private `effective_version`) rather than blindly
    /// trusting [`Self::version`]; see that field's docs for how an
    /// explicit value interacts with this computed minimum.
    pub fn computed_version(&self) -> Option<u8> {
        let mut v: Option<u8> = None;

        // Rows 5/6: EXT-X-MAP, present either as a structured
        // MediaSegment/OpenSegment map, or (a caller that hasn't adopted
        // that field, e.g. a raw #EXT-X-MAP: line) in `extra_tags`.
        let has_map = self.segments.iter().any(|s| s.map.is_some())
            || self.open_segment.as_ref().is_some_and(|o| o.map.is_some())
            || self.extra_tags.iter().any(|t| t.starts_with("#EXT-X-MAP:"));
        if has_map {
            bump_version(
                &mut v,
                if self.iframes_only {
                    VERSION_SAMPLE_AES_OR_KEYFORMAT_OR_MAP_WITH_IFRAMES_ONLY
                } else {
                    VERSION_MAP_WITHOUT_IFRAMES_ONLY
                },
            );
        }

        // Row 4: EXT-X-BYTERANGE, or EXT-X-I-FRAMES-ONLY.
        if self.iframes_only || self.segments.iter().any(|s| s.byte_range.is_some()) {
            bump_version(&mut v, VERSION_BYTERANGE_OR_IFRAMES_ONLY);
        }

        // Row 3: floating-point EXTINF duration.
        if self
            .segments
            .iter()
            .any(|s| is_fractional_duration(s.duration))
        {
            bump_version(&mut v, VERSION_FLOAT_EXTINF);
        }

        // Rows 9/10: EXT-X-SKIP, optionally replacing EXT-X-DATERANGE.
        if let Some(skip) = &self.skip {
            bump_version(&mut v, VERSION_SKIP);
            if !skip.recently_removed_daterange_ids.is_empty() {
                bump_version(&mut v, VERSION_SKIP_REPLACES_DATERANGE);
            }
        }

        // Row 8: variable substitution, wherever this crate carries a URI
        // or tag verbatim.
        if self
            .extra_tags
            .iter()
            .any(|t| contains_variable_substitution(t))
            || self
                .segments
                .iter()
                .any(|s| contains_variable_substitution(&s.uri))
        {
            bump_version(&mut v, VERSION_VARIABLE_SUBSTITUTION);
        }

        // Rows 2/5/11/12: EXT-X-KEY's IV/METHOD/KEYFORMAT*, EXT-X-DEFINE's
        // QUERYPARAM, and any REQ- attribute — reachable today only via
        // extra_tags (none of these tags has a modeled struct field).
        if let Some(m) = scan_tag_lines_for_version(&self.extra_tags) {
            bump_version(&mut v, m);
        }

        v
    }

    /// The `#EXT-X-VERSION` value actually rendered by [`Self::to_m3u8`]:
    /// [`Self::computed_version`], raised — never lowered — to
    /// [`Self::version`] when that field carries a nonzero explicit floor.
    /// See [`Self::version`]'s docs for the full rule.
    fn effective_version(&self) -> Option<u8> {
        effective_version(self.version, self.computed_version())
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
    /// (RFC 8216 §4.3.3.6) in the header block; the version this (and every
    /// other triggering feature) requires is derived by
    /// [`Self::computed_version`] (see the module's "Protocol version
    /// derivation" docs, issue #871) — omitted entirely when nothing in the
    /// playlist triggers a version requirement.
    pub fn to_m3u8(&self) -> String {
        let mut s = String::new();
        s.push_str("#EXTM3U\n");
        if let Some(version) = self.effective_version() {
            s.push_str(&format!("#EXT-X-VERSION:{version}\n"));
        }
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
            // #EXT-X-SERVER-CONTROL — CAN-BLOCK-RELOAD (the actual value,
            // not always YES) + PART-HOLD-BACK (>= 3x part-target, enforced
            // by effective_part_hold_back) + optional CAN-SKIP-UNTIL
            // (RFC 8216bis §4.4.3.8).
            s.push_str(&format!(
                "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD={},PART-HOLD-BACK={}",
                if ll.can_block_reload { "YES" } else { "NO" },
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
                // Same dedup-vs-previous rule as the closed segments' loop
                // above: `#EXT-X-MAP` applies until a new one is seen, so
                // only emit it here if it differs from the last *closed*
                // segment's map (or there were no closed segments at all).
                let prev_map = self.segments.last().and_then(|s| s.map.as_ref());
                if open.map.as_ref() != prev_map {
                    if let Some(map) = &open.map {
                        push_map_line(&mut s, map);
                    }
                }
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
        // `0` (not `1`): distinguishes "no #EXT-X-VERSION line was present
        // on the wire at all" from "the wire explicitly said version 1",
        // so a fully-untagged input round-trips back to no tag rather than
        // gaining one (see `effective_version`/issue #871).
        let mut version: u8 = 0;
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
        // RFC 8216bis §4.4.3.8: absent CAN-BLOCK-RELOAD (or an absent
        // #EXT-X-SERVER-CONTROL tag entirely) means the server does NOT
        // support Blocking Playlist Reload — default false, not the
        // `LowLatencyConfig::default()` convenience value of true.
        let mut can_block_reload = false;
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
                can_block_reload = attrs.get("CAN-BLOCK-RELOAD").map(String::as_str) == Some("YES");
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
            let open = OpenSegment::new(pending_parts);
            Some(match &current_map {
                Some(map) => open.with_map(map.clone()),
                None => open,
            })
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
                can_block_reload,
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
    /// `#EXT-X-VERSION` — an explicit *floor*, not the rendered value. `0`
    /// means "no explicit floor": [`Self::to_m3u8`] renders exactly
    /// [`Self::computed_version`], or no tag at all when nothing triggers
    /// one. A nonzero value is raised — never lowered — to the computed
    /// minimum. See the module's "Protocol version derivation" docs
    /// (issue #871).
    pub version: u8,
    /// Ordered list of variant streams.
    pub variants: Vec<Variant>,
    /// Ordered list of I-frame-only renditions (RFC 8216 §4.3.4.2).
    ///
    /// Each entry is rendered as an `#EXT-X-I-FRAME-STREAM-INF` line with the
    /// URI as an attribute (not a following line).  An empty `Vec` (the
    /// default) produces no such lines.
    pub iframe_variants: Vec<IFrameVariant>,
    /// Extra tag lines emitted verbatim after the variant/I-frame-variant
    /// entries (e.g. `#EXT-X-MEDIA:...`, `#EXT-X-DEFINE:...`) — the
    /// Multivariant-Playlist counterpart of [`MediaPlaylist::extra_tags`].
    /// [`Self::parse`] preserves any unrecognized `#EXT-...` tag here
    /// (forward-compat) instead of dropping it; [`Self::computed_version`]
    /// scans these lines for the §8 rows this crate does not (yet) model as
    /// typed struct fields (rows 7/8/11/12/13 —
    /// `docs/version-compatibility.md`).
    pub extra_tags: Vec<String>,
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
        if let Some(version) = self.effective_version() {
            s.push_str(&format!("#EXT-X-VERSION:{version}\n"));
        }

        for tag in &self.extra_tags {
            s.push_str(tag);
            s.push('\n');
        }

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
    /// audio/subtitle renditions), `#EXT-X-DEFINE`, and any other
    /// unrecognized `#EXT-...` tag are not modeled with typed fields, but
    /// are preserved verbatim into [`Self::extra_tags`] (forward-compat),
    /// mirroring [`MediaPlaylist::parse`]. A malformed
    /// `#EXT-X-STREAM-INF`/`#EXT-X-I-FRAME-STREAM-INF` (missing required
    /// attribute, unparsable value) or a variant URI with no preceding
    /// `#EXT-X-STREAM-INF` returns [`crate::Error::HlsParse`].
    pub fn parse(input: &str) -> Result<Self> {
        // `0` (not `1`): see the identical comment in `MediaPlaylist::parse`.
        let mut version: u8 = 0;
        let mut variants: Vec<Variant> = Vec::new();
        let mut iframe_variants: Vec<IFrameVariant> = Vec::new();
        let mut extra_tags: Vec<String> = Vec::new();
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
            } else if let Some(rest) = line.strip_prefix("#EXT") {
                let _ = rest;
                // A well-formed but unrecognized tag (e.g. #EXT-X-MEDIA,
                // #EXT-X-DEFINE): preserve verbatim (forward-compat, and the
                // substrate `computed_version` scans for rows 7/8/11/12/13),
                // mirroring `MediaPlaylist::parse`.
                extra_tags.push(line.to_string());
            } else if line.starts_with('#') {
                // RFC 8216 §4.1: a non-"#EXT" '#' line is a comment — ignore.
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
            extra_tags,
        })
    }

    /// Compute the minimum `#EXT-X-VERSION` this Multivariant Playlist's
    /// actual content requires, per RFC 8216bis §8
    /// (`docs/version-compatibility.md`). `None` means fully compatible
    /// with version 1 (no tag required). See
    /// [`MediaPlaylist::computed_version`] for the Media-Playlist-only rows
    /// (2–6, 9–10) that do not apply to a Multivariant Playlist.
    pub fn computed_version(&self) -> Option<u8> {
        let mut v = scan_tag_lines_for_version(&self.extra_tags);
        if self
            .extra_tags
            .iter()
            .any(|t| contains_variable_substitution(t))
            || self
                .variants
                .iter()
                .any(|var| contains_variable_substitution(&var.uri))
            || self
                .iframe_variants
                .iter()
                .any(|iv| contains_variable_substitution(&iv.uri))
        {
            bump_version(&mut v, VERSION_VARIABLE_SUBSTITUTION);
        }
        v
    }

    /// The `#EXT-X-VERSION` value actually rendered by [`Self::to_m3u8`] —
    /// see [`MediaPlaylist::effective_version`] for the shared rule.
    fn effective_version(&self) -> Option<u8> {
        effective_version(self.version, self.computed_version())
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
/// use broadcast_hls::{mark_init_discontinuities, MediaSegment};
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
            extra_tags: vec![],
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
            extra_tags: vec![],
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

    /// Issue #717 slice 5 follow-up fix: `#EXT-X-MAP` applies "until the
    /// next `EXT-X-MAP` tag or the end of the Playlist" (RFC 8216bis
    /// §4.4.4.5) — including to the *open* (never-yet-closed) segment, even
    /// when NO segment has closed yet (the very first segment of a
    /// freshly-tuned-into live stream). Before this fix, `OpenSegment`
    /// carried no `map` field at all, so a client parsing a playlist with
    /// only an open segment had no way to learn the init segment's URI
    /// until that segment's first *closed* appearance — needlessly
    /// delaying every part's playback until then.
    #[test]
    fn open_segment_inherits_map_when_no_segment_has_closed_yet() {
        let text = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-MEDIA-SEQUENCE:1\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXT-X-MAP:URI=\"init-1.mp4\"\n\
#EXT-X-PART:DURATION=0.5,URI=\"part-1-1.0.m4s\",INDEPENDENT=YES\n\
#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-1.1.m4s\"\n";
        let pl = MediaPlaylist::parse(text).expect("must parse");
        assert!(pl.segments.is_empty(), "no segment has closed yet");
        let open = pl.open_segment.as_ref().expect("one open segment");
        assert_eq!(
            open.map,
            Some(MapTag {
                uri: "init-1.mp4".into(),
                byte_range: None,
            }),
            "the open segment must inherit the EXT-X-MAP that precedes it, \
             even though no segment has closed yet"
        );
    }

    /// Round trip the same no-closed-segments-yet shape through
    /// `to_m3u8`/`parse`, proving the renderer emits the `#EXT-X-MAP` line
    /// for a bare open segment (not just for closed ones).
    #[test]
    fn round_trip_open_segment_only_with_map() {
        let pl = MediaPlaylist {
            version: 9,
            target_duration: 4,
            media_sequence: 1,
            discontinuity_sequence: 0,
            segments: vec![],
            open_segment: Some(
                OpenSegment::new(vec![PartSpec {
                    uri: "part-1-1.0.m4s".into(),
                    duration: 0.5,
                    independent: true,
                    ..Default::default()
                }])
                .with_map(MapTag {
                    uri: "init-1.mp4".into(),
                    byte_range: None,
                }),
            ),
            endlist: false,
            extra_tags: vec![],
            low_latency: Some(ll_config()),
            iframes_only: false,
            ..Default::default()
        };
        let text = pl.to_m3u8();
        assert!(
            text.contains("#EXT-X-MAP:URI=\"init-1.mp4\""),
            "renderer must emit EXT-X-MAP for a bare open segment:\n{text}"
        );
        let parsed = MediaPlaylist::parse(&text).expect("parse must succeed");
        assert_eq!(parsed, pl, "round trip must be lossless:\n{text}");
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
            can_block_reload: true,
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
            // `#EXT-X-MAP` applies "until the next EXT-X-MAP or the end of
            // the Playlist" (RFC 8216bis §4.4.4.5) — the open segment is
            // still governed by the same map as the preceding closed
            // segment (no new `#EXT-X-MAP` appears between them), so it
            // must carry it too for a lossless round trip.
            open_segment: Some(
                OpenSegment::new(vec![PartSpec {
                    uri: "part-10.0.m4s".into(),
                    duration: 0.5,
                    independent: true,
                    byte_range: None,
                    gap: true,
                }])
                .with_map(map.clone()),
            ),
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
            extra_tags: vec![],
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
        assert!(
            ll.can_block_reload,
            "CAN-BLOCK-RELOAD=YES in the fixture must parse to true"
        );
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

    /// Issue #717 slice 1 fix: an origin that advertises LL-HLS tags but
    /// explicitly declines blocking reload (`CAN-BLOCK-RELOAD=NO`) must
    /// parse to `can_block_reload == false` — a client inferring support
    /// from `low_latency.is_some()` alone would get this wrong.
    #[test]
    fn real_world_sample_can_block_reload_no_is_parsed_not_inferred() {
        let text = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=NO,PART-HOLD-BACK=1.5\n\
#EXTINF:4.00000,\n\
seg0.mp4\n";
        let pl = MediaPlaylist::parse(text).expect("must parse");
        let ll = pl
            .low_latency
            .as_ref()
            .expect("LL config must be present (PART-INF seen)");
        assert!(
            !ll.can_block_reload,
            "CAN-BLOCK-RELOAD=NO must not be inferred as true just because low_latency is Some"
        );
    }

    /// RFC 8216bis §4.4.3.8: an absent `CAN-BLOCK-RELOAD` attribute (or an
    /// entirely absent `#EXT-X-SERVER-CONTROL` tag) means the server does
    /// NOT support blocking reload — default `false`, distinct from
    /// [`LowLatencyConfig::default()`]'s convenience value of `true`.
    #[test]
    fn parse_defaults_can_block_reload_false_when_attribute_absent() {
        let text = "#EXTM3U\n\
#EXT-X-VERSION:9\n\
#EXT-X-TARGETDURATION:4\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n\
#EXTINF:4.00000,\n\
seg0.mp4\n";
        let pl = MediaPlaylist::parse(text).expect("must parse");
        let ll = pl
            .low_latency
            .as_ref()
            .expect("LL config must be present (PART-INF seen)");
        assert!(
            !ll.can_block_reload,
            "absent CAN-BLOCK-RELOAD/SERVER-CONTROL must default to false, not true"
        );
    }

    /// Round trip a `CAN-BLOCK-RELOAD=NO` config through `to_m3u8`/`parse`
    /// to prove the renderer emits the actual value (not a hardcoded YES).
    #[test]
    fn round_trip_can_block_reload_no() {
        let pl = MediaPlaylist {
            version: 9,
            target_duration: 4,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![MediaSegment {
                uri: "seg0.mp4".into(),
                duration: 4.0,
                ..Default::default()
            }],
            low_latency: Some(LowLatencyConfig {
                part_target: 0.5,
                part_hold_back: 1.5,
                can_block_reload: false,
                ..Default::default()
            }),
            ..Default::default()
        };
        let text = pl.to_m3u8();
        assert!(
            text.contains("CAN-BLOCK-RELOAD=NO"),
            "renderer must emit the actual value:\n{text}"
        );
        let parsed = MediaPlaylist::parse(&text).expect("parse must succeed");
        assert_eq!(parsed, pl, "round trip must be lossless:\n{text}");
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
        let Error::HlsParse { reason, .. } = err;
        assert!(reason.contains("DURATION"), "{reason}");
    }

    #[test]
    fn parse_rejects_variant_uri_with_no_preceding_stream_inf() {
        let text = "#EXTM3U\n#EXT-X-VERSION:6\nv300/index.m3u8\n";
        let err = MasterPlaylist::parse(text).expect_err("orphan variant URI must error");
        assert!(matches!(err, Error::HlsParse { .. }));
    }

    #[test]
    fn parse_master_playlist_preserves_ext_x_media_into_extra_tags() {
        // #EXT-X-MEDIA is not modeled with typed fields, but must not cause
        // a parse error, and must be preserved verbatim (not dropped) since
        // `MasterPlaylist` now has its own `extra_tags`.
        let text = "#EXTM3U\n\
#EXT-X-VERSION:7\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"English\",DEFAULT=YES,URI=\"eng.m3u8\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=300000,CODECS=\"avc1.64001e,mp4a.40.2\"\n\
v300/index.m3u8\n";
        let pl = MasterPlaylist::parse(text).expect("EXT-X-MEDIA must be ignored, not error");
        assert_eq!(pl.variants.len(), 1);
        assert_eq!(pl.variants[0].uri, "v300/index.m3u8");
        assert!(
            pl.extra_tags.iter().any(|t| t.starts_with("#EXT-X-MEDIA:")),
            "unrecognized tag must be preserved verbatim, not dropped: {:?}",
            pl.extra_tags
        );
    }

    // -----------------------------------------------------------------------
    // Protocol version derivation (RFC 8216bis §8, issue #871) — table-driven
    // per docs/version-compatibility.md, plus the five named regression
    // cases from that issue.
    // -----------------------------------------------------------------------

    /// A minimal, otherwise-untriggering Media Playlist: integer-second
    /// duration, no map/byte-range/iframes-only/skip/extra_tags. Every
    /// table-driven test starts here and flips exactly one trigger.
    fn base_media_playlist() -> MediaPlaylist {
        MediaPlaylist {
            version: 0,
            target_duration: 6,
            media_sequence: 0,
            segments: vec![seg("s0.m4s", 6.0)],
            endlist: true,
            ..Default::default()
        }
    }

    /// Extract the `#EXT-X-VERSION:<n>` value from rendered text, or `None`
    /// if no such line is present.
    fn rendered_version(m3u8: &str) -> Option<u8> {
        m3u8.lines()
            .find_map(|l| l.strip_prefix("#EXT-X-VERSION:"))
            .map(|v| v.parse::<u8>().expect("version must be a valid u8"))
    }

    #[test]
    fn version_row1_no_trigger_omits_the_tag() {
        let out = base_media_playlist().to_m3u8();
        assert_eq!(rendered_version(&out), None, "no trigger:\n{out}");
    }

    #[test]
    fn version_row2_key_iv_triggers_v2() {
        let mut pl = base_media_playlist();
        pl.extra_tags = vec![
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://k\",IV=0x00000000000000000000000000000001"
                .into(),
        ];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(2), "{out}");
    }

    #[test]
    fn version_row3_float_extinf_triggers_v3() {
        let mut pl = base_media_playlist();
        pl.segments = vec![seg("s0.m4s", 6.5)];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(3), "{out}");
    }

    #[test]
    fn version_row4_byterange_triggers_v4() {
        let mut pl = base_media_playlist();
        pl.segments[0].byte_range = Some(ByteRange {
            length: 1000,
            offset: Some(0),
        });
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(4), "{out}");
    }

    #[test]
    fn version_row4_iframes_only_triggers_v4() {
        let mut pl = base_media_playlist();
        pl.iframes_only = true;
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(4), "{out}");
    }

    #[test]
    fn version_row5_sample_aes_triggers_v5() {
        let mut pl = base_media_playlist();
        pl.extra_tags = vec!["#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://k\",KEYID=0x01".into()];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(5), "{out}");
    }

    #[test]
    fn version_row5_keyformat_triggers_v5() {
        let mut pl = base_media_playlist();
        pl.extra_tags = vec![
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://k\",KEYFORMAT=\"com.example\",\
             KEYFORMATVERSIONS=\"1\""
                .into(),
        ];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(5), "{out}");
    }

    #[test]
    fn version_row5_map_with_iframes_only_triggers_v5_not_v6() {
        let mut pl = base_media_playlist();
        pl.iframes_only = true;
        pl.segments[0].map = Some(MapTag {
            uri: "init.mp4".into(),
            byte_range: None,
        });
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(5), "{out}");
    }

    #[test]
    fn version_row6_map_without_iframes_only_triggers_v6() {
        let mut pl = base_media_playlist();
        pl.segments[0].map = Some(MapTag {
            uri: "init.mp4".into(),
            byte_range: None,
        });
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(6), "{out}");
    }

    #[test]
    fn version_row7_media_service_instream_id_triggers_v7_multivariant_only() {
        let mut pl = MasterPlaylist {
            version: 0,
            variants: vec![Variant {
                bandwidth: 300_000,
                codecs: "avc1.64001e".into(),
                resolution: None,
                uri: "v300/index.m3u8".into(),
            }],
            iframe_variants: vec![],
            extra_tags: vec![],
        };
        pl.extra_tags = vec![
            "#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID=\"cc\",NAME=\"CC1\",\
             INSTREAM-ID=\"SERVICE1\""
                .into(),
        ];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(7), "{out}");
    }

    #[test]
    fn version_row8_variable_substitution_triggers_v8() {
        let mut pl = base_media_playlist();
        pl.segments = vec![seg("seg-{$id}.m4s", 6.0)];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(8), "{out}");
    }

    #[test]
    fn version_row8_variable_substitution_triggers_v8_multivariant() {
        let pl = MasterPlaylist {
            version: 0,
            variants: vec![Variant {
                bandwidth: 300_000,
                codecs: "avc1.64001e".into(),
                resolution: None,
                uri: "{$base}/index.m3u8".into(),
            }],
            iframe_variants: vec![],
            extra_tags: vec![],
        };
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(8), "{out}");
    }

    #[test]
    fn version_row9_skip_triggers_v9() {
        let mut pl = base_media_playlist();
        pl.skip = Some(SkipInfo {
            skipped_segments: 5,
            recently_removed_daterange_ids: vec![],
        });
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(9), "{out}");
    }

    #[test]
    fn version_row10_skip_replacing_daterange_triggers_v10() {
        let mut pl = base_media_playlist();
        pl.skip = Some(SkipInfo {
            skipped_segments: 5,
            recently_removed_daterange_ids: vec!["ad-1".into()],
        });
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(10), "{out}");
    }

    #[test]
    fn version_row11_define_queryparam_triggers_v11() {
        let mut pl = base_media_playlist();
        pl.extra_tags = vec!["#EXT-X-DEFINE:QUERYPARAM=\"auth\"".into()];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(11), "{out}");
    }

    #[test]
    fn version_row11_define_queryparam_triggers_v11_multivariant() {
        let pl = MasterPlaylist {
            version: 0,
            variants: vec![Variant {
                bandwidth: 300_000,
                codecs: "avc1.64001e".into(),
                resolution: None,
                uri: "v300/index.m3u8".into(),
            }],
            iframe_variants: vec![],
            extra_tags: vec!["#EXT-X-DEFINE:QUERYPARAM=\"auth\"".into()],
        };
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(11), "{out}");
    }

    #[test]
    fn version_row12_req_attribute_triggers_v12() {
        let mut pl = base_media_playlist();
        pl.extra_tags = vec!["#EXT-X-FUTURE-FEATURE:REQ-CODEC=\"av01\"".into()];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(12), "{out}");
    }

    #[test]
    fn version_row12_req_attribute_triggers_v12_multivariant() {
        let pl = MasterPlaylist {
            version: 0,
            variants: vec![Variant {
                bandwidth: 300_000,
                codecs: "avc1.64001e".into(),
                resolution: None,
                uri: "v300/index.m3u8".into(),
            }],
            iframe_variants: vec![],
            extra_tags: vec!["#EXT-X-FUTURE-FEATURE:REQ-CODEC=\"av01\"".into()],
        };
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(12), "{out}");
    }

    #[test]
    fn version_row13_media_instream_id_non_cc_triggers_v13_multivariant_only() {
        let pl = MasterPlaylist {
            version: 0,
            variants: vec![Variant {
                bandwidth: 300_000,
                codecs: "avc1.64001e".into(),
                resolution: None,
                uri: "v300/index.m3u8".into(),
            }],
            iframe_variants: vec![],
            extra_tags: vec![
                "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Eng\",INSTREAM-ID=\"CC1\"".into(),
            ],
        };
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(13), "{out}");
    }

    #[test]
    fn version_multivariant_no_trigger_omits_the_tag() {
        let pl = MasterPlaylist {
            version: 0,
            variants: vec![Variant {
                bandwidth: 300_000,
                codecs: "avc1.64001e".into(),
                resolution: None,
                uri: "v300/index.m3u8".into(),
            }],
            iframe_variants: vec![],
            extra_tags: vec![],
        };
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), None, "{out}");
    }

    // --- The five named regression cases from issue #871 ---

    /// Case 1: the specific regression this issue exists for. Shaped like
    /// `hls-runtime`'s actual LL-HLS media playlist (fMP4 segments, an
    /// `EXT-X-MAP` conveyed via `extra_tags` exactly as that origin emits
    /// it, `low_latency` config, no `EXT-X-I-FRAMES-ONLY`, no `EXT-X-SKIP`)
    /// — must render **6**, never the old hardcoded **9**.
    #[test]
    fn named_case_1_fmp4_low_latency_renders_6_not_9() {
        let pl = MediaPlaylist {
            version: 0, // hls-runtime no longer supplies an explicit floor.
            target_duration: 4,
            media_sequence: 100,
            segments: vec![MediaSegment {
                uri: "seg-1-100.m4s".into(),
                duration: 4.0,
                ..Default::default()
            }],
            open_segment: Some(OpenSegment::new(vec![PartSpec {
                uri: "part-1-101.0.m4s".into(),
                duration: 0.5,
                independent: true,
                ..Default::default()
            }])),
            extra_tags: vec!["#EXT-X-MAP:URI=\"init-1.mp4\"".into()],
            low_latency: Some(LowLatencyConfig {
                part_target: 0.5,
                part_hold_back: 1.5,
                ..Default::default()
            }),
            iframes_only: false,
            ..Default::default()
        };
        let out = pl.to_m3u8();
        assert_eq!(
            rendered_version(&out),
            Some(6),
            "fMP4 + low-latency must render 6, not the old hardcoded 9:\n{out}"
        );
        assert!(!out.contains("#EXT-X-VERSION:9"), "{out}");
    }

    /// Case 2: a classic MPEG-TS playlist (no fMP4/LL features at all) whose
    /// segment durations are genuine floating-point values renders **3**.
    #[test]
    fn named_case_2_classic_mpegts_float_extinf_renders_3() {
        let pl = MediaPlaylist {
            version: 0,
            target_duration: 10,
            media_sequence: 0,
            segments: vec![
                seg("seg0.ts", 9.009),
                seg("seg1.ts", 9.009),
                seg("seg2.ts", 3.003),
            ],
            endlist: true,
            ..Default::default()
        };
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(3), "{out}");
    }

    /// Case 3: adding `EXT-X-SKIP` to an otherwise-untriggering playlist
    /// raises the rendered version to **9** automatically.
    #[test]
    fn named_case_3_adding_skip_raises_version_to_9() {
        let mut pl = base_media_playlist();
        assert_eq!(
            rendered_version(&pl.to_m3u8()),
            None,
            "sanity: no trigger before adding EXT-X-SKIP"
        );
        pl.skip = Some(SkipInfo {
            skipped_segments: 3,
            recently_removed_daterange_ids: vec![],
        });
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(9), "{out}");
    }

    /// Case 4: a playlist triggering nothing renders NO `EXT-X-VERSION` tag
    /// at all (RFC 8216bis §8's opening rule).
    #[test]
    fn named_case_4_no_trigger_renders_no_version_tag() {
        let out = base_media_playlist().to_m3u8();
        assert!(!out.contains("#EXT-X-VERSION"), "{out}");
    }

    /// Case 5: `SAMPLE-AES` (the CBCS CENC key tag this crate's own
    /// `cenc_ext_x_key` produces) renders **5**.
    #[test]
    fn named_case_5_sample_aes_renders_5() {
        let tag = cenc_ext_x_key(CencScheme::Cbcs, &[0xab; 16], "https://k.example/key")
            .expect("cbcs must emit an EXT-X-KEY tag");
        let mut pl = base_media_playlist();
        pl.extra_tags = vec![tag];
        let out = pl.to_m3u8();
        assert_eq!(rendered_version(&out), Some(5), "{out}");
    }
}
