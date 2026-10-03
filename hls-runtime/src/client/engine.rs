//! [`HlsClient`] — the sans-IO caller-driven engine.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use broadcast_common::Unpackage;
use broadcast_hls::{ByteRange, MapTag, MediaPlaylist, MediaSegment, OpenSegment, PreloadHintType};
use transmux::{DemuxEvent, Fmp4Demux, Sample, StreamingTsDemux, TrackSpec};

use super::action::{Action, BlockingReload, ResourceId};
use super::error::{Error, Result};
use super::output::Output;
use super::url;

/// First byte of every MPEG-2 TS packet (ITU-T H.222.0 / ISO/IEC 13818-1
/// §2.4.3.2 `sync_byte`). Classic MPEG-TS-segment HLS (HLS v3, RFC 8216 —
/// the dominant legacy/IPTV form) has no `EXT-X-MAP`/init segment at all:
/// each `.ts` segment is a self-contained PAT/PMT/PES stream, so this byte
/// is the only available signal to distinguish one from an fMP4/CMAF
/// segment (which starts with an ISOBMFF box: `ftyp`/`styp`/`moof`) once the
/// playlist itself has never advertised a Media Initialization Section.
const TS_SYNC_BYTE: u8 = 0x47;

/// Movie timescale of the init segment synthesized for classic TS-segment
/// HLS: the 90 kHz MPEG-2 system clock the PES PTS/DTS are expressed in
/// (ISO/IEC 13818-1 §2.4.3.7), the same clock `transmux`'s TS demuxer stamps
/// its samples with.
const TS_MOVIE_TIMESCALE: u32 = 90_000;

/// How many Target Durations before the end of a live Playlist the first
/// segment the client plays must start at the latest (RFC 8216 §6.3.3: "the
/// client SHOULD NOT choose a segment that starts less than three target
/// durations from the end of the Playlist file").
const LIVE_EDGE_TARGET_DURATIONS: f64 = 3.0;

/// [`StreamingTsDemux`] with a `Debug` impl (the transmux type has none), so
/// [`HlsClient`] can keep deriving it.
struct TsDemuxState(StreamingTsDemux);

impl core::fmt::Debug for TsDemuxState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TsDemuxState(StreamingTsDemux)")
    }
}

/// `usize` index -> `u64` (lossless on every supported target; saturates
/// rather than casting if a target ever had a wider `usize`).
fn index_u64(i: usize) -> u64 {
    u64::try_from(i).unwrap_or(u64::MAX)
}

/// The Media Sequence Number a tracked resource belongs to, `None` for the
/// MSN-less init segment.
fn resource_msn(id: &ResourceId) -> Option<u64> {
    match id {
        ResourceId::Part { msn, .. } | ResourceId::Segment { msn } => Some(*msn),
        _ => None,
    }
}

/// How a new playlist relates to the previous one (see
/// [`HlsClient::continuity`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Continuity {
    /// The same stream, moved on (or the same window again).
    Continues,
    /// The same stream, but an older view of it that still overlaps the
    /// previous window.
    Lagging,
    /// The same stream, but a view wholly *older* than the previous window
    /// (nothing in common): its segments are skipped, never delivered behind
    /// what was already delivered.
    OlderWindow,
    /// A different stream: an origin restart.
    Restart,
}

/// The `EXT-X-PROGRAM-DATE-TIME` value attached to `seg`, if any.
fn program_date_time(seg: &MediaSegment) -> Option<&str> {
    seg.pre_tags
        .iter()
        .find_map(|t| t.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:"))
}

/// [`program_date_time`] as milliseconds since the Unix epoch.
fn program_date_time_ms(seg: &MediaSegment) -> Option<i64> {
    parse_rfc3339_ms(program_date_time(seg)?)
}

/// Parse an RFC 3339 / ISO 8601 date-time (`2024-05-01T12:00:00.250Z`,
/// `...+02:00`, `...-0500`) into Unix milliseconds. `None` for anything that
/// does not parse — never a guess. Delegates to [`jiff::Timestamp`] (an offset
/// is required; a leap second is clamped to `:59`).
fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    text.parse::<jiff::Timestamp>()
        .ok()
        .map(|t| t.as_millisecond())
}

/// A driveable, sans-IO Low-Latency HLS (RFC 8216bis) playback client.
///
/// `HlsClient` never touches a socket or a clock. The caller drives it:
///
/// 1. [`HlsClient::new`] seeds the first [`Action::FetchPlaylist`]; drain it
///    with [`HlsClient::poll`] and perform the GET.
/// 2. Feed the response back with [`HlsClient::on_playlist`] (playlist) or
///    [`HlsClient::on_resource`] (init/part/segment bytes) —
///    [`Action::FetchResource`]'s `id` correlates the two.
/// 3. Drain [`HlsClient::poll`] again for the next round of actions (a new
///    reload, newly discoverable parts, a preload-hint prefetch, ...) and
///    [`HlsClient::next_output`] for newly available [`Output`]s.
///
/// # Behaviour
///
/// - **Reload scheduling** (issue #717 slice 2): once a playlist advertises
///   `EXT-X-SERVER-CONTROL`/`EXT-X-PART-INF` **and** the origin's
///   `CAN-BLOCK-RELOAD` attribute is `YES`
///   ([`broadcast_hls::LowLatencyConfig::can_block_reload`] is `true` —
///   *not* merely [`broadcast_hls::MediaPlaylist::low_latency`] being
///   `Some`, since an origin may carry parts/PART-INF while still
///   advertising `CAN-BLOCK-RELOAD=NO`), every reload is a Blocking
///   Playlist Reload (RFC 8216bis §6.2.5.2) naming the next not-yet-seen
///   Partial Segment's `_HLS_msn`/`_HLS_part`. Otherwise reloads are plain GETs
///   paced by an [`Action::WaitMs`] hint derived from `#EXT-X-TARGETDURATION`.
///   `EXT-X-SKIP`/`CAN-SKIP-UNTIL` Playlist Delta Updates (RFC 8216bis §4.4.5.2)
///   are requested once a full-playlist baseline exists, and merged back into
///   a full view before further processing — see `merge_delta` internally.
/// - **Fetch pipeline** (slice 3): the `EXT-X-PRELOAD-HINT`ed part is fetched
///   ahead of its own appearance as a numbered `EXT-X-PART`; `BYTERANGE`
///   parts are supported, including the RFC 8216bis §4.4.4.9 "omitted offset
///   means immediately after the previous sub-range of the same resource"
///   rule (tracked per resource URL). The Media Initialization Section
///   (`EXT-X-MAP`) is fetched once and reused for every following resource
///   until the map changes.
/// - **Dedup / coalescing**: once *any* of a segment's parts have been
///   individually fetched, that segment is never re-fetched whole — when it
///   later closes (`#EXTINF`+URI), the client only fetches whichever of its
///   parts (if any) are still missing, and marks the segment "delivered" once
///   every part is accounted for (fetched, or `GAP=YES`). A playlist whose
///   segments carry **no** parts at all (a non-LL origin) falls back to
///   fetching the whole segment resource — the two paths never overlap for a
///   single segment, so a part's samples are never double-counted against its
///   parent's.
/// - **Output adapter** (slice 4): exactly one [`Output::Init`] precedes any
///   [`Output::Samples`]; parts/segments are demuxed via
///   [`transmux::Fmp4Demux`] (by concatenating the cached init bytes with the
///   fetched resource — this crate never re-implements ISOBMFF box parsing,
///   only reuses transmux's), so `Output::Samples` carries real access units,
///   not opaque container bytes. `#EXT-X-DISCONTINUITY` on a segment surfaces
///   as [`Output::Discontinuity`] immediately before that segment's first
///   samples. **Known limitation**: an in-progress ([`OpenSegment`]) segment
///   carries no discontinuity flag of its own (only a *closed*
///   [`MediaSegment`] does) — if every part of a segment was already
///   delivered while it was still open, a discontinuity revealed only once it
///   closes is signalled late (after those parts' samples, not before). This
///   is a gap in the current wire model ([`broadcast_hls::OpenSegment`]), not
///   something this crate can fix locally.
/// - **Classic MPEG-TS-segment HLS** (issue #760): a playlist that never
///   advertises an `EXT-X-MAP` (HLS v3, the dominant legacy/IPTV form —
///   self-contained `.ts` segments carrying their own PAT/PMT/PES, no
///   separate init resource) routes each fetched Part/Segment through
///   [`transmux::TsDemux`] instead, content-sniffed by the MPEG-TS sync byte
///   rather than blocked on an init fetch that will never come. The first
///   successfully demuxed segment's recovered
///   [`TrackSpec`]s synthesize the one [`Output::Init`] this crate's contract
///   requires (via [`transmux::build_init_segment`]) so downstream callers
///   (e.g. `multimux`'s `HlsPull`, which recovers track specs from
///   `Output::Init`) need no TS-specific handling of their own. The
///   fMP4/CMAF plus LL (parts/preload-hint) path above is entirely
///   unchanged; the two never overlap for a single playlist.
#[derive(Debug)]
pub struct HlsClient {
    playlist_url: String,

