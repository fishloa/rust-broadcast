//! [`LlHlsClient`] — the sans-IO caller-driven engine.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use broadcast_common::Unpackage;
use transmux::hls::{ByteRange, MapTag, PreloadHintType};
use transmux::{Fmp4Demux, MediaPlaylist, MediaSegment, OpenSegment, TrackSpec, TsDemux};

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

/// A driveable, sans-IO Low-Latency HLS (RFC 8216bis) playback client.
///
/// `LlHlsClient` never touches a socket or a clock. The caller drives it:
///
/// 1. [`LlHlsClient::new`] seeds the first [`Action::FetchPlaylist`]; drain it
///    with [`LlHlsClient::poll`] and perform the GET.
/// 2. Feed the response back with [`LlHlsClient::on_playlist`] (playlist) or
///    [`LlHlsClient::on_resource`] (init/part/segment bytes) —
///    [`Action::FetchResource`]'s `id` correlates the two.
/// 3. Drain [`LlHlsClient::poll`] again for the next round of actions (a new
///    reload, newly discoverable parts, a preload-hint prefetch, ...) and
///    [`LlHlsClient::next_output`] for newly available [`Output`]s.
///
/// # Behaviour
///
/// - **Reload scheduling** (issue #717 slice 2): once a playlist advertises
///   `EXT-X-SERVER-CONTROL`/`EXT-X-PART-INF` **and** the origin's
///   `CAN-BLOCK-RELOAD` attribute is `YES`
///   ([`transmux::hls::LowLatencyConfig::can_block_reload`] is `true` —
///   *not* merely [`transmux::hls::MediaPlaylist::low_latency`] being
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
///   is a gap in the current wire model ([`transmux::hls::OpenSegment`]), not
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
pub struct LlHlsClient {
    playlist_url: String,

    pending_actions: VecDeque<Action>,
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
    delivered_parts: BTreeSet<(u64, u64)>,
    delivered_segments: BTreeSet<u64>,
    discontinuous_msns: BTreeSet<u64>,
    discontinuity_emitted: BTreeSet<u64>,
    byte_range_cursor: BTreeMap<String, u64>,

    outstanding_fetches: u64,
    saw_endlist: bool,
    end_emitted: bool,
    last_full_playlist: Option<MediaPlaylist>,
}

