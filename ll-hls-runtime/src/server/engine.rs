//! Blocking-reload/part-availability *decision* logic and playlist rendering
//! for the LL-HLS origin — moved out of `multimux::output::llhls` (issue
//! #663/#717 Stage 2). Sans-IO: every function here is a synchronous poll
//! returning an outcome enum, never a `Future` — the caller (an async
//! adapter, e.g. `multimux`) turns `WouldBlock` into an actual wait using
//! [`super::MediaStore::listen`] (see this module's parent doc for the wait
//! loop shape).
//!
//! Master/media playlist tags are RFC 8216 §4.3.4 (`#EXT-X-STREAM-INF`) and
//! §4.3.3 (`#EXTM3U`/`#EXT-X-VERSION`, rendered by [`media_playlist_m3u8`]);
//! the blocking reload query parameters (`_HLS_msn`/`_HLS_part`) are the
//! Blocking Playlist Reload mechanism of RFC 8216bis §6.2.5.2 — the client
//! asks the origin to hold the response open until the requested Media
//! Sequence Number/part is available, bounded so the origin never hangs
//! indefinitely (the bound itself — a wall-clock timeout — is the adapter's
//! job, not this module's: sans-IO code has no clock).

use transmux::hls::{LowLatencyConfig, MediaPlaylist, MediaSegment, OpenSegment, PartSpec};

use super::store::MediaStore;

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

/// Render the LL-HLS media playlist for `track_id` from `store`'s current
/// segments/live parts.
///
/// RFC 8216bis §4.4.4.9: an in-progress (not yet closed) segment MUST NOT
/// be advertised with an `#EXTINF`/URI pair — that segment has no fetchable
/// resource yet — it may only appear as trailing `#EXT-X-PART` lines.
/// `transmux::hls::MediaPlaylist::open_segment` is exactly this
/// representation: its parts render as trailing `#EXT-X-PART` lines with
/// no `#EXTINF`/URI, so the in-progress segment's parts and the
/// `#EXT-X-PRELOAD-HINT` for the next, not-yet-available part are both
/// rendered by `to_m3u8()` itself — this function only supplies the URI
/// scheme (`part-<track>-<seq>.<idx>.m4s`) and the part metadata.
pub fn media_playlist_m3u8(store: &MediaStore, track_id: u32) -> String {
    // Read these *before* taking `with_segments_and_parts`'s lock below —
    // `MediaStore::max_segment_duration` takes the same `inner` mutex
    // itself, and `std::sync::Mutex` is not reentrant, so calling it from
    // inside the `with_segments_and_parts` closure (as a previous version of
    // this function did) self-deadlocks the calling thread the first time
    // this function is ever invoked with any segment present. Caught by a
    // real network round trip against a live `MediaStore` (issue #717 slice
    // 5's acceptance test) — the existing test suite only ever called this
    // function directly (never over HTTP with two concurrently-scheduled
    // tasks), which happened to never trip the deadlock detector but hung
    // just the same once actually exercised end-to-end. **Preserve this
    // ordering** — see `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`
    // and issue #663/#717.
    let target_duration_secs = store.target_duration_secs();
    let max_segment_duration = store.max_segment_duration();
    store.with_segments_and_parts(|store_segments, live_parts| {
        let media_sequence = store_segments
            .front()
            .map(|s| u64::from(s.segment_seq))
            .or_else(|| live_parts.first().map(|p| u64::from(p.segment_seq)))
            .unwrap_or(1);
        let segments: Vec<MediaSegment> = store_segments
            .iter()
            .map(|s| MediaSegment {
                uri: format!("seg-{track_id}-{}.m4s", s.segment_seq),
                duration: s.duration,
                discontinuous: false,
                parts: Vec::new(),
                ..Default::default()
            })
            .collect();
        let part_target = f64::from(store.part_target_ms()) / 1000.0;
        // The in-progress segment's live parts + the next (not yet available)
        // part's preload-hint URI.
        let open_seq = live_parts.first().map(|p| p.segment_seq);
        let open_segment = open_seq.map(|seq| {
            OpenSegment::new(
                live_parts
                    .iter()
                    .filter(|p| p.segment_seq == seq)
                    .map(|p| PartSpec {
                        uri: format!("part-{track_id}-{}.{}.m4s", p.segment_seq, p.part_index),
                        duration: p.duration,
                        independent: p.independent,
                        ..Default::default()
                    })
                    .collect(),
            )
        });
        let next_part_hint = open_seq.map(|seq| {
            let next_idx = live_parts
                .iter()
                .filter(|p| p.segment_seq == seq)
                .map(|p| p.part_index)
                .max()
                .map(|idx| idx + 1)
                .unwrap_or(0);
            format!("part-{track_id}-{seq}.{next_idx}.m4s")
        });
        // RFC 8216bis §4.4.3.1 (MUST): every Media Segment's EXTINF duration,
        // rounded to the nearest integer, MUST be <= TARGETDURATION. The
        // segmenter cuts on the next keyframe *after* the configured target,
        // so a real segment routinely exceeds it — advertising the
        // configured target alone can under-declare. Use whichever is
        // larger, rounded (not the configured value's `ceil()` alone).
        let target_duration = target_duration_secs.max(max_segment_duration).round() as u32;
        let playlist = MediaPlaylist {
            version: LL_HLS_VERSION,
            target_duration,
            media_sequence,
            discontinuity_sequence: 0,
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
    })
}

/// A minimal single-variant master playlist pointing at `media_playlist_name`
/// (the caller's configured media-playlist filename — e.g. multimux's
/// `Config::playlist_name`, defaulting to `"media.m3u8"`) — the same
/// regardless of `MediaStore` state (no multi-rendition support yet), so this
/// takes no store argument.
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

/// The result of [`MediaStore::resolve_playlist`]: either the rendered
/// playlist is ready now, the request is malformed/abusive (RFC 8216bis
/// §6.2.5.2 abuse prevention — reject immediately, no wait), or the awaited
/// segment/part isn't available *yet* (the caller should wait for the next
/// change notification and re-resolve).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistOutcome {
    /// The rendered media playlist body.
    Ready(String),
    /// The awaited condition (`hls_msn`/`hls_part`) isn't satisfied yet —
    /// wait for [`super::MediaStore::listen`] and re-resolve.
    WouldBlock,
    /// The request is malformed (`hls_part` without `hls_msn`) or abusive
    /// (`hls_msn` unreasonably far beyond the live edge) — reject now, don't
    /// wait.
    BadRequest,
}

