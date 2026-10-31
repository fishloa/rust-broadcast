//! [`HlsClient`] — the sans-IO caller-driven engine.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use container_probe::segment;
use transmux::TrackId;

use broadcast_common::Unpackage;
use broadcast_hls::{
    ByteRange, EncryptionMethod, MapTag, MasterPlaylist, MediaKey, MediaPlaylist, MediaSegment,
    OpenSegment, PreloadHintType,
};
use transmux::sample_aes::{BLOCK_LEN, KEY_LEN, aes128_decrypt_segment};
use transmux::{DemuxEvent, Fmp4Demux, InputDegradation, Sample, StreamingTsDemux, TrackSpec};

use super::action::{Action, BlockingReload, KeyId, ResourceId};
use super::error::{Error, Result};
use super::output::{MasterSelection, Output, RenditionInfo, VariantInfo, VariantPolicy};
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

/// Ceiling on ciphertext held while its key is still missing (total bytes).
const MAX_PENDING_KEYED_BYTES: usize = 64 * 1024 * 1024;

/// Longest the non-blocking reload cadence may wait, however large the
/// untrusted `EXT-X-TARGETDURATION` says the target duration is.
const MAX_RELOAD_WAIT_MS: u64 = 60_000;

const MILLIS_PER_SECOND: u64 = 1000;

/// `EXT-X-VERSION` of the one-segment playlist a single `EXT-X-MAP` line is
/// parsed in: the lowest version that allows `EXT-X-MAP` without
/// `EXT-X-I-FRAMES-ONLY` (RFC 8216 §4.4.4.5).
/// `moov` (ISO/IEC 14496-12 §8.2.1).
const MOVIE_BOX: [u8; 4] = *b"moov";

const MAP_PROBE_VERSION: u32 = 6;

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

/// Half the Target Duration, the non-blocking reload cadence (RFC 8216
/// §4.3.3.1: no more often than once per Target Duration), clamped because
/// the Target Duration is untrusted.
fn reload_wait_ms(target_duration: u32) -> u64 {
    (u64::from(target_duration.max(1)).saturating_mul(MILLIS_PER_SECOND) / 2)
        .min(MAX_RELOAD_WAIT_MS)
}

/// Every `EXT-X-MAP` line of `text` with the identity-`KEYFORMAT` key in effect
/// at it (RFC 8216 §4.3.2.5: a key tag applies to a Media Initialization
/// Section only when it precedes the `EXT-X-MAP` tag), in playlist order. The
/// key is `None` when that map is not encrypted.
fn keys_at_maps(text: &str) -> Vec<(MapTag, Option<MediaKey>)> {
    const MAP_PREFIX: &str = "#EXT-X-MAP:";
    const KEY_PREFIX: &str = "#EXT-X-KEY:";
    let mut current: Option<MediaKey> = None;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with(KEY_PREFIX) {
            let probe = format!(
                "#EXTM3U\n#EXT-X-TARGETDURATION:1\n{}\n#EXTINF:1,\nx\n",
                line.trim_end()
            );
            if let Ok(parsed) = MediaPlaylist::parse(&probe)
                && let Ok(mut keys) = parsed.segment_keys()
                && let Some(key) = keys.pop()
            {
                current = key;
            }
        } else if line.starts_with(MAP_PREFIX) {
            let probe = format!(
                "#EXTM3U\n#EXT-X-VERSION:{MAP_PROBE_VERSION}\n#EXT-X-TARGETDURATION:1\n{}\n#EXTINF:1,\nx\n",
                line.trim_end()
            );
            if let Ok(parsed) = MediaPlaylist::parse(&probe)
                && let Some(map) = parsed.segments.first().and_then(|seg| seg.map.clone())
            {
                out.push((map, current.clone()));
            }
        }
    }
    out
}

/// `true` when `bytes` holds a complete `moov` box: what every Media
/// Initialization Section of fragmented MP4 carries.
fn has_movie_box(bytes: &[u8]) -> bool {
    for step in transmux::box_iter(bytes) {
        let Ok((boxed, _)) = step else { return false };
        if boxed.header.box_type.0 == MOVIE_BOX {
            return true;
        }
    }
    false
}

/// `uri` up to its query string: a CDN that re-signs a segment URL on every
/// reload changes only the query, on every segment of the window at once.
fn without_query(uri: &str) -> &str {
    uri.split_once('?').map_or(uri, |(path, _)| path)
}

/// The `EXT-X-KEY` tags in effect after `dropped` segments, one per
/// `KEYFORMAT`: when a playlist's window slides past the segment carrying a
/// key tag, a merged view must state that key again on its new first segment.
fn carried_key_tags(dropped: &[MediaSegment]) -> Vec<String> {
    const KEY_PREFIX: &str = "#EXT-X-KEY:";
    const KEYFORMAT_ATTR: &str = "KEYFORMAT=\"";
    const IDENTITY: &str = "identity";
    let mut latest: BTreeMap<&str, &String> = BTreeMap::new();
    for tag in dropped.iter().flat_map(|seg| &seg.pre_tags) {
        if !tag.starts_with(KEY_PREFIX) {
            continue;
        }
        let format = tag
            .split_once(KEYFORMAT_ATTR)
            .and_then(|(_, rest)| rest.split_once('"'))
            .map_or(IDENTITY, |(format, _)| format);
        latest.insert(format, tag);
    }
    latest.into_values().cloned().collect()
}

fn decrypt_segment(
    id: ResourceId,
    crypto: SegmentCrypto,
    key: &[u8; KEY_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    aes128_decrypt_segment(key, &crypto.iv, ciphertext).map_err(|_| Error::DecryptFailed {
        id,
        key_id: crypto.key_id,
    })
}

/// The Media Sequence Number a tracked resource belongs to, `None` for the
/// MSN-less init segment.
fn resource_msn(id: &ResourceId) -> Option<u64> {
    match id {
        ResourceId::Part { msn, .. } | ResourceId::Segment { msn } => Some(*msn),
        _ => None,
    }
}

/// Identifies one Media Initialization Section the client holds or wants. Ids
/// are never reused, so a stale id can not alias a later init.
type InitId = u32;

/// Where a Media Initialization Section is in its fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitFetch {
    /// Known and needed, waiting for the one init fetch in flight to finish.
    Wanted,
    /// Requested ([`ResourceId::Init`] refers to it).
    InFlight,
    /// Its fetch failed; the next playlist (or [`HlsClient::retry_init`]) asks again.
    Failed,
    /// Arrived encrypted, waiting for its key.
    Arrived,
    /// Held in plain.
    Ready,
}

/// One Media Initialization Section: the `EXT-X-MAP` of one map-run
/// (RFC 8216 §4.4.4.5 — a map applies to every following segment until the
/// next map).
#[derive(Debug)]
struct InitSlot {
    map: MapTag,
    url: String,
    byte_range: Option<(u64, u64)>,
    encryption: Option<MediaKey>,
    crypto: Option<SegmentCrypto>,
    fetch: InitFetch,
    cipher: Option<Vec<u8>>,
    bytes: Option<Vec<u8>>,
    skipped_reported: BTreeSet<(Option<u32>, String)>,
}

/// An `EXT-X-MAP` of the playlist being processed, with the key that encrypts
/// it and the Media Sequence Number of the first segment it applies to.
#[derive(Debug)]
struct MapEntry {
    map: MapTag,
    encryption: Option<MediaKey>,
    first_msn: u64,
}

impl MapEntry {
    fn unusable(&self) -> Option<Error> {
        let key = self.encryption.as_ref()?;
        let what = if key.method != EncryptionMethod::Aes128 {
            "encrypted EXT-X-MAP: only METHOD=AES-128 is decrypted"
        } else if key.uri.is_none() {
            "encrypted EXT-X-MAP: key without URI"
        } else {
            return None;
        };
        Some(Error::UnsupportedEncryption {
            method: key.method,
            what,
        })
    }
}

