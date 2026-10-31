//! [`HlsOrigin`] — the LL-HLS origin [`ServedEgress`] (plan step 4):
//! blocking-reload/part-availability *decision* logic and playlist rendering,
//! rendered directly from a shared [`Trunk`] instead of the deleted
//! `MediaStore` push-fed rolling window.
//!
//! Master/media playlist tags are RFC 8216 §4.3.4 (`#EXT-X-STREAM-INF`) and
//! §4.3.3 (`#EXTM3U`/`#EXT-X-VERSION`, rendered by [`MediaPlaylist::to_m3u8`]);
//! the blocking reload query parameters (`_HLS_msn`/`_HLS_part`) are the
//! Blocking Playlist Reload mechanism of RFC 8216bis §6.2.5.2 — the client
//! asks the origin to hold the response open until the requested Media
//! Sequence Number/part is available, bounded by the caller's own
//! [`AwaitPolicy`] so the origin never hangs indefinitely.
//!
//! # What comes straight from the `Trunk`, with no cache at all
//!
//! Every part-availability and blocking-reload decision reads the `Trunk`
//! directly, `&self`-shaped, every call:
//!
//! - **Live parts of the open segment** — [`Trunk::part_bytes`]/
//!   [`Trunk::parts_in_segment`] (step 3b-iv's live-part log). This is the
//!   whole reason step 3b-iv exists: before it, nothing in `Trunk` could
//!   answer "does part 3 of the segment currently being written exist",
//!   which is exactly what forced `MediaStore` to keep its own
//!   `live_parts`/`recent_parts` buffers in the first place.
//! - **Whether a segment has closed** — [`Trunk::last_closed_segment`].
//! - **The "in-progress-or-last-active segment" `MediaStore::latest_progress`
//!   used to track as a push-fed field** — [`HlsOrigin::live_edge`] derives
//!   it from the two queries above alone (`last_closed_segment() + 1`, probed
//!   via `parts_in_segment`), needing no field of its own. See that method's
//!   doc for the derivation and why it is exact, not a heuristic.
//! - **A just-closed segment's final part still resolving** — falls out of
//!   [`Trunk::part_bytes`] for free: [`media_plane::trunk::SegmentWriter::publish_segment`]
//!   deliberately never touches the live-part log (see `trunk`'s own module
//!   doc, "The live-part log"), so this crate no longer needs `MediaStore`'s
//!   separate `recent_parts` buffer at all — that buffer existed *only* to
//!   simulate exactly the guarantee the `Trunk` now gives natively.
//!
//! # The one thing that genuinely cannot come from the `Trunk` alone
//!
//! [`Trunk::subscribe_segments`] hands back a moving, single-consumer
//! [`SegmentCursor`] — there is no snapshot query over the segment log the
//! way [`Trunk::events_between`] gives the event log (see
//! `media_plane::egress`'s own module doc, "`ServedEgress::resolve` does not
//! take `&Trunk`", which anticipated exactly this). Rendering a Media
//! Playlist needs the **window** of currently-advertised closed segments
//! (their bytes, durations, and discontinuity bits), plus two numbers that
//! must survive eviction from that window: the lifetime-max segment
//! duration (RFC 8216bis §4.4.3.1's `TARGETDURATION` MUST) and the
//! cumulative discontinuity count that has rolled off the front
//! (`#EXT-X-DISCONTINUITY-SEQUENCE`, RFC 8216 §4.3.3.3). None of that is
//! answerable by a fresh `&self` call on `Trunk` — it has to be assembled by
//! draining a cursor over time.
//!
//! `Window` is that assembly, and it is **not** a second `MediaStore`: it
//! holds only bytes/duration/discontinuity-bit for the segments currently in
//! the advertised window, fed by exactly **one** [`SegmentCursor`] this
//! `HlsOrigin` owns — precisely the shape `media_plane::egress`'s module
//! doc prescribes ("a `ServedEgress` implementation... keeps its own
//! resolvable window in sync by draining [cursors]... `resolve` only ever
//! reads that already-synced state"). It carries none of `MediaStore`'s
//! other fields (`health`, `track_specs`, `created_at`, `window_segments()`
//! diagnostics) — those served `multimux`'s DASH/ll-DASH outputs, not
//! LL-HLS rendering, and are out of this step's scope (Step 5's problem, if
//! still needed once `multimux` is rewritten).
//!
//! The fMP4 **init segment** bytes are the other thing this module holds
//! outside the `Trunk`: an init segment is neither a sample, a finished
//! segment, an event, nor a live part — it is produced once by the
//! segmenter and never changes, so it was never in scope for any of
//! `Trunk`'s four rings. [`HlsOrigin::set_init`] is the (small, honest) side
//! channel for it — not a duplicate of anything `Trunk` holds.

use super::ids::Generation;
use media_plane::ids::{PartIndex, SegmentSeq};
use std::collections::{BTreeMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use transmux::TrackId;

use broadcast_common::{Parse, Timestamp, Unpackage};
use broadcast_hls::{
    ByteRange, ClosedCaptionsRef, DecimalSeconds, FrameRate, LowLatencyConfig, MapTag,
    MasterPlaylist, MediaPlaylist, MediaRendition, MediaSegment, MediaType, OpenSegment, PartSpec,
    Variant, VideoRange,
};
use bytes::Bytes;
use media_plane::egress::{AwaitPolicy, CachePolicy, EgressResponse, ServedEgress};
use media_plane::trunk::{PartEntry, SegmentCursor, SegmentCursorItem, SegmentEntry, Trunk};

/// [`HlsOrigin`]'s `drained_through` before any drain has run: no real head
/// (`u32::MAX + 1` at most) equals it, so the first drain always happens.
const UNDRAINED: u64 = u64::MAX;

/// Process-wide high-water mark of [`next_instance_token`].
static LAST_INSTANCE: AtomicU64 = AtomicU64::new(0);

/// A token that differs for every [`HlsOrigin`] ever built, in this process
/// **and across process restarts**: the wall clock in milliseconds, forced
/// strictly above the previous token handed out (two origins built in the
/// same millisecond, or after the clock stepped back, still differ). It is
/// part of every resource name served `immutable` (see
/// [`HlsOrigin::instance`]).
fn next_instance_token() -> u64 {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    let mut previous = LAST_INSTANCE.load(Ordering::Relaxed);
    loop {
        let token = now_ms.max(previous.saturating_add(1));
        match LAST_INSTANCE.compare_exchange_weak(
            previous,
            token,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => return token,
            Err(actual) => previous = actual,
        }
    }
}

/// Lock `m`, recovering the guard from a poisoned mutex.
///
/// Every critical section in this module leaves its data structurally
/// consistent at each step (plain pushes/pops/assignments), so a panic in
/// some other thread that held the lock must not turn into a panic in every
/// later request (audit r09-O4 / T6, issues #1089/#1134).
fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Track id for the single rendition served per stream (no multi-track/
/// multi-rendition support yet).
pub const DEFAULT_TRACK_ID: TrackId = TrackId::new(1);

/// Which container [`HlsOrigin`] serves segments/parts as — orthogonal to
/// whether LL-HLS is enabled ([`HlsOriginBuilder::low_latency`]); issue #873.
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
    fn segment_extension(self) -> &'static str {
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

/// How many superseded init segments stay resolvable by their versioned name
/// (`init-{track}-{instance}-{generation}.mp4`): segments and playlists that name an
/// older generation are still cached by players and CDNs for a while after a
/// mid-stream codec change.
const INIT_HISTORY: usize = 8;

/// Nanoseconds per second, for the integer bitrate arithmetic.
const NANOS_PER_SEC: u128 = 1_000_000_000;

/// RFC 8216bis §6.2.2: the server MUST NOT remove a Media Segment from a live
/// Playlist if that would leave a Playlist shorter than three times the
/// Target Duration. A segment is cut at (about) the target duration, so a
/// window of fewer than three segments cannot satisfy that; the advertised
/// window is never shorter than this.
const MIN_WINDOW_SEGMENTS: usize = 3;

/// RFC 8216bis §6.2.2: `EXT-X-PART` tags SHOULD be removed once they are
/// greater than three Target Durations from the end of the Playlist (and the
/// part must stay downloadable for as long again).
const PART_RETENTION_TARGET_DURATIONS: f64 = 3.0;

/// RFC 8216bis §6.2.6 (SHOULD): "Not Found" for a resource the server cannot
/// find and that is not specified by an `EXT-X-PRELOAD-HINT`. The live-edge
/// (open) segment is the one after the last closed segment, and a hinted part
/// is in it or in the one right after it — so a part more than this many
/// segments past the last closed one is refused rather than held open.
const FUTURE_PART_SEGMENT_BOUND: u32 = 2;

/// RFC 8216bis §6.2.5.2: the Advance Part Limit is "three divided by the Part
/// Target Duration if the Part Target Duration is less than one second, or
/// three otherwise". Expressed against the part target in milliseconds.
const ADVANCE_PART_LIMIT_NUMERATOR: u64 = 3;
const MILLIS_PER_SEC: u64 = 1_000;

/// RFC 8216bis §6.2.5.2 (SHOULD): a `_HLS_msn` greater than "the Media
/// Sequence Number of the last Media Segment in the current Playlist plus
/// two" should be rejected rather than always blocking to the caller's
/// timeout — a legitimate client only ever asks for the segment/part right
/// after the one it already has, so anything more than two segments beyond
/// the current last closed segment is either a malfunctioning client or abuse.
const ABUSE_MSN_FUTURE_BOUND: u64 = 2;

/// RFC 8216bis / Apple LL-HLS §4.4.3.7: `#EXT-X-SERVER-CONTROL`'s
/// `PART-HOLD-BACK` attribute MUST be at least 3x the part target duration
/// (`#EXT-X-PART-INF`'s `PART-TARGET`).
const PART_HOLD_BACK_MULTIPLIER: f64 = 3.0;

/// RFC 8216bis §4.4.3.8: `HOLD-BACK` MUST be at least three times the Target
/// Duration.
const HOLD_BACK_MULTIPLIER: f64 = 3.0;

/// RFC 8216bis §4.4.3.8: the Skip Boundary is at least six times the Target
/// Duration (advertised as `CAN-SKIP-UNTIL`).
const SKIP_BOUNDARY_MULTIPLIER: f64 = 6.0;

/// A minimal single-variant master playlist pointing at `media_playlist_name`
/// (the caller's configured media-playlist filename — e.g. multimux's
/// `Config::playlist_name`, defaulting to `"media.m3u8"`) — the same
/// regardless of any stream state (no multi-rendition support yet), so this
/// takes no `Trunk`/origin argument.
pub fn master_playlist_m3u8(media_playlist_name: &str, bandwidth_bps: u32) -> String {
    format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH={bandwidth_bps}\n{media_playlist_name}\n")
}

/// Typed value of the `_HLS_skip` Delivery Directive (RFC 8216bis §6.2.5.1):
/// `YES` requests a delta update that skips older Media Segments, `v2` also
/// skips older `EXT-X-DATERANGE` tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipDirective {
    /// Skip older Media Segments only (`_HLS_skip=YES`).
    Yes,
    /// Skip older Media Segments and older `EXT-X-DATERANGE` tags
    /// (`_HLS_skip=v2`).
    V2,
}

impl SkipDirective {
    /// The spec token for this directive value.
    pub fn name(&self) -> &'static str {
        match self {
            SkipDirective::Yes => "YES",
            SkipDirective::V2 => "v2",
        }
    }
}

broadcast_common::impl_spec_display!(SkipDirective);

/// HTTP Priority signalling data for an LL-HLS response (RFC 8216bis §6.2.6
/// / §6.3): the extensible priority scheme (RFC 9218) urgency the adapter
/// should attach to the response. Playlists MUST have urgency 1; Media
/// Segments urgencies 2 through 6; Partial Segments share the segment's
/// urgency band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpPriority {
    /// RFC 9218 urgency value (lower is more urgent).
    pub urgency: u8,
    /// Whether the response may be delivered incrementally.
    pub incremental: bool,
}

impl HttpPriority {
    /// Urgency for a Media Playlist response.
    pub const PLAYLIST: Self = Self {
        urgency: 1,
        incremental: false,
    };
    /// Urgency for a Partial Segment or whole Media Segment response.
    pub const SEGMENT: Self = Self {
        urgency: 2,
        incremental: true,
    };
}

/// Blocking playlist reload query parameters (RFC 8216bis §6.2.5.2) — the
/// sans-IO counterpart of an adapter's own (likely serde-`Deserialize`)
/// query-string type; the adapter maps its wire query params into this.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BlockingQuery {
    /// The Media Sequence Number the client already has, plus one — the
    /// origin should not respond until a segment/part beyond this is ready.
    pub hls_msn: Option<u64>,
    /// The part index (within `hls_msn`) the client is waiting for.
    pub hls_part: Option<u32>,
    /// The `_HLS_skip` Delivery Directive (RFC 8216bis §6.2.5.1), if any.
    pub hls_skip: Option<SkipDirective>,
    /// The `_HLS_primary` / `_HLS_primary_id` value (RFC 8216bis Appendix D.4),
    /// carried opaquely; the origin does not alter playlist rendering for it.
    pub hls_primary: Option<String>,
}

/// [`ServedEgress::Request`] for [`HlsOrigin`]: which wire resource is
/// being asked for. A data-carrying dispatch ADT (matches this crate's
/// `client::action::Action`/`ResourceId` convention) — see
/// `tests/label_coverage.rs`'s SKIP list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HlsRequest {
    /// `GET <media playlist>`, optionally carrying a blocking-reload query.
    Playlist {
        /// The track id to render the playlist for (a naming parameter only —
        /// see [`DEFAULT_TRACK_ID`]).
        track_id: TrackId,
        /// The blocking-reload query parameters, if any.
        query: BlockingQuery,
    },
    /// `GET` a dynamic origin resource by its wire filename (`init-{track}.mp4`,
    /// `seg-{track}-{seq}.m4s`, `part-{track}-{seq}.{idx}.m4s`).
    Resource {
        /// The requested filename, exactly as it appeared in the request path.
        name: String,
    },
    /// `GET <iframe playlist>` for `track_id`, optionally carrying a
    /// blocking-reload query.
    IFramePlaylist {
        /// The track id to render the I-frame playlist for.
        track_id: TrackId,
        /// The blocking-reload query parameters, if any.
        query: BlockingQuery,
        /// Filename of the primary (media) playlist this I-frame playlist's
        /// `#EXT-X-RENDITION-REPORT` points at (RFC 8216bis §4.4.5.4).
        media_playlist_name: String,
    },
}

/// [`ServedEgress::Body`] for [`HlsOrigin`]: the resolved body, typed by
/// which [`HlsRequest`] produced it. A data-carrying ADT — see
/// `tests/label_coverage.rs`'s SKIP list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HlsBody {
    /// A rendered Media Playlist (`#EXTM3U` text).
    Playlist(String),
    /// Resolved resource bytes (init/segment/part).
    Resource(Bytes),
}

/// One playlist-window-resident **closed** segment's identity/bytes —
/// `Window`'s per-entry shape. Deliberately narrower than the old
/// `MediaStore`'s `SegmentInfo`-derived window entries: this crate only ever
/// needs bytes + duration + the discontinuity bit to render a Media
/// Playlist, so that is all this holds.
#[derive(Clone)]
struct WindowSegment {
    sequence_number: SegmentSeq,
    /// The init-segment generation this segment was cut against (see
    /// [`InitStore`]).
    init_gen: Generation,
    bytes: Bytes,
    duration_secs: f64,
    discontinuous: bool,
    /// Byte length of the leading I-frame slice of this segment's first
    /// video sample, when the segment is usable in an I-frame playlist;
    /// `None` otherwise (audio-first, non-sync first sample, parse failure,
    /// or no video track).
    iframe_len: Option<u64>,
    /// This segment's `SegmentEntry::timeline_position`, in nanoseconds —
    /// carried through so [`HlsOrigin::closed_segments`] can hand it to a
    /// caller doing time-based windowing over a *different* source of
    /// segments (e.g. multimux's DVR archive, issue #900), without that
    /// caller needing a second cursor of its own just to learn it.
    start_ns: u64,
}

/// One closed segment's identity/metadata as tracked by this origin's
/// `Window` — [`HlsOrigin::closed_segments`]'s return shape. Deliberately
/// excludes the segment's bytes: a caller merging this with another
/// segment source (issue #900) fetches bytes through the normal
/// [`ServedEgress::resolve`] resource path when it needs them, not through
/// this snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ClosedSegment {
    /// This segment's sequence number (`_HLS_msn`-addressable).
    pub sequence_number: SegmentSeq,
    /// `SegmentEntry::timeline_position`, in nanoseconds — the `Trunk`'s
    /// absolute timeline, for time-based windowing across sources that
    /// share the same `Trunk` (e.g. multimux's DVR archive, whose own
    /// `IndexEntry::start_pts_ns` is exactly this same clock).
    pub start_ns: u64,
    /// Segment duration in seconds.
    pub duration_secs: f64,
    /// Whether `#EXT-X-DISCONTINUITY` precedes this segment (RFC 8216
    /// §4.3.4.3).
    pub discontinuous: bool,
    /// The init-segment generation this segment was cut against (see
    /// [`HlsOrigin::init_name`]).
    pub init_generation: Generation,
}

impl ClosedSegment {
    /// The same segment, cut against init generation `generation`.
    #[must_use]
    pub fn with_init_generation(mut self, generation: Generation) -> Self {
        self.init_generation = generation;
        self
    }

    /// Construct a [`ClosedSegment`] — needed because the type is
    /// `#[non_exhaustive]` (a struct-literal outside this crate does not
    /// typecheck), for a caller building test fixtures over the shape
    /// [`HlsOrigin::closed_segments`] returns (e.g. multimux's own
    /// catch-up merge tests, issue #900) without depending on this
    /// module's private `Window`.
    pub fn new(
        sequence_number: SegmentSeq,
        start_ns: u64,
        duration_secs: f64,
        discontinuous: bool,
    ) -> Self {
        ClosedSegment {
            sequence_number,
            start_ns,
            duration_secs,
            discontinuous,
            init_generation: Generation::new(1),
        }
    }
}

/// The small per-[`HlsOrigin`] synced window this module's own doc
/// ("The one thing that genuinely cannot come from the `Trunk` alone")
/// explains the need for — fed by draining exactly one [`SegmentCursor`],
/// never pushed into directly.
struct Window {
    segments: VecDeque<WindowSegment>,
    capacity: usize,
    /// Largest segment duration ever drained, surviving window eviction —
    /// RFC 8216bis §4.4.3.1's `TARGETDURATION` MUST holds for *every*
    /// segment this origin has ever advertised, not just the ones still in
    /// the window (mirrors the deleted `MediaStore::max_segment_duration`).
    max_segment_duration_secs: f64,
    /// Highest per-segment bitrate (bits/second, rounded up) ever drained,
    /// surviving eviction — what a master playlist's `BANDWIDTH` (RFC 8216
    /// §4.3.4.2: an upper bound of the peak segment bitrate) is built from.
    /// `0` until a segment with a measurable duration has been drained.
    peak_bandwidth_bps: u64,
    /// Cumulative bytes of every segment ever drained, surviving window
    /// eviction — the numerator of the master playlist's
    /// `AVERAGE-BANDWIDTH` (RFC 8216bis §4.4.6.2, Apple authoring item 9.14:
    /// the average segment bit rate over the whole stream, not just the
    /// window).
    total_bytes: u64,
    /// Cumulative duration of every segment ever drained whose duration was
    /// non-zero, in nanoseconds, surviving eviction — the denominator of
    /// `AVERAGE-BANDWIDTH`. `0` until such a segment has been drained.
    total_duration_nanos: u128,
    /// Cumulative peak per-I-frame bitrate (bits/second, rounded up) over
    /// every I-frame-playlist-usable segment ever drained, surviving window
    /// eviction — `BANDWIDTH` of an `#EXT-X-I-FRAME-STREAM-INF`.
    iframe_peak_bandwidth_bps: u64,
    /// Cumulative byte length of every I-frame slice ever drained, surviving
    /// window eviction — the numerator (in bytes, converted to bits) of an
    /// I-frame variant's `AVERAGE-BANDWIDTH`.
    iframe_total_bytes: u64,
    /// Cumulative duration of every I-frame-playlist-usable segment ever
    /// drained whose duration was non-zero, in nanoseconds, surviving
    /// window eviction — the denominator of an I-frame variant's
    /// `AVERAGE-BANDWIDTH`.
    iframe_total_duration_nanos: u128,
    /// Cumulative count of discontinuities that have rolled off the front of
    /// the window — RFC 8216 §4.3.3.3's `#EXT-X-DISCONTINUITY-SEQUENCE`.
    /// Incremented exactly once per **evicted** entry whose
    /// [`WindowSegment::discontinuous`] was `true`; a discontinuity still
    /// inside the window is rendered as a per-segment `#EXT-X-DISCONTINUITY`
    /// tag instead (see [`MediaPlaylist::to_m3u8`]), never double-counted
    /// here.
    discontinuity_sequence: u64,
    /// The cursor reported lost segments (`Lagged`/`Gap`) since the last
    /// push: the next segment does not follow the window's last one.
    gap_pending: bool,
}

impl Window {
    fn new(capacity: NonZeroUsize) -> Self {
        Window {
            segments: VecDeque::new(),
            capacity: capacity.get().max(MIN_WINDOW_SEGMENTS),
            max_segment_duration_secs: 0.0,
            peak_bandwidth_bps: 0,
            total_bytes: 0,
            total_duration_nanos: 0,
            iframe_peak_bandwidth_bps: 0,
            iframe_total_bytes: 0,
            iframe_total_duration_nanos: 0,
            discontinuity_sequence: 0,
            gap_pending: false,
        }
    }

    /// Record that the cursor lost segments: whatever is pushed next does not
    /// follow the current window contents.
    fn note_gap(&mut self) {
        self.gap_pending = true;
    }

    /// Drop every window entry (they roll off the front, so their
    /// discontinuities count towards the discontinuity sequence).
    fn clear_rolling_off(&mut self) {
        let discontinuities = self.segments.iter().filter(|s| s.discontinuous).count();
        self.discontinuity_sequence = self
            .discontinuity_sequence
            .saturating_add(u64::try_from(discontinuities).unwrap_or(u64::MAX));
        self.segments.clear();
    }

    /// Absorb one drained [`SegmentEntry`], evicting the oldest window entry
    /// first if already at `capacity` — same evict-then-push shape as every
    /// ring in `trunk.rs` itself.
    ///
    /// A playlist numbers its segments implicitly (`EXT-X-MEDIA-SEQUENCE` +
    /// index, RFC 8216 §4.3.3.2), so the window must hold consecutive
    /// sequence numbers. An entry that does not directly follow the last one
    /// (numbers skipped, a repeat or a decrease), or that follows reported
    /// lost segments, starts a new run: the old entries are cleared and the
    /// entry is marked discontinuous (RFC 8216 §4.3.4.3) rather than being
    /// appended under a wrong implied number.
    fn push(&mut self, entry: SegmentEntry, init_gen: Generation, iframe_len: Option<u64>) {
        let follows = self
            .segments
            .back()
            .is_none_or(|last| last.sequence_number.checked_next() == Some(entry.sequence_number));
        let mut discontinuous = entry.meta.discontinuous;
        if !follows || self.gap_pending {
            self.clear_rolling_off();
            discontinuous = true;
        }
        self.gap_pending = false;
        // RFC 8216bis §4.4.4.5: a different Media Initialization Section
        // starts a new timeline segment — `EXT-X-DISCONTINUITY` precedes the
        // first segment that uses it (audit r09-C2, issue #1030).
        if self
            .segments
            .back()
            .is_some_and(|last| last.init_gen != init_gen)
        {
            discontinuous = true;
        }

        let duration_secs = entry.duration.as_secs_f64();
        self.max_segment_duration_secs = self.max_segment_duration_secs.max(duration_secs);
        if let Some(bps) = segment_bandwidth_bps(entry.bytes.len(), entry.duration) {
            self.peak_bandwidth_bps = self.peak_bandwidth_bps.max(bps);
        }
        // Totals are cumulative over every drained segment and are never
        // reset on eviction (or on a discontinuity): `AVERAGE-BANDWIDTH`
        // describes the whole stream. A zero-duration segment has no
        // measurable bit rate, so it contributes to neither total.
        if !entry.duration.is_zero() {
            self.total_bytes = self
                .total_bytes
                .saturating_add(u64::try_from(entry.bytes.len()).unwrap_or(u64::MAX));
            self.total_duration_nanos = self
                .total_duration_nanos
                .saturating_add(entry.duration.as_nanos());
        }
        // I-frame accounting: per usable entry, bits = iframe_len * 8 over the
        // segment's EXTINF duration. Cumulative over every drained segment,
        // surviving eviction (mirrors the media totals above).
        if let Some(len) = iframe_len {
            if let Some(bps) =
                segment_bandwidth_bps(usize::try_from(len).unwrap_or(usize::MAX), entry.duration)
            {
                self.iframe_peak_bandwidth_bps = self.iframe_peak_bandwidth_bps.max(bps);
            }
            if !entry.duration.is_zero() {
                self.iframe_total_bytes = self.iframe_total_bytes.saturating_add(len);
                self.iframe_total_duration_nanos = self
                    .iframe_total_duration_nanos
                    .saturating_add(entry.duration.as_nanos());
            }
        }
        if self.segments.len() >= self.capacity
            && let Some(evicted) = self.segments.pop_front()
            && evicted.discontinuous
        {
            self.discontinuity_sequence = self.discontinuity_sequence.saturating_add(1);
        }
        self.segments.push_back(WindowSegment {
            sequence_number: entry.sequence_number,
            init_gen,
            bytes: entry.bytes,
            duration_secs,
            discontinuous,
            start_ns: entry.timeline_position.as_nanos(),
            iframe_len,
        });
    }

    /// The bytes of segment `sequence_number`. The window holds consecutive
    /// sequence numbers (see [`Self::push`]), so the number identifies at
    /// most one entry; it is addressed by offset from the front and the
    /// entry's own number is re-checked.
    fn bytes_of(&self, sequence_number: SegmentSeq) -> Option<Bytes> {
        let front = self.segments.front()?.sequence_number;
        let index = usize::try_from(sequence_number.checked_distance_from(front)?).ok()?;
        let segment = self.segments.get(index)?;
        (segment.sequence_number == sequence_number).then(|| segment.bytes.clone())
    }
}