    pending_actions: VecDeque<Action>,
    /// The caller-clock instant at which the `WaitMs` now at the front of
    /// `pending_actions` was first observed by [`Self::poll_timeout`]; the
    /// wait's absolute deadline is `anchor + wait`. Cleared whenever an action
    /// is drained, so a later wait is anchored afresh.
    #[cfg(feature = "std")]
    wait_anchor: Option<std::time::Instant>,
    pending_outputs: VecDeque<Output>,

    init_uri: Option<String>,
    init_bytes: Option<Vec<u8>>,
    init_emitted: bool,
    /// Part/Segment resources delivered before the init segment arrived —
    /// buffered (in arrival order) and replayed once [`Self::init_bytes`] is
    /// set, so the caller's fetch/response IO can complete in any order
    /// (a real HTTP client has no reason to serialize on init-first).
    pending_demux: VecDeque<(ResourceId, Vec<u8>)>,

    requested: BTreeSet<ResourceId>,
    /// Ids whose bytes `on_resource` has accepted (delivered *or* buffered in
    /// [`Self::pending_demux`]) — the exactly-once guard against a second
    /// delivery of the same id. Pruned with the other MSN-keyed sets.
    fulfilled: BTreeSet<ResourceId>,
    /// `true` once the first playlist has been processed (the live-edge join
    /// of RFC 8216 §6.3.3 applies to that one only).
    joined: bool,
    /// Classic-TS-HLS demuxer kept across segments, so the 33-bit PTS/DTS
    /// unwrap state and the PMT-derived track set survive a segment
    /// boundary. Replaced by a fresh one at `EXT-X-DISCONTINUITY`.
    ts_demux: Option<TsDemuxState>,
    /// The track specs `ts_demux` has reported, by track id.
    ts_specs: BTreeMap<u32, TrackSpec>,
    /// The init segment last synthesized from `ts_specs`; a new
    /// [`Output::Init`] goes out only when a re-synthesis differs from it.
    last_ts_init: Option<Vec<u8>>,
    delivered_parts: BTreeSet<(u64, u64)>,
    delivered_segments: BTreeSet<u64>,
    discontinuous_msns: BTreeSet<u64>,
    discontinuity_emitted: BTreeSet<u64>,
    byte_range_cursor: BTreeMap<String, u64>,

    outstanding_fetches: u64,
    saw_endlist: bool,
    end_emitted: bool,
    last_full_playlist: Option<MediaPlaylist>,
    /// `EXT-X-MEDIA-SEQUENCE` of the previous playlist: RFC 8216 §6.2.2
    /// forbids it ever decreasing, so a smaller one is an origin restart
    /// (see [`Self::restart_after_regression`]).
    last_media_sequence: Option<u64>,
}

impl HlsClient {
    /// Create a new client for the Media Playlist at `playlist_url`, seeding
    /// the first [`Action::FetchPlaylist`] (a plain, non-blocking GET — the
    /// client does not yet know whether the origin supports blocking reload).
    pub fn new(playlist_url: impl Into<String>) -> Self {
        let playlist_url = playlist_url.into();
        let mut pending_actions = VecDeque::new();
        pending_actions.push_back(Action::FetchPlaylist {
            url: playlist_url.clone(),
            blocking: None,
            skip: false,
        });
        Self {
            playlist_url,
            pending_actions,
            #[cfg(feature = "std")]
            wait_anchor: None,
            pending_outputs: VecDeque::new(),
            init_uri: None,
            init_bytes: None,
            init_emitted: false,
            pending_demux: VecDeque::new(),
            requested: BTreeSet::new(),
            fulfilled: BTreeSet::new(),
            joined: false,
            ts_demux: None,
            ts_specs: BTreeMap::new(),
            last_ts_init: None,
            delivered_parts: BTreeSet::new(),
            delivered_segments: BTreeSet::new(),
            discontinuous_msns: BTreeSet::new(),
            discontinuity_emitted: BTreeSet::new(),
            byte_range_cursor: BTreeMap::new(),
            outstanding_fetches: 0,
            saw_endlist: false,
            end_emitted: false,
            last_full_playlist: None,
            last_media_sequence: None,
        }
    }

    /// The Media Playlist URL this client is following.
    pub fn playlist_url(&self) -> &str {
        &self.playlist_url
    }

    /// Drain the next IO [`Action`] the caller must perform, if any.
    pub fn poll(&mut self) -> Option<Action> {
        #[cfg(feature = "std")]
        {
            self.wait_anchor = None;
        }
        self.pending_actions.pop_front()
    }

    /// The `WaitMs` hint at the front of the action queue, as a `Duration`;
    /// `None` when the next queued action is a fetch (or nothing is queued). A
    /// wait is queued BEHIND the fetches `on_playlist` queued with it, so it is
    /// reported only once those have been drained with [`Self::poll`].
    /// Peeking never consumes it, and the core never reads a clock.
    pub fn next_wait(&self) -> Option<core::time::Duration> {
        match self.pending_actions.front() {
            Some(Action::WaitMs(ms)) => Some(core::time::Duration::from_millis(*ms)),
            _ => None,
        }
    }

    /// The absolute deadline of the queued `WaitMs`, for callers that schedule
    /// on `std::time::Instant`: `None` unless a wait is at the front of the
    /// queue (see [`Self::next_wait`]).
    ///
    /// The deadline is anchored the first time this is called for a given wait
    /// (`now` + the wait) and every later call returns that SAME instant, until
    /// the wait is drained with [`Self::poll`]. Re-querying after unrelated
    /// wake-ups therefore never re-arms the timer. The core has no clock of its
    /// own, so "when the wait was scheduled" is the caller's `now` at the first
    /// query: query as soon as the wait reaches the front.
    #[cfg(feature = "std")]
    pub fn poll_timeout(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        let wait = self.next_wait()?;
        let anchor = *self.wait_anchor.get_or_insert(now);
        Some(anchor + wait)
    }

    /// Drain the next [`Output`] event, if any.
    pub fn next_output(&mut self) -> Option<Output> {
        self.pending_outputs.pop_front()
    }