/// `Cache-Control` policy an adapter applies to a resolved resource —
/// playlists are always re-fetched for liveness (not modeled here since
/// [`PlaylistOutcome::Ready`] is playlist-only), while a produced init/
/// segment/part byte range never changes once produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// Safe to cache indefinitely — a given URI's bytes never change once
    /// produced (each segment/part is generated exactly once under a unique
    /// filename).
    Immutable,
    /// Must always be re-fetched (liveness-sensitive).
    NoCache,
}

/// The result of [`MediaStore::resolve_resource`]: the resource's bytes are
/// ready, the request should wait (a preload-hinted part not yet produced),
/// or the resource does not (and, for a part whose segment already closed
/// without it, will never) exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceOutcome {
    /// The resource's bytes, plus the cache policy an adapter should apply.
    Ready {
        /// The resolved bytes.
        bytes: Vec<u8>,
        /// `Cache-Control` policy for these bytes.
        cache: CachePolicy,
    },
    /// A preload-hinted part that hasn't been produced yet — wait for
    /// [`super::MediaStore::listen`] and re-resolve.
    WouldBlock,
    /// The named resource does not exist and never will (unknown filename
    /// shape, or a part whose segment closed without ever producing it).
    NotFound,
}

impl MediaStore {
    /// Resolve a `GET media.m3u8` request against `_HLS_msn`/`_HLS_part`
    /// blocking-reload semantics (RFC 8216bis §6.2.5.2), rendering
    /// [`media_playlist_m3u8`] for `track_id` once the awaited condition is
    /// satisfied (or immediately, if `query` carries no blocking
    /// parameters).
    ///
    /// `_HLS_msn` alone waits for segment `msn` to **close**; `_HLS_msn`+
    /// `_HLS_part` waits only for that part of the (possibly still open)
    /// segment — these are genuinely different conditions (treating a bare
    /// `_HLS_msn` as `_HLS_part=0` would resolve as soon as the segment
    /// merely opens with one live part, before it has an `#EXTINF`/URI at
    /// all). `_HLS_part` without `_HLS_msn` is meaningless (a part is only
    /// addressable relative to a segment) and a `_HLS_msn` unreasonably far
    /// beyond the live edge is either a broken client or abuse — both
    /// [`PlaylistOutcome::BadRequest`] immediately rather than
    /// [`PlaylistOutcome::WouldBlock`]ing.
    pub fn resolve_playlist(&self, track_id: u32, query: BlockingQuery) -> PlaylistOutcome {
        if query.hls_part.is_some() && query.hls_msn.is_none() {
            return PlaylistOutcome::BadRequest;
        }
        if let Some(msn) = query.hls_msn {
            let (current_max_msn, _) = self.latest_progress();
            if msn > u64::from(current_max_msn) + ABUSE_MSN_FUTURE_BOUND {
                return PlaylistOutcome::BadRequest;
            }
            let satisfied = match query.hls_part {
                Some(part) => {
                    let (in_progress_seg_seq, part_count) = self.latest_progress();
                    u64::from(in_progress_seg_seq) > msn
                        || (u64::from(in_progress_seg_seq) == msn && part_count > part)
                }
                None => u64::from(self.last_closed_segment_seq()) >= msn,
            };
            if !satisfied {
                return PlaylistOutcome::WouldBlock;
            }
        }
        PlaylistOutcome::Ready(media_playlist_m3u8(self, track_id))
    }