/// A copy of the window state `render_playlist` reads (see
/// [`HlsOrigin::window_view`]).
struct WindowView {
    segments: VecDeque<WindowSegment>,
    max_segment_duration_secs: f64,
    discontinuity_sequence: u64,
}

/// The init segments this origin has served, by generation.
///
/// A resource name must always map to the same bytes for `Immutable` to be
/// true (audit r09-C2, issue #1030), so a changed init segment gets a new
/// generation and a new versioned name,
/// `init-{track}-{instance}-{generation}.mp4`; the
/// bare `init-{track}.mp4` always means "the current one" and is served
/// [`CachePolicy::NoCache`].
#[derive(Default)]
struct InitStore {
    /// Generation of `current` (`0` = no init set yet); starts at `1`.
    generation: Generation,
    current: Option<Bytes>,
    /// The last [`INIT_HISTORY`] generations' bytes, oldest first.
    history: VecDeque<(Generation, Bytes)>,
    /// `(first origin sequence number cut against it, generation)`, oldest
    /// first: segment `seq` belongs to the last entry whose first number is
    /// `<= seq`.
    starts: VecDeque<(SegmentSeq, Generation)>,
}

impl InitStore {
    /// Record `bytes`; a new generation only if they differ from the current
    /// init. `next_seq` is the first segment not yet closed.
    fn set(&mut self, bytes: Bytes, next_seq: SegmentSeq) {
        if self.current.as_ref() == Some(&bytes) {
            return;
        }
        self.generation = Generation::new(self.generation.value().saturating_add(1));
        // The very first init covers every segment ever seen.
        let first_seq = if self.current.is_none() {
            SegmentSeq::new(0)
        } else {
            next_seq
        };
        self.current = Some(bytes.clone());
        self.history.push_back((self.generation, bytes));
        self.starts.push_back((first_seq, self.generation));
        while self.history.len() > INIT_HISTORY {
            self.history.pop_front();
        }
        while self.starts.len() > INIT_HISTORY {
            self.starts.pop_front();
        }
    }

    /// The generation segment `seq` was cut against (`1` before any init).
    fn generation_for(&self, seq: SegmentSeq) -> Generation {
        self.starts
            .iter()
            .rev()
            .find(|(first, _)| *first <= seq)
            .or_else(|| self.starts.front())
            .map_or(Generation::new(1), |&(_, generation)| generation)
    }

    fn bytes_of(&self, generation: Generation) -> Option<Bytes> {
        self.history
            .iter()
            .find(|(g, _)| *g == generation)
            .map(|(_, b)| b.clone())
    }
}

/// Whether a codec config is a video codec (mirrors
/// `transmux::CodecConfig::is_video`, which is crate-private).
fn is_video_codec(config: &transmux::CodecConfig) -> bool {
    matches!(
        config,
        transmux::CodecConfig::Avc { .. }
            | transmux::CodecConfig::Hevc { .. }
            | transmux::CodecConfig::Vvc { .. }
            | transmux::CodecConfig::Av1 { .. }
            | transmux::CodecConfig::Vp9 { .. }
            | transmux::CodecConfig::Vp8 { .. }
            | transmux::CodecConfig::Mpeg2Video { .. }
    )
}

fn dedupe_rendition_names(mut media: Vec<MediaRendition>) -> Vec<MediaRendition> {
    let mut seen: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    for m in &mut media {
        let base = m.name.clone();
        let mut n = 1u32;
        while !seen.insert((m.group_id.clone(), m.name.clone())) {
            n += 1;
            m.name = format!("{base} ({n})");
        }
    }
    media
}

fn channel_count_string(config: &transmux::CodecConfig) -> Option<String> {
    let n = match config {
        transmux::CodecConfig::Aac { channel_count, .. }
        | transmux::CodecConfig::Ac3 { channel_count, .. }
        | transmux::CodecConfig::Eac3 { channel_count, .. }
        | transmux::CodecConfig::Opus { channel_count, .. }
        | transmux::CodecConfig::Flac { channel_count, .. }
        | transmux::CodecConfig::Ac4 { channel_count, .. }
        | transmux::CodecConfig::MpegH { channel_count, .. }
        | transmux::CodecConfig::MpegAudio { channel_count, .. } => *channel_count,
        _ => return None,
    };
    Some(n.to_string())
}

/// Compute the byte length of the leading I-frame slice of `segment`'s first
/// video sample, or `None` when the segment cannot back an I-frame playlist
/// entry (audio-first layout, non-sync first sample, parse failure, or no
/// usable mdat/moof).
fn compute_iframe_len(segment: &[u8], video_track_id: TrackId) -> Option<u64> {
    // Locate the (single) top-level `moof` box. Its offset is measured from
    // the start of `segment`, matching `frag_offsets::sample_ranges_in`.
    let mut offset = 0usize;
    let mut moof_off: Option<usize> = None;
    let mut moof_total: usize = 0;
    let mut moof_hdr: usize = 0;
    while offset + transmux::box_types::BOX_HEADER_MIN_SIZE <= segment.len() {
        let rest = &segment[offset..];
        let header = transmux::box_types::BoxHeader::parse(rest).ok()?;
        let hdr_sz = header.header_size();
        if hdr_sz > rest.len() {
            break;
        }
        let total = if header.size == u64::from(transmux::box_types::SIZE_TO_EOF) {
            rest.len()
        } else {
            match usize::try_from(header.size) {
                Ok(t) if t >= hdr_sz => t,
                _ => break,
            }
        };
        if header.box_type.is(b"moof") {
            moof_off = Some(offset);
            moof_total = total.min(rest.len());
            moof_hdr = hdr_sz;
            break;
        }
        let step = total.min(rest.len());
        if step == 0 {
            break;
        }
        offset += step;
    }
    let moof_off = moof_off?;
    let body = &segment[moof_off + moof_hdr..moof_off + moof_total];
    let moof = transmux::movie_fragment::MovieFragmentBox::parse_body(body).ok()?;

    let mdats = transmux::frag_offsets::mdat_ranges(segment);
    let samples =
        transmux::frag_offsets::sample_ranges_in(segment, &mdats, moof_off, &moof, video_track_id)
            .ok()?;
    if samples.is_empty() {
        return None;
    }
    let first = samples[0];
    // A sample is a sync sample when bit 16 of its flags is clear.
    if first.flags & (1 << 16) != 0 {
        return None;
    }
    // The first video sample must start exactly at an mdat payload start: no
    // other track's bytes may precede it.
    let mdat_payload_start = mdat_payload_start_for(segment, first.start)?;
    if first.start != mdat_payload_start {
        return None;
    }
    u64::try_from(first.end).ok()
}

/// Byte offset of the `mdat` payload that contains `sample_start`, if any.
fn mdat_payload_start_for(segment: &[u8], sample_start: usize) -> Option<usize> {
    let mut offset = 0usize;
    while offset + transmux::box_types::BOX_HEADER_MIN_SIZE <= segment.len() {
        let rest = &segment[offset..];
        let header = transmux::box_types::BoxHeader::parse(rest).ok()?;
        let hdr_sz = header.header_size();
        if hdr_sz > rest.len() {
            break;
        }
        let total = if header.size == u64::from(transmux::box_types::SIZE_TO_EOF) {
            rest.len()
        } else {
            match usize::try_from(header.size) {
                Ok(t) if t >= hdr_sz => t,
                _ => break,
            }
        };
        if header.box_type.is(b"mdat") {
            let payload = offset + hdr_sz;
            let end = offset + total;
            if sample_start >= payload && sample_start < end {
                return Some(payload);
            }
        }
        let step = total.min(rest.len());
        if step == 0 {
            break;
        }
        offset += step;
    }
    None
}

/// The mean segment bitrate in bits/second over the whole stream — RFC
/// 8216bis §4.4.6.2 `AVERAGE-BANDWIDTH` (Apple authoring item 9.14) — rounded
/// to nearest, or `None` when no measurable segment has been drained
/// (`total_duration_nanos == 0`) or the figure does not fit
/// a `u32`.
///
/// Exact integer arithmetic: `bits * 1e9` is computed in `u128` (a `u64` byte
/// total times 8e9 cannot overflow it) before the single division.
fn average_bandwidth_bps(total_bytes: u64, total_duration_nanos: u128) -> Option<u32> {
    if total_duration_nanos == 0 {
        return None;
    }
    let bits_nanos = u128::from(total_bytes)
        .checked_mul(8)?
        .checked_mul(NANOS_PER_SEC)?;
    let mean = bits_nanos.checked_add(total_duration_nanos / 2)? / total_duration_nanos;
    u32::try_from(mean).ok()
}

/// A segment's bitrate in bits/second, rounded up (so a `BANDWIDTH` built
/// from it is never below the true value), or `None` for a zero duration or
/// an overflowing figure.
fn segment_bandwidth_bps(len: usize, duration: Duration) -> Option<u64> {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        return None;
    }
    let bits = u128::try_from(len).ok()?.checked_mul(8)?;
    let bps = bits.checked_mul(NANOS_PER_SEC)?.div_ceil(nanos);
    u64::try_from(bps).ok()
}

/// Parse `part-{track}-{instance}-{seq}.{idx}.{ext}` (instance-named, the form
/// the playlist uses) or the legacy `part-{track}-{seq}.{idx}.{ext}` into
/// `(instance, seq, idx)`; `instance` is `None` for the legacy form. `None` if it
/// isn't a part filename in `container`'s extension or a field does not parse.
fn parse_part(file: &str, container: Container) -> Option<(Option<u64>, u64, PartIndex)> {
    let suffix = format!(".{}", container.segment_extension());
    let rest = file.strip_prefix("part-")?.strip_suffix(suffix.as_str())?;
    let (names, idx) = rest.rsplit_once('.')?;
    let (epoch, seq) = parse_epoch_seq(names)?;
    Some((epoch, seq, PartIndex::new(idx.parse().ok()?)))
}

/// `{track}-{seq}` or `{track}-{instance}-{seq}` -> `(instance, seq)`; `{track}` is
/// validated but unused (an [`HlsOrigin`] holds a single track).
fn parse_epoch_seq(names: &str) -> Option<(Option<u64>, u64)> {
    let mut fields = names.split('-');
    fields.next()?.parse::<u32>().ok()?;
    let first = fields.next()?;
    match fields.next() {
        None => Some((None, first.parse().ok()?)),
        Some(seq) => {
            if fields.next().is_some() {
                return None;
            }
            Some((Some(first.parse().ok()?), seq.parse().ok()?))
        }
    }
}

/// Wire prefix for an I-frame (trick-play) segment resource (issue #1422).
///
/// An I-frame playlist references its segment bytes under this prefix rather
/// than the media playlist's `seg-` prefix, so the bytes behind an I-frame
/// URI are *always* the stored segment bytes: `multimux`'s DASH `emsg`
/// splice (and the in-progress chunked fallback) key off `seg-` only and
/// never touch `iframe-seg-` URIs, which keeps the advertised
/// `#EXT-X-BYTERANGE` length (measured on the stored bytes by
/// `compute_iframe_len`) from drifting away from the served bytes.
pub const IFRAME_SEGMENT_PREFIX: &str = "iframe-seg-";

/// What [`parse_immediate`] recognised.
enum ImmediateResource {
    /// `init-{track}.mp4`: the current init (changes, never `immutable`).
    CurrentInit,
    /// `init-{track}-{instance}-{generation}.mp4` (`epoch` is the instance
    /// token).
    VersionedInit { epoch: u64, generation: Generation },
    /// `seg-{track}-{seq}.{ext}` (`epoch: None`, legacy) or
    /// `seg-{track}-{instance}-{seq}.{ext}` (`epoch` is the instance token).
    Segment { epoch: Option<u64>, seq: u64 },
}

/// Parse an `init-…`/`seg-…` dynamic filename; `part-…` filenames are handled
/// separately by [`parse_part`] (they can block until available).
///
/// The init forms are only recognised under [`Container::Fmp4`] — a `MpegTs`
/// origin's segments are self-initialising (see [`Container`]'s own doc), so
/// `init-*.mp4` under `MpegTs` is `None` whether or not
/// [`HlsOrigin::set_init`] was ever called (issue #873's cross-container
/// refusal: advertised == servable).
fn parse_immediate(file: &str, container: Container) -> Option<ImmediateResource> {
    if container == Container::Fmp4
        && let Some(rest) = file.strip_prefix("init-")
    {
        let rest = rest.strip_suffix(".mp4")?;
        let mut fields = rest.split('-');
        fields.next()?.parse::<u32>().ok()?;
        return match (fields.next(), fields.next(), fields.next()) {
            (None, ..) => Some(ImmediateResource::CurrentInit),
            (Some(epoch), Some(generation), None) => Some(ImmediateResource::VersionedInit {
                epoch: epoch.parse().ok()?,
                generation: Generation::new(generation.parse().ok()?),
            }),
            _ => None,
        };
    }
    // `iframe-seg-…` resolves to the *same* stored segment bytes as
    // `seg-…` (issue #1422): the I-frame playlist advertises a byte range
    // measured on the stored bytes, so its URI must never be transformed by
    // `multimux`'s emsg splice. Recognising the prefix here makes the
    // origin serve the unmodified bytes for it.
    let seg_rest = file
        .strip_prefix("seg-")
        .or_else(|| file.strip_prefix(IFRAME_SEGMENT_PREFIX));
    if let Some(rest) = seg_rest {
        let suffix = format!(".{}", container.segment_extension());
        let (epoch, seq) = parse_epoch_seq(rest.strip_suffix(suffix.as_str())?)?;
        return Some(ImmediateResource::Segment { epoch, seq });
    }
    None
}

/// Error returned by [`HlsOriginBuilder::build`] when a required field was
/// never set — never a silently-defaulted value (issue #873).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HlsOriginBuildError {
    /// [`HlsOriginBuilder::target_duration_secs`] was never called.
    #[error("HlsOrigin::builder(...).target_duration_secs(...) is required but was never called")]
    MissingTargetDurationSecs,
    /// [`HlsOriginBuilder::window_segments`] was never called.
    #[error("HlsOrigin::builder(...).window_segments(...) is required but was never called")]
    MissingWindowSegments,
    /// [`HlsOriginBuilder::target_duration_secs`] was NaN, infinite, zero or
    /// negative: `EXT-X-TARGETDURATION` must be a positive integer (RFC
    /// 8216bis §4.4.3.1).
    #[error("HlsOrigin::builder(...).target_duration_secs(...) must be finite and positive")]
    InvalidTargetDuration,
    /// [`HlsOriginBuilder::low_latency`] was given a part target of `0` ms:
    /// `#EXT-X-PART-INF`'s `PART-TARGET` must be a positive duration (RFC
    /// 8216bis §4.4.3.7), and the Advance Part Limit divides by it.
    #[error("HlsOrigin::builder(...).low_latency(0): the part target must be positive")]
    ZeroPartTarget,
    /// [`HlsOriginBuilder::media_sequence_offset`] is so large that
    /// `offset + u32::MAX` (the largest origin sequence number) would not fit
    /// a `u64` Media Sequence Number.
    #[error(
        "HlsOrigin::builder(...).media_sequence_offset(...) leaves no room for u32 sequence numbers"
    )]
    MediaSequenceOffsetTooLarge,
    /// [`HlsOriginBuilder::declared_bandwidth_bps`] was given `0` or a value
    /// greater than `u32::MAX`.
    #[error("HlsOrigin::builder(...).declared_bandwidth_bps(...) must be > 0 and <= u32::MAX")]
    InvalidDeclaredBandwidth,
    /// An audio rendition's `group_id` is empty.
    #[error("HlsOrigin::builder(...).audio_renditions(...): group_id must be non-empty")]
    EmptyAudioRenditionGroupId,
    /// An audio rendition's `name` is empty.
    #[error("HlsOrigin::builder(...).audio_renditions(...): name must be non-empty")]
    EmptyAudioRenditionName,
    /// Two audio renditions in one group share a `NAME`.
    #[error("HlsOrigin::builder(...).audio_renditions(...): duplicate NAME in a group")]
    DuplicateAudioRenditionName,
    /// An audio rendition group does not have exactly one `DEFAULT=YES` member.
    #[error(
        "HlsOrigin::builder(...).audio_renditions(...): a group must have exactly one DEFAULT=YES member"
    )]
    AudioRenditionDefaultCount,
    /// A `DEFAULT=YES` audio rendition is not `AUTOSELECT=YES` (RFC 8216bis §4.4.6.1).
    #[error("HlsOrigin::builder(...).audio_renditions(...): DEFAULT=YES requires AUTOSELECT=YES")]
    AudioRenditionDefaultNotAutoselect,
    /// An audio rendition's `language` is empty.
    #[error("HlsOrigin::builder(...).audio_renditions(...): language must be non-empty")]
    EmptyAudioRenditionLanguage,
    /// An audio rendition's `uri` is empty.
    #[error("HlsOrigin::builder(...).audio_renditions(...): uri must be non-empty")]
    EmptyAudioRenditionUri,
    /// Audio rendition groups do not all have the same members (RFC 8216bis §4.4.6.1).
    #[error(
        "HlsOrigin::builder(...).audio_renditions(...): every group must have the same members, differing only in URI and CHANNELS"
    )]
    AudioRenditionGroupsDiffer,
    /// A subtitle rendition's `name` is empty.
    #[error("HlsOrigin::builder(...).subtitle_renditions(...): name must be non-empty")]
    EmptySubtitleRenditionName,
    /// A subtitle rendition's `language` is empty (Apple HLS Authoring Spec 4.7).
    #[error("HlsOrigin::builder(...).subtitle_renditions(...): language must be non-empty")]
    EmptySubtitleRenditionLanguage,
    /// A subtitle rendition's `uri` is empty.
    #[error("HlsOrigin::builder(...).subtitle_renditions(...): uri must be non-empty")]
    EmptySubtitleRenditionUri,
    /// Two subtitle renditions share a `NAME`.
    #[error("HlsOrigin::builder(...).subtitle_renditions(...): duplicate NAME")]
    DuplicateSubtitleRenditionName,
    /// Two subtitle renditions share a `URI`.
    #[error("HlsOrigin::builder(...).subtitle_renditions(...): duplicate URI")]
    DuplicateSubtitleRenditionUri,
    /// A `DEFAULT=YES` subtitle rendition is not `AUTOSELECT=YES` (RFC 8216 §4.3.4.1.1).
    #[error(
        "HlsOrigin::builder(...).subtitle_renditions(...): DEFAULT=YES requires AUTOSELECT=YES"
    )]
    SubtitleRenditionDefaultNotAutoselect,
    /// More than one subtitle rendition is `DEFAULT=YES` (RFC 8216 §4.3.4.1.1).
    #[error("HlsOrigin::builder(...).subtitle_renditions(...): at most one DEFAULT=YES member")]
    MultipleDefaultSubtitleRenditions,
    /// `subtitle_sibling_origins` is non-empty but not one per declared rendition.
    #[error(
        "HlsOrigin::builder(...).subtitle_sibling_origins(...): one origin per declared subtitle rendition"
    )]
    SubtitleSiblingCountMismatch,
    /// A subtitle sibling origin is not a [`Container::WebVtt`] origin.
    #[error(
        "HlsOrigin::builder(...).subtitle_sibling_origins(...): sibling must be a WebVtt origin"
    )]
    SubtitleSiblingNotWebVtt,
    /// A subtitle sibling's `EXT-X-TARGETDURATION` differs from this origin's
    /// (Apple HLS Authoring Spec 5.6).
    #[error(
        "HlsOrigin::builder(...).subtitle_sibling_origins(...): sibling target duration must equal this origin's"
    )]
    SubtitleSiblingTargetDurationMismatch,
    /// A closed-caption override's `instream_id` is not `CC1`..`CC4` or
    /// `SERVICE1`..`SERVICE63`.
    #[error("HlsOrigin::builder(...).closed_caption_overrides(...): invalid INSTREAM-ID")]
    InvalidClosedCaptionInstreamId,
    /// Two closed-caption overrides name the same `INSTREAM-ID`.
    #[error("HlsOrigin::builder(...).closed_caption_overrides(...): duplicate INSTREAM-ID")]
    DuplicateClosedCaptionInstreamId,
}

/// Error returned by [`HlsOrigin::master_playlist`].
#[derive(Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum HlsMasterError {
    /// No segment with a measurable duration has been published yet and no
    /// declared bandwidth was configured, so `BANDWIDTH` cannot be
    /// determined.
    #[error("BANDWIDTH unknown: no measured segment and no declared bandwidth")]
    BandwidthUnknown,
    /// The measured peak segment bitrate does not fit the `u32` `BANDWIDTH`
    /// of a master playlist variant.
    #[error("measured BANDWIDTH {bandwidth} exceeds u32")]
    BandwidthOverflow {
        /// The measured peak segment bitrate, bits per second.
        bandwidth: u64,
    },
    /// The master playlist could not be rendered (a field carries a
    /// character forbidden in an HLS quoted string).
    #[error("master playlist render failed: {0}")]
    Render(broadcast_hls::Error),
    /// An audio rendition's `uri` equals the media playlist name.
    #[error("audio rendition uri must not equal the media playlist name")]
    AudioRenditionUriEqualsMediaPlaylist,
}

/// Validate a list of audio rendition declarations (RFC 8216bis §4.4.6.1).
pub fn validate_audio_renditions(decls: &[AudioRenditionDecl]) -> Result<(), HlsOriginBuildError> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut groups: BTreeMap<&str, Vec<&AudioRenditionDecl>> = BTreeMap::new();
    for decl in decls {
        if decl.group_id.is_empty() {
            return Err(HlsOriginBuildError::EmptyAudioRenditionGroupId);
        }
        if decl.name.is_empty() {
            return Err(HlsOriginBuildError::EmptyAudioRenditionName);
        }
        if decl.language.is_empty() {
            return Err(HlsOriginBuildError::EmptyAudioRenditionLanguage);
        }
        if decl.uri.is_empty() {
            return Err(HlsOriginBuildError::EmptyAudioRenditionUri);
        }
        if decl.default && !decl.autoselect {
            return Err(HlsOriginBuildError::AudioRenditionDefaultNotAutoselect);
        }
        groups.entry(&decl.group_id).or_default().push(decl);
    }
    let mut signatures = BTreeSet::new();
    for members in groups.values() {
        let mut names = BTreeSet::new();
        if !members.iter().all(|m| names.insert(m.name.as_str())) {
            return Err(HlsOriginBuildError::DuplicateAudioRenditionName);
        }
        if members.iter().filter(|m| m.default).count() != 1 {
            return Err(HlsOriginBuildError::AudioRenditionDefaultCount);
        }
        let mut signature: Vec<_> = members
            .iter()
            .map(|m| {
                (
                    m.name.as_str(),
                    m.language.as_str(),
                    m.default,
                    m.autoselect,
                    m.characteristics.as_slice(),
                )
            })
            .collect();
        signature.sort();
        signatures.insert(signature);
    }
    if signatures.len() > 1 {
        return Err(HlsOriginBuildError::AudioRenditionGroupsDiffer);
    }
    Ok(())
}

/// Relaxed validation for split-audio routes where `LANGUAGE`/`NAME`/`CHANNELS`
/// may be left empty to be derived from the sibling audio track at render time
/// (issue #1428 slice 3). Only structural rules that do not depend on those
/// fields are enforced.
pub fn validate_audio_renditions_relaxed(
    decls: &[AudioRenditionDecl],
) -> Result<(), HlsOriginBuildError> {
    use std::collections::BTreeMap;
    for decl in decls {
        if decl.group_id.is_empty() {
            return Err(HlsOriginBuildError::EmptyAudioRenditionGroupId);
        }
        if decl.uri.is_empty() {
            return Err(HlsOriginBuildError::EmptyAudioRenditionUri);
        }
        if decl.default && !decl.autoselect {
            return Err(HlsOriginBuildError::AudioRenditionDefaultNotAutoselect);
        }
    }
    let mut groups: BTreeMap<&str, Vec<&AudioRenditionDecl>> = BTreeMap::new();
    for decl in decls {
        groups.entry(&decl.group_id).or_default().push(decl);
    }
    for members in groups.values() {
        if members.iter().filter(|m| m.default).count() != 1 {
            return Err(HlsOriginBuildError::AudioRenditionDefaultCount);
        }
    }
    Ok(())
}

/// A typed declaration of one audio rendition for a multivariant playlist
/// (RFC 8216bis §4.4.6.1, Apple HLS authoring spec items 9.5-9.8). Consumed
/// by [`HlsOriginBuilder::audio_renditions`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AudioRenditionDecl {
    /// `GROUP-ID` of the audio rendition group.
    pub group_id: String,
    /// `NAME` of the rendition. On a split-audio route it may be left empty
    /// and is then derived from the track (the language, plus ` (AD)` for an
    /// audio-description track), made unique within its group.
    pub name: String,
    /// `LANGUAGE` of the rendition; must be non-empty (`"und"` for
    /// undetermined) unless split-audio derivation is active, which then
    /// fills it from the track's PMT `ISO_639_language_descriptor`,
    /// canonicalised to a BCP 47 (RFC 5646) primary language subtag via
    /// [`bcp47_language`] (ISO 639-2/B and 639-2/T are mapped to ISO 639-1
    /// when one exists).
    pub language: String,
    /// `DEFAULT` attribute.
    pub default: bool,
    /// `AUTOSELECT` attribute.
    pub autoselect: bool,
    /// `CHANNELS` attribute (e.g. `"2"` or `"JOC 3/2.0.2"`). `None` on a
    /// split-audio route derives the codec config's channel count.
    pub channels: Option<String>,
    /// URI of the audio media playlist.
    pub uri: String,
    /// `CHARACTERISTICS` MCTs. Empty on a split-audio route derives
    /// `public.accessibility.describes-video` for an audio-description
    /// track; a non-empty value always wins.
    pub characteristics: Vec<String>,
}

/// `GROUP-ID` of the `EXT-X-MEDIA TYPE=SUBTITLES` group the master playlist
/// advertises for declared subtitle renditions.
pub const SUBTITLES_GROUP_ID: &str = "subs";

