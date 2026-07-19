//! [`LlHlsClient`] — the sans-IO caller-driven engine.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use broadcast_common::Unpackage;
use transmux::hls::{ByteRange, MapTag, PreloadHintType};
use transmux::{Fmp4Demux, MediaPlaylist, MediaSegment, OpenSegment};

use crate::action::{Action, BlockingReload, ResourceId};
use crate::error::{Error, Result};
use crate::output::Output;
use crate::url;

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
///   `EXT-X-SERVER-CONTROL`/`EXT-X-PART-INF` (i.e.
///   [`transmux::hls::MediaPlaylist::low_latency`] is `Some`), every reload is
///   a Blocking Playlist Reload (RFC 8216bis §6.2.5.2) naming the next not-yet-seen
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

        if let Some(map) = playlist.segments.last().and_then(|s| s.map.as_ref()) {
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
            let blocking = playlist.low_latency.is_some().then(|| {
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
    /// [`Error::Demux`] if `transmux::Fmp4Demux` rejects the concatenation of
    /// the cached init + `bytes`.
    pub fn on_resource(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
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
                if self.init_bytes.is_none() {
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
            // Full-segment fallback (non-LL playlist, or a segment whose
            // parts were never individually rendered).
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
