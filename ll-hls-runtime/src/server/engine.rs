//! [`LlHlsOrigin`] — the LL-HLS origin [`ServedEgress`] (plan step 4):
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
//!   used to track as a push-fed field** — [`LlHlsOrigin::live_edge`] derives
//!   it from the two queries above alone (`last_closed_segment() + 1`, probed
//!   via `parts_in_segment`), needing no field of its own. See that method's
//!   doc for the derivation and why it is exact, not a heuristic.
//! - **A just-closed segment's final part still resolving** — falls out of
//!   [`Trunk::part_bytes`] for free: [`TrunkWriter::publish_segment`]
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
//! `LlHlsOrigin` owns — precisely the shape `media_plane::egress`'s module
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
//! `Trunk`'s four rings. [`LlHlsOrigin::set_init`] is the (small, honest) side
//! channel for it — not a duplicate of anything `Trunk` holds.

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use broadcast_common::Timestamp;
use bytes::Bytes;
use media_plane::egress::{AwaitPolicy, CachePolicy, EgressResponse, ServedEgress};
use media_plane::trunk::{PartEntry, SegmentCursor, SegmentCursorItem, SegmentEntry, Trunk};
use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment, OpenSegment, PartSpec};

/// Track id for the single rendition served per stream (no multi-track/
/// multi-rendition support yet).
pub const DEFAULT_TRACK_ID: u32 = 1;

/// Placeholder `BANDWIDTH` (bits/second) advertised in the master playlist's
/// `#EXT-X-STREAM-INF` — actual encoded bitrate isn't measured, so a single
/// fixed estimate is used for the single variant served.
const PLACEHOLDER_BANDWIDTH_BPS: u64 = 5_000_000;

/// RFC 8216bis §6.2.5.2 (SHOULD): a `_HLS_msn` unreasonably far in the future
/// should be rejected rather than always blocking to the caller's timeout — a
/// legitimate client only ever asks for the segment/part right after the one
/// it already has, so anything more than a few segments beyond the current
/// live edge is either a malfunctioning client or abuse.
const ABUSE_MSN_FUTURE_BOUND: u64 = 4;

/// HLS requires HLS protocol version 9 (RFC 8216bis §4.4.3.7/§4.4.3.8: the
/// `#EXT-X-PART-INF`/`#EXT-X-PART` directives this renderer always emits
/// require it).
const LL_HLS_VERSION: u8 = 9;

/// RFC 8216bis / Apple LL-HLS §4.4.3.7: `#EXT-X-SERVER-CONTROL`'s
/// `PART-HOLD-BACK` attribute MUST be at least 3x the part target duration
/// (`#EXT-X-PART-INF`'s `PART-TARGET`).
const PART_HOLD_BACK_MULTIPLIER: f64 = 3.0;

/// A minimal single-variant master playlist pointing at `media_playlist_name`
/// (the caller's configured media-playlist filename — e.g. multimux's
/// `Config::playlist_name`, defaulting to `"media.m3u8"`) — the same
/// regardless of any stream state (no multi-rendition support yet), so this
/// takes no `Trunk`/origin argument.
pub fn master_playlist_m3u8(media_playlist_name: &str) -> String {
    format!(
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH={PLACEHOLDER_BANDWIDTH_BPS}\n{media_playlist_name}\n"
    )
}

/// Blocking playlist reload query parameters (RFC 8216bis §6.2.5.2) — the
/// sans-IO counterpart of an adapter's own (likely serde-`Deserialize`)
/// query-string type; the adapter maps its wire query params into this.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BlockingQuery {
    /// The Media Sequence Number the client already has, plus one — the
    /// origin should not respond until a segment/part beyond this is ready.
    pub hls_msn: Option<u64>,
    /// The part index (within `hls_msn`) the client is waiting for.
    pub hls_part: Option<u32>,
}