    /// Resolve a dynamic origin filename (`init-{track}.mp4`, `seg-{track}-
    /// {seq}.m4s`, `part-{track}-{seq}.{idx}.m4s`) to its bytes.
    ///
    /// A part request is the preload-hinted Partial Segment a client fetches
    /// ahead of time (RFC 8216bis §6.2.2, §6.3.1). If the origin promised it
    /// via `#EXT-X-PRELOAD-HINT` but hasn't produced it yet,
    /// [`ResourceOutcome::WouldBlock`] — the caller should hold the request
    /// open (not 404 immediately, which spams errors and defeats low
    /// latency). [`ResourceOutcome::NotFound`] is returned **promptly**
    /// (without the caller needing to wait out its own timeout) once the
    /// part can no longer appear: its segment has closed (now only
    /// addressable as a whole segment via `seg-…`), or the in-progress
    /// segment has advanced past it — a legitimate 404 the client answers by
    /// fetching the next segment/part.
    pub fn resolve_resource(&self, name: &str) -> ResourceOutcome {
        if let Some((seq, idx)) = parse_part(name) {
            return match self.part_bytes(seq, idx) {
                Some(bytes) => ResourceOutcome::Ready {
                    bytes,
                    cache: CachePolicy::Immutable,
                },
                None => {
                    let (in_progress_seg_seq, _) = self.latest_progress();
                    if in_progress_seg_seq > seq || self.segment_bytes(seq).is_some() {
                        ResourceOutcome::NotFound
                    } else {
                        ResourceOutcome::WouldBlock
                    }
                }
            };
        }
        match resolve_file(self, name) {
            Some(bytes) => ResourceOutcome::Ready {
                bytes,
                cache: CachePolicy::Immutable,
            },
            None => ResourceOutcome::NotFound,
        }
    }
}

/// Parse a `part-{track}-{seq}.{idx}.m4s` dynamic filename into `(seq, idx)`,
/// or `None` if it isn't a part filename (or its numeric fields don't parse).
/// `{track}` is validated but unused (see [`resolve_file`]).
fn parse_part(file: &str) -> Option<(u32, u32)> {
    let rest = file.strip_prefix("part-")?.strip_suffix(".m4s")?;
    let (track_seq, idx) = rest.rsplit_once('.')?;
    let (track, seq) = track_seq.split_once('-')?;
    track.parse::<u32>().ok()?;
    Some((seq.parse().ok()?, idx.parse().ok()?))
}