/// `CHARACTERISTICS` of a subtitles track for deaf or hard-of-hearing viewers
/// (Apple HLS Authoring Spec item 4.5).
const HEARING_IMPAIRED_CHARACTERISTICS: [&str; 2] = [
    "public.accessibility.transcribes-spoken-dialog",
    "public.accessibility.describes-music-and-sound",
];

/// `CODECS` token naming WebVTT subtitles (Apple HLS Authoring Spec item 5.10).
const WEBVTT_CODEC: &str = "wvtt";

/// A typed declaration of one WebVTT subtitle rendition for a multivariant
/// playlist (RFC 8216 §4.3.4.1, Apple HLS Authoring Spec items 4.5-4.7, 5.11).
/// Consumed by [`HlsOriginBuilder::subtitle_renditions`]. All renditions share
/// the group [`SUBTITLES_GROUP_ID`].
///
/// `hearing_impaired` emits the item 4.5 `CHARACTERISTICS` and forces
/// `AUTOSELECT=YES` (item 4.6); `forced` emits `FORCED=YES` and forces
/// `AUTOSELECT=YES` (item 5.11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubtitleRenditionDecl {
    /// `NAME` of the rendition.
    pub name: String,
    /// `LANGUAGE` (BCP 47); required (Apple HLS Authoring Spec item 4.7).
    pub language: String,
    /// URI of the subtitle media playlist.
    pub uri: String,
    /// `DEFAULT` attribute.
    pub default: bool,
    /// `AUTOSELECT` attribute.
    pub autoselect: bool,
    /// `FORCED` attribute.
    pub forced: bool,
    /// Subtitles for deaf or hard-of-hearing viewers.
    pub hearing_impaired: bool,
}

impl SubtitleRenditionDecl {
    /// A plain subtitle rendition: `DEFAULT=NO`, `AUTOSELECT=YES`, not forced,
    /// not for the hard of hearing.
    pub fn new(
        name: impl Into<String>,
        language: impl Into<String>,
        uri: impl Into<String>,
    ) -> Self {
        SubtitleRenditionDecl {
            name: name.into(),
            language: language.into(),
            uri: uri.into(),
            default: false,
            autoselect: true,
            forced: false,
            hearing_impaired: false,
        }
    }

    fn effective_autoselect(&self) -> bool {
        self.autoselect || self.forced || self.hearing_impaired
    }
}

/// Validate subtitle rendition declarations (RFC 8216 §4.3.4.1.1; Apple HLS
/// Authoring Spec item 4.7).
pub fn validate_subtitle_renditions(
    decls: &[SubtitleRenditionDecl],
) -> Result<(), HlsOriginBuildError> {
    use std::collections::BTreeSet;
    let mut names = BTreeSet::new();
    let mut uris = BTreeSet::new();
    let mut defaults = 0usize;
    for decl in decls {
        if decl.name.is_empty() {
            return Err(HlsOriginBuildError::EmptySubtitleRenditionName);
        }
        if decl.language.is_empty() {
            return Err(HlsOriginBuildError::EmptySubtitleRenditionLanguage);
        }
        if decl.uri.is_empty() {
            return Err(HlsOriginBuildError::EmptySubtitleRenditionUri);
        }
        if decl.default && !decl.effective_autoselect() {
            return Err(HlsOriginBuildError::SubtitleRenditionDefaultNotAutoselect);
        }
        if !names.insert(decl.name.as_str()) {
            return Err(HlsOriginBuildError::DuplicateSubtitleRenditionName);
        }
        if !uris.insert(decl.uri.as_str()) {
            return Err(HlsOriginBuildError::DuplicateSubtitleRenditionUri);
        }
        defaults += usize::from(decl.default);
    }
    if defaults > 1 {
        return Err(HlsOriginBuildError::MultipleDefaultSubtitleRenditions);
    }
    Ok(())
}

/// `GROUP-ID` of the `EXT-X-MEDIA TYPE=CLOSED-CAPTIONS` group the master
/// playlist advertises for detected captions.
pub const CLOSED_CAPTIONS_GROUP_ID: &str = "cc";
/// Operator override of how one detected caption stream is advertised
/// (RFC 8216bis §4.4.6.1, Apple HLS authoring spec item 4.4). It never makes
/// a stream appear: a stream is advertised only once the video is seen to
/// carry it. Consumed by [`HlsOriginBuilder::closed_caption_overrides`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ClosedCaptionDecl {
    /// The `INSTREAM-ID` this override applies to: `"CC1"`..`"CC4"` or
    /// `"SERVICE1"`..`"SERVICE63"`.
    pub instream_id: String,
    /// `NAME`; empty keeps the default, the `INSTREAM-ID` itself.
    pub name: String,
    /// `LANGUAGE` (BCP 47); empty omits the attribute. The bitstream carries
    /// no language, and Apple's `mediastreamvalidator` flags the placeholder
    /// `"und"` as an unrecognised subtag, so none is invented; Apple's
    /// authoring rule 4.4 asks for one, which is what this field is for.
    pub language: String,
}

/// Validate closed-caption overrides: well-formed, unique `INSTREAM-ID`s.
pub fn validate_closed_caption_overrides(
    decls: &[ClosedCaptionDecl],
) -> Result<(), HlsOriginBuildError> {
    let mut seen: Vec<broadcast_hls::InstreamId> = Vec::new();
    for decl in decls {
        let id = broadcast_hls::InstreamId::parse_closed_captions(&decl.instream_id)
            .map_err(|_| HlsOriginBuildError::InvalidClosedCaptionInstreamId)?;
        if seen.contains(&id) {
            return Err(HlsOriginBuildError::DuplicateClosedCaptionInstreamId);
        }
        seen.push(id);
    }
    Ok(())
}

/// A sibling rendition's origin + the URI of its media playlist, used to
/// render `#EXT-X-RENDITION-REPORT` (RFC 8216bis §4.4.5.4) into this
/// origin's media playlist. Stored as a [`Weak`] reference so origins can
/// refer to each other without forming a reference cycle.
#[derive(Clone)]
pub struct RenditionSibling {
    /// Weak handle to the sibling origin.
    pub origin: Weak<HlsOrigin>,
    /// URI of the sibling's media playlist, relative to the same stream root
    /// as this origin's own playlist.
    pub playlist_uri: String,
}

/// Fluent builder for [`HlsOrigin`] (issue #873) — replaces the old
/// four-positional `HlsOrigin::new` (deleted; this crate is at 0.4.0
/// unpublished, so there is no compatibility burden), which could not
/// express "classic HLS, no low latency" at all since `part_target_ms` was a
/// mandatory positional argument.
///
/// ```
/// # use std::num::NonZeroUsize;
/// # use std::sync::Arc;
/// # use hls_runtime::server::{Container, HlsOrigin};
/// # use media_plane::trunk::{Trunk, TrunkConfig};
/// # let nz = |n: usize| NonZeroUsize::new(n).unwrap();
/// # let trunk = Arc::new(Trunk::new(TrunkConfig::new(nz(16), nz(4), nz(8), nz(4), nz(16))));
/// let classic_ts = HlsOrigin::builder(Arc::clone(&trunk))
///     .target_duration_secs(6.0)
///     .window_segments(nz(4))
///     .container(Container::MpegTs)
///     // `.low_latency(..)` omitted entirely -> classic HLS.
///     .build()
///     .expect("both required fields were set");
/// # let _ = classic_ts;
/// ```
pub struct HlsOriginBuilder {
    trunk: Arc<Trunk>,
    target_duration_secs: Option<f64>,
    window_segments: Option<NonZeroUsize>,
    container: Container,
    part_target_ms: Option<u32>,
    media_sequence_offset: u64,
    instance: Option<u64>,
    arrival_clock: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
    declared_bandwidth_bps: Option<u64>,
    audio_renditions: Vec<AudioRenditionDecl>,
    audio_sibling_origins: Vec<Arc<HlsOrigin>>,
    subtitle_renditions: Vec<SubtitleRenditionDecl>,
    subtitle_sibling_origins: Vec<Arc<HlsOrigin>>,
    playlist_uri: Option<String>,
    rendition_siblings: Vec<RenditionSibling>,
    closed_captions_none: bool,
    closed_caption_overrides: Vec<ClosedCaptionDecl>,
}

impl HlsOriginBuilder {
    fn new(trunk: Arc<Trunk>) -> Self {
        HlsOriginBuilder {
            trunk,
            target_duration_secs: None,
            window_segments: None,
            container: Container::default(),
            part_target_ms: None,
            media_sequence_offset: 0,
            instance: None,
            arrival_clock: None,
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_sibling_origins: Vec::new(),
            subtitle_renditions: Vec::new(),
            subtitle_sibling_origins: Vec::new(),
            playlist_uri: None,
            rendition_siblings: Vec::new(),
            closed_captions_none: false,
            closed_caption_overrides: Vec::new(),
        }
    }

    /// `#EXT-X-TARGETDURATION`'s configured floor (RFC 8216bis §4.4.3.1) —
    /// required; [`HlsOriginBuilder::build`] errors if this is never called.
    /// The actually-rendered value is raised to the largest real segment
    /// duration seen, if that ever exceeds this (see `render_playlist`).
    pub fn target_duration_secs(mut self, target_duration_secs: f64) -> Self {
        self.target_duration_secs = Some(target_duration_secs);
        self
    }

    /// How many closed segments this origin advertises in a rendered Media
    /// Playlist — required; independent of
    /// [`media_plane::trunk::TrunkConfig::segment_capacity`] (the `Trunk`'s
    /// own retention bound): a caller may legitimately want a shorter
    /// advertised window than the `Trunk` retains for other consumers (e.g. a
    /// DVR `SegmentEgress` reading the same `Trunk`).
    pub fn window_segments(mut self, window_segments: NonZeroUsize) -> Self {
        self.window_segments = Some(window_segments);
        self
    }

    /// Which container this origin serves segments/parts as. Defaults to
    /// [`Container::Fmp4`] if never called, matching every pre-#873 caller's
    /// behaviour. Orthogonal to [`Self::low_latency`] — all four
    /// `{Fmp4, MpegTs} x {classic, low-latency}` combinations are valid.
    pub fn container(mut self, container: Container) -> Self {
        self.container = container;
        self
    }

    /// Opt into LL-HLS: `part_target_ms` becomes `#EXT-X-PART-INF`'s
    /// `PART-TARGET` (milliseconds). **Omit this call entirely for classic
    /// HLS** — no `#EXT-X-PART`/`#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`/
    /// `#EXT-X-PRELOAD-HINT` tags are then rendered, regardless of
    /// [`Self::container`]. This is what the old constructor's mandatory
    /// `part_target_ms` positional could not express.
    pub fn low_latency(mut self, part_target_ms: u32) -> Self {
        self.part_target_ms = Some(part_target_ms);
        self
    }

    /// Add `offset` to every Media Sequence Number this origin shows: the
    /// playlist's `EXT-X-MEDIA-SEQUENCE`, the numbers in segment/part URIs and
    /// the `_HLS_msn` it answers to. Default `0`.
    ///
    /// A fresh [`Trunk`] restarts its segment numbers at `1`; the offset is
    /// the mechanism that keeps the Media Sequence Number a client sees
    /// increasing across a replacement origin (RFC 8216bis §6.2.2: it MUST NOT
    /// decrease). Pass `previous.next_media_sequence()` — not `- 1`: the number
    /// the previous origin's *open* segment was shown with is skipped too, as
    /// its parts were already served (see [`HlsOrigin::next_media_sequence`]).
    pub fn media_sequence_offset(mut self, offset: u64) -> Self {
        self.media_sequence_offset = offset;
        self
    }

    /// Override the per-origin [`HlsOrigin::instance`] token (default: a fresh
    /// wall-clock-seeded one per build). For deterministic tests and for an
    /// embedder that wants names stable across a rebuild it knows is the
    /// *same* media.
    pub fn instance(mut self, token: u64) -> Self {
        self.instance = Some(token);
        self
    }

    /// Configure an **arrival** wall-clock source for `#EXT-X-PROGRAM-DATE-TIME`
    /// (Apple authoring 8.4 MUST — PDT present in live playlists). `clock`
    /// returns Unix epoch milliseconds; it is invoked **at most once** over the
    /// origin's lifetime (to stamp the first drained segment), never per
    /// request, so the caller supplies whatever wall clock it wants (e.g. a
    /// closure over `std::time::SystemTime` or a test counter) without the
    /// sans-IO engine touching a clock itself. When omitted (the default) no
    /// arrival source is used and PDTs come only from an explicit trunk
    /// `time_anchor` (B1); when both are absent no PDT is emitted.
    ///
    /// The arrival source anchors PDTs to the trunk's own continuous segment
    /// timeline rather than to a (possibly jumping) source media clock, so a
    /// source-clock discontinuity never makes the date↔segment mapping
    /// ambiguous (server-requirements.md item 26).
    pub fn program_date_time_arrival(mut self, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.arrival_clock = Some(clock);
        self
    }

    /// Operator-declared peak bitrate (bits/second) for this origin's
    /// variant stream, used as a floor for `BANDWIDTH` in the master
    /// playlist before any segment with a measurable duration has been
    /// published (and as a lower bound afterwards). Validated at
    /// [`Self::build`] (`> 0` and `<= u32::MAX`).
    pub fn declared_bandwidth_bps(mut self, declared_bandwidth_bps: u64) -> Self {
        self.declared_bandwidth_bps = Some(declared_bandwidth_bps);
        self
    }

    /// Declare the audio renditions this origin's master playlist advertises
    /// (RFC 8216bis §4.4.6.1). Each entry becomes one `#EXT-X-MEDIA:TYPE=AUDIO`
    /// line, emitted before the variants; the video variant's `AUDIO` attribute
    /// is set to the entry's `group_id`. Empty by default (no audio
    /// rendition lines, byte-identical to the pre-existing output).
    pub fn audio_renditions(mut self, audio_renditions: Vec<AudioRenditionDecl>) -> Self {
        self.audio_renditions = audio_renditions;
        self
    }

    /// Sibling audio origins (issue #1428 slice 3) whose measured peak/average
    /// bandwidth and `CODECS` are folded into this (video) origin's
    /// multivariant playlist for split-audio routes. One per declared
    /// rendition, in the same order. Each group's `EXT-X-STREAM-INF` carries
    /// `BANDWIDTH` = video peak + the group's largest audio peak and
    /// `AVERAGE-BANDWIDTH` likewise over averages (Apple authoring items
    /// 9.13/9.14); `CODECS` is the video codec plus the group's distinct audio
    /// codecs. [`HlsOriginBuilder::declared_bandwidth_bps`] floors the video
    /// part only, never the audio part. Empty (the default) keeps the slice 1
    /// behaviour: declared-only `CODECS`/`BANDWIDTH`.
    pub fn audio_sibling_origins(mut self, origins: Vec<Arc<HlsOrigin>>) -> Self {
        self.audio_sibling_origins = origins;
        self
    }

    /// Declare the WebVTT subtitle renditions this origin's master playlist
    /// advertises: one `#EXT-X-MEDIA:TYPE=SUBTITLES` per declaration in group
    /// [`SUBTITLES_GROUP_ID`], placed after the audio renditions, with every
    /// variant's `SUBTITLES` attribute set and `wvtt` added to its `CODECS`
    /// (Apple HLS Authoring Spec item 5.10) when `CODECS` is known. Empty by
    /// default: the master is byte-identical to the pre-existing output.
    pub fn subtitle_renditions(mut self, subtitle_renditions: Vec<SubtitleRenditionDecl>) -> Self {
        self.subtitle_renditions = subtitle_renditions;
        self
    }

    /// The [`Container::WebVtt`] origins serving the declared subtitle
    /// renditions, one per declaration in the same order. Checked at
    /// [`Self::build`]: each must be a `WebVtt` origin whose
    /// `EXT-X-TARGETDURATION` equals this origin's (Apple HLS Authoring Spec
    /// item 5.6 — live subtitle playlists must match the other media).
    /// Subtitle bandwidth is not folded into `BANDWIDTH`.
    pub fn subtitle_sibling_origins(mut self, origins: Vec<Arc<HlsOrigin>>) -> Self {
        self.subtitle_sibling_origins = origins;
        self
    }

    /// URI of this origin's own media playlist, relative to the stream root
    /// (e.g. `"media.m3u8"` or `"audio/a0.m3u8"`). Used to compute the
    /// relative `URI` of [`RenditionSibling`] entries when rendering
    /// `#EXT-X-RENDITION-REPORT` (RFC 8216bis §4.4.5.4). Defaults to
    /// `"media.m3u8"` when unset.
    pub fn playlist_uri(mut self, playlist_uri: impl Into<String>) -> Self {
        self.playlist_uri = Some(playlist_uri.into());
        self
    }

    /// Sibling renditions whose live edge is reported by this origin's media
    /// playlist (RFC 8216bis §4.4.5.4). Each entry is a weak handle plus the
    /// sibling's playlist URI. Empty by default.
    pub fn rendition_siblings(mut self, siblings: Vec<RenditionSibling>) -> Self {
        self.rendition_siblings = siblings;
        self
    }

    /// When `true`, emit `CLOSED-CAPTIONS=NONE` on the video variant
    /// (RFC 8216bis §4.4.6.2). Default `false` leaves the attribute omitted,
    /// matching the pre-existing output.
    pub fn closed_captions_none(mut self, closed_captions_none: bool) -> Self {
        self.closed_captions_none = closed_captions_none;
        self
    }

    /// Override `NAME`/`LANGUAGE` of detected caption streams (see
    /// [`ClosedCaptionDecl`]). Empty by default.
    pub fn closed_caption_overrides(mut self, overrides: Vec<ClosedCaptionDecl>) -> Self {
        self.closed_caption_overrides = overrides;
        self
    }

    /// Build the [`HlsOrigin`], subscribing its one [`SegmentCursor`]
    /// immediately (so the window starts empty but never misses a segment
    /// published from this point on).
    ///
    /// Errors, never silently defaults, if [`Self::target_duration_secs`] or
    /// [`Self::window_segments`] was never called, for a zero
    /// [`Self::low_latency`] part target, or for an offset that cannot hold
    /// the `u32` sequence range.
    ///
    /// The advertised window is never shorter than three segments, whatever
    /// [`Self::window_segments`] says (RFC 8216bis §6.2.2: a live Playlist
    /// MUST NOT be shortened below three Target Durations). Part retention is
    /// not checked here: the part ring's capacity belongs to the [`Trunk`]
    /// and is not observable from it; the playlist only advertises the parts
    /// of a closed segment while all of them are still resident.
    pub fn build(self) -> Result<HlsOrigin, HlsOriginBuildError> {
        let target_duration_secs = self
            .target_duration_secs
            .ok_or(HlsOriginBuildError::MissingTargetDurationSecs)?;
        if !target_duration_secs.is_finite() || target_duration_secs <= 0.0 {
            return Err(HlsOriginBuildError::InvalidTargetDuration);
        }
        let window_segments = self
            .window_segments
            .ok_or(HlsOriginBuildError::MissingWindowSegments)?;
        if self.part_target_ms == Some(0) {
            return Err(HlsOriginBuildError::ZeroPartTarget);
        }
        if self
            .media_sequence_offset
            .checked_add(u64::from(u32::MAX))
            .is_none()
        {
            return Err(HlsOriginBuildError::MediaSequenceOffsetTooLarge);
        }
        if let Some(declared) = self.declared_bandwidth_bps
            && (declared == 0 || declared > u64::from(u32::MAX))
        {
            return Err(HlsOriginBuildError::InvalidDeclaredBandwidth);
        }
        if self.audio_sibling_origins.is_empty() {
            validate_audio_renditions(&self.audio_renditions)?;
        } else {
            validate_audio_renditions_relaxed(&self.audio_renditions)?;
        }
        validate_closed_caption_overrides(&self.closed_caption_overrides)?;
        validate_subtitle_renditions(&self.subtitle_renditions)?;
        if !self.subtitle_sibling_origins.is_empty() {
            if self.subtitle_sibling_origins.len() != self.subtitle_renditions.len() {
                return Err(HlsOriginBuildError::SubtitleSiblingCountMismatch);
            }
            let target = target_duration_secs.ceil();
            for sibling in &self.subtitle_sibling_origins {
                if sibling.container != Container::WebVtt {
                    return Err(HlsOriginBuildError::SubtitleSiblingNotWebVtt);
                }
                if sibling.target_duration_secs.ceil() != target {
                    return Err(HlsOriginBuildError::SubtitleSiblingTargetDurationMismatch);
                }
            }
        }
        let cursor = self.trunk.subscribe_segments();
        Ok(HlsOrigin {
            trunk: self.trunk,
            cursor: Mutex::new(cursor),
            drained_through: AtomicU64::new(UNDRAINED),
            window: Mutex::new(Window::new(window_segments)),
            init: Mutex::new(InitStore::default()),
            codecs: Mutex::new(None),
            video_codecs: Mutex::new(None),
            video_range: Mutex::new(None),
            resolution: Mutex::new(None),
            frame_rate: Mutex::new(None),
            video_track_id: Mutex::new(None),
            target_duration_secs,
            container: self.container,
            part_target_ms: self.part_target_ms,
            media_sequence_offset: self.media_sequence_offset,
            instance: self.instance.unwrap_or_else(next_instance_token),
            arrival_clock: self.arrival_clock,
            arrival_origin: Mutex::new(None),
            declared_bandwidth_bps: self.declared_bandwidth_bps,
            audio_renditions: self.audio_renditions,
            audio_sibling_origins: self.audio_sibling_origins,
            subtitle_renditions: Mutex::new(self.subtitle_renditions),
            playlist_uri: self
                .playlist_uri
                .unwrap_or_else(|| "media.m3u8".to_string()),
            rendition_siblings: Mutex::new(self.rendition_siblings),
            track_specs: Mutex::new(Vec::new()),
            closed_captions_none: self.closed_captions_none,
            closed_caption_overrides: self.closed_caption_overrides,
            detected_closed_captions: Mutex::new(Vec::new()),
            frozen_target_duration_secs: Mutex::new(None),
            target_duration_violations: AtomicU64::new(0),
        })
    }
}

/// The LL-HLS origin [`ServedEgress`]: renders playlists and resolves
/// blocking-reload/part-availability requests for one stream, backed by a
/// shared [`Trunk`]. See this module's own doc for exactly what comes
/// straight from the `Trunk` and what needs the small synced `Window`.
pub struct HlsOrigin {
    trunk: Arc<Trunk>,
    /// This origin's **one** [`SegmentCursor`] — see [`Trunk::subscribe_segments`]'s
    /// own docs (and this crate's `media_plane::egress` module doc) for why a
    /// `ServedEgress` must never take one per request/peer.
    cursor: Mutex<SegmentCursor>,
    /// `last closed segment + 1` (`0` = none) at the end of the last full
    /// drain; [`Self::drain`] returns at once while the `Trunk` still shows
    /// the same head. [`UNDRAINED`] before the first drain.
    drained_through: AtomicU64,
    // Lock order (never the reverse): `cursor` -> `window` -> `arrival_origin`
    // -> `init`; `codecs` is taken alone. No `Trunk` call is made while
    // `window` is held except inside `drain` (the cursor poll).
    window: Mutex<Window>,
    /// The fMP4 init segment — see this module's doc for why this, alone, is
    /// not answerable by any `Trunk` ring.
    init: Mutex<InitStore>,
    /// The `CODECS` value (RFC 6381, comma-joined) for the master playlist,
    /// known once track specs have been supplied; `None` = unknown, so the
    /// attribute is omitted rather than invented.
    codecs: Mutex<Option<String>>,
    /// `CODECS` value for the I-frame variant: the video-only codec string
    /// (the first video track by track_id), or `None` when unknown.
    video_codecs: Mutex<Option<String>>,
    /// `VIDEO-RANGE` for the master playlist, derived from the first video
    /// track (in track-id order) whose display carries a colour description;
    /// `None` = unknown / SDR, so the attribute is omitted rather than
    /// invented (RFC 8216bis §4.4.6.2).
    video_range: Mutex<Option<VideoRange>>,
    resolution: Mutex<Option<(u32, u32)>>,
    /// `FRAME-RATE` for the master playlist, derived as the maximum across
    /// all video tracks. Emitted only when an H.264 track's SPS carries VUI
    /// `timing_info`; HEVC and codecs without VUI timing omit it.
    frame_rate: Mutex<Option<FrameRate>>,
    /// The first video track (in track-id order) supplied via
    /// [`Self::set_track_specs`], used to locate the video traf when
    /// computing an I-frame playlist's byte ranges. `None` until track specs
    /// are known (or when there is no video track).
    video_track_id: Mutex<Option<TrackId>>,
    target_duration_secs: f64,
    container: Container,
    /// `Some(part_target_ms)` enables LL-HLS; `None` renders classic HLS —
    /// no `#EXT-X-PART`/`#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`/
    /// `#EXT-X-PRELOAD-HINT` at all, orthogonal to [`Self::container`] (issue
    /// #873).
    part_target_ms: Option<u32>,
    /// Added to every shown Media Sequence Number (see
    /// [`HlsOriginBuilder::media_sequence_offset`]); validated at build so
    /// `offset + u32::MAX` fits a `u64`.
    media_sequence_offset: u64,
    /// See [`Self::instance`].
    instance: u64,
    /// Optional arrival wall-clock source (see
    /// [`HlsOriginBuilder::program_date_time_arrival`]). `None` => no arrival
    /// source.
    arrival_clock: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
    /// Recorded origin: `(segment timeline start_ns, wall epoch ms)`. Set
    /// once, the first time a segment drains into the window while an arrival
    /// clock is configured; never reset for the life of the origin.
    /// Lock order: `window` -> `arrival_origin` (arrival_origin is never
    /// taken before window).
    arrival_origin: Mutex<Option<(u64, i64)>>,
    /// Operator-declared peak bitrate (see
    /// [`HlsOriginBuilder::declared_bandwidth_bps`]) — `None` means none.
    declared_bandwidth_bps: Option<u64>,
    /// Declared audio renditions (issue #1428 slice 1).
    audio_renditions: Vec<AudioRenditionDecl>,
    /// Sibling audio origins (issue #1428 slice 3).
    audio_sibling_origins: Vec<Arc<HlsOrigin>>,
    /// Declared WebVTT subtitle renditions.
    subtitle_renditions: Mutex<Vec<SubtitleRenditionDecl>>,
    /// URI of this origin's own media playlist.
    playlist_uri: String,
    /// Sibling renditions whose live edge this origin reports.
    rendition_siblings: Mutex<Vec<RenditionSibling>>,
    /// Stored track specs (issue #1428 slice 3), for audio metadata derivation.
    track_specs: Mutex<Vec<transmux::TrackSpec>>,
    /// Emit `CLOSED-CAPTIONS=NONE` (issue #1428 slice 1).
    closed_captions_none: bool,
    closed_caption_overrides: Vec<ClosedCaptionDecl>,
    /// Caption streams the video has been seen to carry, in advertising
    /// order. Taken alone (never while `window` or `codecs` is held).
    detected_closed_captions: Mutex<Vec<broadcast_hls::InstreamId>>,
    /// Frozen `EXT-X-TARGETDURATION` (seconds, pre-rounding) captured the
    /// first time a playlist is rendered with at least one segment in the
    /// window: RFC 8216 §4.3.3.1 forbids the tag changing once published,
    /// so later (possibly longer) segments never move it. `None` until then.
    frozen_target_duration_secs: Mutex<Option<f64>>,
    /// Number of segments whose (rounded) duration exceeded the frozen
    /// `EXT-X-TARGETDURATION` after first publication — a conformance
    /// degradation counter (RFC 8216 §4.3.3.1). Incremented once per such
    /// segment at drain time.
    target_duration_violations: AtomicU64,
}