    /// Feed a freshly fetched Media Playlist response.
    ///
    /// # Errors
    /// [`Error::PlaylistNotUtf8`] / [`Error::PlaylistParse`] on malformed
    /// input.
    pub fn on_playlist(&mut self, bytes: &[u8]) -> Result<()> {
        let text = core::str::from_utf8(bytes)?;
        let playlist = MediaPlaylist::parse(text)?;
        let playlist = self.merge_delta(playlist);
        // An older view of the same stream must never make the client deliver
        // backwards: its segments below the previous window are skipped.
        let mut lag_floor: Option<u64> = None;
        if self.last_media_sequence.is_some() {
            match self.continuity(&playlist) {
                Continuity::Restart => self.restart_after_regression()?,
                Continuity::OlderWindow => {
                    lag_floor = self.last_full_playlist.as_ref().map(|p| p.media_sequence);
                }
                Continuity::Lagging | Continuity::Continues => {}
            }
        }
        // A lagging copy never lowers the high-water mark.
        self.last_media_sequence = Some(
            self.last_media_sequence
                .map_or(playlist.media_sequence, |p| p.max(playlist.media_sequence)),
        );

        // `EXT-X-MEDIA-SEQUENCE` is an unbounded remote `u64`: every MSN
        // derived below is `media_sequence + index` with `index <
        // segments.len()`, so proving the end fits proves them all.
        let next_msn = playlist
            .media_sequence
            .checked_add(index_u64(playlist.segments.len()))
            .ok_or(Error::MediaSequenceOverflow {
                media_sequence: playlist.media_sequence,
                segments: playlist.segments.len(),
            })?;

        if !self.joined {
            self.joined = true;
            self.join_start(&playlist)?;
        }

        for (i, seg) in playlist.segments.iter().enumerate() {
            let msn = playlist.media_sequence.saturating_add(index_u64(i));
            if lag_floor.is_some_and(|floor| msn < floor) {
                self.skip_segment(msn, seg)?;
                continue;
            }
            self.process_closed_segment(msn, seg)?;
        }

        if let Some(open) = &playlist.open_segment {
            self.process_open_segment(next_msn, open)?;
        }

        // Prefer the *open* segment's map when present: it's the most
        // recent (`#EXT-X-MAP` carries forward, so the open segment's view
        // is never older than the last closed segment's) and, crucially, is
        // the only way to learn the init segment's URI at all when NO
        // segment has closed yet (issue #717 slice 5 fix — previously this
        // only ever looked at the last *closed* segment's map, so a client
        // tuning into a stream mid-segment couldn't fetch the init segment,
        // and therefore couldn't demux any of that segment's parts, until
        // it closed — needlessly inflating glass-to-glass latency by up to
        // a full segment duration on every fresh connection).
        let map = playlist
            .open_segment
            .as_ref()
            .and_then(|o| o.map.as_ref())
            .or_else(|| playlist.segments.last().and_then(|s| s.map.as_ref()));
        if let Some(map) = map {
            self.ensure_init_requested(map)?;
        }

        if let Some(ll) = &playlist.low_latency
            && let Some(hint_uri) = &ll.preload_hint_part
        {
            match ll.preload_hint_type {
                PreloadHintType::Part => {
                    let part_idx = playlist
                        .open_segment
                        .as_ref()
                        .map(|o| index_u64(o.parts.len()))
                        .unwrap_or(0);
                    let id = ResourceId::Part {
                        msn: next_msn,
                        part: part_idx,
                    };
                    let url = url::resolve(&self.playlist_url, hint_uri);
                    let byte_range = self.resolve_hint_byte_range(&url, ll)?;
                    self.request_resource(id, url, byte_range);
                }
                PreloadHintType::Map => {
                    let map = MapTag {
                        uri: hint_uri.clone(),
                        byte_range: ll.preload_hint_byte_range_length.map(|length| ByteRange {
                            length,
                            offset: ll.preload_hint_byte_range_start,
                        }),
                        extra_attrs: Vec::new(),
                    };
                    self.ensure_init_requested(&map)?;
                }
                _ => {
                    // RFC 8216bis §4.4.5.3 defines only PART/MAP today; a
                    // future hint type from a newer transmux is simply not
                    // prefetched rather than treated as an error
                    // (`PreloadHintType` is `#[non_exhaustive]`).
                }
            }
        }

        if playlist.endlist {
            self.saw_endlist = true;
        } else {
            // Issue #717 slice 1 fix: block only when the origin actually
            // advertises `CAN-BLOCK-RELOAD=YES` — `low_latency.is_some()`
            // alone is not enough (an origin sending `CAN-BLOCK-RELOAD=NO`
            // still carries parts/PART-INF, e.g. while ramping up support).
            let blocking = playlist
                .low_latency
                .as_ref()
                .filter(|ll| ll.can_block_reload)
                .map(|_| {
                    let part = playlist
                        .open_segment
                        .as_ref()
                        .map(|o| index_u64(o.parts.len()))
                        .unwrap_or(0);
                    BlockingReload {
                        msn: next_msn,
                        part: Some(part),
                    }
                });
            let can_skip = playlist
                .low_latency
                .as_ref()
                .and_then(|ll| ll.can_skip_until)
                .is_some();
            let skip = can_skip && self.last_full_playlist.is_some();
            self.pending_actions.push_back(Action::FetchPlaylist {
                url: self.playlist_url.clone(),
                blocking,
                skip,
            });
            if blocking.is_none() {
                // RFC 8216 §4.3.3.1: a client SHOULD NOT reload more
                // frequently than once per Target Duration; half that as a
                // reasonable non-blocking poll cadence.
                let wait_ms = (u64::from(playlist.target_duration.max(1)) * 1000) / 2;
                self.pending_actions.push_back(Action::WaitMs(wait_ms));
            }
        }

        self.prune(&playlist);
        if playlist.skip.is_none() {
            self.last_full_playlist = Some(playlist);
        }

        self.maybe_emit_end_of_stream()?;
        Ok(())
    }

    /// Feed the bytes fetched for a previously requested [`ResourceId`]
    /// (`init`/part/segment). Part/Segment resources delivered before the
    /// init segment are buffered internally and demuxed once the init
    /// arrives — the caller's fetches may complete in any order.
    ///
    /// # Errors
    /// [`Error::UnrequestedResource`] if `id` was never requested (the
    /// `requested` bookkeeping — or, for `Init`, `init_uri` — has no record
    /// of it): a caller/driver bug, or a stale/duplicate delivery after the
    /// client already moved past this id.
    /// [`Error::Demux`] if `transmux::Fmp4Demux` rejects the concatenation of
    /// the cached init + `bytes`.
    pub fn on_resource(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        let was_requested = match id {
            ResourceId::Init => self.init_uri.is_some(),
            ResourceId::Part { .. } | ResourceId::Segment { .. } => self.requested.contains(&id),
        };
        if !was_requested {
            return Err(Error::UnrequestedResource { id });
        }
        let duplicate = match id {
            ResourceId::Init => self.init_bytes.is_some(),
            ResourceId::Part { .. } | ResourceId::Segment { .. } => self.fulfilled.contains(&id),
        };
        if duplicate {
            return Err(Error::DuplicateResource { id });
        }
        self.outstanding_fetches = self.outstanding_fetches.saturating_sub(1);
        if id != ResourceId::Init {
            self.fulfilled.insert(id);
        }
        match id {
            ResourceId::Init => {
                self.init_bytes = Some(bytes.to_vec());
                if !self.init_emitted {
                    self.pending_outputs.push_back(Output::Init(bytes.to_vec()));
                    self.init_emitted = true;
                }
                let buffered: Vec<_> = self.pending_demux.drain(..).collect();
                for (bid, bbytes) in buffered {
                    if let Err(e) = self.finish_media_resource(bid, &bbytes) {
                        self.fulfilled.remove(&bid);
                        return Err(e);
                    }
                }
            }
            ResourceId::Part { .. } | ResourceId::Segment { .. } => {
                let processed = if self.is_ts_segment(bytes) {
                    // Classic MPEG-TS-segment HLS (issue #760): no init
                    // resource will ever arrive for this playlist, so demux
                    // this self-contained TS segment straight away rather
                    // than buffering it forever waiting for one.
                    self.finish_ts_resource(id, bytes)
                } else if self.init_bytes.is_none() {
                    self.pending_demux.push_back((id, bytes.to_vec()));
                    Ok(())
                } else {
                    self.finish_media_resource(id, bytes)
                };
                if let Err(e) = processed {
                    // Nothing was delivered: leave the id free for a retry.
                    self.fulfilled.remove(&id);
                    return Err(e);
                }
            }
        }
        self.maybe_emit_end_of_stream()?;
        Ok(())
    }

    /// Demux + emit + mark-delivered for a Part/Segment resource, once the
    /// init segment is known to be available.
    fn finish_media_resource(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        match id {
            ResourceId::Part { msn, part } => {
                self.emit_discontinuity_if_needed(msn);
                self.demux_and_emit(id, bytes)?;
                self.delivered_parts.insert((msn, part));
            }
            ResourceId::Segment { msn } => {
                self.emit_discontinuity_if_needed(msn);
                self.demux_and_emit(id, bytes)?;
                self.delivered_segments.insert(msn);
            }
            ResourceId::Init => {}
        }
        Ok(())
    }

    /// The classic-TS-HLS counterpart to [`Self::finish_media_resource`]:
    /// demux + emit + mark-delivered for a self-contained MPEG-TS Part/
    /// Segment resource — never buffered pending an init fetch, since
    /// [`Self::is_ts_segment`] only routes here once this playlist is known
    /// to advertise no `EXT-X-MAP` at all.
    fn finish_ts_resource(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        match id {
            ResourceId::Part { msn, part } => {
                self.begin_ts_resource(id, msn)?;
                self.demux_and_emit_ts(id, bytes)?;
                self.delivered_parts.insert((msn, part));
            }
            ResourceId::Segment { msn } => {
                self.begin_ts_resource(id, msn)?;
                self.demux_and_emit_ts(id, bytes)?;
                self.delivered_segments.insert(msn);
            }
            ResourceId::Init => {}
        }
        Ok(())
    }

    /// `true` when `bytes` should be routed to [`Self::finish_ts_resource`]
    /// (classic MPEG-TS-segment HLS, issue #760) rather than the fMP4/CMAF
    /// path: this playlist has never advertised an `EXT-X-MAP` (no init
    /// fetch is outstanding or cached — [`Self::init_uri`] is `None`; by the
    /// time any Part/Segment fetch response reaches [`Self::on_resource`],
    /// [`Self::on_playlist`] has already fully processed the playlist that
    /// requested it, including any map it carries, so this check is never
    /// stale) **and** `bytes` starts with the MPEG-TS sync byte — an
    /// fMP4/CMAF resource always starts with an ISOBMFF box
    /// (`ftyp`/`styp`/`moof`), never [`TS_SYNC_BYTE`].
    fn is_ts_segment(&self, bytes: &[u8]) -> bool {
        self.init_uri.is_none() && bytes.first() == Some(&TS_SYNC_BYTE)
    }