/// Parse a dynamic origin filename and fetch its bytes from `store`:
/// - `init-{track}.mp4` -> [`MediaStore::init_bytes`]
/// - `seg-{track}-{seq}.m4s` -> [`MediaStore::segment_bytes`]
///
/// Part filenames (`part-{track}-{seq}.{idx}.m4s`) are handled separately in
/// [`MediaStore::resolve_resource`] (they can block until available — see
/// [`parse_part`]), not here. `{track}` is validated as a number but
/// otherwise unused: `store` holds a single track's data (see
/// [`DEFAULT_TRACK_ID`]). Returns `None` (-> 404) for any filename that
/// doesn't match one of these shapes, or whose numeric fields don't parse.
fn resolve_file(store: &MediaStore, file: &str) -> Option<Vec<u8>> {
    if let Some(rest) = file.strip_prefix("init-") {
        let track = rest.strip_suffix(".mp4")?;
        track.parse::<u32>().ok()?;
        return store.init_bytes();
    }
    if let Some(rest) = file.strip_prefix("seg-") {
        let rest = rest.strip_suffix(".m4s")?;
        let (track, seq) = rest.split_once('-')?;
        track.parse::<u32>().ok()?;
        let seq: u32 = seq.parse().ok()?;
        return store.segment_bytes(seq);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::ll_hls::{PartInfo, SegmentInfo};

    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![0x10 + idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    fn seg(seq: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![0x20 + seq as u8; 8],
            duration: 4.0,
            segment_seq: seq,
            part_count: 2,
        }
    }

    /// A populated store: a closed segment 1, plus two live parts of
    /// in-progress segment 2 -- so `latest_progress()` is `(2, 2)`.
    fn make_store() -> MediaStore {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_init(vec![0xAA; 8]);
        store.add_segment(seg(1));
        store.add_part(part(2, 0));
        store.add_part(part(2, 1));
        store
    }

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

    #[test]
    fn resolve_playlist_no_query_is_ready_now() {
        let store = make_store();
        let outcome = store.resolve_playlist(DEFAULT_TRACK_ID, BlockingQuery::default());
        match outcome {
            PlaylistOutcome::Ready(body) => assert!(body.contains("#EXT-X-PART"), "body: {body}"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn resolve_playlist_already_satisfied_earlier_msn_is_ready() {
        // latest_progress() for the store is (2, 2): asking for msn=1 (an
        // earlier segment) is already satisfied and must not WouldBlock.
        let store = make_store();
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: Some(1),
                hls_part: Some(0),
            },
        );
        assert!(matches!(outcome, PlaylistOutcome::Ready(_)));
    }

    #[test]
    fn resolve_playlist_already_satisfied_same_msn_lower_part_is_ready() {
        // in_progress_seg_seq == msn and part_count(2) > part(1): satisfied.
        let store = make_store();
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: Some(2),
                hls_part: Some(1),
            },
        );
        assert!(matches!(outcome, PlaylistOutcome::Ready(_)));
    }

    #[test]
    fn resolve_playlist_msn_only_waits_for_closed_segment_not_just_open_parts() {
        // make_store()'s segment 2 is OPEN with 2 live parts
        // (latest_progress() == (2, 2)) but not yet CLOSED. RFC 8216bis
        // §6.2.5.2: a bare `_HLS_msn=2` (no `_HLS_part`) must WouldBlock, not
        // resolve merely because it has live parts — treating this as
        // `_HLS_part=0` (satisfied by part_count(2) > 0) would wrongly return
        // Ready here.
        let store = make_store();
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: Some(2),
                hls_part: None,
            },
        );
        assert_eq!(outcome, PlaylistOutcome::WouldBlock);

        // Once segment 2 actually closes, the same query resolves.
        store.add_segment(seg(2));
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: Some(2),
                hls_part: None,
            },
        );
        match outcome {
            PlaylistOutcome::Ready(body) => assert!(
                body.contains("seg-1-2.m4s"),
                "resolved playlist must show segment 2 as closed: {body}"
            ),
            other => panic!("expected Ready after close, got {other:?}"),
        }
    }

    #[test]
    fn resolve_playlist_msn_within_bound_would_block_until_part_lands() {
        // Sanity check for the abuse-bound logic: a legitimate
        // just-ahead-of-live-edge msn/part WouldBlocks (not BadRequest), then
        // resolves once the part lands.
        let store = make_store(); // latest_progress() == (2, 2)
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: Some(2),
                hls_part: Some(2),
            },
        );
        assert_eq!(outcome, PlaylistOutcome::WouldBlock);

        store.add_part(part(2, 2));
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: Some(2),
                hls_part: Some(2),
            },
        );
        assert!(matches!(outcome, PlaylistOutcome::Ready(_)));
    }

    #[test]
    fn resolve_playlist_far_future_msn_rejected() {
        // latest_progress() for make_store() is (2, 2). A `_HLS_msn` 1000
        // ahead of the live edge is not a legitimate blocking-reload request
        // (RFC 8216bis §6.2.5.2 abuse prevention) — BadRequest immediately.
        let store = make_store();
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: Some(1002),
                hls_part: None,
            },
        );
        assert_eq!(outcome, PlaylistOutcome::BadRequest);
    }

    #[test]
    fn resolve_playlist_part_without_msn_rejected() {
        // RFC 8216bis §6.2.5.2: `_HLS_part` without `_HLS_msn` is
        // meaningless (a part is only addressable relative to a segment).
        let store = make_store();
        let outcome = store.resolve_playlist(
            DEFAULT_TRACK_ID,
            BlockingQuery {
                hls_msn: None,
                hls_part: Some(0),
            },
        );
        assert_eq!(outcome, PlaylistOutcome::BadRequest);
    }

    #[test]
    fn resolve_resource_init_present() {
        let store = make_store();
        let outcome = store.resolve_resource("init-1.mp4");
        match outcome {
            ResourceOutcome::Ready { bytes, cache } => {
                assert_eq!(bytes, vec![0xAA; 8]);
                assert_eq!(cache, CachePolicy::Immutable);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn resolve_resource_segment_present_and_absent() {
        let store = make_store();
        match store.resolve_resource("seg-1-1.m4s") {
            ResourceOutcome::Ready { bytes, .. } => assert_eq!(bytes, vec![0x21; 8]),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert_eq!(
            store.resolve_resource("seg-1-99.m4s"),
            ResourceOutcome::NotFound
        );
    }

    #[test]
    fn resolve_resource_part_present() {
        let store = make_store();
        match store.resolve_resource("part-1-2.0.m4s") {
            ResourceOutcome::Ready { bytes, .. } => assert_eq!(bytes, vec![0x10; 4]),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn resolve_resource_part_not_yet_produced_would_block() {
        // part-1-2.2 is the preload-hinted next part of in-progress segment 2
        // (which currently has parts .0 and .1). Not yet produced -> WouldBlock,
        // not NotFound (the caller waits, doesn't 404 immediately).
        let store = make_store();
        assert_eq!(
            store.resolve_resource("part-1-2.2.m4s"),
            ResourceOutcome::WouldBlock
        );
        store.add_part(part(2, 2));
        match store.resolve_resource("part-1-2.2.m4s") {
            ResourceOutcome::Ready { bytes, .. } => assert_eq!(bytes, vec![0x12; 4]),
            other => panic!("expected Ready once produced, got {other:?}"),
        }
    }

    #[test]
    fn resolve_resource_part_not_found_once_segment_closes_without_it() {
        // part-1-2.9 will never be produced. Once segment 2 closes (advancing
        // the in-progress segment), the part must resolve NotFound promptly —
        // not WouldBlock forever.
        let store = make_store();
        assert_eq!(
            store.resolve_resource("part-1-2.9.m4s"),
            ResourceOutcome::WouldBlock,
            "not yet decidable while segment 2 is still open"
        );
        store.add_segment(seg(2));
        assert_eq!(
            store.resolve_resource("part-1-2.9.m4s"),
            ResourceOutcome::NotFound,
            "must resolve NotFound once segment 2 has closed without producing it"
        );
    }

    #[test]
    fn resolve_resource_part_served_from_recent_after_close() {
        // Segment 2 has live parts .0 and .1; close it. Its final part must
        // still resolve Ready (from recent_parts) — an in-flight
        // preload-hint request racing the segment close must not NotFound.
        let store = make_store();
        store.add_segment(seg(2)); // close segment 2, moving its parts to recent_parts
        match store.resolve_resource("part-1-2.1.m4s") {
            ResourceOutcome::Ready { bytes, .. } => assert_eq!(bytes, vec![0x11; 4]),
            other => panic!("a just-closed segment's part must still resolve Ready, got {other:?}"),
        }
    }

    #[test]
    fn resolve_resource_part_of_old_segment_not_found() {
        // Segment 1 closed in make_store() with no parts recorded and is old
        // enough to be past the recent-parts retention window, so its parts
        // resolve NotFound without ever WouldBlocking (they will never be
        // produced and aren't individually addressable anymore).
        let store = make_store();
        assert_eq!(
            store.resolve_resource("part-1-1.0.m4s"),
            ResourceOutcome::NotFound
        );
    }

    #[test]
    fn resolve_resource_unmatched_filename_not_found() {
        let store = make_store();
        assert_eq!(
            store.resolve_resource("not-a-thing.txt"),
            ResourceOutcome::NotFound
        );
    }

    // --- Playlist-rendering content tests (moved from
    // `multimux::output::llhls`, which now delegates rendering here) ---

    fn plain_seg(seq: u32, parts: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![seq as u8; 8],
            duration: 4.0,
            segment_seq: seq,
            part_count: parts,
        }
    }
    fn plain_part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    #[test]
    fn playlist_has_llhls_tags_and_parts() {
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(plain_part(1, 0));
        s.add_part(plain_part(1, 1));
        let m = media_playlist_m3u8(&s, 1);
        assert!(m.contains("#EXT-X-PART-INF"), "PART-INF present");
        assert!(
            m.contains("#EXT-X-SERVER-CONTROL"),
            "SERVER-CONTROL present"
        );
        assert!(m.contains("#EXT-X-PART"), "at least one PART");
        assert!(
            m.contains("part-1-1.0.m4s") || m.contains("part-1-1.1.m4s"),
            "part URI"
        );
    }

    #[test]
    fn open_segment_has_parts_but_no_extinf() {
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(plain_part(1, 0));
        s.add_part(plain_part(1, 1));
        let m = media_playlist_m3u8(&s, 1);
        // The in-progress segment's parts are advertised...
        assert!(m.contains("#EXT-X-PART"), "at least one PART line");
        assert!(m.contains("part-1-1.0.m4s"), "part 0 URI present");
        assert!(m.contains("part-1-1.1.m4s"), "part 1 URI present");
        // ...but RFC 8216bis §4.4.4.9: no premature #EXTINF/URI for the
        // not-yet-closed segment itself — "seg-1-1.m4s" must not appear
        // anywhere (it isn't fetchable; that segment hasn't been closed).
        assert!(
            !m.contains("seg-1-1.m4s"),
            "no full-segment URI for the open segment: {m}"
        );
        assert!(
            !m.contains("#EXTINF"),
            "no EXTINF for the open segment: {m}"
        );
    }

    #[test]
    fn final_part_fetchable_after_its_segment_closes() {
        // The segmenter emits a segment's final part and then closes the
        // segment in the same step. A preload-hint request for that final part
        // is typically in flight when the close happens, so it must remain
        // fetchable afterwards (from recent_parts) rather than 404 — the LL-HLS
        // preload-hint boundary bug.
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_part(plain_part(1, 0));
        s.add_part(plain_part(1, 1)); // .1 is this segment's final part
        s.add_segment(plain_seg(1, 2)); // close segment 1 (moves its parts to recent_parts)
        assert_eq!(
            s.resolve_resource("part-1-1.1.m4s"),
            ResourceOutcome::Ready {
                bytes: vec![1; 4],
                cache: CachePolicy::Immutable
            },
            "final part of a just-closed segment must still be individually fetchable"
        );
        assert_eq!(
            s.resolve_resource("part-1-1.0.m4s"),
            ResourceOutcome::Ready {
                bytes: vec![0; 4],
                cache: CachePolicy::Immutable
            },
            "earlier parts too"
        );
        // A genuinely-nonexistent part of the closed segment is NotFound.
        assert_eq!(
            s.resolve_resource("part-1-1.9.m4s"),
            ResourceOutcome::NotFound
        );
        // Closing does not resurrect parts into the rendered open segment: the
        // playlist advertises the whole segment, not its parts.
        let m = media_playlist_m3u8(&s, 1);
        assert!(
            m.contains("seg-1-1.m4s"),
            "closed segment rendered whole: {m}"
        );
        assert!(
            !m.contains("part-1-1."),
            "closed parts not rendered as open: {m}"
        );
    }

    #[test]
    fn live_parts_capped_when_segment_never_closes() {
        // target_duration_secs=4.0, part_target_ms=500 -> cap =
        // ceil(4.0 / 0.5) + 4 margin = 12 (see
        // `super::super::store::compute_max_live_parts`).
        let s = MediaStore::new(4.0, 500, 4);
        let cap = super::super::store::compute_max_live_parts(4.0, 500);
        assert_eq!(cap, 12, "sanity-check the expected cap for these params");
        s.set_init(vec![0; 4]);

        // Push far more parts than the cap into a single never-closed
        // segment (no add_segment call) — RAM must stay bounded.
        for i in 0..(cap as u32 * 5) {
            s.add_part(plain_part(1, i));
        }
        assert_eq!(
            s.live_part_count(),
            cap,
            "live_parts must stay capped even though the segment never closed"
        );

        // The playlist must still render correctly from the capped parts:
        // only the most recent (highest-index) parts survive.
        let m = media_playlist_m3u8(&s, 1);
        assert!(m.contains("#EXT-X-PART"), "still has PART lines: {m}");
        let last_idx = cap as u32 * 5 - 1;
        assert!(
            m.contains(&format!("part-1-1.{last_idx}.m4s")),
            "most recent part must survive the cap: {m}"
        );
        let first_idx = cap as u32 * 5 - cap as u32;
        assert!(
            !m.contains(&format!("part-1-1.{}.m4s", first_idx - 1)),
            "an older part beyond the cap must have been dropped: {m}"
        );
    }

    // --- P2 LL-HLS spec-conformance fixes (audit-llhls #1/#2/#3/#4) ---

    #[test]
    fn target_duration_is_max_of_configured_and_actual_segment_duration() {
        // Configured target is 4.0s, but the segmenter cuts on the next
        // keyframe after the target so a real segment can run long (7.5s
        // here) — RFC 8216bis §4.4.3.1 (MUST) requires TARGETDURATION to be
        // >= every EXTINF, rounded. The old hardcoded
        // `ceil(target_duration_secs)` would render `4`, violating the MUST.
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        let mut long_seg = plain_seg(1, 2);
        long_seg.duration = 7.5;
        s.add_segment(long_seg);
        let m = media_playlist_m3u8(&s, 1);
        assert!(
            m.contains("#EXT-X-TARGETDURATION:8"),
            "TARGETDURATION must be round(7.5)=8, not the configured target (4): {m}"
        );
    }

    #[test]
    fn target_duration_falls_back_to_configured_when_segments_are_short() {
        let s = MediaStore::new(4.0, 500, 4);
        s.set_init(vec![0; 4]);
        s.add_segment(plain_seg(1, 2)); // plain_seg's fixed duration is 4.0
        let m = media_playlist_m3u8(&s, 1);
        assert!(
            m.contains("#EXT-X-TARGETDURATION:4"),
            "unchanged behaviour when no segment exceeds the configured target: {m}"
        );
    }
}