impl HlsOrigin {
    /// Start building an [`HlsOrigin`] over `trunk` — see [`HlsOriginBuilder`]
    /// for the required fields (`target_duration_secs`/`window_segments`),
    /// the container choice, and how to opt into LL-HLS.
    pub fn builder(trunk: Arc<Trunk>) -> HlsOriginBuilder {
        HlsOriginBuilder::new(trunk)
    }

    /// Store the init segment bytes — see this module's doc for why an init
    /// segment is not something any `Trunk` ring holds.
    ///
    /// **Documented no-op under [`Container::MpegTs`]**: this origin's
    /// `MpegTs` grammar has no init resource and never emits `#EXT-X-MAP`
    /// by default (see [`Container`]'s own doc), so bytes stored here are
    /// never advertised or served in that mode. The method stays callable
    /// regardless of container so a caller sharing one code path across
    /// both (e.g. a segmenter that always calls `set_init` once available)
    /// does not need to branch on which container it configured.
    ///
    /// Under [`Container::Fmp4`] the init segment is also parsed for its
    /// tracks, from which the master playlist's `CODECS` is derived (see
    /// [`Self::master_playlist`]); bytes that do not parse (or a track whose
    /// codec string cannot be built) make `CODECS` unknown (omitted, and any
    /// value from an earlier init is dropped), never guessed at.
    pub fn set_init(&self, bytes: impl Into<Bytes>) {
        let bytes = bytes.into();
        if self.container == Container::Fmp4 {
            let derived = transmux::Fmp4Demux::new()
                .unpackage(bytes.as_ref())
                .ok()
                .map(|media| {
                    media
                        .tracks
                        .into_iter()
                        .map(|t| t.spec)
                        .collect::<Vec<transmux::TrackSpec>>()
                })
                .map(|specs| self.with_known_es_info(specs))
                .and_then(|specs| self.set_track_specs(&specs).ok());
            if derived.is_none() {
                // A new init that cannot be read must not leave the previous
                // init's codecs (or VIDEO-RANGE) advertised.
                *locked(&self.codecs) = None;
                *locked(&self.video_codecs) = None;
                *locked(&self.video_range) = None;
                *locked(&self.resolution) = None;
                *locked(&self.frame_rate) = None;
                *locked(&self.video_track_id) = None;
                *locked(&self.track_specs) = Vec::new();
            }
        }
        // The first segment not yet closed is the first one cut against
        // these bytes: call this before publishing it.
        let next_seq = self
            .trunk
            .last_closed_segment()
            .unwrap_or(SegmentSeq::new(0))
            .saturating_next();
        locked(&self.init).set(bytes, next_seq);
    }

    /// Tell the origin which tracks it serves, so the master playlist can
    /// carry `CODECS` (RFC 8216 §4.3.4.2: every `EXT-X-STREAM-INF` SHOULD) —
    /// derived from each track's codec config with the RFC 6381 builders
    /// `transmux` uses for DASH. Opaque data/subtitle tracks have no codec
    /// string and are left out. [`Self::set_init`] does this itself for
    /// fMP4; call this for a [`Container::MpegTs`] origin, whose segments
    /// carry no init segment to read the codecs from.
    ///
    /// # Errors
    /// The `transmux` error for a track whose codec string cannot be built
    /// (for example a malformed AAC `esds`); `CODECS` is then left as it was.
    pub fn set_track_specs(&self, specs: &[transmux::TrackSpec]) -> Result<(), transmux::Error> {
        let mut ordered: Vec<&transmux::TrackSpec> = specs.iter().collect();
        ordered.sort_by_key(|spec| spec.track_id);
        let stored_track_specs: Vec<transmux::TrackSpec> =
            ordered.iter().map(|s| (*s).clone()).collect();
        *locked(&self.video_track_id) = ordered
            .iter()
            .find(|spec| is_video_codec(&spec.config))
            .map(|spec| spec.track_id);
        let mut codecs: Vec<String> = Vec::new();
        // Video-only codec string for the I-frame variant: the first video
        // track by track_id (RFC 8216bis §4.4.6.3 / Apple 9.3).
        let mut video_codecs: Option<String> = None;
        let mut video_codecs_decided = false;
        // VIDEO-RANGE is derived from the first video track (in track-id
        // order) whose display carries a colour description (RFC 8216bis
        // §4.4.6.2). `None` when no such track exists OR its transfer code
        // has no defined mapping (attribute omitted).
        let mut video_range: Option<VideoRange> = None;
        let mut video_range_decided = false;
        let mut resolution: Option<(u32, u32)> = None;
        let mut resolution_decided = false;
        let mut frame_rate: Option<FrameRate> = None;
        for spec in ordered {
            match transmux::rfc6381_codec_string(&spec.config) {
                Ok(codec) => codecs.push(codec),
                Err(transmux::Error::UnsupportedCodec { .. }) => {}
                Err(other) => return Err(other),
            }
            if !video_codecs_decided && is_video_codec(&spec.config) {
                match transmux::rfc6381_codec_string(&spec.config) {
                    Ok(codec) => video_codecs = Some(codec),
                    Err(transmux::Error::UnsupportedCodec { .. }) => {}
                    Err(other) => return Err(other),
                }
                video_codecs_decided = true;
            }
            if !video_range_decided
                && let Some(display) = &spec.display
                && let Some(signal) = &display.video_signal
                && let Some(colour) = &signal.colour
            {
                video_range = VideoRange::from_transfer_characteristics(colour.transfer.code());
                video_range_decided = true;
            }
            if !resolution_decided
                && let Some((w, h)) = match &spec.config {
                    transmux::CodecConfig::Avc { width, height, .. } => Some((*width, *height)),
                    transmux::CodecConfig::Hevc { width, height, .. } => Some((*width, *height)),
                    _ => None,
                }
            {
                resolution = Some(transmux::presentation_size(spec.display.as_ref(), w, h));
                resolution_decided = true;
            }
            // FRAME-RATE is the maximum across all video tracks (RFC 8216bis
            // §4.4.6.2). Source is H.264 SPS VUI timing only; HEVC and other
            // codecs yield None and never contribute.
            if let Some(mh) = transmux::nominal_frame_rate_millihertz(&spec.config) {
                let candidate = FrameRate::from_millihertz(mh);
                frame_rate = Some(match frame_rate {
                    Some(existing) if existing.millihertz() >= mh => existing,
                    _ => candidate,
                });
            }
        }
        *locked(&self.codecs) = (!codecs.is_empty()).then(|| codecs.join(","));
        *locked(&self.video_codecs) = video_codecs;
        *locked(&self.video_range) = video_range;
        *locked(&self.resolution) = resolution;
        *locked(&self.frame_rate) = frame_rate;
        *locked(&self.track_specs) = stored_track_specs;
        Ok(())
    }

    /// An init segment carries no PMT `ES_info`; keep what an earlier
    /// [`Self::set_track_specs`] recorded for the same track so the language
    /// and role of a split audio rendition survive the init re-derivation.
    fn with_known_es_info(&self, mut specs: Vec<transmux::TrackSpec>) -> Vec<transmux::TrackSpec> {
        let known = locked(&self.track_specs);
        for spec in &mut specs {
            if !spec.es_info_descriptors.is_empty() {
                continue;
            }
            if let Some(prev) = known.iter().find(|k| k.track_id == spec.track_id) {
                spec.source_pid = prev.source_pid;
                spec.es_info_descriptors = prev.es_info_descriptors.clone();
            }
        }
        specs
    }

    /// One `Window` lock, one consistent snapshot: peak bits/sec, total bytes
    /// and total duration (nanoseconds).
    fn window_snapshot(&self) -> (u64, u64, u128) {
        let w = locked(&self.window);
        (w.peak_bandwidth_bps, w.total_bytes, w.total_duration_nanos)
    }

    /// Record the caption streams the video is now known to carry. The next
    /// master playlist advertises exactly these, one
    /// `EXT-X-MEDIA TYPE=CLOSED-CAPTIONS` per stream in group
    /// [`CLOSED_CAPTIONS_GROUP_ID`], and every variant references the group.
    /// Until this is called with a non-empty list the master claims no
    /// captions.
    pub fn set_detected_closed_captions(&self, detected: Vec<broadcast_hls::InstreamId>) {
        *locked(&self.detected_closed_captions) = detected;
    }

    /// Replace the declared WebVTT subtitle renditions after construction, for
    /// a host that learns them only once the stream is seen (for example from
    /// a teletext descriptor). The next master playlist advertises exactly
    /// these in group [`SUBTITLES_GROUP_ID`]; an empty list removes the group.
    ///
    /// # Errors
    /// The same [`HlsOriginBuildError`] variants as
    /// [`HlsOriginBuilder::subtitle_renditions`] at `build()`; the previous
    /// declarations are kept on error.
    pub fn set_subtitle_renditions(
        &self,
        decls: Vec<SubtitleRenditionDecl>,
    ) -> Result<(), HlsOriginBuildError> {
        validate_subtitle_renditions(&decls)?;
        *locked(&self.subtitle_renditions) = decls;
        Ok(())
    }

    /// The master playlist for this origin, pointing at `media_playlist_name`:
    /// one `#EXT-X-STREAM-INF` whose `BANDWIDTH` is the highest segment
    /// bitrate this origin has published (RFC 8216 §4.3.4.2: an upper bound
    /// of the peak segment bitrate, measured as segment bytes over its
    /// duration, rounded up) or the operator-declared bandwidth, whichever is
    /// larger. Returns [`HlsMasterError::BandwidthUnknown`] until a segment
    /// with a measurable duration has closed and no declared bandwidth was
    /// configured.
    ///
    /// # Errors
    /// [`HlsMasterError::BandwidthUnknown`] when no measurable segment has
    /// been published and no declared bandwidth was configured;
    /// [`HlsMasterError::BandwidthOverflow`] when the measured `BANDWIDTH`
    /// does not fit the `u32` of a master playlist variant;
    /// [`HlsMasterError::Render`] when the playlist cannot be rendered.
    pub fn master_playlist(&self, media_playlist_name: &str) -> Result<String, HlsMasterError> {
        self.master_playlist_inner(media_playlist_name, None)
    }

    /// Like [`Self::master_playlist`], but additionally advertises an
    /// `#EXT-X-I-FRAME-STREAM-INF` variant pointing at
    /// `iframe_playlist_name` when an I-frame playlist is currently
    /// available for this origin (fMP4, a video track, and at least one
    /// usable entry with no mid-stream gap). The plain variant is unchanged.
    ///
    /// # Errors
    /// As for [`Self::master_playlist`].
    pub fn master_playlist_with_iframes(
        &self,
        media_playlist_name: &str,
        iframe_playlist_name: &str,
    ) -> Result<String, HlsMasterError> {
        self.master_playlist_inner(media_playlist_name, Some(iframe_playlist_name))
    }

    fn master_playlist_inner(
        &self,
        media_playlist_name: &str,
        iframe_playlist_name: Option<&str>,
    ) -> Result<String, HlsMasterError> {
        self.drain();
        let (video_peak, video_bytes, video_duration) = {
            let window = locked(&self.window);
            (
                window.peak_bandwidth_bps,
                window.total_bytes,
                window.total_duration_nanos,
            )
        };
        let declared = self.declared_bandwidth_bps.unwrap_or(0);
        let video_part_peak = video_peak.max(declared);
        if video_part_peak == 0 {
            return Err(HlsMasterError::BandwidthUnknown);
        }
        let video_avg = average_bandwidth_bps(video_bytes, video_duration);
        let has_siblings = !self.audio_sibling_origins.is_empty();
        let primary_codecs = locked(&self.codecs).clone();
        let mut extra_attrs: Vec<(String, broadcast_hls::AttrValue)> = Vec::new();
        if let Some(range) = *locked(&self.video_range)
            && matches!(range, VideoRange::Hlg | VideoRange::Pq)
        {
            extra_attrs.push(range.attr());
        }

        for decl in &self.audio_renditions {
            if decl.uri == media_playlist_name {
                return Err(HlsMasterError::AudioRenditionUriEqualsMediaPlaylist);
            }
        }

        let media: Vec<MediaRendition> = self
            .audio_renditions
            .iter()
            .enumerate()
            .map(|(i, decl)| {
                let sibling = self.audio_sibling_origins.get(i);
                let audio_spec = sibling.and_then(|s| {
                    locked(&s.track_specs)
                        .iter()
                        .find(|t| !is_video_codec(&t.config))
                        .cloned()
                });
                let meta = audio_spec.as_ref().map(|t| t.audio_metadata());
                let role = meta
                    .as_ref()
                    .map(|m| m.role)
                    .unwrap_or(transmux::AudioRole::Unknown);
                let derived_iso = meta
                    .as_ref()
                    .and_then(|m| m.languages.first().map(|l| l.language.clone()))
                    .unwrap_or_else(|| "und".to_string());
                // LANGUAGE must be a BCP 47 tag: canonicalise the ISO 639-2
                // code from the PMT. NAME keeps the source code as before.
                let derived_lang =
                    bcp47_language(&derived_iso).unwrap_or_else(|| "und".to_string());
                let language = if decl.language.is_empty() {
                    derived_lang.clone()
                } else {
                    decl.language.clone()
                };
                let name_source = if decl.language.is_empty() {
                    derived_iso.clone()
                } else {
                    decl.language.clone()
                };
                let name = if decl.name.is_empty() {
                    if role == transmux::AudioRole::AudioDescription {
                        format!("{} (AD)", name_source)
                    } else {
                        name_source.clone()
                    }
                } else {
                    decl.name.clone()
                };
                let characteristics = if decl.characteristics.is_empty() {
                    if role == transmux::AudioRole::AudioDescription {
                        vec!["public.accessibility.describes-video".to_string()]
                    } else {
                        Vec::new()
                    }
                } else {
                    decl.characteristics.clone()
                };
                let channels = decl.channels.clone().or_else(|| {
                    audio_spec
                        .as_ref()
                        .and_then(|t| channel_count_string(&t.config))
                });
                let mut m = MediaRendition::default();
                m.media_type = MediaType::Audio;
                m.uri = Some(decl.uri.clone());
                m.group_id = decl.group_id.clone();
                m.language = Some(language);
                m.name = name;
                m.default = decl.default;
                m.autoselect = decl.autoselect;
                m.channels = channels;
                m.characteristics = characteristics;
                m
            })
            .collect();
        let mut media = dedupe_rendition_names(media);
        media.extend(self.subtitle_media_renditions());
        let subtitles_group =
            (!locked(&self.subtitle_renditions).is_empty()).then(|| SUBTITLES_GROUP_ID.to_string());
        let with_wvtt = |codecs: Option<String>| match (&subtitles_group, codecs) {
            (Some(_), Some(c)) => Some(format!("{c},{WEBVTT_CODEC}")),
            (_, c) => c,
        };
        let detected_captions = locked(&self.detected_closed_captions).clone();
        let caption_media = self.closed_caption_renditions(&detected_captions);
        let captions_advertised = !caption_media.is_empty();
        media.extend(caption_media);

        let audio_groups: Vec<String> = {
            let mut g: Vec<String> = Vec::new();
            for decl in &self.audio_renditions {
                if !g.contains(&decl.group_id) {
                    g.push(decl.group_id.clone());
                }
            }
            g
        };

        let closed_captions = if captions_advertised {
            Some(ClosedCaptionsRef::Group(
                CLOSED_CAPTIONS_GROUP_ID.to_string(),
            ))
        } else if self.closed_captions_none {
            Some(ClosedCaptionsRef::None)
        } else {
            None
        };

        let mut variants: Vec<Variant> = Vec::new();
        if audio_groups.is_empty() {
            let bw = video_part_peak;
            let bw = u32::try_from(bw)
                .map_err(|_| HlsMasterError::BandwidthOverflow { bandwidth: bw })?;
            let avg = video_avg.map(|a| a.min(bw));
            variants.push(Variant {
                bandwidth: bw,
                average_bandwidth: avg,
                codecs: with_wvtt(primary_codecs.clone()),
                uri: media_playlist_name.to_string(),
                audio: None,
                video: None,
                subtitles: subtitles_group.clone(),
                closed_captions,
                extra_attrs: extra_attrs.clone(),
                resolution: *locked(&self.resolution),
                frame_rate: *locked(&self.frame_rate),
            });
        } else {
            for group in &audio_groups {
                // Audio peak/average max over this group's sibling origins.
                let mut audio_peak_max = 0u64;
                let mut audio_avg_max = 0u32;
                let mut audio_codecs: Vec<String> = Vec::new();
                for (i, decl) in self.audio_renditions.iter().enumerate() {
                    if decl.group_id != *group {
                        continue;
                    }
                    if let Some(sib) = self.audio_sibling_origins.get(i) {
                        sib.drain();
                        let (peak, tb, td) = sib.window_snapshot();
                        audio_peak_max = audio_peak_max.max(peak);
                        if let Some(avg) = average_bandwidth_bps(tb, td) {
                            audio_avg_max = audio_avg_max.max(avg);
                        }
                        if let Some(codec) = locked(&sib.codecs).clone()
                            && !audio_codecs.contains(&codec)
                        {
                            audio_codecs.push(codec);
                        }
                    }
                }
                let group_peak = video_part_peak.saturating_add(audio_peak_max);
                let group_bw =
                    u32::try_from(group_peak).map_err(|_| HlsMasterError::BandwidthOverflow {
                        bandwidth: group_peak,
                    })?;
                let group_avg_candidate = video_avg.unwrap_or(0) as u64 + u64::from(audio_avg_max);
                let group_avg = if group_avg_candidate == 0 {
                    None
                } else {
                    Some(
                        u32::try_from(group_avg_candidate.min(u64::from(group_bw)))
                            .unwrap_or(group_bw),
                    )
                };
                let group_codecs = if has_siblings {
                    let mut all: Vec<String> = primary_codecs.iter().cloned().collect();
                    all.extend(audio_codecs);
                    if all.is_empty() {
                        None
                    } else {
                        Some(all.join(","))
                    }
                } else {
                    primary_codecs.clone()
                };
                variants.push(Variant {
                    bandwidth: group_bw,
                    average_bandwidth: group_avg,
                    codecs: with_wvtt(group_codecs),
                    uri: media_playlist_name.to_string(),
                    audio: Some(group.clone()),
                    video: None,
                    subtitles: subtitles_group.clone(),
                    closed_captions: closed_captions.clone(),
                    extra_attrs: extra_attrs.clone(),
                    resolution: *locked(&self.resolution),
                    frame_rate: *locked(&self.frame_rate),
                });
            }
        }

        // I-frame variant, only when the I-frame playlist is available.
        let mut iframe_variants: Vec<broadcast_hls::IFrameVariant> = Vec::new();
        if let Some(iframe_name) = iframe_playlist_name
            && self.iframe_playlist_available()
        {
            let (iframe_measured, iframe_total_bytes, iframe_total_duration_nanos) = {
                let window = locked(&self.window);
                (
                    window.iframe_peak_bandwidth_bps,
                    window.iframe_total_bytes,
                    window.iframe_total_duration_nanos,
                )
            };
            if iframe_measured != 0 {
                let iframe_bandwidth = u32::try_from(iframe_measured).map_err(|_| {
                    HlsMasterError::BandwidthOverflow {
                        bandwidth: iframe_measured,
                    }
                })?;
                let iframe_average =
                    average_bandwidth_bps(iframe_total_bytes, iframe_total_duration_nanos)
                        .map(|avg| avg.min(iframe_bandwidth));
                let mut iframe_extra: Vec<(String, broadcast_hls::AttrValue)> = Vec::new();
                if let Some(range) = *locked(&self.video_range)
                    && matches!(range, VideoRange::Hlg | VideoRange::Pq)
                {
                    iframe_extra.push(range.attr());
                }
                iframe_variants.push(broadcast_hls::IFrameVariant {
                    bandwidth: iframe_bandwidth,
                    average_bandwidth: iframe_average,
                    codecs: locked(&self.video_codecs).clone(),
                    resolution: *locked(&self.resolution),
                    uri: iframe_name.to_string(),
                    extra_attrs: iframe_extra,
                });
            }
        }

        let master = MasterPlaylist {
            media,
            variants,
            iframe_variants,
            ..Default::default()
        };
        master.to_m3u8().map_err(HlsMasterError::Render)
    }

    fn subtitle_media_renditions(&self) -> Vec<MediaRendition> {
        locked(&self.subtitle_renditions)
            .iter()
            .map(|decl| {
                let mut m = MediaRendition::default();
                m.media_type = MediaType::Subtitles;
                m.uri = Some(decl.uri.clone());
                m.group_id = SUBTITLES_GROUP_ID.to_string();
                m.language = Some(decl.language.clone());
                m.name = decl.name.clone();
                m.default = decl.default;
                m.autoselect = decl.effective_autoselect();
                m.forced = decl.forced;
                if decl.hearing_impaired {
                    m.characteristics = HEARING_IMPAIRED_CHARACTERISTICS
                        .iter()
                        .map(|c| (*c).to_string())
                        .collect();
                }
                m
            })
            .collect()
    }

    fn closed_caption_renditions(
        &self,
        detected: &[broadcast_hls::InstreamId],
    ) -> Vec<MediaRendition> {
        let renditions = detected
            .iter()
            .map(|id| {
                let rendered = id.render();
                let declared = self.closed_caption_overrides.iter().find(|d| {
                    broadcast_hls::InstreamId::parse_closed_captions(&d.instream_id).as_ref()
                        == Ok(id)
                });
                let pick = |value: Option<&String>, default: &str| match value {
                    Some(v) if !v.is_empty() => v.clone(),
                    _ => default.to_string(),
                };
                let mut m = MediaRendition::default();
                m.media_type = MediaType::ClosedCaptions;
                m.group_id = CLOSED_CAPTIONS_GROUP_ID.to_string();
                m.name = pick(declared.map(|d| &d.name), &rendered);
                m.language = declared
                    .map(|d| d.language.clone())
                    .filter(|l| !l.is_empty());
                m.autoselect = false;
                m.instream_id = Some(id.clone());
                m
            })
            .collect();
        dedupe_rendition_names(renditions)
    }

    fn iframe_playlist_available(&self) -> bool {
        if self.container != Container::Fmp4 {
            return false;
        }
        if locked(&self.video_track_id).is_none() {
            return false;
        }
        let window = self.window_view();
        self.iframe_entries(&window).is_some()
    }

    /// Shared D3' rule: collect I-frame entries, allowing leading `None`s but
    /// rejecting a `None` after the first entry. Returns `None` when there is
    /// no usable entry (or a gap occurred).
    fn iframe_entries(&self, window: &WindowView) -> Option<Vec<(WindowSegment, u64)>> {
        let mut entries: Vec<(WindowSegment, u64)> = Vec::new();
        let mut seen_entry = false;
        for s in window.segments.iter() {
            match s.iframe_len {
                Some(len) => {
                    seen_entry = true;
                    entries.push((s.clone(), len));
                }
                None => {
                    if seen_entry {
                        return None;
                    }
                }
            }
        }
        if entries.is_empty() {
            None
        } else {
            Some(entries)
        }
    }

    /// This origin's instance token: a number that differs for every origin
    /// built, in this process and across restarts (see
    /// [`HlsOriginBuilder::instance`]), and is part of **every resource name
    /// served `immutable`** — `init-{track}-{instance}-{generation}.mp4`,
    /// `seg-{track}-{instance}-{msn}.{ext}`,
    /// `part-{track}-{instance}-{msn}.{idx}.{ext}`. A name therefore maps to
    /// one origin's bytes for ever: a reconnect (new `Trunk`, numbers
    /// restarting, possibly a different SPS/ASC), a process restart with no
    /// DVR, a replacement origin reusing the open segment's number — none can
    /// put new bytes under a name a cache may already hold. Playlists name
    /// these URIs, so players are unaffected. A request carrying another
    /// instance's token is `NotFound`. The unnamed legacy forms
    /// (`seg-{track}-{msn}`, `part-{track}-{msn}.{idx}`, used by DASH/Smooth
    /// templates that cannot carry the token) still resolve but are
    /// [`CachePolicy::NoCache`]; the bare `init-{track}.mp4` is the current
    /// init, also [`CachePolicy::NoCache`].
    pub fn instance(&self) -> u64 {
        self.instance
    }

    /// The wire name of this origin's I-frame playlist for `track_id`.
    pub fn iframe_playlist_name(&self, track_id: TrackId) -> String {
        format!("iframe-{}.m3u8", track_id)
    }

    /// The wire name of init generation `generation` for `track_id`.
    pub fn init_name(&self, track_id: TrackId, generation: Generation) -> String {
        format!("init-{track_id}-{}-{generation}.mp4", self.instance)
    }

    fn segment_name(&self, track_id: TrackId, seq: SegmentSeq, ext: &str) -> String {
        format!(
            "seg-{track_id}-{}-{}.{ext}",
            self.instance,
            self.public_msn(seq)
        )
    }