    /// Report that a previously requested [`ResourceId`] (or the playlist
    /// itself, via [`None`]) failed. Clears the id's "requested" bookkeeping
    /// so the next [`Self::on_playlist`] call naturally re-requests it (no
    /// automatic retry timer — the caller drives retry cadence).
    pub fn on_error(&mut self, id: Option<ResourceId>) {
        if let Some(id) = id {
            self.outstanding_fetches = self.outstanding_fetches.saturating_sub(1);
            match id {
                ResourceId::Init => self.init_uri = None,
                other => {
                    self.requested.remove(&other);
                }
            }
        }
        // `on_error` has no way to report: a failure here can only mean the
        // held-back final TS access units could not be flushed, and the
        // stream still ends.
        let _ = self.maybe_emit_end_of_stream();
    }

    // -- internals ------------------------------------------------------

    /// Reconstruct a full playlist view from an `EXT-X-SKIP` delta update
    /// (RFC 8216bis §4.4.5.2), by splicing the skipped prefix back in from
    /// the last full playlist this client observed. Best-effort: if there is
    /// no cached baseline, or it doesn't cover the skipped range, the delta
    /// is returned as-is (never an error — "at least don't break").
    fn merge_delta(&self, playlist: MediaPlaylist) -> MediaPlaylist {
        let Some(skip) = &playlist.skip else {
            return playlist;
        };
        if skip.skipped_segments == 0 {
            return playlist;
        }
        let Some(prev) = &self.last_full_playlist else {
            return playlist;
        };
        if playlist.media_sequence < prev.media_sequence {
            return playlist;
        }
        let Ok(prefix_start) = usize::try_from(playlist.media_sequence - prev.media_sequence)
        else {
            return playlist;
        };
        // `skip.skipped_segments` (`EXT-X-SKIP`'s `SKIPPED-SEGMENTS`,
        // RFC 8216bis §4.4.5.2) is untrusted `u64` straight from the remote
        // origin's playlist text, with no upper bound enforced by
        // `broadcast_hls::MediaPlaylist::parse`. `usize::try_from` +
        // `checked_add` guard both the u64->usize narrowing and the
        // addition itself, so an adversarial/corrupt value falls through to
        // the same "can't merge, return the delta as-is" fallback as every
        // other guard in this function rather than panicking (debug) or
        // wrapping to a bogus, silently-wrong slice bound (release).
        let prefix_end = usize::try_from(skip.skipped_segments)
            .ok()
            .and_then(|skipped| prefix_start.checked_add(skipped));
        let Some(prefix) = prefix_end.and_then(|end| prev.segments.get(prefix_start..end)) else {
            return playlist;
        };
        let mut merged = playlist;
        let mut segments = prefix.to_vec();
        segments.extend(merged.segments);
        merged.segments = segments;
        merged
    }

    fn process_closed_segment(&mut self, msn: u64, seg: &MediaSegment) -> Result<()> {
        if seg.discontinuous {
            self.discontinuous_msns.insert(msn);
        }
        if self.delivered_segments.contains(&msn) {
            return Ok(());
        }
        if seg.parts.is_empty() {
            // Either a genuinely non-LL segment (never had parts), OR an LL
            // segment whose parts were already fetched individually while it
            // was still open and whose *closed* rendering simply omits them
            // — RFC 8216bis does not require a closed segment to keep
            // listing `#EXT-X-PART` lines, and real origins commonly don't
            // (e.g. `multimux`'s: `MediaSegment.parts` is always empty for a
            // closed segment; only the still-open segment carries parts).
            // Detect the latter via `delivered_parts`: if any part for this
            // `msn` was ever delivered, every one of its non-`GAP` parts was
            // already requested while it was open (`process_open_segment`
            // requests every known part each time it's polled, so by the
            // time the segment closes none can have been missed) — fetching
            // the whole segment *as well* would demux and emit its samples a
            // second time. Caught by `hls-runtime/tests/glass_to_glass.rs`
            // (issue #717 slice 5): every sample was double-delivered for
            // the first two segments of a real, live-paced run.
            let already_have_parts = self
                .delivered_parts
                .range((msn, 0)..=(msn, u64::MAX))
                .next()
                .is_some();
            if already_have_parts {
                self.delivered_segments.insert(msn);
                return Ok(());
            }
            let id = ResourceId::Segment { msn };
            if !self.requested.contains(&id) {
                let url = url::resolve(&self.playlist_url, &seg.uri);
                let byte_range = self.resolve_byte_range(&url, &seg.byte_range)?;
                self.request_resource(id, url, byte_range);
            }
            return Ok(());
        }

        let mut fully_accounted = true;
        for (i, part) in seg.parts.iter().enumerate() {
            let i = index_u64(i);
            if part.gap || self.delivered_parts.contains(&(msn, i)) {
                continue;
            }
            fully_accounted = false;
            let id = ResourceId::Part { msn, part: i };
            if !self.requested.contains(&id) {
                let url = url::resolve(&self.playlist_url, &part.uri);
                let byte_range = self.resolve_byte_range(&url, &part.byte_range)?;
                self.request_resource(id, url, byte_range);
            }
        }
        if fully_accounted {
            self.delivered_segments.insert(msn);
        }
        Ok(())
    }

    fn process_open_segment(&mut self, msn: u64, open: &OpenSegment) -> Result<()> {
        for (i, part) in open.parts.iter().enumerate() {
            let i = index_u64(i);
            if part.gap || self.delivered_parts.contains(&(msn, i)) {
                continue;
            }
            let id = ResourceId::Part { msn, part: i };
            if !self.requested.contains(&id) {
                let url = url::resolve(&self.playlist_url, &part.uri);
                let byte_range = self.resolve_byte_range(&url, &part.byte_range)?;
                self.request_resource(id, url, byte_range);
            }
        }
        Ok(())
    }

    fn ensure_init_requested(&mut self, map: &MapTag) -> Result<()> {
        let url = url::resolve(&self.playlist_url, &map.uri);
        if self.init_uri.as_deref() == Some(url.as_str()) {
            return Ok(());
        }
        self.init_uri = Some(url.clone());
        self.init_bytes = None;
        self.init_emitted = false;
        let byte_range = self.resolve_byte_range(&url, &map.byte_range)?;
        self.pending_actions.push_back(Action::FetchResource {
            id: ResourceId::Init,
            url,
            byte_range,
        });
        self.outstanding_fetches += 1;
        Ok(())
    }

    fn request_resource(&mut self, id: ResourceId, url: String, byte_range: Option<(u64, u64)>) {
        self.requested.insert(id);
        self.outstanding_fetches += 1;
        self.pending_actions.push_back(Action::FetchResource {
            id,
            url,
            byte_range,
        });
    }

    /// Resolve a `PartSpec`/`MediaSegment`/`MapTag` `BYTERANGE` into an
    /// absolute `(offset, length)`, honouring the "omitted offset continues
    /// the previous sub-range of the same resource" rule (tracked per
    /// resolved URL).
    ///
    /// # Errors
    /// [`Error::ByteRangeOverflow`] if `offset + length` (both taken
    /// straight from the untrusted remote playlist) overflows `u64` — see
    /// that variant's doc for why this is rejected rather than saturated.
    fn resolve_byte_range(
        &mut self,
        url: &str,
        br: &Option<ByteRange>,
    ) -> Result<Option<(u64, u64)>> {
        let Some(br) = br.as_ref() else {
            return Ok(None);
        };
        let offset = br
            .offset
            .unwrap_or_else(|| *self.byte_range_cursor.get(url).unwrap_or(&0));
        let next_cursor =
            offset
                .checked_add(br.length)
                .ok_or_else(|| Error::ByteRangeOverflow {
                    url: url.to_string(),
                    offset,
                    length: br.length,
                })?;
        self.byte_range_cursor.insert(url.to_string(), next_cursor);
        Ok(Some((offset, br.length)))
    }

    /// Same overflow contract as [`Self::resolve_byte_range`] — see
    /// [`Error::ByteRangeOverflow`].
    fn resolve_hint_byte_range(
        &mut self,
        url: &str,
        ll: &broadcast_hls::LowLatencyConfig,
    ) -> Result<Option<(u64, u64)>> {
        let Some(length) = ll.preload_hint_byte_range_length else {
            return Ok(None);
        };
        let br = ByteRange {
            length,
            offset: ll.preload_hint_byte_range_start,
        };
        self.resolve_byte_range(url, &Some(br))
    }