impl LlHlsClient {
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
            pending_outputs: VecDeque::new(),
            init_uri: None,
            init_bytes: None,
            init_emitted: false,
            pending_demux: VecDeque::new(),
            requested: BTreeSet::new(),
            delivered_parts: BTreeSet::new(),
            delivered_segments: BTreeSet::new(),
            discontinuous_msns: BTreeSet::new(),
            discontinuity_emitted: BTreeSet::new(),
            byte_range_cursor: BTreeMap::new(),
            outstanding_fetches: 0,
            saw_endlist: false,
            end_emitted: false,
            last_full_playlist: None,
        }
    }

    /// The Media Playlist URL this client is following.
    pub fn playlist_url(&self) -> &str {
        &self.playlist_url
    }

    /// Drain the next IO [`Action`] the caller must perform, if any.
    pub fn poll(&mut self) -> Option<Action> {
        self.pending_actions.pop_front()
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

        for (i, seg) in playlist.segments.iter().enumerate() {
            let msn = playlist.media_sequence + i as u64;
            self.process_closed_segment(msn, seg)?;
        }

        let next_msn = playlist.media_sequence + playlist.segments.len() as u64;
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

        if let Some(ll) = &playlist.low_latency {
            if let Some(hint_uri) = &ll.preload_hint_part {
                match ll.preload_hint_type {
                    PreloadHintType::Part => {
                        let part_idx = playlist
                            .open_segment
                            .as_ref()
                            .map(|o| o.parts.len() as u64)
                            .unwrap_or(0);
                        let id = ResourceId::Part {
                            msn: next_msn,
                            part: part_idx,
                        };
                        let url = url::resolve(&self.playlist_url, hint_uri);
                        let byte_range = self.resolve_hint_byte_range(&url, ll);
                        self.request_resource(id, url, byte_range);
                    }
                    PreloadHintType::Map => {
                        let map = MapTag {
                            uri: hint_uri.clone(),
                            byte_range: ll.preload_hint_byte_range_length.map(|length| ByteRange {
                                length,
                                offset: ll.preload_hint_byte_range_start,
                            }),
                        };
                        self.ensure_init_requested(&map)?;
                    }
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
                        .map(|o| o.parts.len() as u64)
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

        if playlist.skip.is_none() {
            self.last_full_playlist = Some(playlist);
        }

        self.maybe_emit_end_of_stream();
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
        self.outstanding_fetches = self.outstanding_fetches.saturating_sub(1);
        match id {
            ResourceId::Init => {
                self.init_bytes = Some(bytes.to_vec());
                if !self.init_emitted {
                    self.pending_outputs.push_back(Output::Init(bytes.to_vec()));
                    self.init_emitted = true;
                }
                let buffered: Vec<_> = self.pending_demux.drain(..).collect();
                for (bid, bbytes) in buffered {
                    self.finish_media_resource(bid, &bbytes)?;
                }
            }
            ResourceId::Part { .. } | ResourceId::Segment { .. } => {
                if self.is_ts_segment(bytes) {
                    // Classic MPEG-TS-segment HLS (issue #760): no init
                    // resource will ever arrive for this playlist, so demux
                    // this self-contained TS segment straight away rather
                    // than buffering it forever waiting for one.
                    self.finish_ts_resource(id, bytes)?;
                } else if self.init_bytes.is_none() {
                    self.pending_demux.push_back((id, bytes.to_vec()));
                } else {
                    self.finish_media_resource(id, bytes)?;
                }
            }
        }
        self.maybe_emit_end_of_stream();
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
                self.emit_discontinuity_if_needed(msn);
                self.demux_and_emit_ts(id, bytes)?;
                self.delivered_parts.insert((msn, part));
            }
            ResourceId::Segment { msn } => {
                self.emit_discontinuity_if_needed(msn);
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
        self.maybe_emit_end_of_stream();
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
        let prefix_start = (playlist.media_sequence - prev.media_sequence) as usize;
        let prefix_end = prefix_start + skip.skipped_segments as usize;
        let Some(prefix) = prev.segments.get(prefix_start..prefix_end) else {
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
            // second time. Caught by `ll-hls-runtime/tests/glass_to_glass.rs`
            // (issue #717 slice 5): every sample was double-delivered for
            // the first two segments of a real, live-paced run.
            let already_have_parts = self
                .delivered_parts
                .range((msn, 0)..(msn + 1, 0))
                .next()
                .is_some();
            if already_have_parts {
                self.delivered_segments.insert(msn);
                return Ok(());
            }
            let id = ResourceId::Segment { msn };
            if !self.requested.contains(&id) {
                let url = url::resolve(&self.playlist_url, &seg.uri);
                let byte_range = self.resolve_byte_range(&url, &seg.byte_range);
                self.request_resource(id, url, byte_range);
            }
            return Ok(());
        }

        let mut fully_accounted = true;
        for (i, part) in seg.parts.iter().enumerate() {
            let i = i as u64;
            if part.gap || self.delivered_parts.contains(&(msn, i)) {
                continue;
            }
            fully_accounted = false;
            let id = ResourceId::Part { msn, part: i };
            if !self.requested.contains(&id) {
                let url = url::resolve(&self.playlist_url, &part.uri);
                let byte_range = self.resolve_byte_range(&url, &part.byte_range);
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
            let i = i as u64;
            if part.gap || self.delivered_parts.contains(&(msn, i)) {
                continue;
            }
            let id = ResourceId::Part { msn, part: i };
            if !self.requested.contains(&id) {
                let url = url::resolve(&self.playlist_url, &part.uri);
                let byte_range = self.resolve_byte_range(&url, &part.byte_range);
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
        let byte_range = self.resolve_byte_range(&url, &map.byte_range);
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
    fn resolve_byte_range(&mut self, url: &str, br: &Option<ByteRange>) -> Option<(u64, u64)> {
        let br = br.as_ref()?;
        let offset = br
            .offset
            .unwrap_or_else(|| *self.byte_range_cursor.get(url).unwrap_or(&0));
        self.byte_range_cursor
            .insert(url.to_string(), offset + br.length);
        Some((offset, br.length))
    }

    fn resolve_hint_byte_range(
        &mut self,
        url: &str,
        ll: &transmux::hls::LowLatencyConfig,
    ) -> Option<(u64, u64)> {
        let length = ll.preload_hint_byte_range_length?;
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

    /// The classic-TS-HLS counterpart to [`Self::demux_and_emit`]: demux a
    /// self-contained MPEG-TS Part/Segment resource via [`TsDemux`] directly
    /// (no init bytes to concatenate — each `.ts` segment carries its own
    /// PAT/PMT/PES). On the very first such resource this client demuxes,
    /// also synthesizes the one [`Output::Init`] the crate's output contract
    /// requires ("exactly one `Init` precedes any `Samples`") from the
    /// recovered [`TrackSpec`]s via [`transmux::build_init_segment`] — a real
    /// `ftyp`+fragmented-`moov`, byte-for-byte demuxable by
    /// `transmux::Fmp4Demux` like any other init segment, so callers built
    /// against the fMP4 path (e.g. `multimux`'s `HlsPull`, which recovers
    /// track specs from `Output::Init`) need no TS-specific handling.
    fn demux_and_emit_ts(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        let mut demux = TsDemux::new();
        let media = demux
            .demux(bytes)
            .map_err(|source| Error::Demux { id, source })?;
        if !self.init_emitted {
            let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
            let init_bytes = transmux::build_init_segment(&specs, media.movie_timescale)
                .map_err(|source| Error::Demux { id, source })?;
            self.pending_outputs.push_back(Output::Init(init_bytes));
            self.init_emitted = true;
        }
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

    fn emit_discontinuity_if_needed(&mut self, msn: u64) {
        if self.discontinuous_msns.contains(&msn) && !self.discontinuity_emitted.contains(&msn) {
            self.pending_outputs.push_back(Output::Discontinuity);
            self.discontinuity_emitted.insert(msn);
        }
    }

    fn maybe_emit_end_of_stream(&mut self) {
        if self.saw_endlist && !self.end_emitted && self.outstanding_fetches == 0 {
            self.pending_outputs.push_back(Output::EndOfStream);
            self.end_emitted = true;
        }
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
        let mut client = LlHlsClient::new("http://example.com/playlist.m3u8");
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
        let mut client = LlHlsClient::new("http://example.com/playlist.m3u8");
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
        let client = LlHlsClient::new("http://example.com/playlist.m3u8");
        assert!(client.is_ts_segment(&[TS_SYNC_BYTE, 0x40, 0x11, 0x00]));
    }

    // ...but say no to an ISOBMFF (fMP4/CMAF) resource even when no map has
    // been seen yet — the content itself is never TS, so it must fall
    // through to the ordinary init-buffering path rather than being
    // misrouted into `TsDemux` (which would reject it as malformed TS).
    #[test]
    fn is_ts_segment_false_for_an_isobmff_resource_with_no_map_seen() {
        let client = LlHlsClient::new("http://example.com/playlist.m3u8");
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
        let mut client = LlHlsClient::new("http://example.com/playlist.m3u8");
        client
            .ensure_init_requested(&MapTag {
                uri: "init.mp4".to_string(),
                byte_range: None,
            })
            .expect("ensure_init_requested succeeds");
        assert!(!client.is_ts_segment(&[TS_SYNC_BYTE, 0x40, 0x11, 0x00]));
    }
}