/// [`ServedEgress::Request`] for [`LlHlsOrigin`]: which wire resource is
/// being asked for. A data-carrying dispatch ADT (matches this crate's
/// `client::action::Action`/`ResourceId` convention) — see
/// `tests/label_coverage.rs`'s SKIP list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlHlsRequest {
    /// `GET <media playlist>`, optionally carrying a blocking-reload query.
    Playlist {
        /// The track id to render the playlist for (a naming parameter only —
        /// see [`DEFAULT_TRACK_ID`]).
        track_id: u32,
        /// The blocking-reload query parameters, if any.
        query: BlockingQuery,
    },
    /// `GET` a dynamic origin resource by its wire filename (`init-{track}.mp4`,
    /// `seg-{track}-{seq}.m4s`, `part-{track}-{seq}.{idx}.m4s`).
    Resource {
        /// The requested filename, exactly as it appeared in the request path.
        name: String,
    },
}

/// [`ServedEgress::Body`] for [`LlHlsOrigin`]: the resolved body, typed by
/// which [`LlHlsRequest`] produced it. A data-carrying ADT — see
/// `tests/label_coverage.rs`'s SKIP list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlHlsBody {
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
struct WindowSegment {
    sequence_number: u32,
    bytes: Bytes,
    duration_secs: f64,
    discontinuous: bool,
}

/// The small per-[`LlHlsOrigin`] synced window this module's own doc
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
    /// Cumulative count of discontinuities that have rolled off the front of
    /// the window — RFC 8216 §4.3.3.3's `#EXT-X-DISCONTINUITY-SEQUENCE`.
    /// Incremented exactly once per **evicted** entry whose
    /// [`WindowSegment::discontinuous`] was `true`; a discontinuity still
    /// inside the window is rendered as a per-segment `#EXT-X-DISCONTINUITY`
    /// tag instead (see [`MediaPlaylist::to_m3u8`]), never double-counted
    /// here.
    discontinuity_sequence: u64,
}

impl Window {
    fn new(capacity: NonZeroUsize) -> Self {
        Window {
            segments: VecDeque::new(),
            capacity: capacity.get(),
            max_segment_duration_secs: 0.0,
            discontinuity_sequence: 0,
        }
    }

    /// Absorb one drained [`SegmentEntry`], evicting the oldest window entry
    /// first if already at `capacity` — same evict-then-push shape as every
    /// ring in `trunk.rs` itself.
    fn push(&mut self, entry: SegmentEntry) {
        let duration_secs = entry.duration.as_secs_f64();
        self.max_segment_duration_secs = self.max_segment_duration_secs.max(duration_secs);
        if self.segments.len() == self.capacity {
            if let Some(evicted) = self.segments.pop_front() {
                if evicted.discontinuous {
                    self.discontinuity_sequence += 1;
                }
            }
        }
        self.segments.push_back(WindowSegment {
            sequence_number: entry.sequence_number,
            bytes: entry.bytes,
            duration_secs,
            discontinuous: entry.meta.discontinuous,
        });
    }

    fn bytes_of(&self, sequence_number: u32) -> Option<Bytes> {
        self.segments
            .iter()
            .find(|s| s.sequence_number == sequence_number)
            .map(|s| s.bytes.clone())
    }
}

/// Parse a `part-{track}-{seq}.{idx}.m4s` dynamic filename into `(seq, idx)`,
/// or `None` if it isn't a part filename (or its numeric fields don't parse).
/// `{track}` is validated but unused (matches every other dynamic-filename
/// resource in this module).
fn parse_part(file: &str) -> Option<(u32, u32)> {
    let rest = file.strip_prefix("part-")?.strip_suffix(".m4s")?;
    let (track_seq, idx) = rest.rsplit_once('.')?;
    let (track, seq) = track_seq.split_once('-')?;
    track.parse::<u32>().ok()?;
    Some((seq.parse().ok()?, idx.parse().ok()?))
}

/// Parse a `init-{track}.mp4`/`seg-{track}-{seq}.m4s` dynamic filename;
/// `part-…` filenames are handled separately by [`parse_part`] (they can
/// block until available). `{track}` is validated as a number but otherwise
/// unused: an [`LlHlsOrigin`] holds a single track's data (see
/// [`DEFAULT_TRACK_ID`]).
enum ImmediateResource {
    Init,
    Segment(u32),
}