    fn demux_and_emit(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        let init = self
            .init_bytes
            .as_ref()
            .ok_or(Error::InitNotYetAvailable { id })?;
        let mut combined = Vec::with_capacity(init.len() + bytes.len());
        combined.extend_from_slice(init);
        combined.extend_from_slice(bytes);
        let mut demux = Fmp4Demux::new();
        let media = demux
            .unpackage(combined.as_slice())
            .map_err(|source| Error::Demux { id, source })?;
        for track in media.tracks {
            if !track.samples.is_empty() {
                self.pending_outputs.push_back(Output::Samples {
                    track_id: track.spec.track_id,
                    samples: track.samples,
                });
            }
        }
        Ok(())
    }

    /// Emit the pending `EXT-X-DISCONTINUITY` for `msn` and, when one fires,
    /// start the TS demuxer afresh: the timestamps after a discontinuity are
    /// a new timeline (RFC 8216 §4.3.4.3), so the previous 33-bit unwrap
    /// anchor and track set must not carry across it. The old timeline's
    /// held-back final access units are flushed *before* the marker.
    fn begin_ts_resource(&mut self, id: ResourceId, msn: u64) -> Result<()> {
        if self.discontinuous_msns.contains(&msn) && !self.discontinuity_emitted.contains(&msn) {
            self.flush_ts_demux(id)?;
            self.emit_discontinuity_if_needed(msn);
            self.ts_demux = None;
            self.ts_specs.clear();
        }
        Ok(())
    }

    /// The classic-TS-HLS counterpart to [`Self::demux_and_emit`]: demux a
    /// self-contained MPEG-TS Part/Segment resource through the one
    /// [`StreamingTsDemux`] this client keeps across segments (no init bytes
    /// to concatenate — each `.ts` segment carries its own PAT/PMT/PES), so
    /// the 33-bit PTS/DTS wrap (ISO/IEC 13818-1 §2.4.3.7, every ~26.5 h) is
    /// unrolled across segment boundaries instead of restarting per segment.
    ///
    /// The demuxer is **not** finished per segment: it resolves each video
    /// sample's duration from the next access unit's timestamp, so a
    /// segment's final access unit per stream is held back and emitted once
    /// the next segment supplies its real duration (or, at
    /// `EXT-X-ENDLIST`/`EXT-X-DISCONTINUITY`, flushed with the stream's last
    /// known duration). Samples therefore stay in decode order with true
    /// durations, one access unit per stream later than a per-segment flush.
    ///
    /// The [`Output::Init`] the crate's output contract requires is
    /// synthesized from the demuxer's track set via
    /// [`transmux::build_init_segment`] — a real `ftyp`+fragmented-`moov`,
    /// byte-for-byte demuxable by `transmux::Fmp4Demux` like any other init
    /// segment, so callers built against the fMP4 path (e.g. `multimux`'s
    /// `HlsPull`) need no TS-specific handling. It is emitted before the
    /// first samples and again whenever the track set (a new PID, a codec
    /// change) makes the re-synthesized init differ from the last one.
    fn demux_and_emit_ts(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        let TsDemuxState(demux) = self
            .ts_demux
            .get_or_insert_with(|| TsDemuxState(StreamingTsDemux::new()));
        demux.feed(bytes);
        self.drain_ts_demux(id)
    }

    /// Flush the TS demuxer's held-back access units (no more input on this
    /// timeline) and emit them.
    fn flush_ts_demux(&mut self, id: ResourceId) -> Result<()> {
        let Some(TsDemuxState(demux)) = self.ts_demux.as_mut() else {
            return Ok(());
        };
        demux.finish();
        self.drain_ts_demux(id)
    }