    /// The wire name of the I-frame (trick-play) resource for `track_id`'s
    /// segment `seq` (issue #1422): `iframe-seg-{track}-{instance}-{seq}.{ext}`,
    /// the same grammar as `segment_name` but under the
    /// [`IFRAME_SEGMENT_PREFIX`] prefix, so the bytes it resolves to are
    /// always the stored segment bytes and never the `emsg`-spliced ones.
    pub fn iframe_segment_name(&self, track_id: TrackId, seq: SegmentSeq, ext: &str) -> String {
        format!(
            "iframe-seg-{track_id}-{}-{}.{ext}",
            self.instance,
            self.public_msn(seq)
        )
    }

    fn part_name(&self, track_id: TrackId, seq: SegmentSeq, idx: PartIndex, ext: &str) -> String {
        format!(
            "part-{track_id}-{}-{}.{idx}.{ext}",
            self.instance,
            self.public_msn(seq)
        )
    }

    /// The offset added to every Media Sequence Number this origin shows (see
    /// [`HlsOriginBuilder::media_sequence_offset`]): the number in a segment's
    /// URI/playlist is this plus its origin sequence number
    /// ([`ClosedSegment::sequence_number`]).
    pub fn media_sequence_offset(&self) -> u64 {
        self.media_sequence_offset
    }

    /// The Media Sequence Number the next closed segment will be shown with
    /// (last closed segment + 1, plus the configured offset).
    ///
    /// A fresh [`Trunk`] numbers its segments from `1` again, so an origin
    /// replacing this one over a new `Trunk` (a reconnect) must pass
    /// `next_media_sequence()` to
    /// [`HlsOriginBuilder::media_sequence_offset`]: the new origin's first
    /// segment (sequence number one) is then shown one above this value — the
    /// number this origin's *open* segment was shown with is skipped, since
    /// its parts were already served — and the Media Sequence Number clients
    /// see keeps increasing (RFC 8216bis §6.2.2).
    pub fn next_media_sequence(&self) -> u64 {
        self.public_msn(
            self.trunk
                .last_closed_segment()
                .unwrap_or(SegmentSeq::new(0))
                .saturating_next(),
        )
    }

    /// HTTP Priority signalling for `request` (RFC 8216bis §6.2.6): the
    /// urgency the adapter should attach to the response. Playlists get
    /// urgency 1, parts and segments urgency 2.
    pub fn http_priority(&self, request: &HlsRequest) -> HttpPriority {
        match request {
            HlsRequest::Playlist { .. } | HlsRequest::IFramePlaylist { .. } => {
                HttpPriority::PLAYLIST
            }
            HlsRequest::Resource { name } => {
                let _ = parse_part(name, self.container);
                HttpPriority::SEGMENT
            }
        }
    }

    /// The Media Sequence Number shown for origin sequence number `seq`.
    fn public_msn(&self, seq: SegmentSeq) -> u64 {
        // `media_sequence_offset + u32::MAX` fits a `u64` (checked in `build`).
        self.media_sequence_offset
            .saturating_add(u64::from(seq.value()))
    }

    /// The origin sequence number a shown Media Sequence Number refers to, or
    /// `None` when it lies outside the `u32` range this origin numbers in.
    fn internal_seq(&self, public: u64) -> Option<SegmentSeq> {
        u32::try_from(public.checked_sub(self.media_sequence_offset)?)
            .ok()
            .map(SegmentSeq::new)
    }

    /// How many parts past the live edge a request may name (RFC 8216bis
    /// §6.2.5.2's Advance Part Limit): three over the part target in seconds
    /// when that is under one second, else three. Zero when this origin is
    /// not low-latency.
    fn advance_part_limit(&self) -> u64 {
        match self.part_target_ms {
            None | Some(0) => 0,
            Some(ms) if u64::from(ms) < MILLIS_PER_SEC => {
                (ADVANCE_PART_LIMIT_NUMERATOR * MILLIS_PER_SEC).div_ceil(u64::from(ms))
            }
            Some(_) => ADVANCE_PART_LIMIT_NUMERATOR,
        }
    }

    /// Number of segments whose (rounded) duration exceeded the frozen
    /// `EXT-X-TARGETDURATION` after first publication — a conformance
    /// degradation count (RFC 8216 §4.3.3.1).
    pub fn target_duration_violations(&self) -> u64 {
        self.target_duration_violations.load(Ordering::Relaxed)
    }

    /// Replace the sibling renditions whose live edge this origin reports.
    pub fn set_rendition_siblings(&self, siblings: Vec<RenditionSibling>) {
        *locked(&self.rendition_siblings) = siblings;
    }

    /// Snapshot of this origin's current live edge as `(LAST-MSN, LAST-PART)`,
    /// suitable for an `#EXT-X-RENDITION-REPORT` (RFC 8216bis §4.4.5.4).
    /// Drains this origin first and queries only this origin's own `Trunk`,
    /// so callers never hold another origin's lock.
    pub fn current_live_edge(&self) -> (u64, Option<u64>) {
        self.drain();
        let (seq, open_parts) = self.live_edge();
        if !open_parts.is_empty() {
            let msn = self.public_msn(seq);
            let last_part = open_parts.last().map(|p| u64::from(p.part_index.value()));
            return (msn, last_part);
        }
        // Live edge is a closed segment: report its last part (if any) so
        // the sibling's partial segments are accounted for (RFC 8216bis
        // §4.4.5.4; Apple's validator requires LAST-PART whenever the
        // rendition carries partial segments).
        let last_closed = self
            .trunk
            .last_closed_segment()
            .unwrap_or(SegmentSeq::new(0));
        let msn = self.public_msn(last_closed);
        let closed_parts = self.trunk.parts_in_segment(last_closed);
        let last_part = closed_parts.last().map(|p| u64::from(p.part_index.value()));
        (msn, last_part)
    }

    /// The fMP4 init segment bytes, if set.
    pub fn init_bytes(&self) -> Option<Bytes> {
        locked(&self.init).current.clone()
    }