/// An AES-128 key whose `Debug` never prints the key octets.
#[derive(Clone, Copy)]
struct KeyBytes([u8; KEY_LEN]);

impl core::fmt::Debug for KeyBytes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("KeyBytes(<redacted>)")
    }
}

/// The decryption parameters of one AES-128 segment (RFC 8216 §5.2).
#[derive(Debug, Clone, Copy)]
struct SegmentCrypto {
    key_id: KeyId,
    iv: [u8; BLOCK_LEN],
}

/// RFC 8216 §5.2: the Media Sequence Number as a big-endian 128-bit number,
/// zero-padded on the left.
fn media_sequence_iv(msn: u64) -> [u8; BLOCK_LEN] {
    let mut iv = [0u8; BLOCK_LEN];
    let msn_bytes = msn.to_be_bytes();
    let start = BLOCK_LEN - msn_bytes.len();
    iv[start..].copy_from_slice(&msn_bytes);
    iv
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
fn program_date_time(seg: &MediaSegment) -> Option<broadcast_hls::ProgramDateTime> {
    seg.program_date_time
}

/// [`program_date_time`] as milliseconds since the Unix epoch.
fn program_date_time_ms(seg: &MediaSegment) -> Option<i64> {
    program_date_time(seg).map(|pdt| pdt.timestamp().as_millisecond())
}

/// Parse an RFC 3339 / ISO 8601 date-time (`2024-05-01T12:00:00.250Z`,
/// `...+02:00`, `...-0500`) into Unix milliseconds. `None` for anything that
/// does not parse — never a guess. Delegates to [`jiff::Timestamp`] (an offset
/// is required; a leap second is clamped to `:59`).
#[cfg(test)]
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
///   rule (tracked per resource URL). Each Media Initialization Section
///   (`EXT-X-MAP`) is fetched once, one at a time, and used for exactly the
///   segments of its own map-run (RFC 8216 §4.4.4.5: a map applies to every
///   following segment until the next map).
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
    base_override: Option<String>,
    variant_policy: VariantPolicy,
    master: Option<MasterSelection>,

    pending_actions: VecDeque<Action>,
    /// The caller-clock instant at which the `WaitMs` now at the front of
    /// `pending_actions` was first observed by [`Self::poll_timeout`]; the
    /// wait's absolute deadline is `anchor + wait`. Cleared whenever an action
    /// is drained, so a later wait is anchored afresh.
    #[cfg(feature = "std")]
    wait_anchor: Option<std::time::Instant>,
    pending_outputs: VecDeque<Output>,

    /// Every Media Initialization Section the client holds or wants, by id.
    inits: BTreeMap<InitId, InitSlot>,
    next_init_id: InitId,
    /// The init [`ResourceId::Init`] currently refers to: the one in flight, or
    /// the last one that was.
    init_target: Option<InitId>,
    /// The init whose [`Output::Init`] the consumer saw last.
    announced_init: Option<InitId>,
    /// The init that applies to each Media Sequence Number being fetched (RFC
    /// 8216 §4.4.4.5: the map preceding the segment).
    msn_init: BTreeMap<u64, InitId>,
    /// The maps of the playlist being processed.
    playlist_maps: Vec<MapEntry>,
    /// Last fMP4 track spec set for which [`Output::Tracks`] was emitted, so a
    /// repeat is only emitted when a new init yields a different set.
    last_fmp4_specs: Option<Vec<TrackSpec>>,
    /// Part/Segment resources delivered before the init segment arrived —
    /// buffered (in arrival order) and replayed once their init is ready, so
    /// the caller's fetch/response IO can complete in any order (a real HTTP
    /// client has no reason to serialize on init-first).
    pending_demux: VecDeque<(ResourceId, Vec<u8>)>,

    /// Key id per distinct absolute key URI.
    key_ids: BTreeMap<String, KeyId>,
    /// The URI each assigned id was issued for.
    key_urls: BTreeMap<KeyId, String>,
    /// Keys fetched so far, cached for the life of the client.
    key_material: BTreeMap<KeyId, KeyBytes>,
    /// Keys whose fetch is outstanding.
    keys_in_flight: BTreeSet<KeyId>,
    /// Decryption parameters of every encrypted segment requested, by MSN.
    segment_crypto: BTreeMap<u64, SegmentCrypto>,
    /// Encrypted segments delivered before their key arrived (ciphertext).
    pending_keyed: Vec<(ResourceId, Vec<u8>)>,
    /// Next [`KeyId`] to hand out; never reused, so evicting a key can not
    /// make a stale id alias a live one.
    next_key_id: u32,
    /// A full playlist was asked for after an unmergeable delta; a repeat
    /// is paced by a wait instead of hot-looping.
    full_reload_requested: bool,

    requested: BTreeSet<ResourceId>,
    /// URL and byte range of every Part/Segment fetch issued, so [`Self::retry_resource`] can re-request a failed one
    /// without a playlist reload. Pruned with the other MSN-keyed records.
    resource_requests: BTreeMap<ResourceId, (String, Option<(u64, u64)>)>,
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
    ts_specs: BTreeMap<TrackId, TrackSpec>,
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

/// Choose a variant index according to the policy.
fn choose_variant(variants: &[VariantInfo], policy: &VariantPolicy) -> usize {
    if variants.is_empty() {
        return 0;
    }
    let mut candidates: Vec<usize> = Vec::new();
    for (i, v) in variants.iter().enumerate() {
        let mut ok = true;
        if let Some(max_bw) = policy.max_bandwidth
            && v.bandwidth > max_bw
        {
            ok = false;
        }
        if ok
            && let Some(max_h) = policy.max_height
            && let Some(res) = v.resolution
            && res.1 > max_h
        {
            ok = false;
        }
        if ok
            && let Some(supported) = &policy.supported_codecs
            && let Some(codecs) = &v.codecs
        {
            for entry in codecs.split(',') {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                if !supported.iter().any(|s| entry.starts_with(s)) {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            candidates.push(i);
        }
    }
    if !candidates.is_empty() {
        let mut best = candidates[0];
        for &i in &candidates[1..] {
            if variants[i].bandwidth > variants[best].bandwidth {
                best = i;
            }
        }
        best
    } else {
        let mut best = 0usize;
        for i in 1..variants.len() {
            if variants[i].bandwidth < variants[best].bandwidth {
                best = i;
            }
        }
        best
    }
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
            base_override: None,
            variant_policy: VariantPolicy::default(),
            master: None,
            pending_actions,
            #[cfg(feature = "std")]
            wait_anchor: None,
            pending_outputs: VecDeque::new(),
            inits: BTreeMap::new(),
            next_init_id: 0,
            init_target: None,
            announced_init: None,
            msn_init: BTreeMap::new(),
            playlist_maps: Vec::new(),
            last_fmp4_specs: None,
            pending_demux: VecDeque::new(),
            key_ids: BTreeMap::new(),
            key_urls: BTreeMap::new(),
            key_material: BTreeMap::new(),
            keys_in_flight: BTreeSet::new(),
            segment_crypto: BTreeMap::new(),
            pending_keyed: Vec::new(),
            next_key_id: 0,
            full_reload_requested: false,
            requested: BTreeSet::new(),
            resource_requests: BTreeMap::new(),
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

    /// Set the URL relative playlist URIs resolve against: the final URL of the next playlist
    /// response after redirects (RFC 3986 section 5.1.3). Reloads keep requesting
    /// [`HlsClient::playlist_url`]; the base is cleared when the client switches to another
    /// playlist URL.
    pub fn set_playlist_base(&mut self, base: impl Into<String>) {
        self.base_override = Some(base.into());
    }

    fn base(&self) -> &str {
        self.base_override.as_deref().unwrap_or(&self.playlist_url)
    }

    /// Whether a playlist carrying `EXT-X-ENDLIST` has been processed (the stream is VOD or
    /// has finished).
    pub fn endlist_seen(&self) -> bool {
        self.saw_endlist
    }

    /// `EXT-X-TARGETDURATION` in seconds of the last full media playlist, `None` before one
    /// was processed.
    pub fn target_duration_secs(&self) -> Option<u32> {
        self.last_full_playlist.as_ref().map(|p| p.target_duration)
    }

    /// Set the variant selection policy. Must be callable before the first
    /// `on_playlist`.
    pub fn set_variant_policy(&mut self, policy: VariantPolicy) {
        self.variant_policy = policy;
    }

    /// Switch to a different variant by index (stall switching).
    pub fn select_variant(&mut self, index: usize) -> Result<()> {
        let variants_len = self
            .master
            .as_ref()
            .ok_or(Error::NoSuchVariant { index })?
            .variants
            .len();
        if index >= variants_len {
            return Err(Error::NoSuchVariant { index });
        }
        let url = self
            .master
            .as_ref()
            .and_then(|m| m.variants.get(index))
            .map(|v| v.url.clone())
            .ok_or(Error::NoSuchVariant { index })?;
        self.restart_after_regression()?;
        self.pending_actions.clear();
        self.playlist_url = url.clone();
        self.base_override = None;
        self.pending_actions.push_back(Action::FetchPlaylist {
            url,
            blocking: None,
            skip: false,
        });
        if let Some(m) = &mut self.master {
            m.chosen = index;
        }
        Ok(())
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
        if text.contains("#EXT-X-STREAM-INF") {
            let master = MasterPlaylist::parse(text)?;
            let base = self.base().to_string();
            let mut variants: Vec<VariantInfo> = Vec::new();
            for v in &master.variants {
                variants.push(VariantInfo {
                    url: url::resolve(&base, &v.uri),
                    bandwidth: v.bandwidth,
                    average_bandwidth: v.average_bandwidth,
                    resolution: v.resolution,
                    codecs: v.codecs.clone(),
                    audio_group: v.audio.clone(),
                    subtitles_group: v.subtitles.clone(),
                });
            }
            let mut renditions: Vec<RenditionInfo> = Vec::new();
            for m in &master.media {
                renditions.push(RenditionInfo {
                    media_type: m.media_type,
                    group_id: m.group_id.clone(),
                    name: m.name.clone(),
                    language: m.language.clone(),
                    default: m.default,
                    autoselect: m.autoselect,
                    url: m.uri.as_ref().map(|u| url::resolve(&base, u)),
                });
            }
            let chosen = choose_variant(&variants, &self.variant_policy);
            let selection = MasterSelection {
                variants,
                chosen,
                renditions,
            };
            self.pending_outputs
                .push_back(Output::Master(selection.clone()));
            self.master = Some(selection);
            let Some(chosen_url) = self
                .master
                .as_ref()
                .and_then(|m| m.variants.get(chosen))
                .map(|v| v.url.clone())
            else {
                return Err(Error::NoSuchVariant { index: chosen });
            };
            self.playlist_url = chosen_url.clone();
            self.base_override = None;
            self.pending_actions.clear();
            self.pending_actions.push_back(Action::FetchPlaylist {
                url: chosen_url,
                blocking: None,
                skip: false,
            });
            return Ok(());
        }
        let playlist = MediaPlaylist::parse(text)?;
        let playlist = self.merge_delta(playlist);
        if playlist.skip.is_some() {
            return Err(self.request_full_playlist(&playlist));
        }
        self.full_reload_requested = false;
        let segment_keys = playlist.segment_key_results()?;
        let open_key = if playlist.open_segment.is_some() {
            playlist.open_segment_key()?
        } else {
            None
        };
        let mut first_error: Option<Error> = None;
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
        self.forget_keys_below(playlist.media_sequence);

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

        self.playlist_maps = self.map_entries(&playlist, text, next_msn);

        for (i, seg) in playlist.segments.iter().enumerate() {
            let msn = playlist.media_sequence.saturating_add(index_u64(i));
            if lag_floor.is_some_and(|floor| msn < floor) {
                self.skip_segment(msn, seg)?;
                continue;
            }
            if let Some(e) = seg.map.as_ref().and_then(|m| self.unusable_map(m)) {
                self.skip_segment(msn, seg)?;
                first_error.get_or_insert(e);
                continue;
            }
            let key = match segment_keys.get(i) {
                Some(Ok(key)) => key.as_ref(),
                Some(Err(e)) => {
                    self.skip_segment(msn, seg)?;
                    first_error.get_or_insert(Error::PlaylistParse(e.clone()));
                    continue;
                }
                None => None,
            };
            match self.process_closed_segment(msn, seg, key, playlist.iframes_only) {
                Ok(()) => {}
                Err(e @ Error::UnsupportedEncryption { .. }) => {
                    self.skip_segment(msn, seg)?;
                    first_error.get_or_insert(e);
                }
                Err(e) => return Err(e),
            }
        }

        if let Some(open) = &playlist.open_segment {
            if let Some(e) = open.map.as_ref().and_then(|m| self.unusable_map(m)) {
                first_error.get_or_insert(e);
            } else {
                match &open_key {
                    Some(key) => {
                        first_error.get_or_insert(Error::UnsupportedEncryption {
                            method: key.method,
                            what: "encrypted partial segments",
                        });
                    }
                    None => self.process_open_segment(next_msn, open)?,
                }
            }
        }

        self.pump_init_fetch();

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
                    if !self.requested.contains(&id)
                        && !self.delivered_parts.contains(&(next_msn, part_idx))
                    {
                        let url = url::resolve(self.base(), hint_uri);
                        let byte_range = self.resolve_hint_byte_range(&url, ll)?;
                        self.request_resource(id, url, byte_range);
                    }
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
                self.pending_actions
                    .push_back(Action::WaitMs(reload_wait_ms(playlist.target_duration)));
            }
        }

        self.prune(&playlist);
        self.last_full_playlist = Some(playlist);

        self.maybe_emit_end_of_stream()?;
        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Queue a reload that does not ask for a delta, because the delta just
    /// received cannot be merged into a full view (no baseline, or it no
    /// longer covers the skipped range). Returns the error to report.
    fn request_full_playlist(&mut self, delta: &MediaPlaylist) -> Error {
        if self.full_reload_requested {
            self.pending_actions
                .push_back(Action::WaitMs(reload_wait_ms(delta.target_duration)));
        }
        self.full_reload_requested = true;
        self.pending_actions.push_back(Action::FetchPlaylist {
            url: self.playlist_url.clone(),
            blocking: None,
            skip: false,
        });
        Error::DeltaNotMergeable {
            media_sequence: delta.media_sequence,
            skipped_segments: delta.skip.as_ref().map_or(0, |s| s.skipped_segments),
        }
    }

    /// Queue a plain playlist reload unless one is already queued. For a
    /// driver that has just dropped a response the core rejected before it
    /// could schedule the next reload itself.
    pub fn ensure_playlist_reload(&mut self) -> bool {
        if self
            .pending_actions
            .iter()
            .any(|a| matches!(a, Action::FetchPlaylist { .. }))
        {
            return false;
        }
        self.pending_actions.push_back(Action::FetchPlaylist {
            url: self.playlist_url.clone(),
            blocking: None,
            skip: false,
        });
        true
    }

    /// Request a Media Initialization Section again after a failed fetch
    /// ([`Self::on_error`] with [`ResourceId::Init`]). A live playlist's next
    /// reload does this by itself; a VOD playlist (`EXT-X-ENDLIST`) has no
    /// further reload, so its driver calls this. `false` when there is
    /// nothing to retry (an init is in flight, or none failed).
    pub fn retry_init(&mut self) -> bool {
        if self
            .inits
            .values()
            .any(|slot| slot.fetch == InitFetch::InFlight)
        {
            return false;
        }
        let failed: Vec<InitId> = self
            .inits
            .iter()
            .filter(|(_, slot)| slot.fetch == InitFetch::Failed)
            .map(|(id, _)| *id)
            .collect();
        for id in &failed {
            if let Some(slot) = self.inits.get_mut(id) {
                slot.fetch = InitFetch::Wanted;
            }
        }
        self.pump_init_fetch();
        !failed.is_empty()
    }

    /// Request a Part/Segment again after a failed fetch ([`Self::on_error`]
    /// or an [`Error`] from [`Self::on_resource`]) without a playlist reload:
    /// a VOD playlist (`EXT-X-ENDLIST`) has no further reload to do it. `false`
    /// when there is nothing to retry (the fetch is in flight, the resource
    /// was accepted, or it left the window).
    pub fn retry_resource(&mut self, id: ResourceId) -> bool {
        if self.requested.contains(&id) {
            return false;
        }
        let Some((url, byte_range)) = self.resource_requests.get(&id).cloned() else {
            return false;
        };
        self.request_resource(id, url, byte_range);
        true
    }

    /// Give up on a Part/Segment that cannot be fetched and carry on with the
    /// next one: it is never requested again, the timeline restarts (held-back
    /// final access units of the old one are flushed first) and
    /// [`Output::Discontinuity`] follows, so the consumer re-syncs across the
    /// gap. No-op for [`ResourceId::Init`], which cannot be skipped.
    pub fn skip_resource(&mut self, id: ResourceId) {
        match id {
            ResourceId::Init => return,
            ResourceId::Segment { msn } => {
                self.delivered_segments.insert(msn);
            }
            ResourceId::Part { msn, part } => {
                self.delivered_parts.insert((msn, part));
            }
        }
        self.requested.remove(&id);
        self.resource_requests.remove(&id);
        // Flushing can only fail to rebuild a track init; the gap is reported either way.
        let _ = self.restart_ts_demux(id);
        self.pending_outputs.push_back(Output::Discontinuity);
        let _ = self.maybe_emit_end_of_stream();
    }

    /// The `EXT-X-MAP`s of `playlist`, in playlist order, each with the key
    /// that encrypts it and the Media Sequence Number it first applies to.
    fn map_entries(&self, playlist: &MediaPlaylist, text: &str, next_msn: u64) -> Vec<MapEntry> {
        let keyed = keys_at_maps(text);
        let mut entries: Vec<MapEntry> = Vec::new();
        let listed = playlist
            .segments
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.map
                    .as_ref()
                    .map(|m| (m, playlist.media_sequence.saturating_add(index_u64(i))))
            })
            .chain(
                playlist
                    .open_segment
                    .as_ref()
                    .and_then(|o| o.map.as_ref())
                    .map(|m| (m, next_msn)),
            );
        for (map, msn) in listed {
            if entries.iter().any(|e| e.map == *map) {
                continue;
            }
            let key = match keyed.iter().rev().find(|(m, _)| m == map) {
                Some((_, key)) => key.clone(),
                None => self
                    .init_slot_for(&url::resolve(self.base(), &map.uri), &map.byte_range)
                    .and_then(|id| self.inits.get(&id))
                    .and_then(|slot| slot.encryption.clone()),
            };
            entries.push(MapEntry {
                map: map.clone(),
                encryption: key,
                first_msn: msn,
            });
        }
        entries
    }

    fn unusable_map(&self, map: &MapTag) -> Option<Error> {
        self.playlist_maps
            .iter()
            .find(|e| e.map == *map)
            .and_then(MapEntry::unusable)
    }

    /// The init slot holding the resource at `url` (resolved) with `byte_range`.
    /// The query is not part of the identity: a CDN that re-signs the map URI
    /// on every reload (as it does segment URIs, see [`without_query`]) still
    /// names the same init.
    fn init_slot_for(&self, url: &str, byte_range: &Option<ByteRange>) -> Option<InitId> {
        self.inits
            .iter()
            .find(|(_, slot)| {
                without_query(&slot.url) == without_query(url) && slot.map.byte_range == *byte_range
            })
            .map(|(id, _)| *id)
    }

    /// The init slot for `map`, created (and queued for fetching) when the
    /// client has none for that resource yet; a slot whose fetch failed is
    /// queued again.
    fn init_for_map(&mut self, map: &MapTag) -> Result<InitId> {
        let url = url::resolve(self.base(), &map.uri);
        if let Some(id) = self.init_slot_for(&url, &map.byte_range)
            && let Some(slot) = self.inits.get_mut(&id)
        {
            if slot.url != url {
                slot.url = url;
                slot.map = map.clone();
            }
            if slot.fetch == InitFetch::Failed {
                slot.fetch = InitFetch::Wanted;
            }
            return Ok(id);
        }
        let (key, first_msn) = self
            .playlist_maps
            .iter()
            .find(|e| e.map == *map)
            .map_or((None, 0), |e| (e.encryption.clone(), e.first_msn));
        let byte_range = self.resolve_byte_range(&url, &map.byte_range)?;
        let crypto = match key.as_ref().and_then(|k| k.uri.as_deref().map(|u| (k, u))) {
            Some((key, uri)) => {
                let key_id = self.ensure_key_requested(uri);
                let iv = key.iv.unwrap_or_else(|| media_sequence_iv(first_msn));
                Some(SegmentCrypto { key_id, iv })
            }
            None => None,
        };
        let id = self.next_init_id;
        self.next_init_id = self.next_init_id.saturating_add(1);
        self.inits.insert(
            id,
            InitSlot {
                map: map.clone(),
                url,
                byte_range,
                encryption: key,
                crypto,
                fetch: InitFetch::Wanted,
                cipher: None,
                bytes: None,
                skipped_reported: BTreeSet::new(),
            },
        );
        Ok(id)
    }

    /// Start the next wanted init fetch unless one is already in flight:
    /// [`ResourceId::Init`] names the one in flight, so inits go one at a time,
    /// in the order the playlist needs them.
    fn pump_init_fetch(&mut self) {
        if self
            .inits
            .values()
            .any(|slot| slot.fetch == InitFetch::InFlight)
        {
            return;
        }
        let Some((id, slot)) = self
            .inits
            .iter_mut()
            .find(|(_, slot)| slot.fetch == InitFetch::Wanted)
        else {
            return;
        };
        slot.fetch = InitFetch::InFlight;
        let id = *id;
        let fetch = Action::FetchResource {
            id: ResourceId::Init,
            url: slot.url.clone(),
            byte_range: slot.byte_range,
        };
        self.init_target = Some(id);
        self.pending_actions.push_back(fetch);
        self.outstanding_fetches += 1;
    }

    /// Bind Media Sequence Number `msn` to the init of `map`, the one that
    /// applies to it (RFC 8216 §4.4.4.5), creating that init on first use.
    fn bind_init(&mut self, msn: u64, map: Option<&MapTag>) -> Result<()> {
        if let Some(map) = map {
            let id = self.init_for_map(map)?;
            self.msn_init.insert(msn, id);
        }
        Ok(())
    }

    /// The init slot a Part/Segment resource is demuxed with: the one its
    /// Media Sequence Number is bound to, else the newest.
    fn init_of(&self, id: &ResourceId) -> Option<InitId> {
        resource_msn(id)
            .and_then(|msn| self.msn_init.get(&msn).copied())
            .or_else(|| self.inits.keys().next_back().copied())
    }

    fn init_ready(&self, slot: Option<InitId>) -> bool {
        slot.and_then(|id| self.inits.get(&id))
            .is_some_and(|slot| slot.bytes.is_some())
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
            ResourceId::Init => self.init_target.is_some(),
            ResourceId::Part { .. } | ResourceId::Segment { .. } => self.requested.contains(&id),
        };
        if !was_requested {
            return Err(Error::UnrequestedResource { id });
        }
        let duplicate = match id {
            ResourceId::Init => !self
                .init_target
                .and_then(|t| self.inits.get(&t))
                .is_some_and(|slot| slot.fetch == InitFetch::InFlight),
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
            ResourceId::Init => self.accept_init(bytes)?,
            ResourceId::Part { .. } | ResourceId::Segment { .. } => {
                let processed = self.accept_media_resource(id, bytes);
                match processed {
                    Ok(()) => {}
                    Err(e @ Error::PendingKeyedOverflow { .. }) => return Err(e),
                    Err(e) => {
                        self.fulfilled.remove(&id);
                        self.requested.remove(&id);
                        return Err(e);
                    }
                }
            }
        }
        self.maybe_emit_end_of_stream()?;
        Ok(())
    }

    fn accept_init(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(target) = self.init_target else {
            return Ok(());
        };
        let Some(slot) = self.inits.get_mut(&target) else {
            return Ok(());
        };
        if slot.crypto.is_none() && !has_movie_box(bytes) {
            slot.fetch = InitFetch::Failed;
            return Err(Error::InitUndecodable);
        }
        match slot.crypto {
            Some(crypto) => match self.key_material.get(&crypto.key_id).copied() {
                Some(KeyBytes(key)) => self.complete_encrypted_init(target, crypto, &key, bytes),
                None => {
                    slot.cipher = Some(bytes.to_vec());
                    slot.fetch = InitFetch::Arrived;
                    Ok(())
                }
            },
            None => self.complete_init(target, bytes),
        }
    }

    /// Take `plain` as the Media Initialization Section of `target`: emit it
    /// when it is the first the consumer sees, and replay the media that
    /// arrived before it.
    fn complete_init(&mut self, target: InitId, plain: &[u8]) -> Result<()> {
        if let Some(slot) = self.inits.get_mut(&target) {
            slot.bytes = Some(plain.to_vec());
            slot.cipher = None;
            slot.fetch = InitFetch::Ready;
        }
        if self.announced_init.is_none() {
            self.announce_init(target);
        }
        self.pump_init_fetch();
        let buffered: Vec<_> = self.pending_demux.drain(..).collect();
        let mut waiting = VecDeque::new();
        let mut failure = None;
        for (bid, bbytes) in buffered {
            if failure.is_some() || !self.init_ready(self.init_of(&bid)) {
                waiting.push_back((bid, bbytes));
                continue;
            }
            if let Err(e) = self.finish_media_resource(bid, &bbytes) {
                self.fulfilled.remove(&bid);
                self.requested.remove(&bid);
                failure = Some(e);
            }
        }
        self.pending_demux = waiting;
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Make `slot` the init the consumer sees: its [`Output::Init`] goes out
    /// unless it was the last one announced.
    fn announce_init(&mut self, slot: InitId) {
        if self.announced_init == Some(slot) {
            return;
        }
        let Some(bytes) = self.inits.get(&slot).and_then(|s| s.bytes.clone()) else {
            return;
        };
        self.pending_outputs.push_back(Output::Init(bytes));
        self.announced_init = Some(slot);
        self.last_fmp4_specs = None;
    }

    fn complete_encrypted_init(
        &mut self,
        target: InitId,
        crypto: SegmentCrypto,
        key: &[u8; KEY_LEN],
        cipher: &[u8],
    ) -> Result<()> {
        match decrypt_segment(ResourceId::Init, crypto, key, cipher) {
            Ok(plain) if !has_movie_box(&plain) => {
                if let Some(slot) = self.inits.get_mut(&target) {
                    slot.fetch = InitFetch::Failed;
                }
                Err(Error::InitUndecodable)
            }
            Ok(plain) => self.complete_init(target, &plain),
            Err(e) => {
                if let Some(slot) = self.inits.get_mut(&target) {
                    slot.fetch = InitFetch::Failed;
                }
                Err(e)
            }
        }
    }

    /// Route a delivered Part/Segment: decrypt it first when its segment is
    /// AES-128 encrypted (holding the ciphertext until the key arrives).
    fn accept_media_resource(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        if let ResourceId::Segment { msn } = id
            && let Some(crypto) = self.segment_crypto.get(&msn).copied()
        {
            return match self.key_material.get(&crypto.key_id).copied() {
                Some(KeyBytes(key)) => {
                    let plain = decrypt_segment(id, crypto, &key, bytes)?;
                    self.route_media_resource(id, &plain)
                }
                None => self.buffer_until_keyed(id, bytes),
            };
        }
        self.route_media_resource(id, bytes)
    }

    /// Hold ciphertext until its key arrives, within
    /// [`MAX_PENDING_KEYED_BYTES`]: the oldest waiting segments are dropped
    /// (and become requestable again) to make room, reported as
    /// [`Error::PendingKeyedOverflow`].
    fn buffer_until_keyed(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_PENDING_KEYED_BYTES {
            self.fulfilled.remove(&id);
            self.requested.remove(&id);
            return Err(Error::PendingKeyedOverflow { dropped: id });
        }
        let mut held: usize = self.pending_keyed.iter().map(|(_, b)| b.len()).sum();
        let mut dropped: Option<ResourceId> = None;
        while held.saturating_add(bytes.len()) > MAX_PENDING_KEYED_BYTES
            && !self.pending_keyed.is_empty()
        {
            let (old_id, old) = self.pending_keyed.remove(0);
            held = held.saturating_sub(old.len());
            self.fulfilled.remove(&old_id);
            self.requested.remove(&old_id);
            dropped.get_or_insert(old_id);
        }
        self.pending_keyed.push((id, bytes.to_vec()));
        match dropped {
            Some(dropped) => Err(Error::PendingKeyedOverflow { dropped }),
            None => Ok(()),
        }
    }

    fn route_media_resource(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        let recognised = if self.is_ts_segment(bytes) {
            segment::ts_structure(bytes)
        } else {
            segment::media_fragment(bytes) || segment::progressive_movie(bytes)
        };
        if !recognised {
            return Err(Error::NotMedia { id });
        }
        if self.is_ts_segment(bytes) {
            // Classic MPEG-TS-segment HLS (issue #760): no init
            // resource will ever arrive for this playlist, so demux
            // this self-contained TS segment straight away rather
            // than buffering it forever waiting for one.
            self.finish_ts_resource(id, bytes)
        } else if !self.init_ready(self.init_of(&id)) {
            self.pending_demux.push_back((id, bytes.to_vec()));
            Ok(())
        } else {
            self.finish_media_resource(id, bytes)
        }
    }

    /// Feed the key file fetched for an [`Action::FetchKey`]. Encrypted
    /// segments that arrived earlier are decrypted and demuxed in Media
    /// Sequence Number order.
    ///
    /// # Errors
    /// [`Error::UnrequestedKey`] for an unknown id or a key already held;
    /// [`Error::BadKeyLength`] unless `bytes` is exactly 16 octets;
    /// [`Error::DecryptFailed`] if a waiting segment does not decrypt.
    pub fn on_key(&mut self, id: KeyId, bytes: &[u8]) -> Result<()> {
        if !self.key_urls.contains_key(&id) || self.key_material.contains_key(&id) {
            return Err(Error::UnrequestedKey { id });
        }
        if self.keys_in_flight.remove(&id) {
            self.outstanding_fetches = self.outstanding_fetches.saturating_sub(1);
        }
        let key: [u8; KEY_LEN] = bytes.try_into().map_err(|_| Error::BadKeyLength {
            id,
            len: bytes.len(),
        })?;
        self.key_material.insert(id, KeyBytes(key));
        let waiting_inits: Vec<(InitId, SegmentCrypto)> = self
            .inits
            .iter()
            .filter(|(_, slot)| slot.fetch == InitFetch::Arrived)
            .filter_map(|(init, slot)| slot.crypto.filter(|c| c.key_id == id).map(|c| (*init, c)))
            .collect();
        for (init, crypto) in waiting_inits {
            let cipher = self
                .inits
                .get_mut(&init)
                .and_then(|slot| slot.cipher.take());
            if let Some(cipher) = cipher {
                self.complete_encrypted_init(init, crypto, &key, &cipher)?;
            }
        }
        let (mut ready, rest): (Vec<_>, Vec<_>) = core::mem::take(&mut self.pending_keyed)
            .into_iter()
            .partition(|(rid, _)| {
                resource_msn(rid)
                    .and_then(|msn| self.segment_crypto.get(&msn))
                    .is_some_and(|c| c.key_id == id)
            });
        self.pending_keyed = rest;
        ready.sort_by_key(|(rid, _)| resource_msn(rid));
        let mut ready = ready.into_iter();
        while let Some((rid, cipher)) = ready.next() {
            let crypto = resource_msn(&rid).and_then(|msn| self.segment_crypto.get(&msn).copied());
            let result = match crypto {
                Some(crypto) => decrypt_segment(rid, crypto, &key, &cipher)
                    .and_then(|plain| self.route_media_resource(rid, &plain)),
                None => Ok(()),
            };
            if let Err(e) = result {
                self.fulfilled.remove(&rid);
                self.requested.remove(&rid);
                self.pending_keyed.extend(ready);
                return Err(e);
            }
        }
        self.maybe_emit_end_of_stream()?;
        Ok(())
    }

    /// Report that fetching key `id` failed. The next [`Self::on_playlist`]
    /// requests it again; segments already waiting on it keep waiting.
    pub fn on_key_error(&mut self, id: KeyId) {
        if self.keys_in_flight.remove(&id) {
            self.outstanding_fetches = self.outstanding_fetches.saturating_sub(1);
        }
    }

    /// Request key `uri` once per distinct URI; the id is stable for the life
    /// of the client.
    fn ensure_key_requested(&mut self, uri: &str) -> KeyId {
        let url = url::resolve(self.base(), uri);
        let id = match self.key_ids.get(&url) {
            Some(id) => *id,
            None => {
                let id = KeyId(self.next_key_id);
                self.next_key_id = self.next_key_id.saturating_add(1);
                self.key_ids.insert(url.clone(), id);
                self.key_urls.insert(id, url);
                id
            }
        };
        self.request_key(id);
        id
    }

    /// Queue the fetch of key `id` unless it is held or already in flight.
    fn request_key(&mut self, id: KeyId) {
        let Some(url) = self.key_urls.get(&id).cloned() else {
            return;
        };
        if !self.key_material.contains_key(&id) && self.keys_in_flight.insert(id) {
            self.outstanding_fetches += 1;
            self.pending_actions.push_back(Action::FetchKey { id, url });
        }
    }

    /// Drop every key no segment of the window (nor the init) refers to, its
    /// in-flight fetch included: the per-URL maps would otherwise grow
    /// with every tokenised key URL, and a key behind a URI that returns
    /// after its segments left the window is fetched afresh.
    fn forget_keys_below(&mut self, first_msn: u64) {
        self.segment_crypto.retain(|&m, _| m >= first_msn);
        let mut live: BTreeSet<KeyId> = self.segment_crypto.values().map(|c| c.key_id).collect();
        live.extend(
            self.inits
                .values()
                .filter_map(|slot| slot.crypto.map(|c| c.key_id)),
        );
        let abandoned: Vec<KeyId> = self
            .keys_in_flight
            .iter()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in abandoned {
            self.keys_in_flight.remove(&id);
            self.outstanding_fetches = self.outstanding_fetches.saturating_sub(1);
            self.pending_actions
                .retain(|a| !matches!(a, Action::FetchKey { id: queued, .. } if *queued == id));
        }
        self.key_ids.retain(|_, id| live.contains(id));
        self.key_urls.retain(|id, _| live.contains(id));
        self.key_material.retain(|id, _| live.contains(id));
    }

    /// Record the decryption parameters of encrypted segment `msn` and make
    /// sure its key is requested.
    fn prepare_encrypted_segment(
        &mut self,
        msn: u64,
        seg: &MediaSegment,
        key: &MediaKey,
        iframes_only: bool,
    ) -> Result<()> {
        let unsupported = |what| Error::UnsupportedEncryption {
            method: key.method,
            what,
        };
        if key.method != EncryptionMethod::Aes128 {
            return Err(unsupported("only METHOD=AES-128 is decrypted"));
        }
        if iframes_only {
            return Err(unsupported("EXT-X-I-FRAMES-ONLY with encryption"));
        }
        if !seg.parts.is_empty() {
            return Err(unsupported("encrypted partial segments"));
        }
        let Some(uri) = key.uri.as_deref() else {
            return Err(unsupported("key without URI"));
        };
        if let Some(existing) = self.segment_crypto.get(&msn).copied() {
            self.request_key(existing.key_id);
            return Ok(());
        }
        let id = self.ensure_key_requested(uri);
        let iv = key.iv.unwrap_or_else(|| media_sequence_iv(msn));
        self.segment_crypto
            .insert(msn, SegmentCrypto { key_id: id, iv });
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
                self.release_ts_segment_end(id)?;
                self.delivered_segments.insert(msn);
            }
            ResourceId::Init => {}
        }
        Ok(())
    }

    /// `true` when `bytes` should be routed to [`Self::finish_ts_resource`]
    /// (classic MPEG-TS-segment HLS, issue #760) rather than the fMP4/CMAF
    /// path: this playlist has never advertised an `EXT-X-MAP` (no init
    /// fetch is outstanding or cached — [`Self::inits`] is empty; by the
    /// time any Part/Segment fetch response reaches [`Self::on_resource`],
    /// [`Self::on_playlist`] has already fully processed the playlist that
    /// requested it, including any map it carries, so this check is never
    /// stale) **and** `bytes` starts with the MPEG-TS sync byte — an
    /// fMP4/CMAF resource always starts with an ISOBMFF box
    /// (`ftyp`/`styp`/`moof`), never [`TS_SYNC_BYTE`].
    fn is_ts_segment(&self, bytes: &[u8]) -> bool {
        self.inits.is_empty()
            && (bytes.first() == Some(&TS_SYNC_BYTE) || segment::ts_structure(bytes))
    }

    /// Report that a previously requested [`ResourceId`] (or the playlist
    /// itself, via [`None`]) failed. Clears the id's "requested" bookkeeping
    /// so the next [`Self::on_playlist`] call naturally re-requests it (no
    /// automatic retry timer — the caller drives retry cadence).
    pub fn on_error(&mut self, id: Option<ResourceId>) {
        if let Some(id) = id {
            self.outstanding_fetches = self.outstanding_fetches.saturating_sub(1);
            match id {
                ResourceId::Init => {
                    if let Some(slot) = self
                        .init_target
                        .and_then(|target| self.inits.get_mut(&target))
                        .filter(|slot| slot.fetch == InitFetch::InFlight)
                    {
                        slot.fetch = InitFetch::Failed;
                    }
                    self.pump_init_fetch();
                }
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
    /// the last playlist this client processed (itself a full view, merged or
    /// not). A merged playlist has `skip` cleared; if there is no baseline,
    /// or it doesn't cover the skipped range, the delta is returned as-is
    /// with `skip` still set, which [`Self::on_playlist`] answers by
    /// requesting a full playlist.
    fn merge_delta(&self, playlist: MediaPlaylist) -> MediaPlaylist {
        let Some(skip) = &playlist.skip else {
            return playlist;
        };
        if skip.skipped_segments == 0 {
            let mut merged = playlist;
            merged.skip = None;
            return merged;
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
        if let (Some(first), Some(dropped)) =
            (segments.first_mut(), prev.segments.get(..prefix_start))
        {
            let mut carried = carried_key_tags(dropped);
            carried.append(&mut first.pre_tags);
            first.pre_tags = carried;
        }
        segments.extend(merged.segments);
        merged.segments = segments;
        merged.skip = None;
        merged
    }

    fn process_closed_segment(
        &mut self,
        msn: u64,
        seg: &MediaSegment,
        key: Option<&MediaKey>,
        iframes_only: bool,
    ) -> Result<()> {
        if seg.discontinuous {
            self.discontinuous_msns.insert(msn);
        }
        if self.delivered_segments.contains(&msn) {
            return Ok(());
        }
        match key {
            Some(key) => self.prepare_encrypted_segment(msn, seg, key, iframes_only)?,
            None => {
                self.segment_crypto.remove(&msn);
            }
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
                self.bind_init(msn, seg.map.as_ref())?;
                let url = url::resolve(self.base(), &seg.uri);
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
                self.bind_init(msn, seg.map.as_ref())?;
                let url = url::resolve(self.base(), &part.uri);
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
                self.bind_init(msn, open.map.as_ref())?;
                let url = url::resolve(self.base(), &part.uri);
                let byte_range = self.resolve_byte_range(&url, &part.byte_range)?;
                self.request_resource(id, url, byte_range);
            }
        }
        Ok(())
    }

    fn ensure_init_requested(&mut self, map: &MapTag) -> Result<()> {
        self.init_for_map(map)?;
        self.pump_init_fetch();
        Ok(())
    }

    fn request_resource(&mut self, id: ResourceId, url: String, byte_range: Option<(u64, u64)>) {
        self.requested.insert(id);
        self.resource_requests.insert(id, (url.clone(), byte_range));
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
        let slot = self.init_of(&id).filter(|s| self.init_ready(Some(*s)));
        let Some(slot) = slot else {
            return Err(Error::InitNotYetAvailable { id });
        };
        self.announce_init(slot);
        let init = self
            .inits
            .get(&slot)
            .and_then(|s| s.bytes.as_ref())
            .ok_or(Error::InitNotYetAvailable { id })?;
        if segment::progressive_movie(bytes) {
            return Err(Error::NotFragmented { id });
        }
        let mut combined = Vec::with_capacity(init.len() + bytes.len());
        combined.extend_from_slice(init);
        combined.extend_from_slice(bytes);
        let mut demux = Fmp4Demux::new();
        let media = demux
            .unpackage(combined.as_slice())
            .map_err(|source| Error::Demux { id, source })?;
        let damaged = media
            .degraded
            .iter()
            .filter(|d| matches!(d, InputDegradation::MalformedUnit { .. }))
            .count();
        if damaged > 0 {
            self.pending_outputs.push_back(Output::FragmentDamaged {
                id,
                skipped: damaged,
            });
        }
        for skipped in &media.skipped {
            let key = (skipped.track_id.map(TrackId::value), skipped.fourcc.clone());
            let first = self
                .inits
                .get_mut(&slot)
                .is_some_and(|s| s.skipped_reported.insert(key));
            if first {
                self.pending_outputs.push_back(Output::TrackSkipped {
                    track_id: skipped.track_id,
                    fourcc: skipped.fourcc.clone(),
                    reason: skipped.reason.clone(),
                });
            }
        }
        let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let specs_changed = self
            .last_fmp4_specs
            .as_ref()
            .map(|prev| format!("{:?}", prev) != format!("{:?}", specs))
            .unwrap_or(true);
        if specs_changed {
            self.pending_outputs
                .push_back(Output::Tracks(specs.clone()));
            self.last_fmp4_specs = Some(specs);
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

    /// Emit the pending `EXT-X-DISCONTINUITY` for `msn` and, when one fires,
    /// restart the TS demuxer's timeline: the timestamps after a
    /// discontinuity are a new timeline (RFC 8216 §4.3.4.3), so the previous
    /// 33-bit unwrap anchor must not carry across it. The old timeline's
    /// held-back final access units are flushed *before* the marker. The
    /// track set is kept ([`StreamingTsDemux::restart_timeline`]): a fresh
    /// demuxer would have to re-probe every PID from the new timeline alone
    /// and loses any PID whose parameter sets are not in it (a segment that
    /// opens mid-GOP).
    fn begin_ts_resource(&mut self, id: ResourceId, msn: u64) -> Result<()> {
        if self.discontinuous_msns.contains(&msn) && !self.discontinuity_emitted.contains(&msn) {
            self.restart_ts_demux(id)?;
            self.emit_discontinuity_if_needed(msn);
        }
        Ok(())
    }

    fn restart_ts_demux(&mut self, id: ResourceId) -> Result<()> {
        let Some(TsDemuxState(demux)) = self.ts_demux.as_mut() else {
            return Ok(());
        };
        demux.restart_timeline();
        self.drain_ts_demux(id)
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

    /// A whole media segment has been fed: a PID its PMT declares that sent
    /// nothing in it is absent, so it must not hold the rest of its program
    /// back until `EXT-X-ENDLIST` (a live playlist has none).
    fn release_ts_segment_end(&mut self, id: ResourceId) -> Result<()> {
        let Some(TsDemuxState(demux)) = self.ts_demux.as_mut() else {
            return Ok(());
        };
        demux.release_silent_heads();
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
        let mut per_track: BTreeMap<TrackId, Vec<Sample>> = BTreeMap::new();
        let mut events: Vec<DemuxEvent> = Vec::new();
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    if spec.config.is_muxable_in_bmff() {
                        self.ts_specs.insert(spec.track_id, spec);
                    } else {
                        self.ts_specs.remove(&spec.track_id);
                        let stream_type = match &spec.config {
                            transmux::CodecConfig::Data { stream_type, .. } => Some(*stream_type),
                            _ => None,
                        };
                        self.pending_outputs.push_back(Output::TrackDropped {
                            track_id: spec.track_id,
                            stream_type,
                        });
                    }
                }
                DemuxEvent::TrackRemoved { track_id, .. } => {
                    self.ts_specs.remove(&track_id);
                }
                DemuxEvent::Sample { track_id, .. }
                | DemuxEvent::DisplayChanged { track_id, .. }
                    if self.ts_specs.contains_key(&track_id) =>
                {
                    events.push(event);
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
                    .push_back(Output::Tracks(specs.clone()));
                self.pending_outputs
                    .push_back(Output::Init(init_bytes.clone()));
                self.last_ts_init = Some(init_bytes);
            }
        }
        for event in events {
            match event {
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    per_track.entry(track_id).or_default().push(sample);
                }
                DemuxEvent::DisplayChanged {
                    track_id,
                    display,
                    pts,
                    ..
                } => {
                    if let Some(samples) = per_track.remove(&track_id)
                        && !samples.is_empty()
                    {
                        self.pending_outputs
                            .push_back(Output::Samples { track_id, samples });
                    }
                    self.pending_outputs.push_back(Output::DisplayChanged {
                        track_id,
                        pts,
                        display,
                    });
                }
                _ => {}
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
            let url = url::resolve(self.base(), &seg.uri);
            self.resolve_byte_range(&url, &seg.byte_range)?;
        } else {
            for part in seg.parts.iter().filter(|p| !p.gap) {
                let url = url::resolve(self.base(), &part.uri);
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
        self.resource_requests.retain(|id, _| !below(id));
        self.fulfilled.retain(|id| !below(id));
        self.pending_keyed.retain(|(id, _)| !below(id));
        self.segment_crypto.retain(|&m, _| m >= first);
        self.delivered_parts.retain(|&(m, _)| m >= first);
        self.delivered_segments.retain(|&m| m >= first);
        self.discontinuous_msns.retain(|&m| m >= first);
        self.discontinuity_emitted.retain(|&m| m >= first);
        self.msn_init.retain(|&m, _| m >= first);
        self.prune_inits();

        if playlist.skip.is_none() {
            let base = self.base();
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

    /// Drop every held init no segment still being fetched refers to, keeping
    /// the newest.
    fn prune_inits(&mut self) {
        let newest = self.inits.keys().next_back().copied();
        let bound: BTreeSet<InitId> = self.msn_init.values().copied().collect();
        self.inits.retain(|id, slot| {
            Some(*id) == newest || bound.contains(id) || slot.fetch != InitFetch::Ready
        });
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
    ///    higher number is caught here. The URI's query string is ignored only
    ///    when it differs on *every* shared segment (a CDN re-signing the whole
    ///    window); a window where some shared segments keep their query and
    ///    others change it is a restart.
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

        // 1. Identity at every shared number. A query that differs is a re-signing
        //    only when it differs on every shared segment; on some of them it is a
        //    restart that reuses names.
        let mut overlap = false;
        let mut shared = 0usize;
        let mut requeried = 0usize;
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
            if without_query(&old.uri) != without_query(&seg.uri)
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
            shared = shared.saturating_add(1);
            if old.uri != seg.uri {
                requeried = requeried.saturating_add(1);
            }
        }
        if requeried != 0 && requeried != shared {
            return Continuity::Restart;
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
        self.pending_actions
            .retain(|a| !matches!(a, Action::FetchKey { .. }));
        self.keys_in_flight.clear();
        self.segment_crypto.clear();
        self.pending_keyed.clear();
        self.requested.clear();
        self.fulfilled.clear();
        self.delivered_parts.clear();
        self.delivered_segments.clear();
        self.discontinuous_msns.clear();
        self.discontinuity_emitted.clear();
        self.byte_range_cursor.clear();
        self.outstanding_fetches = 0;
        self.last_full_playlist = None;
        self.inits.clear();
        self.init_target = None;
        self.announced_init = None;
        self.msn_init.clear();
        self.last_fmp4_specs = None;
        self.full_reload_requested = false;
        self.joined = false;
        self.pending_outputs.push_back(Output::Discontinuity);
        Ok(())
    }

    fn all_playlist_segments_delivered(&self) -> bool {
        let Some(playlist) = &self.last_full_playlist else {
            return true;
        };
        (0..playlist.segments.len() as u64).all(|i| {
            let msn = playlist.media_sequence.saturating_add(i);
            self.delivered_segments.contains(&msn)
                || self.fulfilled.contains(&ResourceId::Segment { msn })
        })
    }

    fn maybe_emit_end_of_stream(&mut self) -> Result<()> {
        if self.saw_endlist
            && !self.end_emitted
            && self.outstanding_fetches == 0
            && self.pending_keyed.is_empty()
            && self.inits.values().all(|slot| slot.bytes.is_some())
            && self.all_playlist_segments_delivered()
        {
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
    use transmux::sample_aes::aes128_encrypt_segment;

    const EMPTY_FRAGMENT: &[u8] = &[
        0, 0, 0, 8, b'm', b'o', b'o', b'f', 0, 0, 0, 8, b'm', b'd', b'a', b't',
    ];

    #[test]
    fn decrypt_segment_strips_pkcs7_padding_to_the_exact_plaintext() {
        let key = [0x5Au8; KEY_LEN];
        let iv = [0xC3u8; BLOCK_LEN];
        let id = ResourceId::Segment { msn: 9 };
        let crypto = SegmentCrypto {
            key_id: KeyId(0),
            iv,
        };
        for len in [1usize, 15, 16, 17, 37, 188] {
            let plain: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let cipher = aes128_encrypt_segment(&key, &iv, &plain);
            assert_eq!(decrypt_segment(id, crypto, &key, &cipher).unwrap(), plain);
        }
    }

    #[test]
    fn media_sequence_iv_is_big_endian_left_padded() {
        let iv = media_sequence_iv(0x0102_0304_0506_0708);
        assert_eq!(iv[..8], [0u8; 8]);
        assert_eq!(iv[8..], [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(media_sequence_iv(0)[..], [0u8; 16]);
        assert_eq!(media_sequence_iv(0x1FF)[14..], [1, 0xFF]);
    }

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
        let result = client.on_resource(id, EMPTY_FRAGMENT);
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
            .on_resource(ResourceId::Segment { msn: 0 }, EMPTY_FRAGMENT)
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
            .on_resource(ResourceId::Segment { msn: 5 }, EMPTY_FRAGMENT)
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

    fn keyed_playlist(first_msn: u64, count: u64, key_uri: &str) -> String {
        let mut text = format!(
            "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n\
             #EXT-X-MEDIA-SEQUENCE:{first_msn}\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"{key_uri}\"\n"
        );
        for msn in first_msn..first_msn + count {
            text.push_str(&format!("#EXTINF:2.0,\nseg{msn}.ts\n"));
        }
        text
    }

    fn pending_keyed_bytes(client: &HlsClient) -> usize {
        client.pending_keyed.iter().map(|(_, b)| b.len()).sum()
    }

    #[test]
    fn pending_keyed_is_pruned_with_the_window() {
        let mut client = HlsClient::new("http://example.com/p.m3u8");
        client
            .on_playlist(keyed_playlist(0, 1, "key.bin").as_bytes())
            .unwrap();
        client
            .on_resource(ResourceId::Segment { msn: 0 }, &[0u8; 32])
            .unwrap();
        assert_eq!(client.pending_keyed.len(), 1);
        client
            .on_playlist(keyed_playlist(5, 1, "key.bin").as_bytes())
            .unwrap();
        assert!(
            client.pending_keyed.is_empty(),
            "ciphertext below the window can never be played: {:?}",
            client
                .pending_keyed
                .iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pending_keyed_is_capped_and_drops_the_oldest_with_a_typed_error() {
        let mut client = HlsClient::new("http://example.com/p.m3u8");
        client
            .on_playlist(keyed_playlist(0, 2, "key.bin").as_bytes())
            .unwrap();
        let big = vec![0u8; 40 * 1024 * 1024];
        client
            .on_resource(ResourceId::Segment { msn: 0 }, &big)
            .unwrap();
        let err = client
            .on_resource(ResourceId::Segment { msn: 1 }, &big)
            .expect_err("the second 40 MiB segment exceeds the 64 MiB cap");
        assert!(
            matches!(err, Error::PendingKeyedOverflow { dropped } if dropped == ResourceId::Segment { msn: 0 }),
            "{err:?}"
        );
        assert!(
            pending_keyed_bytes(&client) <= MAX_PENDING_KEYED_BYTES,
            "{} bytes buffered",
            pending_keyed_bytes(&client)
        );
        assert_eq!(
            client
                .pending_keyed
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec![ResourceId::Segment { msn: 1 }]
        );
    }

    #[test]
    fn key_maps_do_not_grow_with_per_reload_tokenised_key_urls() {
        let mut client = HlsClient::new("http://example.com/p.m3u8");
        for n in 0..8u64 {
            client
                .on_playlist(keyed_playlist(n, 2, &format!("key.bin?t={n}")).as_bytes())
                .unwrap();
        }
        assert!(
            client.key_ids.len() <= 3 && client.key_urls.len() <= 3,
            "{:?} {:?}",
            client.key_ids,
            client.key_urls
        );
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