    /// Turn the TS demuxer's pending events into [`Output`]s: the track set
    /// (re-synthesizing the init when it changed), then the samples per track.
    fn drain_ts_demux(&mut self, id: ResourceId) -> Result<()> {
        let Some(TsDemuxState(demux)) = self.ts_demux.as_mut() else {
            return Ok(());
        };
        let mut per_track: BTreeMap<u32, Vec<Sample>> = BTreeMap::new();
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    self.ts_specs.insert(spec.track_id, spec);
                }
                DemuxEvent::TrackRemoved { track_id, .. } => {
                    self.ts_specs.remove(&track_id);
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    per_track.entry(track_id).or_default().push(sample);
                }
                _ => {}
            }
        }
        if !self.ts_specs.is_empty() {
            let specs: Vec<TrackSpec> = self.ts_specs.values().cloned().collect();
            let init_bytes = transmux::build_init_segment(&specs, TS_MOVIE_TIMESCALE)
                .map_err(|source| Error::Demux { id, source })?;
            if self.last_ts_init.as_deref() != Some(init_bytes.as_slice()) {
                self.pending_outputs
                    .push_back(Output::Init(init_bytes.clone()));
                self.last_ts_init = Some(init_bytes);
                self.init_emitted = true;
            }
        }
        for (track_id, samples) in per_track {
            if !samples.is_empty() {
                self.pending_outputs
                    .push_back(Output::Samples { track_id, samples });
            }
        }
        Ok(())
    }

    /// Emit [`Output::Discontinuity`] once for a discontinuous `msn`;
    /// `true` when it was emitted by this call.
    fn emit_discontinuity_if_needed(&mut self, msn: u64) -> bool {
        if self.discontinuous_msns.contains(&msn) && !self.discontinuity_emitted.contains(&msn) {
            self.pending_outputs.push_back(Output::Discontinuity);
            self.discontinuity_emitted.insert(msn);
            return true;
        }
        false
    }

    /// Choose the first segment to play on the first playlist load and mark
    /// every closed segment before it as already delivered, so the join does
    /// not replay (or, for a long EVENT/DVR window, download) the history.
    ///
    /// - A live playlist (no `EXT-X-ENDLIST`) is not joined closer to its end
    ///   than the server's hold-back (RFC 8216 §6.3.3, RFC 8216bis §6.3.3):
    ///   `PART-HOLD-BACK` when the client plays in Low-Latency Mode
    ///   (`CAN-BLOCK-RELOAD=YES`), else the larger of `HOLD-BACK` and three
    ///   Target Durations (§4.4.3.8). The parts of the open segment count
    ///   towards the distance from the end.
    /// - `EXT-X-START` (§4.4.2.2) is honoured at segment granularity — a
    ///   positive `TIME-OFFSET` counts from the start, a negative one from the
    ///   end of the last segment; a live playlist never starts later than the
    ///   hold-back allows. `PRECISE` does not apply: segments are delivered
    ///   whole.
    /// - A VOD playlist without `EXT-X-START` is played from its start.
    ///
    /// A segment whose `EXTINF` is zero counts as one Target Duration, so a
    /// degenerate window still joins near its end. The byte-range cursor is
    /// advanced over every skipped segment, so a later omitted-offset
    /// `EXT-X-BYTERANGE` on the same resource continues from the right offset
    /// (RFC 8216 §4.3.2.2).
    ///
    /// # Errors
    /// [`Error::ByteRangeOverflow`] from the skipped segments' ranges.
    fn join_start(&mut self, playlist: &MediaPlaylist) -> Result<()> {
        let segments = &playlist.segments;
        let target = f64::from(playlist.target_duration.max(1));
        let duration_of = |seg: &MediaSegment| {
            let d = seg.duration.get();
            if d > 0.0 { d } else { target }
        };

        let from_start_tag = playlist.start.as_ref().map(|start| {
            let total: f64 = segments.iter().map(duration_of).sum();
            let offset = start.time_offset.get();
            let position = if offset >= 0.0 {
                offset
            } else {
                total + offset
            };
            let position = position.clamp(0.0, total);
            let mut elapsed = 0.0_f64;
            for (i, seg) in segments.iter().enumerate() {
                elapsed += duration_of(seg);
                if elapsed > position {
                    return i;
                }
            }
            // At (or past) the end: the last segment.
            segments.len().saturating_sub(1)
        });

        let from_hold_back = (!playlist.endlist).then(|| {
            let ll = playlist.low_latency.as_ref();
            let wanted = match ll.filter(|ll| ll.can_block_reload) {
                Some(ll) if ll.part_hold_back.is_some() => {
                    ll.part_hold_back.map_or(0.0, |p| p.get())
                }
                _ => {
                    let hold_back = ll.and_then(|ll| ll.hold_back).map_or(0.0, |h| h.get());
                    (LIVE_EDGE_TARGET_DURATIONS * target).max(hold_back)
                }
            };
            let mut behind: f64 = playlist
                .open_segment
                .as_ref()
                .map_or(0.0, |o| o.parts.iter().map(|p| p.duration.get()).sum());
            if behind >= wanted {
                return segments.len();
            }
            for (i, seg) in segments.iter().enumerate().rev() {
                behind += duration_of(seg);
                if behind >= wanted {
                    return i;
                }
            }
            0
        });

        let start = match (from_start_tag, from_hold_back) {
            (Some(s), Some(h)) => s.min(h),
            (Some(s), None) => s,
            (None, Some(h)) => h,
            (None, None) => 0,
        };
        for (i, seg) in segments.iter().enumerate().take(start) {
            let msn = playlist.media_sequence.saturating_add(index_u64(i));
            self.skip_segment(msn, seg)?;
        }
        Ok(())
    }

    /// Mark a segment skipped at join time as delivered, advancing the
    /// byte-range cursor exactly as fetching it would have (a segment with
    /// parts is addressed by its parts' ranges, a plain one by its own).
    fn skip_segment(&mut self, msn: u64, seg: &MediaSegment) -> Result<()> {
        if seg.parts.is_empty() {
            let url = url::resolve(&self.playlist_url, &seg.uri);
            self.resolve_byte_range(&url, &seg.byte_range)?;
        } else {
            for part in seg.parts.iter().filter(|p| !p.gap) {
                let url = url::resolve(&self.playlist_url, &part.uri);
                self.resolve_byte_range(&url, &part.byte_range)?;
            }
        }
        self.delivered_segments.insert(msn);
        Ok(())
    }

    /// Drop every MSN-keyed record below the playlist's first segment (it
    /// can never be referenced again, and a long-lived pull would otherwise
    /// grow these sets without bound), and — for a full playlist — every
    /// byte-range cursor whose URL the playlist no longer references.
    /// Resources still in flight (requested, not yet fulfilled) are kept so
    /// their delivery is still accepted.
    fn prune(&mut self, playlist: &MediaPlaylist) {
        let first = playlist.media_sequence;
        let below = |id: &ResourceId| resource_msn(id).is_some_and(|m| m < first);
        let fulfilled = &self.fulfilled;
        self.requested
            .retain(|id| !(below(id) && fulfilled.contains(id)));
        self.fulfilled.retain(|id| !below(id));
        self.delivered_parts.retain(|&(m, _)| m >= first);
        self.delivered_segments.retain(|&m| m >= first);
        self.discontinuous_msns.retain(|&m| m >= first);
        self.discontinuity_emitted.retain(|&m| m >= first);

        if playlist.skip.is_none() {
            let base = &self.playlist_url;
            let mut live: BTreeSet<String> = BTreeSet::new();
            for seg in &playlist.segments {
                live.insert(url::resolve(base, &seg.uri));
                for part in &seg.parts {
                    live.insert(url::resolve(base, &part.uri));
                }
                if let Some(map) = &seg.map {
                    live.insert(url::resolve(base, &map.uri));
                }
            }
            if let Some(open) = &playlist.open_segment {
                for part in &open.parts {
                    live.insert(url::resolve(base, &part.uri));
                }
                if let Some(map) = &open.map {
                    live.insert(url::resolve(base, &map.uri));
                }
            }
            if let Some(hint) = playlist
                .low_latency
                .as_ref()
                .and_then(|ll| ll.preload_hint_part.as_ref())
            {
                live.insert(url::resolve(base, hint));
            }
            self.byte_range_cursor.retain(|u, _| live.contains(u));
        }
    }

    /// Decide whether `new` continues the stream the previous full playlist
    /// described, is merely an *older* copy of it (a CDN edge a poll — or a
    /// whole window — behind another), or is a different stream (an origin
    /// restart; audit r09-C3, issue #1031).
    ///
    /// Signals, strongest first:
    ///
    /// 1. **Segment identity at a shared number** — a media sequence number
    ///    listed by both windows must name the same segment (URI, duration,
    ///    byte range) and, when both carry `EXT-X-PROGRAM-DATE-TIME` for it,
    ///    the same wall-clock time. A mismatch is a restart *whatever the
    ///    direction of the numbers* — an origin that restarts to an equal or
    ///    higher number is caught here.
    /// 2. **`EXT-X-DISCONTINUITY-SEQUENCE`** may never decrease while the
    ///    media sequence number does not (RFC 8216 §6.2.2): a decrease at an
    ///    equal or higher number is a restart.
    /// 3. When the media sequence number *went backwards* with **no** shared
    ///    number (the new window is wholly older, or a restart):
    ///    - a URI that the previous window listed under a *different* number
    ///      is a restart (names reused under new numbering);
    ///    - else, with `PROGRAM-DATE-TIME` on both sides, a new window that
    ///      ends before the previous one began is a wholly older copy
    ///      ([`Continuity::OlderWindow`], skipped), anything else a restart;
    ///    - else nothing distinguishes them and it is treated as a restart.
    ///
    /// **Residual, undetectable cases** (documented, not guessed at): a CDN
    /// edge lagging by a whole window behind an origin that carries *no*
    /// `PROGRAM-DATE-TIME` and whose segment names are all new reads as a
    /// restart (one spurious `Discontinuity` and a re-join); and an origin
    /// restarting to a *higher*, non-overlapping number with consistent
    /// discontinuity sequence and no `PROGRAM-DATE-TIME` reads as the client
    /// having fallen behind.
    fn continuity(&self, new: &MediaPlaylist) -> Continuity {
        let Some(prev) = &self.last_full_playlist else {
            return if self
                .last_media_sequence
                .is_some_and(|p| new.media_sequence < p)
            {
                Continuity::Restart
            } else {
                Continuity::Continues
            };
        };
        let (p0, n0) = (prev.media_sequence, new.media_sequence);
        let regressed = n0 < self.last_media_sequence.unwrap_or(p0).max(p0);

        // 1. Identity at every shared number.
        let mut overlap = false;
        for (i, seg) in new.segments.iter().enumerate() {
            let Some(msn) = n0.checked_add(index_u64(i)) else {
                return Continuity::Restart;
            };
            let Some(old) = msn
                .checked_sub(p0)
                .and_then(|d| usize::try_from(d).ok())
                .and_then(|d| prev.segments.get(d))
            else {
                continue;
            };
            if old.uri != seg.uri
                || old.duration != seg.duration
                || old.byte_range != seg.byte_range
            {
                return Continuity::Restart;
            }
            if let (Some(a), Some(b)) = (program_date_time(old), program_date_time(seg))
                && a != b
            {
                return Continuity::Restart;
            }
            overlap = true;
        }

        // 2. The discontinuity sequence never decreases while the number does
        //    not.
        if !regressed && new.discontinuity_sequence < prev.discontinuity_sequence {
            return Continuity::Restart;
        }
        if !regressed {
            return Continuity::Continues;
        }

        // 3. Regressed.
        if overlap {
            return Continuity::Lagging;
        }
        let old_uris: BTreeSet<&str> = prev.segments.iter().map(|s| s.uri.as_str()).collect();
        if new
            .segments
            .iter()
            .any(|s| old_uris.contains(s.uri.as_str()))
        {
            // A URI the previous window lists under another number.
            return Continuity::Restart;
        }
        let new_last = new.segments.iter().rev().find_map(program_date_time_ms);
        let prev_first = prev.segments.iter().find_map(program_date_time_ms);
        match (new_last, prev_first) {
            (Some(end), Some(start)) if end < start => Continuity::OlderWindow,
            _ => Continuity::Restart,
        }
    }

    /// The Media Sequence Number went backwards (RFC 8216 §6.2.2 forbids it;
    /// restarting origins do it anyway — ffmpeg without `-start_number`,
    /// nginx-rtmp, a replaced origin; audit r09-C3, issue #1031). Every
    /// number the new playlist uses may name different media from the same
    /// number already delivered, so all MSN-keyed state is dropped (a new
    /// segment must not be skipped as "already delivered"), not-yet-polled
    /// fetches of the old numbering are withdrawn, the live-edge join is
    /// redone, the init segment is re-fetched (the restart may carry a new
    /// codec configuration), and [`Output::Discontinuity`] is emitted. A
    /// response still in flight for an old id is rejected as
    /// [`Error::UnrequestedResource`].
    fn restart_after_regression(&mut self) -> Result<()> {
        // Held-back TS access units belong to the old timeline: emit them
        // before the discontinuity, then start a fresh demuxer.
        self.flush_ts_demux(ResourceId::Init)?;
        self.ts_demux = None;
        self.ts_specs.clear();
        self.last_ts_init = None;
        self.pending_actions
            .retain(|a| !matches!(a, Action::FetchResource { .. }));
        self.pending_demux.clear();
        self.requested.clear();
        self.fulfilled.clear();
        self.delivered_parts.clear();
        self.delivered_segments.clear();
        self.discontinuous_msns.clear();
        self.discontinuity_emitted.clear();
        self.byte_range_cursor.clear();
        self.outstanding_fetches = 0;
        self.last_full_playlist = None;
        self.init_uri = None;
        self.init_bytes = None;
        self.init_emitted = false;
        self.joined = false;
        self.pending_outputs.push_back(Output::Discontinuity);
        Ok(())
    }

    fn maybe_emit_end_of_stream(&mut self) -> Result<()> {
        if self.saw_endlist && !self.end_emitted && self.outstanding_fetches == 0 {
            // The TS demuxer holds back each stream's last access unit until
            // the next one shows its real duration; nothing follows now.
            self.flush_ts_demux(ResourceId::Init)?;
            self.pending_outputs.push_back(Output::EndOfStream);
            self.end_emitted = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: `on_resource` documents (see `Error::UnrequestedResource`)
    // that it rejects a `ResourceId` the client never requested, but
    // previously never actually checked — any bytes for any id (a
    // caller/driver bug, or a stale/duplicate delivery) were silently
    // accepted. Must FAIL if that check is ever removed.
    #[test]
    fn on_resource_rejects_a_never_requested_id() {
        let mut client = HlsClient::new("http://example.com/playlist.m3u8");
        let id = ResourceId::Segment { msn: 0 };

        let err = client
            .on_resource(id, b"some bytes")
            .expect_err("an id the client never requested must be rejected");
        assert!(
            matches!(err, Error::UnrequestedResource { id: got } if got == id),
            "wrong error variant: {err:?}"
        );

        // Init is checked too (tracked via `init_uri` rather than
        // `requested`, since it's never inserted into that set).
        let err = client
            .on_resource(ResourceId::Init, b"init bytes")
            .expect_err("an unrequested Init must be rejected");
        assert!(
            matches!(
                err,
                Error::UnrequestedResource {
                    id: ResourceId::Init
                }
            ),
            "wrong error variant: {err:?}"
        );
    }

    // The flip side of the regression above: a `ResourceId` the client
    // actually asked for (via its own internal `request_resource`
    // bookkeeping, mirroring what a real `poll()`-driven fetch populates)
    // must still be accepted, not spuriously rejected.
    #[test]
    fn on_resource_accepts_a_previously_requested_id() {
        let mut client = HlsClient::new("http://example.com/playlist.m3u8");
        let id = ResourceId::Segment { msn: 0 };
        client.request_resource(id, "http://example.com/seg0.m4s".to_string(), None);

        // No init segment cached yet, so this is buffered rather than
        // demuxed — the point here is only that it is *not* rejected as
        // unrequested.
        let result = client.on_resource(id, b"some bytes");
        assert!(
            result.is_ok(),
            "a requested id must be accepted: {result:?}"
        );
        assert!(
            client.pending_demux.iter().any(|(bid, _)| *bid == id),
            "expected the resource to be buffered pending the init segment"
        );
    }

    // Issue #760: classic MPEG-TS-segment HLS routing. `is_ts_segment` must
    // say yes to a genuine TS resource (sync byte, no map ever seen)...
    #[test]
    fn is_ts_segment_true_when_no_map_seen_and_sync_byte_present() {
        let client = HlsClient::new("http://example.com/playlist.m3u8");
        assert!(client.is_ts_segment(&[TS_SYNC_BYTE, 0x40, 0x11, 0x00]));
    }

    // ...but say no to an ISOBMFF (fMP4/CMAF) resource even when no map has
    // been seen yet — the content itself is never TS, so it must fall
    // through to the ordinary init-buffering path rather than being
    // misrouted into `TsDemux` (which would reject it as malformed TS).
    #[test]
    fn is_ts_segment_false_for_an_isobmff_resource_with_no_map_seen() {
        let client = HlsClient::new("http://example.com/playlist.m3u8");
        let ftyp_box = b"\x00\x00\x00\x18ftypiso5\x00\x00\x02\x00iso5iso6mp41";
        assert!(!client.is_ts_segment(ftyp_box));
    }

    // The playlist signal takes precedence over content-sniffing: once this
    // playlist is known to advertise an `EXT-X-MAP` (an init fetch has been
    // requested/cached), even a resource whose first byte happens to be
    // `0x47` must NOT be misrouted through `TsDemux` -- it is that
    // playlist's own fMP4/CMAF init + part/segment concatenation the
    // fetched bytes belong with.
    #[test]
    fn is_ts_segment_false_once_a_map_has_been_requested() {
        let mut client = HlsClient::new("http://example.com/playlist.m3u8");
        client
            .ensure_init_requested(&MapTag {
                uri: "init.mp4".to_string(),
                byte_range: None,
                extra_attrs: Vec::new(),
            })
            .expect("ensure_init_requested succeeds");
        assert!(!client.is_ts_segment(&[TS_SYNC_BYTE, 0x40, 0x11, 0x00]));
    }

    // Biting test for the u64-overflow defect: a remote origin's
    // `#EXT-X-BYTERANGE` (or preload-hint byte range) is untrusted text —
    // `broadcast_hls::MediaPlaylist::parse` places no upper bound on either
    // the offset or the length (confirmed: bare `str::parse::<u64>()`). A
    // playlist advertising `BYTERANGE:18446744073709551615@1` must be
    // rejected with `Error::ByteRangeOverflow`, not panic (debug) or
    // silently wrap `offset + length` into a bogus cursor (release).
    //
    // MUTATION VERIFIED: reverting `resolve_byte_range`'s
    // `offset.checked_add(br.length).ok_or_else(...)` back to the original
    // `offset + br.length` makes this test fail — the debug build panics
    // with "attempt to add with overflow" before `expect_err` ever runs
    // (confirmed by running it), rather than the release build's silent
    // wraparound the issue actually reports. Recompiled and re-ran to
    // observe that exact panic, then restored the checked_add.
    #[test]
    fn resolve_byte_range_rejects_an_offset_plus_length_that_overflows_u64() {
        let mut client = HlsClient::new("http://example.com/playlist.m3u8");
        let br = Some(ByteRange {
            length: u64::MAX,
            offset: Some(1),
        });
        let err = client
            .resolve_byte_range("http://example.com/seg0.m4s", &br)
            .expect_err("offset 1 + length u64::MAX must overflow and be rejected");
        assert!(
            matches!(
                err,
                Error::ByteRangeOverflow {
                    offset: 1,
                    length: u64::MAX,
                    ..
                }
            ),
            "wrong error variant/fields: {err:?}"
        );
    }

    // Same defect, the other trigger named in the issue: repeated
    // omitted-offset ranges on the *same* resource URL accumulate via
    // `byte_range_cursor` (RFC 8216bis §4.4.4.9's "omitted offset continues
    // the previous sub-range" rule) — a long-lived pull can walk that
    // cursor arbitrarily close to `u64::MAX` before a single request's
    // `offset + length` itself overflows.
    #[test]
    fn resolve_byte_range_rejects_a_cursor_accumulation_that_overflows_u64() {
        let mut client = HlsClient::new("http://example.com/playlist.m3u8");
        let url = "http://example.com/seg0.m4s";
        // Seed the cursor near the top of the u64 range with an explicit
        // offset, then the next omitted-offset range pushes it over.
        let seed = Some(ByteRange {
            length: u64::MAX - 10,
            offset: Some(5),
        });
        let (offset, length) = client
            .resolve_byte_range(url, &seed)
            .expect("seed range does not itself overflow")
            .expect("Some for a Some(ByteRange)");
        assert_eq!((offset, length), (5, u64::MAX - 10));

        let next = Some(ByteRange {
            length: 100,
            offset: None, // continues the cursor left at 5 + (u64::MAX - 10)
        });
        let err = client
            .resolve_byte_range(url, &next)
            .expect_err("cursor + next length must overflow and be rejected");
        assert!(
            matches!(err, Error::ByteRangeOverflow { length: 100, .. }),
            "wrong error variant/fields: {err:?}"
        );
    }

    // Biting test for the `merge_delta` defect: an `#EXT-X-SKIP` delta's
    // `SKIPPED-SEGMENTS` is untrusted `u64` text with no upper bound
    // enforced by `broadcast_hls::MediaPlaylist::parse`. A malicious/corrupt
    // origin claiming a `SKIPPED-SEGMENTS` value that overflows
    // `prefix_start + skipped_segments` (here: `prefix_start == 10` from a
    // 10-segment `media_sequence` advance, plus `skipped_segments ==
    // u64::MAX - 5`) must fall through to the existing "can't merge, return
    // the delta as-is" fallback, not panic. `prefix_start` is deliberately
    // nonzero: `0 + u64::MAX` does not overflow, so a `prefix_start == 0`
    // case would pass even with the guard removed, for the wrong reason
    // (the subsequent `.get()` bounds check catching it) rather than the
    // one this test targets (the addition itself).
    //
    // MUTATION VERIFIED: reverting the guard back to the original
    // `prefix_start + skip.skipped_segments as usize` makes this test fail:
    // the debug build panics with "attempt to add with overflow" inside
    // `merge_delta` before the `let merged = ...` assertions ever run
    // (confirmed by running it), rather than returning the delta unmerged.
    // Recompiled and re-ran to observe that exact panic, then restored the
    // checked/try_from guard.
    #[test]
    fn merge_delta_does_not_panic_on_an_overflowing_skipped_segments() {
        let mut client = HlsClient::new("http://example.com/playlist.m3u8");
        let prev = MediaPlaylist {
            media_sequence: 0,
            segments: vec![MediaSegment::default(); 2],
            ..Default::default()
        };
        // `last_full_playlist` is set directly (private field, same module)
        // rather than driven through a full `on_playlist` round-trip — the
        // point under test is purely `merge_delta`'s own arithmetic guard.
        client.last_full_playlist = Some(prev);

        let delta = MediaPlaylist {
            media_sequence: 10, // prefix_start == 10 - 0 == 10
            skip: Some(broadcast_hls::SkipInfo {
                skipped_segments: u64::MAX - 5,
                ..Default::default()
            }),
            segments: Vec::new(),
            ..Default::default()
        };

        let merged = client.merge_delta(delta.clone());
        assert_eq!(
            merged, delta,
            "an unmergeable skip count must fall back to the delta as-is, not panic"
        );
    }

    fn byte_range_playlist(first_msn: u64, file: &str) -> String {
        // Two sub-ranges of one resource; the second omits its offset
        // (RFC 8216 §4.4.4.9 / §4.3.2.2), so the per-URL cursor matters.
        format!(
            "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:2\n\
             #EXT-X-MEDIA-SEQUENCE:{first_msn}\n\
             #EXTINF:2.0,\n#EXT-X-BYTERANGE:100@0\n{file}\n\
             #EXTINF:2.0,\n#EXT-X-BYTERANGE:50\n{file}\n"
        )
    }

    // r09-W12: every set keyed by MSN (and the per-URL byte-range cursor)
    // must shrink back to what the current window references, not grow for
    // the life of the pull. Expected contents are literals.
    #[test]
    fn state_is_pruned_to_the_current_window() {
        let mut client = HlsClient::new("http://example.com/p.m3u8");
        client
            .on_playlist(byte_range_playlist(0, "a.ts").as_bytes())
            .unwrap();
        assert_eq!(
            client.byte_range_cursor.iter().collect::<Vec<_>>(),
            vec![(&"http://example.com/a.ts".to_string(), &150u64)]
        );
        // Deliver msn 0 (buffered: no init), leave msn 1 in flight.
        client
            .on_resource(ResourceId::Segment { msn: 0 }, b"opaque")
            .unwrap();

        client
            .on_playlist(byte_range_playlist(1000, "b.ts").as_bytes())
            .unwrap();

        let requested: Vec<ResourceId> = client.requested.iter().copied().collect();
        assert_eq!(
            requested,
            vec![
                // in flight when the window moved: kept
                ResourceId::Segment { msn: 1 },
                ResourceId::Segment { msn: 1000 },
                ResourceId::Segment { msn: 1001 },
            ]
        );
        assert!(client.fulfilled.is_empty(), "{:?}", client.fulfilled);
        assert_eq!(
            client.byte_range_cursor.iter().collect::<Vec<_>>(),
            vec![(&"http://example.com/b.ts".to_string(), &150u64)],
            "the cursor of the unreferenced a.ts must be dropped"
        );
    }

    // r09-W12/W13 on a discontinuity-bearing window: MSN-keyed discontinuity
    // bookkeeping below the window is dropped too.
    #[test]
    fn discontinuity_bookkeeping_below_the_window_is_dropped() {
        let mut client = HlsClient::new("http://example.com/p.m3u8");
        let pl = |first: u64| {
            format!(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                 #EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-DISCONTINUITY\n#EXTINF:2.0,\ns.ts\n"
            )
        };
        client.on_playlist(pl(5).as_bytes()).unwrap();
        client
            .on_resource(ResourceId::Segment { msn: 5 }, b"opaque")
            .unwrap();
        assert_eq!(
            client
                .discontinuous_msns
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![5]
        );
        client.on_playlist(pl(9).as_bytes()).unwrap();
        assert_eq!(
            client
                .discontinuous_msns
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![9]
        );
        assert!(client.discontinuity_emitted.is_empty());
        assert!(client.delivered_segments.is_empty());
    }

    #[test]
    fn next_wait_reports_the_queued_wait_hint_without_consuming_it() {
        use core::time::Duration;
        let mut c = HlsClient::new("http://h/p.m3u8");
        assert_eq!(c.next_wait(), None, "the first queued action is a fetch");
        let _ = c.poll();
        c.on_playlist(
            b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n",
        )
        .unwrap();
        while c.next_wait().is_none() {
            c.poll()
                .expect("a WaitMs must be queued after a non-blocking live playlist");
        }
        assert_eq!(
            c.next_wait(),
            Some(Duration::from_millis(1000)),
            "half the 2 s target duration"
        );
        assert_eq!(
            c.next_wait(),
            Some(Duration::from_millis(1000)),
            "peeking does not consume"
        );
        // The deadline is ABSOLUTE: re-queries at later `now`s (unrelated
        // wake-ups) return the same instant instead of re-arming the wait.
        let t0 = std::time::Instant::now();
        let first = c.poll_timeout(t0).expect("a wait is at the front");
        assert_eq!(first, t0 + Duration::from_millis(1000));
        for later in [250u64, 600, 999, 5000] {
            assert_eq!(
                c.poll_timeout(t0 + Duration::from_millis(later)),
                Some(first),
                "re-query at +{later} ms must not slide the deadline"
            );
        }
        // Draining the wait supersedes it: the next wait is anchored afresh.
        assert!(matches!(c.poll(), Some(Action::WaitMs(1000))));
        assert_eq!(c.poll_timeout(t0 + Duration::from_secs(9)), None);
    }

    #[test]
    fn rfc3339_accepts_the_spellings_the_old_parser_accepted() {
        assert_eq!(
            parse_rfc3339_ms("2026-10-02 10:00:00Z"),
            Some(1_790_935_200_000),
            "space separator"
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-02t10:00:00z"),
            Some(1_790_935_200_000),
            "lower-case t and z"
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-02T10:00:00.123456789Z"),
            Some(1_790_935_200_123),
            "extra digits truncated"
        );
        assert_eq!(
            parse_rfc3339_ms("1969-12-31T23:59:59Z"),
            Some(-1_000),
            "before the epoch"
        );
    }

    #[test]
    fn rfc3339_offset_spellings_all_name_the_same_instant() {
        let want = Some(1_790_935_200_000);
        for text in [
            "2026-10-02T12:00:00+02:00",
            "2026-10-02T12:00:00+0200",
            "2026-10-02T05:00:00-05:00",
            "2026-10-02T05:00:00-0500",
            "2026-10-02T10:00:00+00:00",
            "2026-10-02T10:00:00-00:00",
        ] {
            assert_eq!(parse_rfc3339_ms(text), want, "{text}");
        }
        // An hour-only offset: whatever `jiff` decides, it must never be a guess.
        let hour_only = parse_rfc3339_ms("2026-10-02T12:00:00+02");
        assert!(hour_only == want || hour_only.is_none(), "{hour_only:?}");
    }

    /// A leap second is clamped to :59 (jiff), where the old parser added 60 s to the minute.
    #[test]
    fn a_leap_second_is_clamped_to_the_last_second_of_the_minute() {
        assert_eq!(
            parse_rfc3339_ms("2016-12-31T23:59:60Z"),
            Some(1_483_228_799_000)
        );
    }

    #[test]
    fn rfc3339_parses_to_literal_epoch_milliseconds() {
        // 2026-10-02T10:00:00Z = 1_790_935_200 s (independently: `date -u -d`).
        assert_eq!(
            parse_rfc3339_ms("2026-10-02T10:00:00Z"),
            Some(1_790_935_200_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-02T10:00:00.250Z"),
            Some(1_790_935_200_250)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-02T10:00:00.5Z"),
            Some(1_790_935_200_500)
        );
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2000-02-29T23:59:59Z"),
            Some(951_868_799_000)
        );
        // The same instant with an offset, both spellings.
        assert_eq!(
            parse_rfc3339_ms("2026-10-02T12:00:00+02:00"),
            Some(1_790_935_200_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-02T05:00:00-0500"),
            Some(1_790_935_200_000)
        );
        // Anything not exactly RFC 3339 is `None`, never a guess.
        for bad in [
            "",
            "2026-10-02",
            "2026-13-02T10:00:00Z",
            "2026-10-02T25:00:00Z",
            "2026-10-02T10:00:00",
            "2026-10-02T10:00:00Zjunk",
            "2026-10-02T10:00:00.Z",
            "2026-10-02T10:00:00+2:00",
            "xxxx-10-02T10:00:00Z",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), None, "{bad:?}");
        }
    }
}