fn parse_immediate(file: &str) -> Option<ImmediateResource> {
    if let Some(rest) = file.strip_prefix("init-") {
        let track = rest.strip_suffix(".mp4")?;
        track.parse::<u32>().ok()?;
        return Some(ImmediateResource::Init);
    }
    if let Some(rest) = file.strip_prefix("seg-") {
        let rest = rest.strip_suffix(".m4s")?;
        let (track, seq) = rest.split_once('-')?;
        track.parse::<u32>().ok()?;
        return Some(ImmediateResource::Segment(seq.parse().ok()?));
    }
    None
}

/// The LL-HLS origin [`ServedEgress`]: renders playlists and resolves
/// blocking-reload/part-availability requests for one stream, backed by a
/// shared [`Trunk`]. See this module's own doc for exactly what comes
/// straight from the `Trunk` and what needs the small synced `Window`.
pub struct LlHlsOrigin {
    trunk: Arc<Trunk>,
    /// This origin's **one** [`SegmentCursor`] — see [`Trunk::subscribe_segments`]'s
    /// own docs (and this crate's `media_plane::egress` module doc) for why a
    /// `ServedEgress` must never take one per request/peer.
    cursor: Mutex<SegmentCursor>,
    window: Mutex<Window>,
    /// The fMP4 init segment — see this module's doc for why this, alone, is
    /// not answerable by any `Trunk` ring.
    init: Mutex<Option<Bytes>>,
    target_duration_secs: f64,
    part_target_ms: u32,
}

impl LlHlsOrigin {
    /// Build a fresh origin over `trunk`, subscribing its one [`SegmentCursor`]
    /// immediately (so the window starts empty but never misses a segment
    /// published from this point on).
    ///
    /// `window_segments` bounds how many closed segments this origin
    /// advertises in a rendered Media Playlist — independent of
    /// [`media_plane::trunk::TrunkConfig::segment_capacity`] (the `Trunk`'s own
    /// retention bound): a caller may legitimately want a shorter advertised
    /// window than the `Trunk` retains for other consumers (e.g. a DVR
    /// `SegmentEgress` reading the same `Trunk`).
    pub fn new(
        trunk: Arc<Trunk>,
        target_duration_secs: f64,
        part_target_ms: u32,
        window_segments: NonZeroUsize,
    ) -> Self {
        let cursor = trunk.subscribe_segments();
        LlHlsOrigin {
            trunk,
            cursor: Mutex::new(cursor),
            window: Mutex::new(Window::new(window_segments)),
            init: Mutex::new(None),
            target_duration_secs,
            part_target_ms,
        }
    }

    /// Store the fMP4 init segment bytes — see this module's doc for why an
    /// init segment is not something any `Trunk` ring holds.
    pub fn set_init(&self, bytes: impl Into<Bytes>) {
        *self.init.lock().unwrap() = Some(bytes.into());
    }