    /// Drain this origin's [`SegmentCursor`] into `Window` — called at the
    /// top of every [`ServedEgress::resolve`] so a render always reflects
    /// whatever has published since the last call. Non-blocking, bounded by
    /// however many segments actually published since the last drain.
    ///
    /// A [`SegmentCursorItem::Lagged`] report (this origin's `window_segments`/
    /// polling cadence fell behind the `Trunk`'s own
    /// `segment_capacity` eviction) is accepted, not treated as an error:
    /// exactly like every other lossy cursor in this workspace, the honest
    /// response is to resume from the next segment, not to fabricate the
    /// lost entries' duration/discontinuity data.
    fn drain(&self) {
        // Nothing published since the last drain: skip both locks. The head
        // is read *before* draining and stored after, so a segment published
        // in between just makes the next call drain once more (audit r09-O4).
        let head = self
            .trunk
            .last_closed_segment()
            .map_or(0, |n| u64::from(n.value()).saturating_add(1));
        if self.drained_through.load(Ordering::Acquire) == head {
            return;
        }
        // Poll the cursor first, so the heavy I-frame byte scan below runs
        // without holding the window lock (and without holding the cursor
        // lock while the init store is consulted).
        let mut cursor = locked(&self.cursor);
        let mut polled = Vec::new();
        while let Some(item) = cursor.poll() {
            polled.push(item);
        }
        drop(cursor);

        enum Prepared {
            Segment(SegmentEntry, Generation, Option<u64>),
            Gap,
        }
        let video_track_id = *locked(&self.video_track_id);
        let mut prepared = Vec::with_capacity(polled.len());
        for item in polled {
            match item {
                SegmentCursorItem::Segment(entry) => {
                    let iframe_len = video_track_id
                        .and_then(|track_id| compute_iframe_len(&entry.bytes, track_id));
                    let init_gen = locked(&self.init).generation_for(entry.sequence_number);
                    prepared.push(Prepared::Segment(entry, init_gen, iframe_len));
                }
                // `Lagged`/`Gap`/anything newer: segments were lost, so the
                // next one does not follow the window's contents.
                _ => prepared.push(Prepared::Gap),
            }
        }

        let mut window = locked(&self.window);
        for item in prepared {
            match item {
                Prepared::Segment(entry, init_gen, iframe_len) => {
                    // Record the arrival origin exactly once, the first time a
                    // segment drains into the window while an arrival clock is
                    // configured. "this segment ended about now, so it started
                    // duration ago" (issue #1423 B2a).
                    if self.arrival_clock.is_some() {
                        let mut origin = locked(&self.arrival_origin);
                        if origin.is_none()
                            && let Some(clock) = &self.arrival_clock
                        {
                            let now_ms = clock();
                            let dur_nanos = entry.duration.as_nanos();
                            let dur_ms = (dur_nanos + 500_000) / 1_000_000;
                            let dur_ms = i64::try_from(dur_ms).unwrap_or(i64::MAX);
                            let epoch_ms = now_ms.saturating_sub(dur_ms);
                            *origin = Some((entry.timeline_position.as_nanos(), epoch_ms));
                        }
                    }
                    let seg_duration = entry.duration;
                    window.push(entry, init_gen, iframe_len);
                    if let Some(frozen) = *locked(&self.frozen_target_duration_secs)
                        && seg_duration.as_secs_f64().round() > frozen.round()
                    {
                        self.target_duration_violations
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                Prepared::Gap => window.note_gap(),
            }
        }
        self.drained_through.store(head, Ordering::Release);
    }

    /// What `render_playlist` needs from the window, copied out so the window
    /// lock is not held while the `Trunk` is queried. Segment bytes are
    /// `Bytes` (reference-counted), so the copy shares them.
    fn window_view(&self) -> WindowView {
        let w = locked(&self.window);
        WindowView {
            segments: w.segments.clone(),
            max_segment_duration_secs: w.max_segment_duration_secs,
            discontinuity_sequence: w.discontinuity_sequence,
        }
    }

    /// Private PDT helper shared by the media, open-segment and I-frame
    /// renderers so the three can never disagree (issue #1423 B2a, Apple
    /// 9.x/8.15). `seq` is the segment's sequence number (needed for the
    /// explicit-anchor path); `start_ns` is the segment's position on the
    /// trunk's continuous timeline (nanoseconds).
    fn segment_pdt(
        &self,
        seq: SegmentSeq,
        start_ns: u64,
    ) -> Option<broadcast_hls::ProgramDateTime> {
        // (1) Explicit trunk time anchor wins.
        if let Some(anchor) = self.trunk.time_anchor()
            && let Some(media_start) = self.trunk.segment_start_time(seq)
            && let Ok(ts) = jiff::Timestamp::from_millisecond(anchor.media_to_epoch_ms(media_start))
        {
            return Some(broadcast_hls::ProgramDateTime::from_timestamp(ts));
        }
        // (2) Arrival origin.
        let origin = *locked(&self.arrival_origin);
        if let Some((origin_start_ns, origin_epoch_ms)) = origin {
            let delta_ns = i128::from(start_ns) - i128::from(origin_start_ns);
            let delta_ms = delta_ns / 1_000_000;
            let epoch_ms = i128::from(origin_epoch_ms) + delta_ms;
            let epoch_ms: i64 = epoch_ms.try_into().ok()?;
            if let Ok(ts) = jiff::Timestamp::from_millisecond(epoch_ms) {
                return Some(broadcast_hls::ProgramDateTime::from_timestamp(ts));
            }
        }
        // (3) None.
        None
    }

    /// A snapshot of this origin's currently-advertised closed segments
    /// (drains the cursor first, same as `render_playlist`) — ascending by
    /// sequence number.
    ///
    /// Exists for a caller that needs to merge this origin's live window
    /// with a *different* source of segments over the same numbering
    /// (multimux's DVR archive, issue #900: catch-up serving must present
    /// one continuous playlist spanning the archive and the still-live
    /// tail that hasn't been archived yet). Reuses the one cursor `drain`
    /// already maintains rather than making the caller open a second
    /// cursor on the same `Trunk` just to learn the same window
    /// `render_playlist` itself renders — `media_plane`'s own module doc:
    /// writer cost is O(N) in cursor count, so a cursor is per distinct
    /// consumer, never per peer, and never duplicated for data another
    /// cursor already tracks.
    pub fn closed_segments(&self) -> Vec<ClosedSegment> {
        self.drain();
        locked(&self.window)
            .segments
            .iter()
            .map(|s| ClosedSegment {
                sequence_number: s.sequence_number,
                start_ns: s.start_ns,
                duration_secs: s.duration_secs,
                discontinuous: s.discontinuous,
                init_generation: s.init_gen,
            })
            .collect()
    }

    /// `(in-progress-or-last-active segment sequence number, its currently
    /// resident live parts)` — the `Trunk`-only replacement for the deleted
    /// `MediaStore::latest_progress`.
    ///
    /// Derivation: the only segment that can possibly have live, not-yet-
    /// closed parts is the one immediately after
    /// [`Trunk::last_closed_segment`] (a segmenter never opens segment N+2's
    /// parts before N+1 closes) — so probing exactly that one candidate via
    /// [`Trunk::parts_in_segment`] is exact, not a heuristic. If that probe
    /// is empty (nothing has started for the next segment yet — e.g. the
    /// instant after a close, before its successor's first part lands), the
    /// answer falls back to `last_closed_segment` itself, with an empty part
    /// list — exactly the degenerate state `MediaStore::latest_progress`
    /// also returned right after `add_segment` cleared `live_parts`.
    fn live_edge(&self) -> (SegmentSeq, Vec<PartEntry>) {
        let last_closed = self
            .trunk
            .last_closed_segment()
            .unwrap_or(SegmentSeq::new(0));
        let candidate = last_closed.saturating_next();
        let parts = self.trunk.parts_in_segment(candidate);
        if parts.is_empty() {
            (last_closed, Vec::new())
        } else {
            (candidate, parts)
        }
    }

    /// The `EXT-X-PART` entries of a closed segment, taken from the parts the
    /// `Trunk` still holds. Empty unless *all* of them are resident, as a
    /// run `0..n`: a part ring that already evicted some of this segment's
    /// parts cannot back the playlist's promise that they stay downloadable
    /// (RFC 8216bis §6.2.2), and a run with a hole would number the parts
    /// wrongly.
    fn resident_part_specs(&self, track_id: TrackId, seq: SegmentSeq, ext: &str) -> Vec<PartSpec> {
        // Keyed by index: a stale entry reusing a number is overwritten by
        // the newer one (publish order).
        let by_index: BTreeMap<PartIndex, PartEntry> = self
            .trunk
            .parts_in_segment(seq)
            .into_iter()
            .map(|p| (p.part_index, p))
            .collect();
        let contiguous = by_index
            .keys()
            .copied()
            .zip(0u32..)
            .all(|(index, expected)| index.value() == expected);
        if by_index.is_empty() || !contiguous {
            return Vec::new();
        }
        by_index
            .values()
            .map(|p| PartSpec {
                uri: self.part_name(track_id, p.segment_number, p.part_index, ext),
                // `Duration::as_secs_f64`: finite and >= 0 (issue #1140).
                duration: DecimalSeconds::new(p.duration.as_secs_f64())
                    .expect("Duration::as_secs_f64 is finite, >= 0"),
                independent: p.independent,
                ..Default::default()
            })
            .collect()
    }

    /// Resolve the `EXT-X-TARGETDURATION` to publish for `window`, freezing
    /// it at the first render that carries at least one segment (RFC 8216
    /// §4.3.3.1): once published it never changes, even as later (possibly
    /// longer) segments ratchet `max_segment_duration_secs`.
    fn published_target_duration_secs(&self, window: &WindowView) -> f64 {
        let candidate = self
            .target_duration_secs
            .max(window.max_segment_duration_secs);
        let mut frozen = locked(&self.frozen_target_duration_secs);
        if let Some(existing) = *frozen {
            existing
        } else if window.segments.is_empty() {
            candidate
        } else {
            *frozen = Some(candidate);
            candidate
        }
    }

    /// Render the LL-HLS media playlist for `track_id` from this origin's
    /// current `Window` (closed segments) and the `Trunk`'s live edge (open
    /// segment's parts + preload hint).
    ///
    /// RFC 8216bis §4.4.4.9: an in-progress (not yet closed) segment MUST NOT
    /// be advertised with an `#EXTINF`/URI pair — that segment has no
    /// fetchable resource yet — it may only appear as trailing `#EXT-X-PART`
    /// lines. `broadcast_hls::MediaPlaylist::open_segment` is exactly this
    /// representation: its parts render as trailing `#EXT-X-PART` lines with
    /// no `#EXTINF`/URI, so the in-progress segment's parts and the
    /// `#EXT-X-PRELOAD-HINT` for the next, not-yet-available part are both
    /// rendered by `to_m3u8()` itself — this method only supplies the URI
    /// scheme (`part-<track>-<seq>.<idx>.m4s`) and the part metadata.
    /// Returns the rendered playlist, or a human-readable reason the
    /// playlist could not be rendered (a field carrying a character
    /// forbidden in an HLS quoted-string/URI — audit BH-W7, issue
    /// #1111; unreachable for this origin, which only ever supplies
    /// its own generated URIs).
    fn render_playlist(
        &self,
        track_id: TrackId,
        skip: Option<SkipDirective>,
    ) -> core::result::Result<String, String> {
        self.drain();
        // Snapshot the window and release its lock before any `Trunk` call:
        // `live_edge`, the part lookups and the event lookups below take the
        // `Trunk`'s own lock, which must not nest inside this origin's
        // (audit r09-O4).
        let window = self.window_view();
        let (open_seq, open_parts) = self.live_edge();
        // Only render an open segment/preload-hint once the live edge is
        // genuinely a not-yet-closed segment with at least one live part —
        // never re-render an already-closed segment's lingering parts (the
        // `Trunk`'s live-part log deliberately does not evict them on close;
        // see `trunk`'s own module doc) as if they were still open.
        // Classic HLS (no `.low_latency(...)` call, issue #873) never
        // advertises an in-progress segment at all — RFC 8216bis §4.4.4.9's
        // trailing-`#EXT-X-PART`-only representation is itself an LL-HLS
        // directive, so it is gated on low latency being enabled, not merely
        // on the Trunk happening to have live parts.
        let low_latency_enabled = self.part_target_ms.is_some();
        let has_open_parts = low_latency_enabled && !open_parts.is_empty();
        let ext = self.container.segment_extension();

        let media_sequence = window
            .segments
            .front()
            .map(|s| self.public_msn(s.sequence_number))
            .or_else(|| has_open_parts.then(|| self.public_msn(open_seq)))
            .unwrap_or_else(|| self.public_msn(SegmentSeq::new(1)));
        // RFC 8216bis §4.4.3.1 (MUST): every Media Segment's EXTINF duration,
        // rounded to the nearest integer, MUST be <= TARGETDURATION. The
        // segmenter cuts on the next keyframe *after* the configured target,
        // so a real segment routinely exceeds it — advertising the
        // configured target alone can under-declare. Use whichever is
        // larger, rounded (not the configured value's `ceil()` alone).
        // RFC 8216 §4.3.3.1: `EXT-X-TARGETDURATION` is frozen once published.
        let target_duration = self.published_target_duration_secs(&window);
        // `target_duration_secs` is validated finite and positive at build
        // and segment durations come from `Duration`, so this is finite;
        // the clamp makes the float->int conversion exact-or-saturating.
        let target_duration = target_duration.ceil().clamp(0.0, f64::from(u32::MAX)) as u32;
        // RFC 8216bis §6.2.5.1: the Skip Boundary is at least six times the
        // Target Duration (advertised as CAN-SKIP-UNTIL). A delta update skips
        // the Media Segments further from the end of the last segment than
        // this boundary.
        let skip_boundary = if low_latency_enabled {
            Some(f64::from(target_duration) * SKIP_BOUNDARY_MULTIPLIER)
        } else {
            None
        };
        let playlist_has_endlist = false;
        let skip_accepted = skip.is_some() && skip_boundary.is_some() && !playlist_has_endlist;
        // RFC 8216bis §6.2.5.1: count the leading segments whose distance
        // (from their start to the end of the last segment) exceeds the Skip
        // Boundary; never skip every segment in the window.
        let omitted = if skip_accepted {
            let durations: Vec<f64> = window.segments.iter().map(|s| s.duration_secs).collect();
            let total: f64 = durations.iter().sum();
            let mut prefix = 0.0_f64;
            let mut count = 0usize;
            for d in &durations {
                let distance = total - prefix;
                if distance > skip_boundary.unwrap_or(f64::INFINITY) {
                    count += 1;
                    prefix += d;
                } else {
                    break;
                }
            }
            count.min(durations.len().saturating_sub(1))
        } else {
            0
        };
        // RFC 8216bis §6.2.2: EXT-X-PART tags stay in the Playlist until they
        // are more than three Target Durations from its end — so a closed
        // segment keeps its parts while the media after it (later closed
        // segments plus the open segment's parts) is within that distance.
        let retention_secs = PART_RETENTION_TARGET_DURATIONS * f64::from(target_duration);
        let mut behind_secs: f64 = if has_open_parts {
            open_parts.iter().map(|p| p.duration.as_secs_f64()).sum()
        } else {
            0.0
        };
        let mut closed_parts: Vec<Vec<PartSpec>> = Vec::with_capacity(window.segments.len());
        for s in window.segments.iter().rev() {
            let keep = low_latency_enabled && behind_secs <= retention_secs;
            closed_parts.push(if keep {
                self.resident_part_specs(track_id, s.sequence_number, ext)
            } else {
                Vec::new()
            });
            behind_secs += s.duration_secs;
        }
        closed_parts.reverse();
        let any_closed_parts = closed_parts.iter().any(|parts| !parts.is_empty());
        let fmp4 = self.container == Container::Fmp4;
        let map_for = |generation: Generation| {
            fmp4.then(|| MapTag {
                uri: self.init_name(track_id, generation),
                byte_range: None,
                extra_attrs: Vec::new(),
            })
        };
        // PDT per segment: explicit trunk time anchor (B1) wins, else the
        // arrival wall-clock source (B2a), else none. One private helper
        // keeps the media, open-segment and I-frame renderers in agreement.
        let segments: Vec<MediaSegment> = window
            .segments
            .iter()
            .zip(closed_parts)
            .map(|(s, parts)| {
                let program_date_time = self.segment_pdt(s.sequence_number, s.start_ns);
                MediaSegment {
                    map: map_for(s.init_gen),
                    uri: self.segment_name(track_id, s.sequence_number, ext),
                    // `duration_secs` is always `Duration::as_secs_f64()`
                    // (issue #1140): a `std::time::Duration` can never be
                    // negative or non-finite.
                    duration: DecimalSeconds::new(s.duration_secs)
                        .expect("Duration::as_secs_f64 is finite, >= 0"),
                    discontinuous: s.discontinuous,
                    parts,
                    program_date_time,
                    // A segment published with no media bytes is a gap placeholder
                    // (issue #1428 slice 2b): it occupies its MSN in the
                    // playlist without carrying media, so the client skips it.
                    gap: s.bytes.is_empty(),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        // Playlist Delta Update (RFC 8216bis §6.2.5.1): drop the segments
        // older than the Skip Boundary; MEDIA-SEQUENCE stays at the original
        // first segment's number and EXT-X-SKIP records how many were omitted.
        let segments: Vec<MediaSegment> = if omitted > 0 {
            segments.into_iter().skip(omitted).collect()
        } else {
            segments
        };
        // The open segment is cut against the init generation that covers its
        // sequence number (`#EXT-X-MAP` carries forward, so this repeats the
        // last closed segment's unless the init changed in between).
        let open_gen = locked(&self.init).generation_for(open_seq);
        let open_segment = has_open_parts.then(|| {
            let mut open = OpenSegment::new(
                open_parts
                    .iter()
                    .map(|p| PartSpec {
                        uri: self.part_name(track_id, p.segment_number, p.part_index, ext),
                        // `p.duration` is a `std::time::Duration`: always
                        // finite and non-negative (issue #1140).
                        duration: DecimalSeconds::new(p.duration.as_secs_f64())
                            .expect("Duration::as_secs_f64 is finite, >= 0"),
                        independent: p.independent,
                        ..Default::default()
                    })
                    .collect(),
            );
            if let Some(map) = map_for(open_gen) {
                open = open.with_map(map);
            }
            // Open (in-progress) segment PDT. The arrival path uses the last
            // closed segment's start + its duration (B2a); with no closed
            // segment yet it is omitted. The explicit-anchor path is handled
            // inside `segment_pdt` via `segment_start_time(open_seq)`.
            let open_start_ns = window.segments.back().map(|s| {
                s.start_ns.saturating_add(
                    u64::try_from((s.duration_secs * 1_000_000_000.0).round() as i64).unwrap_or(0),
                )
            });
            open.program_date_time = match open_start_ns {
                Some(sn) => self.segment_pdt(open_seq, sn),
                None => {
                    // No closed segment: explicit anchor only.
                    if self.trunk.time_anchor().is_some() {
                        self.segment_pdt(open_seq, 0)
                    } else {
                        None
                    }
                }
            };
            open
        });
        // RFC 8216bis Appendix B.1: "A Playlist that contains an EXT-X-PART
        // tag but no EXT-X-ENDLIST tag will also contain an
        // EXT-X-PRELOAD-HINT tag that specifies the next Partial Segment that
        // is expected to be added to the Playlist." Right after a segment
        // closes and before its successor has a part, the playlist still
        // carries the closed segments' EXT-X-PART tags, so the hint names
        // part 0 of the next Media Sequence Number (section 4.4.5.3).
        let next_part_hint = if has_open_parts {
            let next_idx = open_parts
                .iter()
                .map(|p| p.part_index)
                .max()
                .map(PartIndex::saturating_next)
                .unwrap_or(PartIndex::new(0));
            Some(self.part_name(track_id, open_seq, next_idx, ext))
        } else if low_latency_enabled && any_closed_parts {
            Some(self.part_name(track_id, open_seq.saturating_next(), PartIndex::new(0), ext))
        } else {
            None
        };
        // `#EXT-X-MAP`: unconditional under Fmp4 (RFC 8216bis §3.1.2 MUST);
        // omitted under MpegTs by default (§3.1.1's PAT/PMT-or-MAP
        // disjunction — see `Container`'s own doc for why this is a default,
        // not a hard restriction). Then any SCTE-35 cues published to the
        // trunk's event ring for this window render as `#EXT-X-DATERANGE`
        // tag lines (issue #965): they carry a wall-clock `START-DATE` only
        // once the trunk's `time_anchor` has been set, so events are
        // silently skipped while the anchor is absent (their `to_daterange`
        // fails the same way it does for a non-SCTE-35 source).
        // The map rides on the segments (each one names the init generation
        // it was cut against, so a changed init is announced together with its
        // `EXT-X-DISCONTINUITY`); only a playlist with no segment at all
        // carries it as a bare tag.
        // Date-range tags, collected per segment so a Playlist Delta
        // Update can drop the ones belonging to skipped segments (and, for
        // `_HLS_skip=v2`, report their ids in RECENTLY-REMOVED-DATERANGES).
        let mut segment_dateranges: Vec<Vec<(String, String)>> =
            vec![Vec::new(); window.segments.len()];
        let mut recently_removed_daterange_ids: Vec<String> = Vec::new();
        if let Some(anchor) = self.trunk.time_anchor() {
            let timeline = timed_metadata::Timeline::with_anchor(anchor);
            for (idx, seg) in window.segments.iter().enumerate() {
                for entry in self.trunk.events_in_segment(seg.sequence_number) {
                    if let Ok(dr) = timeline.to_daterange(&entry.event)
                        && let Ok(tag) = dr.to_tag_line()
                    {
                        if idx < omitted && skip_accepted && skip == Some(SkipDirective::V2) {
                            recently_removed_daterange_ids.push(dr.id.clone());
                        }
                        segment_dateranges[idx].push((tag, dr.id));
                    }
                }
            }
        }
        let mut extra_tags = Vec::new();
        if fmp4 && segments.is_empty() && open_segment.is_none() {
            let generation = locked(&self.init).generation.max(Generation::new(1));
            extra_tags.push(format!(
                "#EXT-X-MAP:URI=\"{}\"",
                self.init_name(track_id, generation)
            ));
        }
        for (idx, tags) in segment_dateranges.iter().enumerate() {
            // RFC 8216bis §6.2.5.1: only `_HLS_skip=v2` additionally omits
            // `EXT-X-DATERANGE` tags; with `YES` every daterange stays.
            if idx < omitted && skip == Some(SkipDirective::V2) {
                continue;
            }
            for (tag, _) in tags {
                extra_tags.push(tag.clone());
            }
        }
        let low_latency = self.part_target_ms.map(|part_target_ms| {
            // `part_target_ms: u32` / 1000.0 (issue #1140): always finite
            // and non-negative, as is the product with the positive
            // `PART_HOLD_BACK_MULTIPLIER` constant.
            let part_target = DecimalSeconds::new(f64::from(part_target_ms) / 1000.0)
                .expect("u32 / 1000.0 is finite, >= 0");
            let part_hold_back = DecimalSeconds::new(part_target.get() * PART_HOLD_BACK_MULTIPLIER)
                .expect("finite, >= 0 times a positive constant is finite, >= 0");
            LowLatencyConfig {
                part_target: Some(part_target),
                part_hold_back: Some(part_hold_back),
                // RFC 8216bis §4.4.3.8: HOLD-BACK MUST be >= 3x the Target
                // Duration; Apple's validator also requires it in LL mode.
                hold_back: DecimalSeconds::new(f64::from(target_duration) * HOLD_BACK_MULTIPLIER)
                    .ok(),
                preload_hint_part: next_part_hint,
                can_skip_until: skip_boundary.and_then(|b| DecimalSeconds::new(b).ok()),
                can_skip_dateranges: true,
                can_block_reload: true,
                ..Default::default()
            }
        });
        let skip_info = if skip_accepted {
            Some(broadcast_hls::SkipInfo {
                skipped_segments: omitted as u64,
                recently_removed_daterange_ids,
                extra_attrs: Vec::new(),
            })
        } else {
            None
        };
        // #EXT-X-RENDITION-REPORT for sibling renditions (RFC 8216bis
        // §4.4.5.4): one per OTHER rendition, snapshotted under each
        // sibling's own lock at render time.
        let rendition_reports = if self.part_target_ms.is_some() {
            let siblings = locked(&self.rendition_siblings).clone();
            let mut reports = Vec::with_capacity(siblings.len());
            for sibling in siblings {
                let Some(origin) = sibling.origin.upgrade() else {
                    continue;
                };
                if sibling.playlist_uri == self.playlist_uri {
                    continue;
                }
                let uri = relative_playlist_uri(&self.playlist_uri, &sibling.playlist_uri);
                let (last_msn, last_part) = origin.current_live_edge();
                reports.push(broadcast_hls::RenditionReport {
                    uri,
                    last_msn,
                    last_part,
                    extra_attrs: Vec::new(),
                });
            }
            reports
        } else {
            Vec::new()
        };

        let playlist = MediaPlaylist {
            // No explicit floor: `broadcast_hls::MediaPlaylist::to_m3u8`
            // computes `EXT-X-VERSION` from the content actually emitted
            // (RFC 8216bis §8) rather than this origin choosing a value
            // ahead of time (issue #871) — none of the LL-HLS directives
            // below (`EXT-X-PART`/`EXT-X-PART-INF`/`EXT-X-PRELOAD-HINT`/
            // `EXT-X-SERVER-CONTROL`) carry any version requirement at all.
            target_duration,
            media_sequence,
            discontinuity_sequence: window.discontinuity_sequence,
            always_emit_discontinuity_sequence: true,
            segments,
            open_segment,
            endlist: false,
            extra_tags,
            low_latency,
            iframes_only: false,
            skip: skip_info,
            rendition_reports,
            ..Default::default()
        };
        playlist.to_m3u8().map_err(|error| error.to_string())
    }

    fn resolve_playlist(
        &self,
        track_id: TrackId,
        query: BlockingQuery,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<HlsBody> {
        if query.hls_part.is_some() && query.hls_msn.is_none() {
            return EgressResponse::BadRequest {
                reason: "_HLS_part without _HLS_msn is meaningless",
            };
        }
        if let Some(msn) = query.hls_msn {
            let (in_progress_seg, live_parts) = self.live_edge();
            let in_progress = self.public_msn(in_progress_seg);
            if msn > in_progress.saturating_add(ABUSE_MSN_FUTURE_BOUND) {
                return EgressResponse::BadRequest {
                    reason: "_HLS_msn unreasonably far beyond the live edge",
                };
            }
            // RFC 8216bis §6.2.5.2 (SHOULD): an `_HLS_part` that exceeds the
            // last Partial Segment in the Playlist by the Advance Part Limit
            // is a Bad Request at once, not a request to park. Measured
            // against the live-edge segment's parts (none exist yet for a
            // later `_HLS_msn`).
            if let Some(part) = query.hls_part
                && msn >= in_progress
            {
                let existing = if msn == in_progress {
                    u64::try_from(live_parts.len()).unwrap_or(u64::MAX)
                } else {
                    0
                };
                let limit = existing
                    .saturating_sub(1)
                    .saturating_add(self.advance_part_limit());
                if u64::from(part) > limit {
                    return EgressResponse::BadRequest {
                        reason: "_HLS_part beyond the Advance Part Limit",
                    };
                }
            }
            let satisfied = match query.hls_part {
                Some(part) => {
                    in_progress > msn
                        || (in_progress == msn
                            && u64::try_from(live_parts.len()).unwrap_or(u64::MAX)
                                > u64::from(part))
                }
                None => {
                    self.public_msn(
                        self.trunk
                            .last_closed_segment()
                            .unwrap_or(SegmentSeq::new(0)),
                    ) >= msn
                }
            };
            if !satisfied {
                return EgressResponse::pending(await_policy, now, now);
            }
        }
        match self.render_playlist(track_id, query.hls_skip) {
            Ok(playlist) => EgressResponse::Ready {
                body: HlsBody::Playlist(playlist),
                cache: CachePolicy::NoCache,
            },
            // A field the renderer would have to quote carries a character
            // forbidden there (audit BH-W7, issue #1111) — unreachable for
            // this origin's own generated URIs, but never emitted mangled.
            // Answered as a plain miss, never a debug panic: a hostile or
            // malformed data-driven field must not take the origin down.
            Err(_) => EgressResponse::NotFound,
        }
    }

    fn resolve_iframe_playlist(
        &self,
        track_id: TrackId,
        query: BlockingQuery,
        media_playlist_name: String,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<HlsBody> {
        // An I-frame playlist has no parts, so any `_HLS_part` is meaningless.
        if query.hls_part.is_some() {
            return EgressResponse::BadRequest {
                reason: "_HLS_part is not valid on an I-frame playlist",
            };
        }
        if let Some(msn) = query.hls_msn {
            let last_closed = self
                .trunk
                .last_closed_segment()
                .unwrap_or(SegmentSeq::new(0));
            if msn
                > self
                    .public_msn(last_closed.saturating_next())
                    .saturating_add(ABUSE_MSN_FUTURE_BOUND)
            {
                return EgressResponse::BadRequest {
                    reason: "_HLS_msn unreasonably far beyond the live edge",
                };
            }
            let satisfied = self.public_msn(last_closed) >= msn;
            if !satisfied {
                return EgressResponse::pending(await_policy, now, now);
            }
        }
        match self.render_iframe_playlist(track_id, &media_playlist_name) {
            Ok(playlist) => EgressResponse::Ready {
                body: HlsBody::Playlist(playlist),
                cache: CachePolicy::NoCache,
            },
            Err(_) => EgressResponse::NotFound,
        }
    }

    /// Render the I-frame trick-play playlist for `track_id`.
    fn render_iframe_playlist(
        &self,
        track_id: TrackId,
        media_playlist_name: &str,
    ) -> core::result::Result<String, String> {
        self.drain();
        if self.container != Container::Fmp4 {
            return Err("not fmp4".to_string());
        }
        if locked(&self.video_track_id).is_none() {
            return Err("no video track".to_string());
        }
        let window = self.window_view();
        // D3': collect entries; a `None` is allowed only before the first entry.
        let Some(entries) = self.iframe_entries(&window) else {
            return Err("no iframe entry".to_string());
        };

        // RFC 8216 §4.3.3.1: `EXT-X-TARGETDURATION` is frozen once published.
        let target_duration = self.published_target_duration_secs(&window);
        let target_duration = target_duration.ceil().clamp(0.0, f64::from(u32::MAX)) as u32;

        let fmp4 = self.container == Container::Fmp4;
        let map_for = |generation: Generation| {
            fmp4.then(|| MapTag {
                uri: self.init_name(track_id, generation),
                byte_range: None,
                extra_attrs: Vec::new(),
            })
        };

        let first_entry_seq = entries[0].0.sequence_number;
        let media_sequence = self.public_msn(first_entry_seq);
        let segments: Vec<MediaSegment> = entries
            .iter()
            .map(|(s, len)| MediaSegment {
                map: map_for(s.init_gen),
                uri: self.iframe_segment_name(
                    track_id,
                    s.sequence_number,
                    self.container.segment_extension(),
                ),
                duration: DecimalSeconds::new(s.duration_secs).unwrap_or_default(),
                discontinuous: s.discontinuous,
                byte_range: Some(ByteRange {
                    length: *len,
                    offset: Some(0),
                }),
                parts: Vec::new(),
                program_date_time: self.segment_pdt(s.sequence_number, s.start_ns),
                ..Default::default()
            })
            .collect();

        let low_latency = if let Some(part_target_ms) = self.part_target_ms {
            let part_target = DecimalSeconds::new(f64::from(part_target_ms) / 1000.0)
                .expect("u32 / 1000.0 is finite, >= 0");
            let part_hold_back = DecimalSeconds::new(part_target.get() * PART_HOLD_BACK_MULTIPLIER)
                .expect("finite, >= 0 times a positive constant is finite, >= 0");
            Some(broadcast_hls::LowLatencyConfig {
                part_target: Some(part_target),
                part_hold_back: Some(part_hold_back),
                hold_back: DecimalSeconds::new(f64::from(target_duration) * HOLD_BACK_MULTIPLIER)
                    .ok(),
                can_skip_until: DecimalSeconds::new(
                    f64::from(target_duration) * SKIP_BOUNDARY_MULTIPLIER,
                )
                .ok(),
                can_skip_dateranges: true,
                can_block_reload: true,
                ..Default::default()
            })
        } else {
            None
        };

        // Rendition report pointing back at the primary media playlist (RFC
        // 8216bis §4.4.5.4 SHOULD; Apple accepts iframe->media reports).
        let rendition_reports = if self.part_target_ms.is_some() {
            let last_closed = self
                .trunk
                .last_closed_segment()
                .unwrap_or(entries[0].0.sequence_number);
            let mut reports = vec![broadcast_hls::RenditionReport {
                uri: media_playlist_name.to_string(),
                last_msn: self.public_msn(last_closed),
                last_part: None,
                extra_attrs: Vec::new(),
            }];
            // Split-audio routes: report each audio sibling rendition as well
            // (RFC 8216bis §4.4.5.4), keeping the existing iframe->media
            // report above unchanged.
            let siblings = locked(&self.rendition_siblings).clone();
            for sibling in siblings {
                let Some(origin) = sibling.origin.upgrade() else {
                    continue;
                };
                if sibling.playlist_uri == self.playlist_uri {
                    continue;
                }
                let uri = relative_playlist_uri(
                    &self.iframe_playlist_name(track_id),
                    &sibling.playlist_uri,
                );
                let (last_msn, last_part) = origin.current_live_edge();
                reports.push(broadcast_hls::RenditionReport {
                    uri,
                    last_msn,
                    last_part,
                    extra_attrs: Vec::new(),
                });
            }
            reports
        } else {
            Vec::new()
        };

        let playlist = MediaPlaylist {
            target_duration,
            media_sequence,
            discontinuity_sequence: window.discontinuity_sequence,
            always_emit_discontinuity_sequence: true,
            segments,
            open_segment: None,
            endlist: false,
            extra_tags: Vec::new(),
            low_latency,
            rendition_reports,
            iframes_only: true,
            ..Default::default()
        };
        playlist.to_m3u8().map_err(|error| error.to_string())
    }

    /// A part request is the preload-hinted Partial Segment a client fetches
    /// ahead of time (RFC 8216bis §6.2.2, §6.3.1). If the origin promised it
    /// via `#EXT-X-PRELOAD-HINT` but hasn't produced it yet,
    /// [`EgressResponse::Await`] — the caller should hold the request open
    /// (not 404 immediately, which spams errors and defeats low latency).
    /// [`EgressResponse::NotFound`] is returned **promptly** (without the
    /// caller needing to wait out its own [`AwaitPolicy`]) once the part can
    /// no longer appear: its segment has closed (now only addressable as a
    /// whole segment via `seg-…`) — a legitimate 404 the client answers by
    /// fetching the next segment/part.
    fn resolve_resource(
        &self,
        name: &str,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<HlsBody> {
        if let Some((epoch, public_seq, idx)) = parse_part(name, self.container) {
            // Another instance's token never resolves here; the legacy
            // (token-less) form resolves but is not `immutable`.
            if epoch.is_some_and(|e| e != self.instance) {
                return EgressResponse::NotFound;
            }
            let Some(seq) = self.internal_seq(public_seq) else {
                return EgressResponse::NotFound;
            };
            if let Some(bytes) = self.trunk.part_bytes(seq, idx) {
                return EgressResponse::Ready {
                    body: HlsBody::Resource(bytes),
                    cache: if epoch.is_some() {
                        CachePolicy::Immutable
                    } else {
                        CachePolicy::NoCache
                    },
                };
            }
            // A classic (non-LL) origin has no parts at all.
            if self.part_target_ms.is_none() {
                return EgressResponse::NotFound;
            }
            // The requested part's segment has already closed (whether or
            // not this origin's own `Window` still retains its bytes) -> it
            // will never be produced. `Trunk::last_closed_segment` answers
            // this exactly, with no dependence on `Window`'s retention.
            let never_will = self.trunk.last_closed_segment().is_some_and(|c| c >= seq);
            if never_will {
                return EgressResponse::NotFound;
            }
            // Only a part a client could have been hinted at is worth holding
            // a request open for (RFC 8216bis §6.2.6): the hinted next part,
            // in the open segment or the one after it. Anything else is not
            // found at once rather than parked for the whole blocking
            // timeout.
            let last_closed = self
                .trunk
                .last_closed_segment()
                .unwrap_or(SegmentSeq::new(0));
            if last_closed
                .checked_add(FUTURE_PART_SEGMENT_BOUND)
                .is_some_and(|limit| seq > limit)
            {
                return EgressResponse::NotFound;
            }
            // Exactly one part is ever hinted: the one after the last part
            // the ring holds for that segment (index 0 when it holds none).
            // A lower index that is not resident was evicted (or never
            // existed) and will not come back; a higher one was never hinted.
            let next_hinted = self
                .trunk
                .parts_in_segment(seq)
                .iter()
                .map(|p| p.part_index)
                .max()
                .map_or(PartIndex::new(0), PartIndex::saturating_next);
            if idx != next_hinted {
                return EgressResponse::NotFound;
            }
            return EgressResponse::pending(await_policy, now, now);
        }
        self.drain();
        // `Immutable` only where the name can never map to other bytes: the
        // instance-named init/segment forms (see `Self::instance`). The
        // token-less forms and the bare `init-{track}.mp4` can.
        let (bytes, cache) = match parse_immediate(name, self.container) {
            Some(ImmediateResource::CurrentInit) => (self.init_bytes(), CachePolicy::NoCache),
            Some(ImmediateResource::VersionedInit { epoch, generation })
                if epoch == self.instance =>
            {
                (
                    locked(&self.init).bytes_of(generation),
                    CachePolicy::Immutable,
                )
            }
            Some(ImmediateResource::Segment { epoch, seq })
                if epoch.is_none_or(|e| e == self.instance) =>
            {
                (
                    self.internal_seq(seq)
                        .and_then(|s| locked(&self.window).bytes_of(s)),
                    if epoch.is_some() {
                        CachePolicy::Immutable
                    } else {
                        CachePolicy::NoCache
                    },
                )
            }
            _ => (None, CachePolicy::NoCache),
        };
        match bytes {
            Some(bytes) => EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                cache,
            },
            None => EgressResponse::NotFound,
        }
    }
}

impl ServedEgress for HlsOrigin {
    type Request = HlsRequest;
    type Body = HlsBody;

    fn resolve(
        &self,
        request: HlsRequest,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<HlsBody> {
        match request {
            HlsRequest::Playlist { track_id, query } => {
                self.resolve_playlist(track_id, query, now, await_policy)
            }
            HlsRequest::IFramePlaylist {
                track_id,
                query,
                media_playlist_name,
            } => self.resolve_iframe_playlist(
                track_id,
                query,
                media_playlist_name,
                now,
                await_policy,
            ),
            HlsRequest::Resource { name } => self.resolve_resource(&name, now, await_policy),
        }
    }
}

/// Canonicalise an ISO 639-2 language code into a BCP 47 (RFC 5646) primary
/// language subtag, as required by RFC 8216bis `EXT-X-MEDIA` `LANGUAGE`.
///
/// HLS `LANGUAGE` must be an RFC 5646 (BCP 47) language tag; in BCP 47 the
/// primary subtag is the ISO 639-1 two-letter code when one exists, otherwise
/// the ISO 639-2/T (== ISO 639-3) three-letter code. ISO 639-2/B
/// bibliographic codes (`fre`, `ger`, `dut`, `chi`, ...) are not valid BCP 47
/// subtags and are mapped to their terminology (T) equivalents first.
///
/// `qaa`..`qtz` (private use), `und`, `mul`, `mis` and `zxx` pass through
/// unchanged. Unknown or malformed codes return `None`.
pub fn bcp47_language(iso639_2: &str) -> Option<String> {
    let code = iso639_2.trim().to_ascii_lowercase();
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    // Private-use range and the reserved singleton codes pass through verbatim.
    if code == "und" || code == "mul" || code == "mis" || code == "zxx" {
        return Some(code);
    }
    if code.starts_with('q') {
        let bytes = code.as_bytes();
        if (b'a'..=b't').contains(&bytes[1]) && bytes[2].is_ascii_lowercase() {
            return Some(code);
        }
    }
    let lang = iso_639::Language::from_str(&code)?;
    if let Some(a2) = lang.alpha2() {
        return Some(a2.to_string());
    }
    if let Some(t) = lang.alpha3_t() {
        return Some(t.to_string());
    }
    lang.alpha3_b().map(|b| b.to_string())
}

/// Relative URI from one playlist to another (RFC 3986 relative reference),
/// computed over slash-separated path segments. The carrying playlist's own
/// filename is dropped before differencing, so two playlists in the same
/// directory resolve to a bare filename.
fn relative_playlist_uri(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = match from.rfind('/') {
        Some(i) => from[..i].split('/').filter(|s| !s.is_empty()).collect(),
        None => Vec::new(),
    };
    let to_segs: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let mut common = 0usize;
    while common < from_dir.len() && common < to_segs.len() && from_dir[common] == to_segs[common] {
        common += 1;
    }
    let mut parts: Vec<&str> = Vec::new();
    parts.extend(std::iter::repeat_n(
        "..",
        from_dir.len().saturating_sub(common),
    ));
    parts.extend(&to_segs[common..]);
    if parts.is_empty() {
        // Same path: emit the target's last segment (or empty).
        to_segs.last().copied().unwrap_or("").to_string()
    } else {
        parts.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::trunk::{EventAnchor, TrunkConfig};
    use std::time::{Duration, Instant};
    use timed_metadata::{MediaTime, TimeAnchor};
    use transmux::SegmentMeta;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    /// A fresh `Trunk` sized generously for these tests, plus the one
    /// `HlsOrigin` under test — `Fmp4` + low-latency, matching every
    /// pre-#873 test in this module (regression guard).
    fn make_origin() -> (Arc<Trunk>, HlsOrigin, media_plane::trunk::SegmentWriter) {
        let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
        let writer = trunk.segment_writer().expect("first segment writer");
        let origin = HlsOrigin::builder(Arc::clone(&trunk))
            .target_duration_secs(4.0)
            .window_segments(nz(4))
            .low_latency(500)
            .instance(7)
            .build()
            .expect("both required fields set");
        origin.set_init(vec![0xAAu8; 8]);
        (trunk, origin, writer)
    }

    /// #1159: the resource-naming functions take distinct id types, and every
    /// argument lands in its own slot of the wire name. All six numbers differ
    /// (track 3, instance 7, generation 4, sequence 5, part 2, offset 100), so
    /// a swapped or misrouted argument changes the exact string.
    #[test]
    fn resource_names_place_every_distinct_id_in_its_own_slot() {
        let (_trunk, origin, _writer) = make_origin();
        let track = TrackId::new(3);
        assert_eq!(origin.instance(), 7);
        assert_eq!(
            origin.init_name(track, Generation::new(4)),
            "init-3-7-4.mp4"
        );
        assert_eq!(
            origin.segment_name(track, SegmentSeq::new(5), "m4s"),
            "seg-3-7-5.m4s"
        );
        assert_eq!(
            origin.part_name(track, SegmentSeq::new(5), PartIndex::new(2), "m4s"),
            "part-3-7-5.2.m4s"
        );

        let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
        let offset = HlsOrigin::builder(trunk)
            .target_duration_secs(4.0)
            .window_segments(nz(4))
            .instance(7)
            .media_sequence_offset(100)
            .build()
            .expect("both required fields set");
        assert_eq!(
            offset.segment_name(track, SegmentSeq::new(5), "m4s"),
            "seg-3-7-105.m4s",
            "the Media Sequence Number offset applies to the sequence slot only"
        );
        assert_eq!(
            offset.part_name(track, SegmentSeq::new(5), PartIndex::new(2), "m4s"),
            "part-3-7-105.2.m4s"
        );
        assert_eq!(
            offset.init_name(track, Generation::new(4)),
            "init-3-7-4.mp4",
            "the offset never touches the generation"
        );
    }

    fn seg(
        writer: &media_plane::trunk::SegmentWriter,
        seq: u32,
        duration_secs: f64,
        discontinuous: bool,
    ) {
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![seq as u8; 8]),
                SegmentSeq::new(seq),
                Duration::from_secs_f64(duration_secs),
                Timestamp::from_nanos(0),
                SegmentMeta { discontinuous },
            ))
            .unwrap();
    }

    fn part(writer: &media_plane::trunk::SegmentWriter, seg_no: u32, idx: u32, independent: bool) {
        writer.publish_part(PartEntry::new(
            Bytes::from(vec![idx as u8; 4]),
            SegmentSeq::new(seg_no),
            PartIndex::new(idx),
            Duration::from_millis(500),
            independent,
        ));
    }

    fn resolve_now(origin: &HlsOrigin, request: HlsRequest) -> EgressResponse<HlsBody> {
        origin.resolve(
            request,
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(0)),
        )
    }

    // --- EXT-X-DATERANGE rendering from the trunk event ring (#965) -----

    /// MUTATION VERIFIED: dropping the `events_in_segment` loop (or the
    /// `time_anchor()` gate) from `render_playlist` makes all three
    /// assertions below fail — the playlist no longer carries the
    /// `#EXT-X-DATERANGE` line, its `SCTE35-OUT=` attribute, or its
    /// `ID="2002"`. `if let Ok(dr)` (rather than `unwrap`) is the deliberate
    /// skip, not asserted here, because `time_anchor` is set in this test so
    /// no event legitimately fails conversion.
    #[test]
    fn trunk_events_render_as_ext_x_daterange() {
        // splice_insert, event_id=2002, pts=2_160_000 (~24s at 90kHz) — the
        // same fixture timed-metadata's own timeline tests use.
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        let splice_bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();

        let (trunk, origin, writer) = make_origin();

        // Give the event log a wall-clock anchor so drafts can carry a
        // START-DATE, and a media-clock one so the event resolves immediately.
        writer.set_time_anchor(TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000_000_000_000,
        });

        // Parse the splice into a TimedEvent and publish it to the event ring,
        // Media-anchored at its own PTS (2_160_000, unwrapped on first call).
        let mut timeline = timed_metadata::Timeline::new();
        let ev = timeline.push_scte35(&splice_bytes).unwrap();
        let trunk_writer = trunk.writer().expect("first trunk writer");
        trunk_writer.publish_event(
            ev.clone(),
            EventAnchor::Media(ev.at.unwrap_or(MediaTime(0))),
        );

        // Segment 1 spans [0, 4_000_000) — includes the cue's PTS.
        writer.note_segment_start(SegmentSeq::new(1), MediaTime(0));
        writer.note_segment_start(SegmentSeq::new(2), MediaTime(4_000_000));
        seg(&writer, 1, 4.0, false);

        let playlist = origin
            .render_playlist(DEFAULT_TRACK_ID, None)
            .expect("render");
        assert!(
            playlist.contains("#EXT-X-DATERANGE"),
            "playlist should contain EXT-X-DATERANGE, got: {playlist}"
        );
        assert!(
            playlist.contains("SCTE35-OUT="),
            "daterange should carry SCTE35-OUT attribute: {playlist}"
        );
        assert!(
            playlist.contains("ID=\"2002\""),
            "daterange ID should be the event_id: {playlist}"
        );
    }

    /// MUTATION VERIFIED: replacing the `if let Ok(dr)` guard in
    /// `render_playlist` with a bare `timeline.to_daterange(&entry.event)`
    /// (i.e. unwrapping the `Err`) makes this test fail — an Emsg-sourced
    /// event that `to_daterange` rejects ("event is not SCTE-35-sourced")
    /// would panic `render_playlist` instead of being skipped. This test
    /// pins that skip: with a time anchor set and the event inside the
    /// segment span, no `#EXT-X-DATERANGE` line may appear (and rendering
    /// must not panic).
    #[test]
    fn non_scte35_events_are_silently_skipped() {
        use timed_metadata::event::{EventKind, SourcePayload};
        use timed_metadata::{MediaDuration, TimedEvent};

        let (trunk, origin, writer) = make_origin();
        writer.set_time_anchor(TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000_000_000_000,
        });

        // An Emsg-sourced event, Media-anchored inside segment 1's span — the
        // kinds of event that legitimately cannot become a DATERANGE.
        let emsg_ev = TimedEvent {
            id: Some(7),
            kind: EventKind::Unspecified,
            at: Some(MediaTime(2_000_000)),
            duration: Some(MediaDuration(2_000_000)),
            source: SourcePayload::Emsg {
                scheme_id_uri: "urn:example".to_string(),
                value: "1".to_string(),
                raw: vec![0x01, 0x02],
            },
        };
        let trunk_writer = trunk.writer().expect("first trunk writer");
        trunk_writer.publish_event(emsg_ev, EventAnchor::Media(MediaTime(2_000_000)));

        writer.note_segment_start(SegmentSeq::new(1), MediaTime(0));
        writer.note_segment_start(SegmentSeq::new(2), MediaTime(4_000_000));
        seg(&writer, 1, 4.0, false);

        let playlist = origin
            .render_playlist(DEFAULT_TRACK_ID, None)
            .expect("render");
        assert!(
            !playlist.contains("#EXT-X-DATERANGE"),
            "no DATERANGE expected for a non-SCTE-35 event: {playlist}"
        );
    }

    // --- master playlist (unaffected by the Trunk migration) -------------

    #[test]
    fn master_playlist_has_stream_inf() {
        let m = master_playlist_m3u8("media.m3u8", 5_000_000);
        assert!(m.contains("#EXTM3U"));
        assert!(m.contains("#EXT-X-STREAM-INF"));
        assert!(m.contains("media.m3u8"));
    }

    #[test]
    fn master_playlist_points_at_configured_playlist_name() {
        let m = master_playlist_m3u8("index.m3u8", 5_000_000);
        assert!(m.contains("index.m3u8"));
        assert!(!m.contains("media.m3u8"));
    }

    // --- 1. playlist rendered from a populated Trunk matches the expected
    //        shape ---------------------------------------------------------

    /// MUTATION VERIFIED: changing `render_playlist`'s
    /// `low_latency: Some(...)` to `None` makes this test's
    /// `assert!(m.contains("#EXT-X-PART-INF"))` (and every other
    /// LL-HLS-tag assertion) fail — `to_m3u8()` omits the entire
    /// low-latency header block when `low_latency` is `None`, so none of
    /// `#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`/`#EXT-X-PART` appear in the
    /// rendered body. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn playlist_rendered_from_populated_trunk_matches_expected_shape() {
        let (_trunk, origin, writer) = make_origin();
        seg(&writer, 1, 4.0, false);
        part(&writer, 2, 0, true);
        part(&writer, 2, 1, false);

        let body = match resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(m),
                cache,
            } => {
                assert_eq!(cache, CachePolicy::NoCache);
                m
            }
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };

        // RFC 8216bis §8 (issue #871): this playlist's true minimum is 6
        // (EXT-X-MAP without EXT-X-I-FRAMES-ONLY) — none of the LL-HLS
        // directives it also carries require any version at all. The old
        // hardcoded `EXT-X-VERSION:9` over-declared and would have locked
        // out every client on protocol version 6, 7, or 8.
        assert!(body.contains("#EXT-X-VERSION:6"), "body: {body}");
        assert!(!body.contains("#EXT-X-VERSION:9"), "body: {body}");
        assert!(body.contains("#EXT-X-TARGETDURATION:4"), "body: {body}");
        assert!(
            body.contains("#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5"),
            "body: {body}"
        );
        assert!(
            body.contains("#EXT-X-PART-INF:PART-TARGET=0.5"),
            "body: {body}"
        );
        assert!(
            body.contains("#EXT-X-MAP:URI=\"init-1-7-1.mp4\""),
            "body: {body}"
        );
        assert!(body.contains("seg-1-7-1.m4s"), "body: {body}");
        assert!(
            body.contains("#EXT-X-PART:DURATION=0.5") && body.contains("INDEPENDENT=YES"),
            "body: {body}"
        );
        assert!(body.contains("#EXT-X-PRELOAD-HINT"), "body: {body}");
        assert!(
            body.contains("part-1-7-2.2.m4s"),
            "preload hint for the next part: {body}"
        );
    }

    /// Issue #1577. RFC 8216bis Appendix B.1: a Playlist with an EXT-X-PART
    /// tag and no EXT-X-ENDLIST names the next Partial Segment in an
    /// EXT-X-PRELOAD-HINT. Right after segment 1 closes, with no part of
    /// segment 2 yet, the hint is part 0 of segment 2, and the server holds a
    /// request for that resource open (section 6.2.6) rather than 404ing it.
    #[test]
    fn preload_hint_present_between_segments() {
        let (_trunk, origin, writer) = make_origin();
        part(&writer, 1, 0, true);
        part(&writer, 1, 1, false);
        seg(&writer, 1, 4.0, false);

        let body = render_body(&origin);
        assert!(body.contains("#EXT-X-PART:"), "body: {body}");
        assert!(
            body.contains("#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-7-2.0.m4s\""),
            "hint names part 0 of the next segment: {body}"
        );

        let first = origin.resolve(
            HlsRequest::Resource {
                name: "part-1-7-2.0.m4s".to_string(),
            },
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(5_000_000_000)),
        );
        assert!(
            matches!(first, EgressResponse::Await { .. }),
            "the hinted resource blocks, got {first:?}"
        );
        part(&writer, 2, 0, true);
        let served = resolve_now(
            &origin,
            HlsRequest::Resource {
                name: "part-1-7-2.0.m4s".to_string(),
            },
        );
        assert!(
            matches!(served, EgressResponse::Ready { .. }),
            "served once produced, got {served:?}"
        );
    }

    // --- 2. a preload-hinted part BLOCKS until produced, then serves -----

    /// MUTATION VERIFIED: changing `resolve_resource`'s `never_will` check
    /// (whether the requested part's segment has already closed, via
    /// `last_closed_segment`) to always `true` ("never will produce this
    /// part") makes this test's first assertion fail: the not-yet-produced
    /// part resolves `NotFound` immediately instead of `Await`, so
    /// `assert!(matches!(first, EgressResponse::Await { .. }))` sees
    /// `NotFound` and fails. Recompiled and re-run to confirm the failure,
    /// then reverted. This is the RFC 8216bis section 6.2.2 behaviour that
    /// shipped as multimux 0.2.1's bug fix — regressing it would break the
    /// live camera route.
    #[test]
    fn preload_hinted_part_blocks_until_produced_then_serves() {
        let (trunk, origin, writer) = make_origin();
        let origin = Arc::new(origin);

        // Not produced yet: must Await, not NotFound.
        let deadline = Timestamp::from_nanos(5_000_000_000);
        let policy = AwaitPolicy::new(deadline);
        let first = origin.resolve(
            HlsRequest::Resource {
                name: "part-1-7-1.0.m4s".to_string(),
            },
            Timestamp::from_nanos(0),
            policy,
        );
        assert!(
            matches!(first, EgressResponse::Await { .. }),
            "expected Await before the part exists, got {first:?}"
        );

        // Register a real Trunk::listen() wake-up and block a worker thread
        // on it -- the actual mechanism a real adapter (Step 5) uses, not a
        // poll loop -- to prove the part genuinely blocks rather than
        // merely returning Await once and never resolving.
        let listener = trunk.listen().expect("listener slot available");
        let woken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let woken2 = std::sync::Arc::clone(&woken);
        // HANG GUARD (issue #807): deliberately generous, same reasoning as
        // `media-plane/src/trunk.rs`'s own `Trunk::listen()` wake tests --
        // the claim is "wakes rather than parking forever", not "wakes
        // within N seconds"; the publish happens on another thread, so a
        // tight bound would measure the machine's scheduler, not this code.
        let waiter = std::thread::spawn(move || {
            let ok = listener.wait_deadline(Instant::now() + Duration::from_secs(60));
            woken2.store(ok, std::sync::atomic::Ordering::SeqCst);
        });

        // Produce the part the request was waiting on.
        part(&writer, 1, 0, true);

        waiter.join().expect("waiter thread must not panic");
        assert!(
            woken.load(std::sync::atomic::Ordering::SeqCst),
            "Trunk::listen() must wake once publish_part lands"
        );

        // Re-resolving now must serve it -- not 404.
        match origin.resolve(
            HlsRequest::Resource {
                name: "part-1-7-1.0.m4s".to_string(),
            },
            Timestamp::from_nanos(1),
            policy,
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                cache,
            } => {
                assert_eq!(bytes, Bytes::from(vec![0u8; 4]));
                assert_eq!(cache, CachePolicy::Immutable);
            }
            other => panic!("expected Ready once produced, got {other:?}"),
        }
    }

    /// MUTATION VERIFIED: removing `EgressResponse::pending`'s expiry check
    /// (i.e. always returning `Await`) would make a client wait forever for
    /// a part that will never exist -- this test proves the OTHER half of
    /// the bound: once `now` reaches the caller's own `AwaitPolicy::deadline`,
    /// resolve must stop Awaiting. Changing the deadline comparison in
    /// `resolve_resource`'s `EgressResponse::pending(await_policy, now, now)`
    /// call to ignore `now` (always pass `Timestamp::from_nanos(0)`) makes
    /// this test's final assertion fail: `resolve` at `now == deadline`
    /// keeps returning `Await` instead of `NotFound`. Recompiled and re-run
    /// to confirm the failure, then reverted.
    #[test]
    fn awaiting_part_is_bounded_by_await_policy_deadline() {
        let (_trunk, origin, _writer) = make_origin();
        let deadline = Timestamp::from_nanos(1_000_000_000);
        let policy = AwaitPolicy::new(deadline);

        let still_waiting = origin.resolve(
            HlsRequest::Resource {
                name: "part-1-7-1.0.m4s".to_string(),
            },
            Timestamp::from_nanos(999_999_999),
            policy,
        );
        assert!(matches!(still_waiting, EgressResponse::Await { .. }));

        let expired = origin.resolve(
            HlsRequest::Resource {
                name: "part-1-7-1.0.m4s".to_string(),
            },
            deadline,
            policy,
        );
        assert!(
            matches!(expired, EgressResponse::NotFound),
            "expected NotFound once the deadline passed, got {expired:?}"
        );
    }

    // --- 3. a just-closed segment's final part still serves ---------------

    /// MUTATION VERIFIED: this behaviour depends entirely on
    /// `media_plane::trunk::SegmentWriter::publish_segment` (`media-plane/src/trunk.rs`) never
    /// touching the live-part log. Simulating the old `MediaStore` bug here
    /// by having `resolve_resource` check `last_closed_segment() >= seq`
    /// ("this segment already closed -> NotFound") **before** checking
    /// `Trunk::part_bytes` (i.e. swapping the two checks' order) makes this
    /// test's first assertion fail: the just-closed segment's final part
    /// resolves `NotFound` instead of `Ready` (`panicked at ...: the
    /// just-closed segment's final part must still serve, got NotFound`),
    /// because the eager closed-check now shadows the still-valid
    /// `part_bytes` hit. Recompiled and re-run to confirm the failure, then
    /// reverted. This is the RFC 8216bis boundary behaviour that shipped as
    /// multimux 0.2.2's bug fix — regressing it would break the live camera
    /// route (its own `#EXT-X-PRELOAD-HINT` part races exactly this
    /// boundary every segment).
    #[test]
    fn just_closed_segment_final_part_still_serves() {
        let (_trunk, origin, writer) = make_origin();
        part(&writer, 1, 0, true);
        part(&writer, 1, 1, false); // segment 1's final part
        seg(&writer, 1, 4.0, false); // close segment 1

        match resolve_now(
            &origin,
            HlsRequest::Resource {
                name: "part-1-7-1.1.m4s".to_string(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                ..
            } => assert_eq!(bytes, Bytes::from(vec![1u8; 4])),
            other => panic!("the just-closed segment's final part must still serve, got {other:?}"),
        }

        // A genuinely-nonexistent part of the closed segment is NotFound.
        assert_eq!(
            resolve_now(
                &origin,
                HlsRequest::Resource {
                    name: "part-1-7-1.9.m4s".to_string(),
                }
            ),
            EgressResponse::NotFound
        );

        // The playlist must not resurrect the closed segment's parts as
        // "open" -- it is rendered whole.
        let body = match resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(
            body.contains("seg-1-7-1.m4s"),
            "closed segment rendered whole: {body}"
        );
        // RFC 8216bis §6.2.2: the closed segment keeps its EXT-X-PART tags
        // (inside the segment, before its EXTINF) for three Target
        // Durations. They are not an *open* segment, and every part line
        // precedes the segment's own EXTINF/URI. The playlist carries parts,
        // so Appendix B.1 requires a preload hint: part 0 of the next
        // segment (#1577).
        assert_eq!(
            body.lines().skip(5).collect::<Vec<_>>(),
            vec![
                "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.5,HOLD-BACK=12,CAN-SKIP-UNTIL=24,CAN-SKIP-DATERANGES=YES",
                "#EXT-X-PART-INF:PART-TARGET=0.5",
                "#EXT-X-MAP:URI=\"init-1-7-1.mp4\"",
                "#EXT-X-PART:DURATION=0.5,URI=\"part-1-7-1.0.m4s\",INDEPENDENT=YES",
                "#EXT-X-PART:DURATION=0.5,URI=\"part-1-7-1.1.m4s\"",
                "#EXTINF:4,",
                "seg-1-7-1.m4s",
                "#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"part-1-7-2.0.m4s\"",
            ],
            "{body}"
        );
    }

    // --- 4. MEDIA-SEQUENCE / DISCONTINUITY-SEQUENCE advance as the window
    //        rolls -----------------------------------------------------

    /// MUTATION VERIFIED: changing `Window::push`'s eviction guard from
    /// `if evicted.discontinuous` to `if false` (never counting an evicted
    /// discontinuity) makes this test's
    /// `assert!(body.contains("#EXT-X-DISCONTINUITY-SEQUENCE:1"))` fail --
    /// the tag is omitted entirely (the renderer only emits it when
    /// `discontinuity_sequence > 0`), because the counter never advances
    /// past `0`. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn media_sequence_and_discontinuity_sequence_advance_as_window_rolls() {
        let (_trunk, origin, writer) = make_origin(); // window_segments = 4

        seg(&writer, 1, 4.0, false);
        seg(&writer, 2, 4.0, true); // discontinuous
        seg(&writer, 3, 4.0, false);
        seg(&writer, 4, 4.0, false);

        // Window (capacity 4) holds exactly 1..=4 -- MEDIA-SEQUENCE=1, and
        // segment 2's own #EXT-X-DISCONTINUITY renders in-window (no
        // DISCONTINUITY-SEQUENCE yet, nothing has rolled off).
        let body = match resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:1"), "body: {body}");
        assert!(
            body.contains("#EXT-X-DISCONTINUITY-SEQUENCE:0"),
            "fresh live playlist always carries DISCONTINUITY-SEQUENCE:0 (Apple 8.17): {body}"
        );
        assert!(body.contains("#EXT-X-DISCONTINUITY\n"), "body: {body}");

        // Roll the window: segment 5 evicts segment 1 (not discontinuous;
        // DISCONTINUITY-SEQUENCE stays 0), segment 6 evicts segment 2
        // (discontinuous -- DISCONTINUITY-SEQUENCE becomes 1).
        seg(&writer, 5, 4.0, false);
        let body = match resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:2"), "body: {body}");
        assert!(
            body.contains("#EXT-X-DISCONTINUITY-SEQUENCE:0"),
            "evicted segment 1 was not discontinuous: {body}"
        );

        seg(&writer, 6, 4.0, false);
        let body = match resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:3"), "body: {body}");
        assert!(
            body.contains("#EXT-X-DISCONTINUITY-SEQUENCE:1"),
            "segment 2 (discontinuous) has now rolled off the window: {body}"
        );
    }

    // --- misc: target-duration MUST, abuse bound, bad request -------------

    #[test]
    fn target_duration_is_max_of_configured_and_actual_segment_duration() {
        let (_trunk, origin, writer) = make_origin(); // configured target 4.0
        seg(&writer, 1, 7.5, false);
        let body = match resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(
            body.contains("#EXT-X-TARGETDURATION:8"),
            "TARGETDURATION must be round(7.5)=8, not the configured target: {body}"
        );
    }

    #[test]
    fn far_future_msn_rejected() {
        let (_trunk, origin, writer) = make_origin();
        seg(&writer, 1, 4.0, false);
        let outcome = resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: Some(1002),
                    hls_part: None,
                    hls_skip: None,
                    hls_primary: None,
                },
            },
        );
        assert!(matches!(outcome, EgressResponse::BadRequest { .. }));
    }

    /// RFC 8216bis §6.2.5.2: `_HLS_msn` at `last_closed + 2` is the
    /// spec's bound — it MUST be accepted. With one segment closed (seq=1),
    /// the last closed is 1, so `_HLS_msn=3` (last_closed + 2) is accepted
    /// and the request blocks (returns Pending, not BadRequest).
    #[test]
    fn msn_at_spec_bound_is_accepted() {
        let (_trunk, origin, writer) = make_origin();
        seg(&writer, 1, 4.0, false);
        let outcome = resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: Some(3),
                    hls_part: None,
                    hls_skip: None,
                    hls_primary: None,
                },
            },
        );
        assert!(
            !matches!(outcome, EgressResponse::BadRequest { .. }),
            "msn at spec bound (last_closed+2) must be accepted, not rejected"
        );
    }

    /// RFC 8216bis §6.2.5.2: `_HLS_msn` one beyond the spec's +2 SHOULD
    /// boundary is rejected. With one segment closed (seq=1, last_closed=1),
    /// the live edge in_progress_seg is 1 (not 2, because no parts exist to
    /// advance it), so `_HLS_msn=4` (1 + 2 + 1) is rejected.
    #[test]
    fn msn_one_beyond_spec_bound_is_rejected() {
        let (_trunk, origin, writer) = make_origin();
        seg(&writer, 1, 4.0, false);
        // With segments up to 1 closed, in_progress_seg is 1.
        // _HLS_msn > 1 + 2 == 3 → rejected. So msn=4 is rejected.
        let outcome = resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: Some(4),
                    hls_part: None,
                    hls_skip: None,
                    hls_primary: None,
                },
            },
        );
        assert!(
            matches!(outcome, EgressResponse::BadRequest { .. }),
            "msn at spec bound + 1 (last_closed+3) must be rejected"
        );
    }

    #[test]
    fn part_without_msn_rejected() {
        let (_trunk, origin, _writer) = make_origin();
        let outcome = resolve_now(
            &origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: None,
                    hls_part: Some(0),
                    hls_skip: None,
                    hls_primary: None,
                },
            },
        );
        assert!(matches!(outcome, EgressResponse::BadRequest { .. }));
    }

    #[test]
    fn resolve_resource_init_present() {
        let (_trunk, origin, _writer) = make_origin();
        match resolve_now(
            &origin,
            HlsRequest::Resource {
                name: "init-1.mp4".to_string(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                cache,
            } => {
                assert_eq!(bytes, Bytes::from(vec![0xAAu8; 8]));
                // The bare name means "the current init" and can change.
                assert_eq!(cache, CachePolicy::NoCache);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn resolve_resource_unmatched_filename_not_found() {
        let (_trunk, origin, _writer) = make_origin();
        assert_eq!(
            resolve_now(
                &origin,
                HlsRequest::Resource {
                    name: "not-a-thing.txt".to_string(),
                }
            ),
            EgressResponse::NotFound
        );
    }

    // --- issue #873: Container x low-latency matrix -----------------------
    //
    // Every cell below asserts on rendered playlist text AND on request
    // resolution (fetch every URI the rendered text itself advertised, never
    // a hard-coded filename) -- "advertised == servable" is the entire
    // reason `HlsOrigin` exists, per the issue. `EXT-X-VERSION` is checked
    // against `broadcast_hls::MediaPlaylist::computed_version()` re-derived
    // from a round-trip parse of the rendered text -- never an independently
    // guessed integer literal, which is exactly the bug issue #871 removed.

    fn make_origin_with(
        container: Container,
        low_latency_ms: Option<u32>,
    ) -> (Arc<Trunk>, HlsOrigin, media_plane::trunk::SegmentWriter) {
        let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
        let writer = trunk.segment_writer().expect("first segment writer");
        let mut builder = HlsOrigin::builder(Arc::clone(&trunk))
            .target_duration_secs(4.0)
            .window_segments(nz(4))
            .container(container);
        if let Some(ms) = low_latency_ms {
            builder = builder.low_latency(ms);
        }
        let origin = builder
            .instance(7)
            .build()
            .expect("both required fields set");
        (trunk, origin, writer)
    }

    fn render_body(origin: &HlsOrigin) -> String {
        match resolve_now(
            origin,
            HlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        }
    }

    /// The `#EXT-X-VERSION:<n>` value present in `body`, if any.
    fn extract_version_tag(body: &str) -> Option<u8> {
        body.lines()
            .find_map(|l| l.strip_prefix("#EXT-X-VERSION:")?.parse::<u8>().ok())
    }

    /// Every segment URI (the line immediately after a `#EXTINF:` line), in
    /// playlist order -- exactly what a real client would fetch next.
    fn segment_uris(body: &str) -> Vec<String> {
        let lines: Vec<&str> = body.lines().collect();
        let mut out = Vec::new();
        for i in 0..lines.len() {
            if lines[i].starts_with("#EXTINF:")
                && let Some(next) = lines.get(i + 1)
                && !next.starts_with('#')
            {
                out.push((*next).to_string());
            }
        }
        out
    }

    /// Every `#EXT-X-PART:` line's `URI="..."` value, in order.
    fn part_uris(body: &str) -> Vec<String> {
        body.lines()
            .filter(|l| l.starts_with("#EXT-X-PART:"))
            .filter_map(|l| {
                let start = l.find("URI=\"")? + "URI=\"".len();
                let rest = &l[start..];
                let end = rest.find('"')?;
                Some(rest[..end].to_string())
            })
            .collect()
    }

    /// Assert the rendered `#EXT-X-VERSION` equals `broadcast_hls`'s own
    /// `computed_version()`, re-derived by round-trip parsing the rendered
    /// text -- never an integer this test independently guessed. Returns the
    /// rendered value so a caller can make a further, non-vacuous claim
    /// about it.
    fn assert_version_matches_broadcast_hls_derivation(body: &str) -> Option<u8> {
        let parsed = MediaPlaylist::parse(body).expect("rendered body must round-trip parse");
        let rendered = extract_version_tag(body);
        assert_eq!(
            rendered,
            parsed.computed_version(),
            "rendered #EXT-X-VERSION must equal broadcast_hls's own derivation, body: {body}"
        );
        rendered
    }

    /// [`assert_version_matches_broadcast_hls_derivation`] **plus a
    /// non-vacuity guard**: the playlist must actually trigger at least one
    /// RFC 8216bis §8 version rule, so the equality above cannot pass by
    /// comparing `None` against `None`.
    ///
    /// Without this, a cell whose every `#EXTINF` is integral (e.g. a
    /// duration of exactly `4.0`, which renders `#EXTINF:4,`) trips no §8
    /// row at all, and the derivation could be entirely broken while the
    /// test stayed green. Real segmenters cut on keyframes, not whole
    /// seconds, so a fractional `#EXTINF` is also the realistic shape --
    /// cf. `fixtures/hls/spec/9.1-simple-media-playlist.m3u8` (`#EXTINF:9.009`,
    /// `#EXT-X-VERSION:3`).
    fn assert_version_present_and_matches_derivation(body: &str) -> u8 {
        assert_version_matches_broadcast_hls_derivation(body).unwrap_or_else(|| {
            panic!(
                "this cell must trigger a real RFC 8216bis §8 version rule -- a \
                 missing #EXT-X-VERSION makes the derivation check vacuous, body: {body}"
            )
        })
    }

    /// MUTATION VERIFIED (issue #873): making `HlsOriginBuilder::container`
    /// a no-op (so every origin renders as `Fmp4` regardless of the
    /// `Container` passed to it) makes this test's
    /// `assert!(!body.contains("#EXT-X-MAP"))` fail -- the mutated build
    /// unconditionally emits `#EXT-X-MAP:URI="init-1.mp4"`, and the `.ts`
    /// URI assertions fail too (segments render as `seg-1-7-1.m4s` instead of
    /// `seg-1-7-1.ts`). Recompiled and re-run to confirm the failure (see the
    /// PR description for the pasted `cargo test` output), then reverted.
    /// The mutation bites four tests in total -- this one, the low-latency
    /// `MpegTs` cell, the integral-`EXTINF` version case, and the
    /// cross-container refusal.
    #[test]
    fn mpegts_classic_no_map_ts_uris_no_ll_tags() {
        let (_trunk, origin, writer) = make_origin_with(Container::MpegTs, None);
        // Fractional, like every real keyframe-cut segment (and like the
        // RFC's own classic examples) -- so RFC 8216bis §8 row 3
        // (floating-point EXTINF) genuinely fires and the version assertion
        // below is not `None == None`.
        seg(&writer, 1, 4.004, false);

        let body = render_body(&origin);
        assert!(!body.contains("#EXT-X-MAP"), "body: {body}");
        assert!(!body.contains("#EXT-X-PART"), "body: {body}");
        assert!(!body.contains("#EXT-X-SERVER-CONTROL"), "body: {body}");
        assert!(!body.contains("#EXT-X-PRELOAD-HINT"), "body: {body}");
        assert!(!body.contains(".m4s"), "body: {body}");
        assert!(body.contains("seg-1-7-1.ts"), "body: {body}");
        assert_version_present_and_matches_derivation(&body);

        // advertised == servable: fetch every URI the rendered text itself
        // named, and check its bytes against what was actually published
        // (via this crate's own `parse_immediate`, not a hard-coded filename).
        let uris = segment_uris(&body);
        assert_eq!(uris, vec!["seg-1-7-1.ts".to_string()]);
        for uri in uris {
            let ImmediateResource::Segment { seq, .. } = parse_immediate(&uri, Container::MpegTs)
                .expect("advertised segment URI must parse under MpegTs")
            else {
                panic!("expected a Segment resource for {uri}");
            };
            match resolve_now(&origin, HlsRequest::Resource { name: uri.clone() }) {
                EgressResponse::Ready {
                    body: HlsBody::Resource(bytes),
                    ..
                } => assert_eq!(bytes, Bytes::from(vec![seq as u8; 8])),
                other => panic!("expected Ready for {uri}, got {other:?}"),
            }
        }
    }

    /// MUTATION VERIFIED (issue #873): making `HlsOriginBuilder::container`
    /// a no-op makes this test's `assert!(!body.contains("#EXT-X-MAP"))`
    /// fail identically to the classic-`MpegTs` test above, and the part
    /// URIs render as `part-1-7-2.0.m4s` instead of `part-1-7-2.0.ts`, so
    /// `assert!(body.contains("part-1-7-2.0.ts"))` also fails. Recompiled and
    /// re-run to confirm, then reverted.
    #[test]
    fn mpegts_low_latency_part_ts_uris_blocking_part_requests_resolve() {
        let (trunk, origin, writer) = make_origin_with(Container::MpegTs, Some(500));
        let origin = Arc::new(origin);
        // A closed segment with a fractional (keyframe-cut) duration, so
        // RFC 8216bis §8 row 3 fires and the version assertion below is not
        // `None == None`; segment 2 is then the open one carrying live parts.
        seg(&writer, 1, 4.004, false);
        part(&writer, 2, 0, true);
        part(&writer, 2, 1, false);

        let body = render_body(&origin);
        assert!(!body.contains("#EXT-X-MAP"), "body: {body}");
        assert!(body.contains("#EXT-X-PART-INF"), "body: {body}");
        assert!(body.contains("#EXT-X-PART:"), "body: {body}");
        assert!(body.contains("seg-1-7-1.ts"), "body: {body}");
        assert!(body.contains("part-1-7-2.0.ts"), "body: {body}");
        assert!(!body.contains(".m4s"), "body: {body}");
        // LL-HLS directives add no §8 version requirement of their own --
        // the finding that killed the old hardcoded `EXT-X-VERSION:9` --
        // so this must derive to exactly the same value as the classic
        // MpegTs cell above.
        assert_version_present_and_matches_derivation(&body);

        // advertised == servable for every advertised part.
        let parts = part_uris(&body);
        assert!(!parts.is_empty(), "body: {body}");
        for uri in &parts {
            let (_epoch, seq, idx) = parse_part(uri, Container::MpegTs)
                .unwrap_or_else(|| panic!("advertised part URI {uri} must parse under MpegTs"));
            match resolve_now(&origin, HlsRequest::Resource { name: uri.clone() }) {
                EgressResponse::Ready {
                    body: HlsBody::Resource(bytes),
                    ..
                } => {
                    assert_eq!(bytes, Bytes::from(vec![idx.value() as u8; 4]));
                    assert_eq!(seq, 2);
                }
                other => panic!("expected Ready for {uri}, got {other:?}"),
            }
        }

        // A blocking request for a `.ts` part not yet produced must Await,
        // then resolve once produced -- the same guarantee the pre-#873
        // fMP4-only test proves, now exercised over the `.ts` URI scheme.
        let deadline = Timestamp::from_nanos(5_000_000_000);
        let policy = AwaitPolicy::new(deadline);
        let pending = origin.resolve(
            HlsRequest::Resource {
                name: "part-1-7-2.2.ts".to_string(),
            },
            Timestamp::from_nanos(0),
            policy,
        );
        assert!(
            matches!(pending, EgressResponse::Await { .. }),
            "expected Await before the part exists, got {pending:?}"
        );

        let listener = trunk.listen().expect("listener slot available");
        let woken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let woken2 = std::sync::Arc::clone(&woken);
        let waiter = std::thread::spawn(move || {
            let ok = listener.wait_deadline(Instant::now() + Duration::from_secs(60));
            woken2.store(ok, std::sync::atomic::Ordering::SeqCst);
        });
        part(&writer, 2, 2, false);
        waiter.join().expect("waiter thread must not panic");
        assert!(
            woken.load(std::sync::atomic::Ordering::SeqCst),
            "Trunk::listen() must wake once publish_part lands"
        );

        match origin.resolve(
            HlsRequest::Resource {
                name: "part-1-7-2.2.ts".to_string(),
            },
            Timestamp::from_nanos(1),
            policy,
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                ..
            } => assert_eq!(bytes, Bytes::from(vec![2u8; 4])),
            other => panic!("expected Ready once produced, got {other:?}"),
        }
    }

    /// RFC 8216bis §8's opening rule: a playlist that triggers no version
    /// row at all is version-1 compatible and need not carry the tag. Kept
    /// as its own case (rather than as the `MpegTs` cells' only version
    /// check, which made those assertions vacuous) because an integral
    /// `#EXTINF` is a real, if unusual, shape.
    ///
    /// MUTATION VERIFIED (issue #873): making `HlsOriginBuilder::container`
    /// a no-op makes this fail with `left: Some(6), right: None` -- the
    /// mutated build emits `#EXT-X-MAP`, which trips §8 row 6. Recompiled
    /// and re-run to confirm, then reverted.
    #[test]
    fn mpegts_classic_integral_extinf_emits_no_version_tag() {
        let (_trunk, origin, writer) = make_origin_with(Container::MpegTs, None);
        seg(&writer, 1, 4.0, false);
        let body = render_body(&origin);
        // A whole number of seconds renders as an integer, so the playlist
        // genuinely contains no floating-point EXTINF value -- §8 row 3
        // does not fire, and omitting `EXT-X-VERSION` is honest rather than
        // a lie to a v1/v2 client. (`broadcast-hls` used to render `4.000`
        // here while still reporting no version requirement; fixed in the
        // same PR as this test.)
        assert!(body.contains("#EXTINF:4,"), "body: {body}");
        assert!(!body.contains("4.000"), "body: {body}");
        assert_eq!(
            assert_version_matches_broadcast_hls_derivation(&body),
            None,
            "nothing in this playlist triggers an RFC 8216bis §8 row: {body}"
        );
    }

    /// LL-HLS directives carry no RFC 8216bis §8 version requirement of
    /// their own -- the finding that killed the old hardcoded
    /// `EXT-X-VERSION:9`. Enabling low latency must therefore not change
    /// the derived version for otherwise-identical content.
    #[test]
    fn low_latency_does_not_raise_the_derived_version() {
        let (_t1, classic, w1) = make_origin_with(Container::MpegTs, None);
        seg(&w1, 1, 4.004, false);
        let classic_version = assert_version_present_and_matches_derivation(&render_body(&classic));

        let (_t2, low_latency, w2) = make_origin_with(Container::MpegTs, Some(500));
        seg(&w2, 1, 4.004, false);
        part(&w2, 2, 0, true);
        let ll_body = render_body(&low_latency);
        assert!(ll_body.contains("#EXT-X-PART:"), "body: {ll_body}");
        assert_eq!(
            assert_version_present_and_matches_derivation(&ll_body),
            classic_version,
            "enabling low latency must not raise the derived version"
        );
    }

    #[test]
    fn fmp4_classic_map_present_no_ll_tags() {
        let (_trunk, origin, writer) = make_origin_with(Container::Fmp4, None);
        origin.set_init(vec![0xBBu8; 8]);
        // Fractional, so §8 row 3 fires alongside row 6 (EXT-X-MAP without
        // EXT-X-I-FRAMES-ONLY). `max(6, 3) = 6`, so the rendered value is
        // unchanged -- this removes the ambiguity of an integral EXTINF
        // leaving row 3 entirely untested here.
        seg(&writer, 1, 4.004, false);

        let body = render_body(&origin);
        assert!(
            body.contains("#EXT-X-MAP:URI=\"init-1-7-1.mp4\""),
            "body: {body}"
        );
        assert!(!body.contains("#EXT-X-PART"), "body: {body}");
        assert!(!body.contains("#EXT-X-SERVER-CONTROL"), "body: {body}");
        assert!(!body.contains("#EXT-X-PRELOAD-HINT"), "body: {body}");
        assert!(body.contains("seg-1-7-1.m4s"), "body: {body}");
        assert_version_present_and_matches_derivation(&body);

        // advertised == servable, including the init segment the MAP names.
        match resolve_now(
            &origin,
            HlsRequest::Resource {
                name: "init-1.mp4".to_string(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                ..
            } => assert_eq!(bytes, Bytes::from(vec![0xBBu8; 8])),
            other => panic!("expected Ready(init), got {other:?}"),
        }
        for uri in segment_uris(&body) {
            let ImmediateResource::Segment { seq, .. } = parse_immediate(&uri, Container::Fmp4)
                .expect("advertised segment URI must parse under Fmp4")
            else {
                panic!("expected a Segment resource for {uri}");
            };
            match resolve_now(&origin, HlsRequest::Resource { name: uri.clone() }) {
                EgressResponse::Ready {
                    body: HlsBody::Resource(bytes),
                    ..
                } => assert_eq!(bytes, Bytes::from(vec![seq as u8; 8])),
                other => panic!("expected Ready for {uri}, got {other:?}"),
            }
        }
    }

    /// Regression guard (issue #873, matrix cell 4): the pre-#873 default
    /// shape (`Fmp4` + low-latency) must render unchanged.
    #[test]
    fn fmp4_low_latency_existing_behaviour_preserved() {
        let (_trunk, origin, writer) = make_origin_with(Container::Fmp4, Some(500));
        origin.set_init(vec![0xAAu8; 8]);
        // Fractional for the same reason as the Fmp4-classic cell above.
        seg(&writer, 1, 4.004, false);
        part(&writer, 2, 0, true);
        part(&writer, 2, 1, false);

        let body = render_body(&origin);
        assert!(
            body.contains("#EXT-X-MAP:URI=\"init-1-7-1.mp4\""),
            "body: {body}"
        );
        assert!(body.contains("#EXT-X-PART-INF"), "body: {body}");
        assert!(body.contains("#EXT-X-SERVER-CONTROL"), "body: {body}");
        assert!(body.contains("#EXT-X-PRELOAD-HINT"), "body: {body}");
        assert!(body.contains("seg-1-7-1.m4s"), "body: {body}");
        assert!(body.contains("part-1-7-2.0.m4s"), "body: {body}");
        assert!(!body.contains(".ts\""), "body: {body}");
        assert_version_present_and_matches_derivation(&body);

        match resolve_now(
            &origin,
            HlsRequest::Resource {
                name: "init-1.mp4".to_string(),
            },
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                ..
            } => assert_eq!(bytes, Bytes::from(vec![0xAAu8; 8])),
            other => panic!("expected Ready(init), got {other:?}"),
        }
        for uri in segment_uris(&body) {
            let ImmediateResource::Segment { seq, .. } = parse_immediate(&uri, Container::Fmp4)
                .expect("advertised segment URI must parse under Fmp4")
            else {
                panic!("expected a Segment resource for {uri}");
            };
            match resolve_now(&origin, HlsRequest::Resource { name: uri.clone() }) {
                EgressResponse::Ready {
                    body: HlsBody::Resource(bytes),
                    ..
                } => assert_eq!(bytes, Bytes::from(vec![seq as u8; 8])),
                other => panic!("expected Ready for {uri}, got {other:?}"),
            }
        }
        for uri in part_uris(&body) {
            let (_epoch, _seq, idx) = parse_part(&uri, Container::Fmp4)
                .unwrap_or_else(|| panic!("advertised part URI {uri} must parse under Fmp4"));
            match resolve_now(&origin, HlsRequest::Resource { name: uri.clone() }) {
                EgressResponse::Ready {
                    body: HlsBody::Resource(bytes),
                    ..
                } => assert_eq!(bytes, Bytes::from(vec![idx.value() as u8; 4])),
                other => panic!("expected Ready for {uri}, got {other:?}"),
            }
        }
    }

    /// MUTATION VERIFIED (issue #873): making `HlsOriginBuilder::container`
    /// a no-op makes every origin behave as `Fmp4`, so `init-1.mp4` under a
    /// nominally-`MpegTs` origin resolves `Ready` instead of `NotFound` --
    /// this test's final assertion fails. Recompiled and re-run to confirm,
    /// then reverted.
    #[test]
    fn cross_container_refusal_mp4_init_under_mpegts_not_found() {
        let (_trunk, origin, _writer) = make_origin_with(Container::MpegTs, None);
        // Still callable (documented no-op) -- the bytes are simply never
        // advertised or served under `MpegTs`.
        origin.set_init(vec![0xCCu8; 8]);
        assert_eq!(
            resolve_now(
                &origin,
                HlsRequest::Resource {
                    name: "init-1.mp4".to_string(),
                }
            ),
            EgressResponse::NotFound
        );
    }

    #[test]
    fn builder_errors_on_missing_required_fields() {
        let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
        match HlsOrigin::builder(Arc::clone(&trunk))
            .window_segments(nz(4))
            .instance(7)
            .build()
        {
            Err(e) => assert_eq!(e, HlsOriginBuildError::MissingTargetDurationSecs),
            Ok(_) => panic!("expected an error: target_duration_secs was never set"),
        }
        match HlsOrigin::builder(Arc::clone(&trunk))
            .target_duration_secs(4.0)
            .instance(7)
            .build()
        {
            Err(e) => assert_eq!(e, HlsOriginBuildError::MissingWindowSegments),
            Ok(_) => panic!("expected an error: window_segments was never set"),
        }
    }

    #[test]
    fn container_label_and_display() {
        assert_eq!(Container::Fmp4.name(), "fmp4");
        assert_eq!(Container::MpegTs.name(), "mpeg-ts");
        assert_eq!(Container::Fmp4.to_string(), "fmp4");
        assert_eq!(Container::default(), Container::Fmp4);
    }

    // --- issue #900: `closed_segments()` — the snapshot multimux's DVR
    //     catch-up serving merges with its on-disk archive ---

    /// MUTATION VERIFIED: changing `Window::push`'s
    /// `start_ns: entry.timeline_position.as_nanos()` to a constant `0`
    /// makes this test's `assert_eq!(snapshot[1].start_ns, 4_000_000_000)`
    /// fail (`left: 0, right: 4000000000`) — `closed_segments()` would then
    /// report every segment as starting at time zero, which is exactly the
    /// bug that would make a caller's time-based catch-up window (issue
    /// #900) unable to tell segments apart. Recompiled and re-run to
    /// confirm the failure, then reverted.
    #[test]
    fn closed_segments_snapshot_matches_published_segments_ascending() {
        let (_trunk, origin, writer) = make_origin();
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![1u8; 8]),
                SegmentSeq::new(1),
                Duration::from_secs_f64(4.0),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .unwrap();
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![2u8; 8]),
                SegmentSeq::new(2),
                Duration::from_secs_f64(4.0),
                Timestamp::from_nanos(4_000_000_000),
                SegmentMeta {
                    discontinuous: true,
                },
            ))
            .unwrap();

        let snapshot = origin.closed_segments();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].sequence_number, SegmentSeq::new(1));
        assert_eq!(snapshot[0].start_ns, 0);
        assert!(!snapshot[0].discontinuous);
        assert_eq!(snapshot[1].sequence_number, SegmentSeq::new(2));
        assert_eq!(snapshot[1].start_ns, 4_000_000_000);
        assert!(snapshot[1].discontinuous);
    }

    /// A segment evicted from the window (beyond `window_segments`
    /// capacity) no longer appears in the snapshot — `closed_segments()`
    /// reports the *advertised* window, the same one `render_playlist`
    /// renders, not every segment ever published.
    #[test]
    fn closed_segments_reflects_window_eviction() {
        let (_trunk, origin, writer) = make_origin(); // window_segments = 4
        for seq in 1..=5 {
            seg(&writer, seq, 4.0, false);
        }
        let snapshot = origin.closed_segments();
        let seqs: Vec<u32> = snapshot.iter().map(|s| s.sequence_number.value()).collect();
        assert_eq!(
            seqs,
            vec![2, 3, 4, 5],
            "oldest segment 1 must have rolled off"
        );
    }

    fn window_entry(seq: u32, tag: u8, discontinuous: bool) -> SegmentEntry {
        SegmentEntry::new(
            Bytes::from(vec![tag; 4]),
            SegmentSeq::new(seq),
            Duration::from_secs(4),
            Timestamp::from_nanos(0),
            SegmentMeta { discontinuous },
        )
    }

    fn window_seqs(w: &Window) -> Vec<(u32, bool)> {
        w.segments
            .iter()
            .map(|s| (s.sequence_number.value(), s.discontinuous))
            .collect()
    }

    // r09-W11: the Trunk's `publish_segment` refuses a decreasing number, but
    // the window must not depend on that: a repeat or decrease clears it
    // instead of holding two entries under one number (the old `bytes_of`
    // served the stale one).
    #[test]
    fn window_restarts_on_a_repeated_or_decreasing_sequence_number() {
        let mut w = Window::new(nz(6));
        w.push(window_entry(5, 0xA5, false), Generation::new(1), None);
        w.push(window_entry(6, 0xA6, true), Generation::new(1), None);
        assert_eq!(window_seqs(&w), vec![(5, false), (6, true)]);

        // Decrease: 3 after 6.
        w.push(window_entry(3, 0xB3, false), Generation::new(1), None);
        assert_eq!(window_seqs(&w), vec![(3, true)]);
        // The discontinuous 6 rolled off the front.
        assert_eq!(w.discontinuity_sequence, 1);
        assert_eq!(
            w.bytes_of(SegmentSeq::new(3)),
            Some(Bytes::from(vec![0xB3; 4]))
        );
        assert_eq!(w.bytes_of(SegmentSeq::new(5)), None);
        assert_eq!(w.bytes_of(SegmentSeq::new(6)), None);

        // Repeat: 3 again with different bytes replaces, never duplicates.
        w.push(window_entry(3, 0xC3, false), Generation::new(1), None);
        assert_eq!(window_seqs(&w), vec![(3, true)]);
        assert_eq!(
            w.bytes_of(SegmentSeq::new(3)),
            Some(Bytes::from(vec![0xC3; 4]))
        );
    }

    // Hostile edge: the largest sequence number has no successor; a repeat of
    // it must restart the window, not overflow `last + 1`.
    #[test]
    fn window_handles_u32_max_sequence_numbers() {
        let mut w = Window::new(nz(4));
        w.push(
            window_entry(u32::MAX - 1, 1, false),
            Generation::new(1),
            None,
        );
        w.push(window_entry(u32::MAX, 2, false), Generation::new(1), None);
        assert_eq!(
            window_seqs(&w),
            vec![(u32::MAX - 1, false), (u32::MAX, false)]
        );
        w.push(window_entry(u32::MAX, 3, false), Generation::new(1), None);
        assert_eq!(window_seqs(&w), vec![(u32::MAX, true)]);
        assert_eq!(
            w.bytes_of(SegmentSeq::new(u32::MAX)),
            Some(Bytes::from(vec![3; 4]))
        );
    }

    #[test]
    fn window_capacity_is_never_below_three_segments() {
        let mut w = Window::new(nz(1));
        for seq in 1..=5 {
            w.push(window_entry(seq, 0, false), Generation::new(1), None);
        }
        assert_eq!(window_seqs(&w), vec![(3, false), (4, false), (5, false)]);
    }

    /// A measured peak bitrate above `u32::MAX` must surface as the typed
    /// [`HlsMasterError::BandwidthOverflow`] (was an untyped `String`).
    #[test]
    fn master_playlist_reports_typed_bandwidth_overflow() {
        let (_trunk, origin, writer) = make_origin();
        // 8 bytes (64 bits) in 10 ns = 6.4e9 b/s > u32::MAX.
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(vec![1u8; 8]),
                SegmentSeq::new(1),
                Duration::from_nanos(10),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .unwrap();
        assert_eq!(
            origin.master_playlist("media.m3u8"),
            Err(HlsMasterError::BandwidthOverflow {
                bandwidth: 6_400_000_000
            })
        );
    }

    #[test]
    fn segment_bandwidth_is_rounded_up_and_rejects_degenerate_input() {
        // 1001 B * 8 / 3 s = 2669.33 -> 2670.
        assert_eq!(
            segment_bandwidth_bps(1001, Duration::from_secs(3)),
            Some(2670)
        );
        // Exact division stays exact.
        assert_eq!(
            segment_bandwidth_bps(1_500_000, Duration::from_secs(1)),
            Some(12_000_000)
        );
        // Zero duration: nothing to measure.
        assert_eq!(segment_bandwidth_bps(10, Duration::ZERO), None);
        // A one-nanosecond segment of huge size overflows u64 b/s.
        assert_eq!(
            segment_bandwidth_bps(usize::MAX, Duration::from_nanos(1)),
            None
        );
    }

    #[test]
    fn bytes_of_addresses_exactly_the_matching_entry() {
        let mut w = Window::new(nz(6));
        for (seq, tag) in [(5u32, 0x15u8), (6, 0x16), (7, 0x17)] {
            w.push(window_entry(seq, tag, false), Generation::new(1), None);
        }
        assert_eq!(
            w.bytes_of(SegmentSeq::new(5)),
            Some(Bytes::from(vec![0x15; 4]))
        );
        assert_eq!(
            w.bytes_of(SegmentSeq::new(6)),
            Some(Bytes::from(vec![0x16; 4]))
        );
        assert_eq!(
            w.bytes_of(SegmentSeq::new(7)),
            Some(Bytes::from(vec![0x17; 4]))
        );
        assert_eq!(w.bytes_of(SegmentSeq::new(4)), None);
        assert_eq!(w.bytes_of(SegmentSeq::new(8)), None);
        assert_eq!(w.bytes_of(SegmentSeq::new(u32::MAX)), None);
    }

    // --- r09-O4 / T6 (#1089, #1134): lock discipline ---------------------

    fn segment_request(name: &str) -> HlsRequest {
        HlsRequest::Resource {
            name: name.to_string(),
        }
    }

    /// With nothing published since the last drain, a request must not need
    /// the cursor lock. Holding that lock from the test thread makes the old
    /// always-drain code block (the request thread then misses the timeout).
    #[test]
    fn a_request_with_nothing_new_does_not_take_the_cursor_lock() {
        let (_trunk, origin, writer) = make_origin();
        seg(&writer, 1, 4.0, false);
        assert!(matches!(
            resolve_now(&origin, segment_request("seg-1-7-1.m4s")),
            EgressResponse::Ready { .. }
        ));

        let guard = locked(&origin.cursor);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|s| {
            s.spawn(|| {
                let _ = tx.send(resolve_now(&origin, segment_request("seg-1-7-1.m4s")));
            });
            let got = rx.recv_timeout(Duration::from_secs(10));
            drop(guard); // always release, so the scope can join
            assert!(
                matches!(got, Ok(EgressResponse::Ready { .. })),
                "request blocked on the cursor lock: {got:?}"
            );
        });
    }

    /// The skip never hides a real publish: a segment closed after the last
    /// drain is served by the very next request.
    #[test]
    fn a_segment_published_after_a_drain_is_served_by_the_next_request() {
        let (_trunk, origin, writer) = make_origin();
        seg(&writer, 1, 4.0, false);
        assert!(matches!(
            resolve_now(&origin, segment_request("seg-1-7-1.m4s")),
            EgressResponse::Ready { .. }
        ));
        assert_eq!(
            resolve_now(&origin, segment_request("seg-1-7-2.m4s")),
            EgressResponse::NotFound
        );
        seg(&writer, 2, 4.0, false);
        assert!(matches!(
            resolve_now(&origin, segment_request("seg-1-7-2.m4s")),
            EgressResponse::Ready { .. }
        ));
    }

    /// A panic while some thread held an origin lock must not become a panic
    /// in every later request.
    #[test]
    fn a_poisoned_lock_does_not_cascade_into_later_requests() {
        let (_trunk, origin, writer) = make_origin();
        seg(&writer, 1, 4.0, false);
        std::thread::scope(|s| {
            let origin = &origin;
            for which in 0..4 {
                let handle = s.spawn(move || {
                    // Hold one of the origin's mutexes across a panic.
                    match which {
                        0 => drop(origin.window.lock().map(|_g| panic!("poison window"))),
                        1 => drop(origin.cursor.lock().map(|_g| panic!("poison cursor"))),
                        2 => drop(origin.init.lock().map(|_g| panic!("poison init"))),
                        _ => drop(origin.codecs.lock().map(|_g| panic!("poison codecs"))),
                    }
                });
                assert!(handle.join().is_err(), "setup thread {which} must panic");
            }
        });
        assert!(origin.window.is_poisoned());
        assert!(origin.cursor.is_poisoned());
        assert!(origin.init.is_poisoned());
        assert!(origin.codecs.is_poisoned());
        // Every public entry point still answers.
        seg(&writer, 2, 4.0, false);
        assert_eq!(origin.closed_segments().len(), 2);
        assert!(origin.init_bytes().is_some());
        assert!(origin.master_playlist("media.m3u8").is_ok());
        origin.set_init(vec![0xCC; 8]);
        assert!(render_body(&origin).contains("seg-1-7-2.m4s"));
        assert!(matches!(
            resolve_now(&origin, segment_request("seg-1-7-1.m4s")),
            EgressResponse::Ready { .. }
        ));
    }
}