    /// The fMP4 init segment bytes, if set.
    pub fn init_bytes(&self) -> Option<Bytes> {
        self.init.lock().unwrap().clone()
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
        let mut cursor = self.cursor.lock().unwrap();
        let mut window = self.window.lock().unwrap();
        while let Some(item) = cursor.poll() {
            if let SegmentCursorItem::Segment(entry) = item {
                window.push(entry);
            }
        }
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
    fn live_edge(&self) -> (u32, Vec<PartEntry>) {
        let last_closed = self.trunk.last_closed_segment().unwrap_or(0);
        let candidate = last_closed + 1;
        let parts = self.trunk.parts_in_segment(candidate);
        if parts.is_empty() {
            (last_closed, Vec::new())
        } else {
            (candidate, parts)
        }
    }

    /// Render the LL-HLS media playlist for `track_id` from this origin's
    /// current `Window` (closed segments) and the `Trunk`'s live edge (open
    /// segment's parts + preload hint).
    ///
    /// RFC 8216bis §4.4.4.9: an in-progress (not yet closed) segment MUST NOT
    /// be advertised with an `#EXTINF`/URI pair — that segment has no
    /// fetchable resource yet — it may only appear as trailing `#EXT-X-PART`
    /// lines. `transmux::hls::MediaPlaylist::open_segment` is exactly this
    /// representation: its parts render as trailing `#EXT-X-PART` lines with
    /// no `#EXTINF`/URI, so the in-progress segment's parts and the
    /// `#EXT-X-PRELOAD-HINT` for the next, not-yet-available part are both
    /// rendered by `to_m3u8()` itself — this method only supplies the URI
    /// scheme (`part-<track>-<seq>.<idx>.m4s`) and the part metadata.
    fn render_playlist(&self, track_id: u32) -> String {
        self.drain();
        let window = self.window.lock().unwrap();
        let (open_seq, open_parts) = self.live_edge();
        // Only render an open segment/preload-hint once the live edge is
        // genuinely a not-yet-closed segment with at least one live part —
        // never re-render an already-closed segment's lingering parts (the
        // `Trunk`'s live-part log deliberately does not evict them on close;
        // see `trunk`'s own module doc) as if they were still open.
        let has_open_parts = !open_parts.is_empty();

        let media_sequence = window
            .segments
            .front()
            .map(|s| u64::from(s.sequence_number))
            .or_else(|| has_open_parts.then_some(u64::from(open_seq)))
            .unwrap_or(1);
        let segments: Vec<MediaSegment> = window
            .segments
            .iter()
            .map(|s| MediaSegment {
                uri: format!("seg-{track_id}-{}.m4s", s.sequence_number),
                duration: s.duration_secs,
                discontinuous: s.discontinuous,
                parts: Vec::new(),
                ..Default::default()
            })
            .collect();
        let part_target = f64::from(self.part_target_ms) / 1000.0;
        let open_segment = has_open_parts.then(|| {
            OpenSegment::new(
                open_parts
                    .iter()
                    .map(|p| PartSpec {
                        uri: format!("part-{track_id}-{}.{}.m4s", p.segment_number, p.part_index),
                        duration: p.duration.as_secs_f64(),
                        independent: p.independent,
                        ..Default::default()
                    })
                    .collect(),
            )
        });
        let next_part_hint = has_open_parts.then(|| {
            let next_idx = open_parts
                .iter()
                .map(|p| p.part_index)
                .max()
                .map(|idx| idx + 1)
                .unwrap_or(0);
            format!("part-{track_id}-{open_seq}.{next_idx}.m4s")
        });
        // RFC 8216bis §4.4.3.1 (MUST): every Media Segment's EXTINF duration,
        // rounded to the nearest integer, MUST be <= TARGETDURATION. The
        // segmenter cuts on the next keyframe *after* the configured target,
        // so a real segment routinely exceeds it — advertising the
        // configured target alone can under-declare. Use whichever is
        // larger, rounded (not the configured value's `ceil()` alone).
        let target_duration = self
            .target_duration_secs
            .max(window.max_segment_duration_secs)
            .round() as u32;
        let playlist = MediaPlaylist {
            version: LL_HLS_VERSION,
            target_duration,
            media_sequence,
            discontinuity_sequence: window.discontinuity_sequence,
            segments,
            open_segment,
            endlist: false,
            extra_tags: vec![format!("#EXT-X-MAP:URI=\"init-{track_id}.mp4\"")],
            low_latency: Some(LowLatencyConfig {
                part_target,
                part_hold_back: part_target * PART_HOLD_BACK_MULTIPLIER,
                preload_hint_part: next_part_hint,
                ..Default::default()
            }),
            iframes_only: false,
            ..Default::default()
        };
        playlist.to_m3u8()
    }

    fn resolve_playlist(
        &self,
        track_id: u32,
        query: BlockingQuery,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<LlHlsBody> {
        if query.hls_part.is_some() && query.hls_msn.is_none() {
            return EgressResponse::BadRequest {
                reason: "_HLS_part without _HLS_msn is meaningless",
            };
        }
        if let Some(msn) = query.hls_msn {
            let (in_progress_seg, live_parts) = self.live_edge();
            if msn > u64::from(in_progress_seg) + ABUSE_MSN_FUTURE_BOUND {
                return EgressResponse::BadRequest {
                    reason: "_HLS_msn unreasonably far beyond the live edge",
                };
            }
            let satisfied = match query.hls_part {
                Some(part) => {
                    u64::from(in_progress_seg) > msn
                        || (u64::from(in_progress_seg) == msn
                            && live_parts.len() as u64 > u64::from(part))
                }
                None => self.trunk.last_closed_segment().unwrap_or(0) as u64 >= msn,
            };
            if !satisfied {
                return EgressResponse::pending(await_policy, now, now);
            }
        }
        EgressResponse::Ready {
            body: LlHlsBody::Playlist(self.render_playlist(track_id)),
            cache: CachePolicy::NoCache,
        }
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
    ) -> EgressResponse<LlHlsBody> {
        if let Some((seq, idx)) = parse_part(name) {
            if let Some(bytes) = self.trunk.part_bytes(seq, idx) {
                return EgressResponse::Ready {
                    body: LlHlsBody::Resource(bytes),
                    cache: CachePolicy::Immutable,
                };
            }
            // The requested part's segment has already closed (whether or
            // not this origin's own `Window` still retains its bytes) -> it
            // will never be produced. `Trunk::last_closed_segment` answers
            // this exactly, with no dependence on `Window`'s retention.
            let never_will = self.trunk.last_closed_segment().is_some_and(|c| c >= seq);
            return if never_will {
                EgressResponse::NotFound
            } else {
                EgressResponse::pending(await_policy, now, now)
            };
        }
        self.drain();
        let bytes = match parse_immediate(name) {
            Some(ImmediateResource::Init) => self.init_bytes(),
            Some(ImmediateResource::Segment(seq)) => self.window.lock().unwrap().bytes_of(seq),
            None => None,
        };
        match bytes {
            Some(bytes) => EgressResponse::Ready {
                body: LlHlsBody::Resource(bytes),
                cache: CachePolicy::Immutable,
            },
            None => EgressResponse::NotFound,
        }
    }
}

impl ServedEgress for LlHlsOrigin {
    type Request = LlHlsRequest;
    type Body = LlHlsBody;

    fn resolve(
        &self,
        request: LlHlsRequest,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<LlHlsBody> {
        match request {
            LlHlsRequest::Playlist { track_id, query } => {
                self.resolve_playlist(track_id, query, now, await_policy)
            }
            LlHlsRequest::Resource { name } => self.resolve_resource(&name, now, await_policy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::trunk::TrunkConfig;
    use std::time::{Duration, Instant};
    use transmux::SegmentMeta;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    /// A fresh `Trunk` sized generously for these tests, plus the one
    /// `LlHlsOrigin` under test.
    fn make_origin() -> (Arc<Trunk>, LlHlsOrigin, media_plane::trunk::TrunkWriter) {
        let trunk = Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)));
        let writer = trunk.writer().expect("first writer");
        let origin = LlHlsOrigin::new(Arc::clone(&trunk), 4.0, 500, nz(4));
        origin.set_init(vec![0xAAu8; 8]);
        (trunk, origin, writer)
    }

    fn seg(
        writer: &media_plane::trunk::TrunkWriter,
        seq: u32,
        duration_secs: f64,
        discontinuous: bool,
    ) {
        writer.publish_segment(SegmentEntry::new(
            Bytes::from(vec![seq as u8; 8]),
            seq,
            Duration::from_secs_f64(duration_secs),
            Timestamp::from_nanos(0),
            SegmentMeta { discontinuous },
        ));
    }

    fn part(writer: &media_plane::trunk::TrunkWriter, seg_no: u32, idx: u32, independent: bool) {
        writer.publish_part(PartEntry::new(
            Bytes::from(vec![idx as u8; 4]),
            seg_no,
            idx,
            Duration::from_millis(500),
            independent,
        ));
    }

    fn resolve_now(origin: &LlHlsOrigin, request: LlHlsRequest) -> EgressResponse<LlHlsBody> {
        origin.resolve(
            request,
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(0)),
        )
    }

    // --- master playlist (unaffected by the Trunk migration) -------------

    #[test]
    fn master_playlist_has_stream_inf() {
        let m = master_playlist_m3u8("media.m3u8");
        assert!(m.contains("#EXTM3U"));
        assert!(m.contains("#EXT-X-STREAM-INF"));
        assert!(m.contains("media.m3u8"));
    }

    #[test]
    fn master_playlist_points_at_configured_playlist_name() {
        let m = master_playlist_m3u8("index.m3u8");
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
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
                cache,
            } => {
                assert_eq!(cache, CachePolicy::NoCache);
                m
            }
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };

        assert!(body.contains("#EXT-X-VERSION:9"), "body: {body}");
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
            body.contains("#EXT-X-MAP:URI=\"init-1.mp4\""),
            "body: {body}"
        );
        assert!(body.contains("seg-1-1.m4s"), "body: {body}");
        assert!(
            body.contains("#EXT-X-PART:DURATION=0.5") && body.contains("INDEPENDENT=YES"),
            "body: {body}"
        );
        assert!(body.contains("#EXT-X-PRELOAD-HINT"), "body: {body}");
        assert!(
            body.contains("part-1-2.2.m4s"),
            "preload hint for the next part: {body}"
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
            LlHlsRequest::Resource {
                name: "part-1-1.0.m4s".to_string(),
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
        let waiter = std::thread::spawn(move || {
            let ok = listener.wait_deadline(Instant::now() + Duration::from_secs(2));
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
            LlHlsRequest::Resource {
                name: "part-1-1.0.m4s".to_string(),
            },
            Timestamp::from_nanos(1),
            policy,
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Resource(bytes),
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
            LlHlsRequest::Resource {
                name: "part-1-9.0.m4s".to_string(),
            },
            Timestamp::from_nanos(999_999_999),
            policy,
        );
        assert!(matches!(still_waiting, EgressResponse::Await { .. }));

        let expired = origin.resolve(
            LlHlsRequest::Resource {
                name: "part-1-9.0.m4s".to_string(),
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
    /// `TrunkWriter::publish_segment` (`media-plane/src/trunk.rs`) never
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
            LlHlsRequest::Resource {
                name: "part-1-1.1.m4s".to_string(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Resource(bytes),
                ..
            } => assert_eq!(bytes, Bytes::from(vec![1u8; 4])),
            other => panic!("the just-closed segment's final part must still serve, got {other:?}"),
        }

        // A genuinely-nonexistent part of the closed segment is NotFound.
        assert_eq!(
            resolve_now(
                &origin,
                LlHlsRequest::Resource {
                    name: "part-1-1.9.m4s".to_string(),
                }
            ),
            EgressResponse::NotFound
        );

        // The playlist must not resurrect the closed segment's parts as
        // "open" -- it is rendered whole.
        let body = match resolve_now(
            &origin,
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(
            body.contains("seg-1-1.m4s"),
            "closed segment rendered whole: {body}"
        );
        assert!(
            !body.contains("part-1-1."),
            "closed parts not rendered as open: {body}"
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
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:1"), "body: {body}");
        assert!(
            !body.contains("#EXT-X-DISCONTINUITY-SEQUENCE"),
            "nothing has rolled off the window yet: {body}"
        );
        assert!(body.contains("#EXT-X-DISCONTINUITY\n"), "body: {body}");

        // Roll the window: segment 5 evicts segment 1 (not discontinuous;
        // DISCONTINUITY-SEQUENCE stays 0), segment 6 evicts segment 2
        // (discontinuous -- DISCONTINUITY-SEQUENCE becomes 1).
        seg(&writer, 5, 4.0, false);
        let body = match resolve_now(
            &origin,
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        };
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:2"), "body: {body}");
        assert!(
            !body.contains("#EXT-X-DISCONTINUITY-SEQUENCE"),
            "evicted segment 1 was not discontinuous: {body}"
        );

        seg(&writer, 6, 4.0, false);
        let body = match resolve_now(
            &origin,
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
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
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery::default(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
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
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: Some(1002),
                    hls_part: None,
                },
            },
        );
        assert!(matches!(outcome, EgressResponse::BadRequest { .. }));
    }

    #[test]
    fn part_without_msn_rejected() {
        let (_trunk, origin, _writer) = make_origin();
        let outcome = resolve_now(
            &origin,
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: BlockingQuery {
                    hls_msn: None,
                    hls_part: Some(0),
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
            LlHlsRequest::Resource {
                name: "init-1.mp4".to_string(),
            },
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Resource(bytes),
                cache,
            } => {
                assert_eq!(bytes, Bytes::from(vec![0xAAu8; 8]));
                assert_eq!(cache, CachePolicy::Immutable);
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
                LlHlsRequest::Resource {
                    name: "not-a-thing.txt".to_string(),
                }
            ),
            EgressResponse::NotFound
        );
    }
}
