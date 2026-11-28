//! [`HlsClient`] — the sans-IO caller-driven engine.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use container_probe::segment;
use core::num::NonZeroUsize;
use core::time::Duration;
use mpeg_ts::ts::TS_SYNC_BYTE;
use transmux::TrackId;

use ::url::Url;
use broadcast_common::range::ByteSpan;
use broadcast_common::url_ref;
use broadcast_core::timescale::Timescale;
use broadcast_hls::limits::Limits;
use broadcast_hls::{
    ByteRange, DecimalSeconds, EncryptionMethod, MapTag, MasterPlaylist, MediaKey, MediaPlaylist,
    MediaSegment, OpenSegment, ParseDiagnosticKind, PreloadHintType,
};
use isobmff::cenc::KEY_LEN;
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::sample_aes::{BLOCK_LEN, aes128_decrypt_segment};
use transmux::{AbandonReason, DemuxEvent, InputDegradation, Sample, StreamingTsDemux, TrackSpec};

use super::action::{Action, BlockingReload, KeyId, ResourceId};
use super::error::{Error, Result};
use super::output::{MasterSelection, Output, RenditionInfo, VariantInfo, VariantPolicy};
use super::url;
use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::units::MILLIS_PER_SEC;

/// Movie timescale of the init segment synthesized for classic TS-segment
/// HLS: the 90 kHz MPEG-2 system clock the PES PTS/DTS are expressed in
/// (ISO/IEC 13818-1 §2.4.3.7), the same clock `transmux`'s TS demuxer stamps
/// its samples with.
const TS_MOVIE_TIMESCALE: Timescale = transmux::VIDEO_CLOCK_TIMESCALE;

/// How many Target Durations before the end of a live Playlist the first
/// segment the client plays must start at the latest (RFC 8216 §6.3.3: "the
/// client SHOULD NOT choose a segment that starts less than three target
/// durations from the end of the Playlist file").
const LIVE_EDGE_TARGET_DURATIONS: f64 = 3.0;

/// Ceiling on ciphertext held while its key is still missing (total bytes).
const MAX_PENDING_KEYED_BYTES: usize = 64 * 1024 * 1024;

/// Ceiling on Part/Segment bodies held until their init segment is ready
/// (total bytes). A playlist whose `EXT-X-MAP` never becomes ready while media
/// keeps arriving would otherwise copy every segment into
/// [`HlsClient::pending_demux`] for the life of the pull (audit 20 finding 3,
/// issue #2814), so the oldest waiting body is dropped past this cap and
/// reported as [`Error::PendingDemuxOverflow`].
const MAX_PENDING_DEMUX_BYTES: usize = 64 * 1024 * 1024;

/// Ceiling on errors queued for [`HlsClient::next_error`]. The final slot
/// coalesces any further report into one counted
/// [`Error::UnresolvableReferences`], so a playlist naming an unbounded number
/// of bad references cannot grow the queue without bound even when the host
/// never drains it.
const MAX_PENDING_ERRORS: usize = 256;

/// Ceiling on held Media Initialization Section slots. A playlist may name a
/// new `EXT-X-MAP` per segment (and a failing fetch leaves a slot stuck
/// non-`Ready`), so without a bound the map grows for the life of the pull and
/// `init_slot_for`'s linear scan gets slower with it (audit 20 finding 4,
/// issue #2815).
const MAX_INIT_SLOTS: usize = 64;

/// Bytes handed to one streaming MP4 `feed` of a Part/Segment body: the stage
/// holds this piece plus the one fragment it is resolving, so the client's own
/// buffer never grows to the body's length.
const DEMUX_FEED_BYTES: usize = 64 * 1024;

/// Longest the non-blocking reload cadence may wait, however large the
/// untrusted `EXT-X-TARGETDURATION` says the target duration is.
const MAX_RELOAD_WAIT_MS: u64 = 60_000;
/// Half the Target Duration: the non-blocking reload cadence (RFC 8216
/// §4.3.3.1), expressed as the divisor of the Target Duration.
const RELOAD_WAIT_DIVISOR: u64 = 2;

/// `EXT-X-VERSION` of the one-segment playlist a single `EXT-X-MAP` line is
/// parsed in: the lowest version that allows `EXT-X-MAP` without
/// `EXT-X-I-FRAMES-ONLY` (RFC 8216 §4.3.3.6).
const MAP_PROBE_VERSION: u32 = 6;

/// [`StreamingTsDemux`] with a `Debug` impl (the transmux type has none), so
/// [`HlsClient`] can keep deriving it.
struct TsDemuxState(StreamingTsDemux);

impl core::fmt::Debug for TsDemuxState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TsDemuxState(StreamingTsDemux)")
    }
}

/// [`Mp4Demux`] with a `Debug` impl (the transmux type has none), so
/// [`HlsClient`] can keep deriving it.
struct Mp4DemuxState(Mp4Demux);

impl core::fmt::Debug for Mp4DemuxState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Mp4DemuxState(Mp4Demux)")
    }
}

/// A forward-only streaming MP4 stage holding `plain`, the Media
/// Initialization Section every following Part/Segment continues.
fn start_demux(plain: &[u8]) -> core::result::Result<Mp4Demux, transmux::Error> {
    let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
    demux.feed(Mp4Input::Next(plain), Timestamp::ZERO)?;
    demux.feed(Mp4Input::SegmentEnd, Timestamp::ZERO)?;
    Ok(demux)
}

/// The sample-entry four-CC and reason [`Output::TrackSkipped`] reports for a
/// track the demuxer abandoned.
fn abandoned_track(reason: AbandonReason) -> (String, String) {
    match reason {
        AbandonReason::UnsupportedSampleEntry { fourcc, detail } => {
            let fourcc = String::from_utf8_lossy(&fourcc).into_owned();
            let text = match detail {
                Some(detail) => format!("sample entry '{fourcc}' rejected: {detail}"),
                None => format!(
                    "sample entry '{fourcc}' has no CodecConfig reconstruction in this crate"
                ),
            };
            (fourcc, text)
        }
        AbandonReason::UnsupportedCodec => (
            String::from("unknown"),
            String::from("sample entry 'unknown' has no CodecConfig reconstruction in this crate"),
        ),
        AbandonReason::MalformedTrack { expected } => (
            String::from("unknown"),
            format!("unexpected box: expected {expected}"),
        ),
        other => (String::from("unknown"), String::from(other.name())),
    }
}

/// A track with `TrackSpec::protection` is CENC/CBCS-protected: this client
/// decrypts AES-128 HLS segments only, never sample-level encryption, so the
/// track is reported and left out.
fn encrypted_data_track(spec: &transmux::TrackSpec) -> Option<AbandonReason> {
    spec.protection.as_ref()?;
    let fourcc = if spec.config.is_audio() {
        *b"enca"
    } else {
        *b"encv"
    };
    Some(AbandonReason::UnsupportedSampleEntry {
        fourcc,
        detail: Some(String::from(
            "CENC/CBCS sample encryption is not supported by this client",
        )),
    })
}

/// `usize` index -> `u64` (lossless on every supported target; saturates
/// rather than casting if a target ever had a wider `usize`).
fn index_u64(i: usize) -> u64 {
    u64::try_from(i).unwrap_or(u64::MAX)
}

/// Half the Target Duration, the non-blocking reload cadence (RFC 8216
/// §4.3.3.1: no more often than once per Target Duration), clamped because
/// the Target Duration is untrusted.
fn reload_wait(target_duration: u32) -> Duration {
    let ms = (u64::from(target_duration.max(1)).saturating_mul(MILLIS_PER_SEC)
        / RELOAD_WAIT_DIVISOR)
        .min(MAX_RELOAD_WAIT_MS);
    Duration::from_millis(ms)
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
fn resource_msn(id: &ResourceId) -> Option<broadcast_hls::MediaSequence> {
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
/// (RFC 8216bis §4.4.4.5 — a map applies to every following segment until the
/// next map).
#[derive(Debug)]
struct InitSlot {
    map: MapTag,
    url: Url,
    byte_range: Option<ByteSpan>,
    encryption: Option<MediaKey>,
    crypto: Option<SegmentCrypto>,
    fetch: InitFetch,
    cipher: Option<Vec<u8>>,
    bytes: Option<Vec<u8>>,
    skipped_reported: BTreeSet<(Option<u32>, String)>,
    /// The one streaming MP4 stage this init feeds, kept across every Part and
    /// Segment of its map-run.
    demux: Option<Mp4DemuxState>,
    /// Events the stage produced while it was fed this init, replayed before
    /// the first Part/Segment's own.
    init_events: Vec<DemuxEvent>,
    /// The stage's error from reading this init, if it could not.
    demux_error: Option<transmux::Error>,
    /// Track specs the stage has announced for this init.
    specs: Vec<TrackSpec>,
    /// Whether the stage has announced any track (added or abandoned) for this
    /// init.
    tracks_known: bool,
}

/// An `EXT-X-MAP` of the playlist being processed, with the key that encrypts
/// it and the Media Sequence Number of the first segment it applies to.
#[derive(Debug)]
struct MapEntry {
    map: MapTag,
    encryption: Option<MediaKey>,
    first_msn: broadcast_hls::MediaSequence,
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
fn media_sequence_iv(msn: broadcast_hls::MediaSequence) -> [u8; BLOCK_LEN] {
    let mut iv = [0u8; BLOCK_LEN];
    let msn_bytes = msn.value().to_be_bytes();
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

/// Why a Part/Segment was given up on, as the report of its loss names it.
///
/// The caller knows what it saw (an HTTP status, a timeout); the client maps
/// it to the structure the report names, so a host counting losses can tell a
/// missing resource from a slow one without parsing log text.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// The origin answered 404.
    NotFound,
    /// The origin answered 410.
    Gone,
    /// The fetch did not complete within the caller's deadline.
    TimedOut,
    /// A failure with no status (a transport error, a short body).
    Unusable,
}

impl SkipReason {
    /// Stable label of this reason — used by `Display`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            SkipReason::NotFound => "not found",
            SkipReason::Gone => "gone",
            SkipReason::TimedOut => "timed out",
            SkipReason::Unusable => "unusable",
        }
    }

    /// The label of a segment lost for this reason.
    #[must_use]
    pub const fn segment(self) -> &'static str {
        match self {
            SkipReason::NotFound => "HLS segment (not found)",
            SkipReason::Gone => "HLS segment (gone)",
            SkipReason::TimedOut => "HLS segment (timed out)",
            SkipReason::Unusable => "HLS segment (unusable)",
        }
    }

    /// The label of a part lost for this reason.
    #[must_use]
    pub const fn part(self) -> &'static str {
        match self {
            SkipReason::NotFound => "HLS part (not found)",
            SkipReason::Gone => "HLS part (gone)",
            SkipReason::TimedOut => "HLS part (timed out)",
            SkipReason::Unusable => "HLS part (unusable)",
        }
    }
}

broadcast_core::impl_spec_display!(SkipReason);

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
///   paced by an [`Action::Wait`] hint derived from `#EXT-X-TARGETDURATION`.
///   `EXT-X-SKIP`/`CAN-SKIP-UNTIL` Playlist Delta Updates (RFC 8216bis §4.4.5.2)
///   are requested once a full-playlist baseline exists, and merged back into
///   a full view before further processing — see `merge_delta` internally.
/// - **Fetch pipeline** (slice 3): the `EXT-X-PRELOAD-HINT`ed part is fetched
///   ahead of its own appearance as a numbered `EXT-X-PART`; `BYTERANGE`
///   parts are supported, including the RFC 8216bis §4.4.4.9 "omitted offset
///   means immediately after the previous sub-range of the same resource"
///   rule (tracked per resource URL). Each Media Initialization Section
///   (`EXT-X-MAP`) is fetched once, one at a time, and used for exactly the
///   segments of its own map-run (RFC 8216bis §4.4.4.5: a map applies to every
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
///   [`Output::Samples`]; parts/segments are demuxed by one streaming
///   [`transmux::mp4_demux::Mp4Demux`] per init segment (this crate never
///   re-implements ISOBMFF box parsing, only reuses transmux's), fed the init
///   once and then each fetched resource in bounded pieces, so `Output::Samples`
///   carries real access units, not opaque container bytes. `#EXT-X-DISCONTINUITY`
///   on a segment surfaces as [`Output::Discontinuity`] immediately before that
///   segment's first samples. **Known limitation**: an in-progress
///   ([`OpenSegment`]) segment carries no discontinuity flag of its own (only a
///   *closed* [`MediaSegment`] does) — if every part of a segment was already
///   delivered while it was still open, a discontinuity revealed only once it
///   closes is signalled late (after those parts' samples, not before). This
///   is a gap in the current wire model ([`broadcast_hls::OpenSegment`]), not
///   something this crate can fix locally.
/// - **Classic MPEG-TS-segment HLS** (issue #760): a playlist that never
///   advertises an `EXT-X-MAP` (HLS v3, the dominant legacy/IPTV form —
///   self-contained `.ts` segments carrying their own PAT/PMT/PES, no
///   separate init resource) routes each fetched Part/Segment through
///   [`transmux::StreamingTsDemux`] instead, content-sniffed by the MPEG-TS sync byte
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
    playlist_url: Url,
    base_override: Option<Url>,
    variant_policy: VariantPolicy,
    limits: Limits,
    /// Caller-supplied ceiling on one PID's in-progress PES buffer in the
    /// classic-TS demuxer; `None` is unlimited.
    max_pes_unit_bytes: Option<NonZeroUsize>,
    /// What each damage report means for this client.
    damage_policy: broadcast_core::damage::DamagePolicy,
    /// How far a segment's demuxed media may differ from its `EXTINF` before
    /// it is reported; `None` never compares.
    segment_duration_tolerance: Option<core::time::Duration>,
    /// Playlist diagnostics already reported, so a live reload of the same
    /// malformed tag is one report rather than one per reload.
    reported_diagnostics: BTreeSet<String>,
    master: Option<MasterSelection>,

    pending_actions: VecDeque<Action>,
    /// The caller-clock instant at which the `Wait` now at the front of
    /// `pending_actions` was first observed by [`Self::poll_timeout`]; the
    /// wait's absolute deadline is `anchor + wait`. Cleared whenever an action
    /// is drained, so a later wait is anchored afresh.
    wait_anchor: Option<Timestamp>,
    pending_outputs: VecDeque<Output>,
    /// Playlist references the guarded resolver refused (issue #2458), reported
    /// through [`Self::next_error`] in the order the playlist named them.
    pending_errors: VecDeque<Error>,

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
    msn_init: BTreeMap<broadcast_hls::MediaSequence, InitId>,
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
    key_ids: BTreeMap<Url, KeyId>,
    /// The URI each assigned id was issued for.
    key_urls: BTreeMap<KeyId, Url>,
    /// Keys fetched so far, cached for the life of the client.
    key_material: BTreeMap<KeyId, KeyBytes>,
    /// Keys whose fetch is outstanding.
    keys_in_flight: BTreeSet<KeyId>,
    /// Decryption parameters of every encrypted segment requested, by MSN.
    segment_crypto: BTreeMap<broadcast_hls::MediaSequence, SegmentCrypto>,
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
    resource_requests: BTreeMap<ResourceId, (Url, Option<ByteSpan>)>,
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
    delivered_parts: BTreeSet<(broadcast_hls::MediaSequence, broadcast_hls::PartIndex)>,
    delivered_segments: BTreeSet<broadcast_hls::MediaSequence>,
    discontinuous_msns: BTreeSet<broadcast_hls::MediaSequence>,
    discontinuity_emitted: BTreeSet<broadcast_hls::MediaSequence>,
    byte_range_cursor: BTreeMap<String, u64>,

    outstanding_fetches: u64,
    saw_endlist: bool,
    end_emitted: bool,
    last_full_playlist: Option<MediaPlaylist>,
    /// `EXT-X-MEDIA-SEQUENCE` of the previous playlist: RFC 8216 §6.2.2
    /// forbids it ever decreasing, so a smaller one is an origin restart
    /// (see [`Self::restart_after_regression`]).
    last_media_sequence: Option<broadcast_hls::MediaSequence>,
    /// The `EXTINF` duration each segment declared, so the media its bytes
    /// actually carry can be compared with it once demuxed.
    segment_durations: BTreeMap<broadcast_hls::MediaSequence, DecimalSeconds>,
    /// The media span accumulated per (segment, track) while that segment's
    /// bytes are processed.
    duration_spans: BTreeMap<(broadcast_hls::MediaSequence, u32), (Timescale, i64, i64)>,
    /// The segment whose bytes are being processed right now, so a drain can
    /// attribute the samples it sees to it.
    draining_msn: Option<broadcast_hls::MediaSequence>,
}

/// Choose a variant index according to the policy: the highest-bandwidth variant the policy
/// allows, else the lowest-bandwidth one when it allows none; `0` for an empty list.
#[must_use]
pub fn choose_variant(variants: &[VariantInfo], policy: &VariantPolicy) -> usize {
    if variants.is_empty() {
        return 0;
    }
    let mut candidates: Vec<usize> = Vec::new();
    for (i, v) in variants.iter().enumerate() {
        let mut ok = true;
        if let Some(max_bw) = policy.max_bandwidth
            && v.bandwidth > u64::from(max_bw)
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

/// What an [`HlsClient`] is configured by: the playlist it follows and how it
/// chooses variants.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HlsClientConfig {
    /// The Media (or Master) Playlist URL to follow.
    pub playlist_url: Url,
    /// How a Master Playlist's variant is chosen.
    pub variant_policy: VariantPolicy,
    /// The URL relative playlist URIs resolve against until the first playlist
    /// response says otherwise; `None` resolves against `playlist_url`.
    pub playlist_base: Option<Url>,
    /// Hostile-input bounds: the playlist byte size and segment count the
    /// parser accepts, and the largest resource body [`HlsClient::on_resource`]
    /// accepts (audit 20 finding 41, issue #2852).
    pub limits: Limits,
    /// Caller-supplied ceiling on one PID's in-progress PES buffer in the
    /// classic-TS demuxer; `None` (the default) is unlimited. A caller whose
    /// segments arrive over a network supplies one.
    pub max_pes_unit_bytes: Option<NonZeroUsize>,
    /// What each damage report means: `Report` (the default for everything)
    /// publishes it as [`Output::Damage`] and keeps playing; `Fail` stops the
    /// client at the report and returns [`crate::client::Error::Damage`]
    /// carrying it.
    pub damage_policy: broadcast_core::damage::DamagePolicy,
    /// How far a segment's demuxed media may differ from its `EXTINF` before
    /// it is reported ([`Output::Damage`], `MediaUnitDamaged`): `None` (the
    /// default) never compares, so no number is chosen by the library. A
    /// caller sets a tolerance it can justify for its own content; one frame
    /// of a 30 fps segment is `Duration::from_millis(34)`.
    pub segment_duration_tolerance: Option<core::time::Duration>,
}

impl HlsClientConfig {
    /// A config following `playlist_url` with the default [`VariantPolicy`].
    pub fn new(playlist_url: Url) -> Self {
        HlsClientConfig {
            playlist_url,
            variant_policy: VariantPolicy::default(),
            playlist_base: None,
            limits: Limits::default(),
            max_pes_unit_bytes: None,
            damage_policy: broadcast_core::damage::DamagePolicy::new(),
            segment_duration_tolerance: None,
        }
    }

    /// Replace [`HlsClientConfig::limits`].
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Replace [`HlsClientConfig::max_pes_unit_bytes`].
    #[must_use]
    pub fn with_max_pes_unit_bytes(mut self, max_pes_unit_bytes: Option<NonZeroUsize>) -> Self {
        self.max_pes_unit_bytes = max_pes_unit_bytes;
        self
    }

    /// Replace [`HlsClientConfig::variant_policy`].
    #[must_use]
    pub fn with_variant_policy(mut self, variant_policy: VariantPolicy) -> Self {
        self.variant_policy = variant_policy;
        self
    }

    /// Set what each damage report means (see [`HlsClientConfig::damage_policy`]).
    #[must_use]
    pub fn with_damage_policy(
        mut self,
        damage_policy: broadcast_core::damage::DamagePolicy,
    ) -> Self {
        self.damage_policy = damage_policy;
        self
    }

    /// Set [`HlsClientConfig::segment_duration_tolerance`].
    #[must_use]
    pub fn with_segment_duration_tolerance(
        mut self,
        segment_duration_tolerance: Option<core::time::Duration>,
    ) -> Self {
        self.segment_duration_tolerance = segment_duration_tolerance;
        self
    }

    /// Replace [`HlsClientConfig::playlist_base`].
    #[must_use]
    pub fn with_playlist_base(mut self, playlist_base: Url) -> Self {
        self.playlist_base = Some(playlist_base);
        self
    }
}

impl HlsClient {
    /// Create a new client from `config`, seeding the first
    /// [`Action::FetchPlaylist`] (a plain, non-blocking GET — the client does
    /// not yet know whether the origin supports blocking reload).
    pub fn new(config: HlsClientConfig) -> Self {
        let playlist_url = config.playlist_url;
        let mut pending_actions = VecDeque::new();
        pending_actions.push_back(Action::FetchPlaylist(super::action::PlaylistRequest {
            url: playlist_url.clone(),
            blocking: None,
            skip: false,
        }));
        Self {
            playlist_url,
            base_override: config.playlist_base,
            variant_policy: config.variant_policy,
            limits: config.limits,
            max_pes_unit_bytes: config.max_pes_unit_bytes,
            damage_policy: config.damage_policy,
            segment_duration_tolerance: config.segment_duration_tolerance,
            reported_diagnostics: BTreeSet::new(),
            master: None,
            pending_actions,
            pending_errors: VecDeque::new(),
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
            segment_durations: BTreeMap::new(),
            duration_spans: BTreeMap::new(),
            draining_msn: None,
        }
    }

    /// The Media Playlist URL this client is following.
    pub fn playlist_url(&self) -> &Url {
        &self.playlist_url
    }

    /// Set the URL relative playlist URIs resolve against: the final URL of the next playlist
    /// response after redirects (RFC 3986 section 5.1.3). Reloads keep requesting
    /// [`HlsClient::playlist_url`]; the base is cleared when the client switches to another
    /// playlist URL. A pull client that follows redirects uses this per response (the config's
    /// [`HlsClientConfig::playlist_base`] is only the initial value).
    pub fn set_playlist_base(&mut self, base: Url) {
        self.base_override = Some(base);
    }

    fn base(&self) -> &Url {
        self.base_override.as_ref().unwrap_or(&self.playlist_url)
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
            .and_then(|v| v.uri.clone())
            .ok_or(Error::NoSuchVariant { index })?;
        self.restart_after_regression()?;
        self.pending_actions.clear();
        self.playlist_url = url.clone();
        self.base_override = None;
        self.pending_actions
            .push_back(Action::FetchPlaylist(super::action::PlaylistRequest {
                url,
                blocking: None,
                skip: false,
            }));
        if let Some(m) = &mut self.master {
            m.chosen = index;
        }
        Ok(())
    }

    /// Drain the next IO [`Action`] the caller must perform, if any.
    pub fn poll(&mut self) -> Option<Action> {
        self.wait_anchor = None;
        self.pending_actions.pop_front()
    }

    /// The `Wait` hint at the front of the action queue, as a `Duration`;
    /// `None` when the next queued action is a fetch (or nothing is queued). A
    /// wait is queued BEHIND the fetches `on_playlist` queued with it, so it is
    /// reported only once those have been drained with [`Self::poll`].
    /// Peeking never consumes it, and the core never reads a clock.
    pub fn next_wait(&self) -> Option<core::time::Duration> {
        match self.pending_actions.front() {
            Some(Action::Wait(wait)) => Some(*wait),
            _ => None,
        }
    }

    /// The absolute deadline of the queued `Wait`, on the caller's own clock
    /// (a [`Timestamp`], the same one every fetch callback takes): `None`
    /// unless a wait is at the front of the queue (see [`Self::next_wait`]).
    ///
    /// The deadline is anchored the first time this is called for a given wait
    /// (`now` + the wait) and every later call returns that SAME instant, until
    /// the wait is drained with [`Self::poll`]. Re-querying after unrelated
    /// wake-ups therefore never re-arms the timer. The core has no clock of its
    /// own, so "when the wait was scheduled" is the caller's `now` at the first
    /// query: query as soon as the wait reaches the front.
    pub fn poll_timeout(&mut self, now: Timestamp) -> Option<Timestamp> {
        let wait = self.next_wait()?;
        let anchor = *self.wait_anchor.get_or_insert(now);
        Some(anchor.saturating_add(wait))
    }

    /// Hand one report to the caller's policy: publish it, or stop feeding
    /// with the same report inside [`Error::Damage`].
    fn publish_damage(&mut self, damage: broadcast_core::damage::Damage) -> Result<()> {
        match self.damage_policy.admit(damage) {
            Ok(damage) => {
                self.pending_outputs.push_back(Output::Damage(damage));
                Ok(())
            }
            Err(error) => Err(Error::Damage(error)),
        }
    }

    /// Report each playlist diagnostic the parser recorded, once per
    /// (kind, line text): a live reload of the same malformed tag is one
    /// report, not one per reload.
    ///
    /// The severity is the cost to the viewer: a dangling segment tag drops
    /// the segment it would have described, while a bad `PROGRAM-DATE-TIME`
    /// or a `DATERANGE` without `START-DATE` only loses metadata.
    fn report_playlist_diagnostics(&mut self, playlist: &MediaPlaylist) -> Result<()> {
        for diagnostic in &playlist.diagnostics {
            let (severity, what) = match diagnostic.kind {
                ParseDiagnosticKind::DanglingSegmentTags => (
                    broadcast_core::damage::Severity::Degraded,
                    "HLS segment tag",
                ),
                ParseDiagnosticKind::InvalidProgramDateTime => (
                    broadcast_core::damage::Severity::Informational,
                    "HLS PROGRAM-DATE-TIME",
                ),
                ParseDiagnosticKind::DateRangeWithoutStartDate => (
                    broadcast_core::damage::Severity::Informational,
                    "HLS date range",
                ),
                // A diagnostic a newer parser adds: it recorded a defect it
                // could not apply, so it is reported as metadata it skipped.
                _ => (
                    broadcast_core::damage::Severity::Informational,
                    "HLS playlist tag",
                ),
            };
            let key = alloc::format!("{}|{}", diagnostic.kind.name(), diagnostic.line);
            if !self.reported_diagnostics.insert(key) {
                continue;
            }
            let damage = broadcast_core::damage::Damage::new(
                broadcast_core::damage::DamageKind::MetadataSkipped,
                severity,
                what,
            );
            self.publish_damage(damage)?;
        }
        Ok(())
    }

    /// Report the segments a forward-continuing reload jumped over: the new
    /// window's first Media Sequence Number is later than the one after the
    /// previous window's last, and nothing ever requests the numbers between.
    fn report_media_sequence_gap(&mut self, playlist: &MediaPlaylist) -> Result<()> {
        let Some(previous) = &self.last_full_playlist else {
            return Ok(());
        };
        let Some(expected) = previous.msn.checked_add(index_u64(previous.segments.len())) else {
            return Ok(());
        };
        if playlist.msn <= expected {
            return Ok(());
        }
        let damage = broadcast_core::damage::Damage::new(
            broadcast_core::damage::DamageKind::MediaUnitDropped,
            broadcast_core::damage::Severity::Degraded,
            "HLS media sequence",
        )
        .with_ordinal(expected.value());
        self.publish_damage(damage)
    }

    /// Record the media span this drain saw per track, so the segment's whole
    /// span can be compared with its `EXTINF` once its bytes have all been
    /// processed (a resource arrives in pieces, and the classic-TS demuxer
    /// holds one access unit back; only the accumulated span is the segment).
    fn note_durations(&mut self, samples: &BTreeMap<TrackId, Vec<Sample>>, specs: &[TrackSpec]) {
        let Some(id_msn) = self.draining_msn else {
            return;
        };
        for (track_id, list) in samples {
            let Some(spec) = specs.iter().find(|spec| spec.track_id == *track_id) else {
                continue;
            };
            for sample in list {
                let Some(timing) = sample.timing else {
                    continue;
                };
                let (Some(start), Some(duration)) = (timing.pts(), timing.duration()) else {
                    continue;
                };
                let end = start.saturating_add(i64::from(duration));
                let entry = self
                    .duration_spans
                    .entry((id_msn, track_id.value()))
                    .or_insert((spec.timescale, start, end));
                entry.1 = entry.1.min(start);
                entry.2 = entry.2.max(end);
            }
        }
    }

    /// Compare a completed segment's demuxed media span with the `EXTINF` it
    /// declared, when the caller set a tolerance.
    ///
    /// The span is the longest track's, from its first sample's presentation
    /// time to the end of its last, in that track's own timescale — a segment
    /// with audio and video is as long as the longer of the two.
    fn check_declared_duration(&mut self, id: ResourceId) -> Result<()> {
        let ResourceId::Segment { msn } = id else {
            return Ok(());
        };
        let Some(tolerance) = self.segment_duration_tolerance else {
            return Ok(());
        };
        let spans: Vec<(u32, Timescale, i64, i64)> = self
            .duration_spans
            .iter()
            .filter(|((key, _), _)| *key == msn)
            .map(|((_, track), (timescale, start, end))| (*track, *timescale, *start, *end))
            .collect();
        let stale: Vec<(broadcast_hls::MediaSequence, u32)> = self
            .duration_spans
            .keys()
            .filter(|(key, _)| *key == msn)
            .copied()
            .collect();
        for key in stale {
            self.duration_spans.remove(&key);
        }
        let Some(expected) = self.segment_durations.get(&msn).copied() else {
            return Ok(());
        };
        let mut longest: Option<(u32, Timescale, i64, i64)> = None;
        for (track, timescale, start, end) in spans {
            let span = end.saturating_sub(start);
            if longest.is_none_or(|(_, _, _, kept)| span > kept) {
                longest = Some((track, timescale, start, span));
            }
        }
        let Some((track, timescale, start, span)) = longest else {
            return Ok(());
        };
        let actual = span as f64 / f64::from(timescale.get());
        if (actual - expected.get()).abs() <= tolerance.as_secs_f64() {
            return Ok(());
        }
        let damage = broadcast_core::damage::Damage::new(
            broadcast_core::damage::DamageKind::MediaUnitDamaged,
            broadcast_core::damage::Severity::Degraded,
            "HLS segment duration",
        )
        .for_track(track)
        .at_media_time(broadcast_core::damage::MediaTimeRange::new(
            timescale,
            start,
            start.saturating_add(span),
        ))
        .with_ordinal(msn.value());
        self.publish_damage(damage)
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
            let master =
                MasterPlaylist::parse_with(text, &self.limits).map_err(Self::playlist_error)?;
            if !master.defines.is_empty() {
                return Err(Error::UnsupportedDefine);
            }
            let base = self.base().clone();
            let mut variants: Vec<VariantInfo> = Vec::new();
            for v in &master.variants {
                let Some(resolved) = self.fetch_url(&base, &v.uri) else {
                    continue;
                };
                variants.push(VariantInfo {
                    uri: Url::parse(&resolved).ok(),
                    bandwidth: u64::from(v.bandwidth),
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
                    url: match m.uri.as_deref() {
                        Some(u) => self.fetch_url(&base, u),
                        None => None,
                    },
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
                .and_then(|v| v.uri.clone())
            else {
                return Err(Error::NoSuchVariant { index: chosen });
            };
            self.playlist_url = chosen_url.clone();
            self.base_override = None;
            self.pending_actions.clear();
            self.pending_actions
                .push_back(Action::FetchPlaylist(super::action::PlaylistRequest {
                    url: chosen_url,
                    blocking: None,
                    skip: false,
                }));
            return Ok(());
        }
        let playlist =
            MediaPlaylist::parse_with(text, &self.limits).map_err(Self::playlist_error)?;
        if !playlist.defines.is_empty() {
            return Err(Error::UnsupportedDefine);
        }
        let playlist = self.merge_delta(playlist);
        if playlist.skip.is_some() {
            // The delta could not be spliced into the view this client holds:
            // the reload it stands for is lost, a full playlist is requested
            // instead, and the caller is told what the fallback cost.
            let damage = broadcast_core::damage::Damage::new(
                broadcast_core::damage::DamageKind::MetadataSkipped,
                broadcast_core::damage::Severity::Degraded,
                "HLS delta update",
            )
            .with_ordinal(playlist.msn.value());
            self.publish_damage(damage)?;
            return Err(self.request_full_playlist(&playlist));
        }
        self.full_reload_requested = false;
        self.report_playlist_diagnostics(&playlist)?;
        let segment_keys = playlist.segment_key_results()?;
        if let Some(Err(e)) = segment_keys.first()
            && segment_keys.iter().all(|r| r.is_err())
        {
            return Err(Error::PlaylistParse(e.clone()));
        }
        let open_key = if playlist.open_segment.is_some() {
            playlist.open_segment_key()?
        } else {
            None
        };
        // `EXT-X-MEDIA-SEQUENCE` is an unbounded remote `u64`: every MSN
        // derived below is `media_sequence + index` with `index <
        // segments.len()`, so proving the end fits proves them all. Computed
        // before any state is mutated, so a hostile sequence number cannot
        // poison the high-water mark or drop key material before rejection.
        let next_msn = playlist
            .msn
            .checked_add(index_u64(playlist.segments.len()))
            .ok_or(Error::MediaSequenceOverflow {
                msn: playlist.msn,
                segments: playlist.segments.len(),
            })?;
        let mut first_error: Option<Error> = None;
        // An older view of the same stream must never make the client deliver
        // backwards: its segments below the previous window are skipped.
        let mut lag_floor: Option<broadcast_hls::MediaSequence> = None;
        if self.last_media_sequence.is_some() {
            match self.continuity(&playlist) {
                Continuity::Restart => self.restart_after_regression()?,
                Continuity::OlderWindow => {
                    lag_floor = self.last_full_playlist.as_ref().map(|p| p.msn);
                }
                // A window that continues forwards but starts past the next
                // Media Sequence Number has jumped over segments: nothing
                // requests them, so they are lost and reported here.
                Continuity::Continues => self.report_media_sequence_gap(&playlist)?,
                Continuity::Lagging => {}
            }
        }
        // A lagging copy never lowers the high-water mark.
        self.last_media_sequence = Some(
            self.last_media_sequence
                .map_or(playlist.msn, |p| p.max(playlist.msn)),
        );
        self.forget_keys_below(playlist.msn);

        if !self.joined {
            self.joined = true;
            self.join_start(&playlist)?;
        }

        self.playlist_maps = self.map_entries(&playlist, text, next_msn);

        for (i, seg) in playlist.segments.iter().enumerate() {
            let msn = broadcast_hls::MediaSequence::new(playlist.msn.value() + index_u64(i));
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
            self.segment_durations.insert(msn, seg.duration);
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
                    let part_index =
                        broadcast_hls::PartIndex::new(u32::try_from(part_idx).unwrap_or(u32::MAX));
                    let id = ResourceId::Part {
                        msn: next_msn,
                        part: part_index,
                    };
                    if !self.requested.contains(&id)
                        && !self.delivered_parts.contains(&(next_msn, part_index))
                        && let Ok(url) = url_ref::resolve_http(self.base(), hint_uri)
                    {
                        let byte_range = self.resolve_hint_byte_range(url.as_str(), ll)?;
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
                        part: Some(broadcast_hls::PartIndex::new(
                            u32::try_from(part).unwrap_or(u32::MAX),
                        )),
                    }
                });
            let can_skip = playlist
                .low_latency
                .as_ref()
                .and_then(|ll| ll.can_skip_until)
                .is_some();
            let skip = can_skip && self.last_full_playlist.is_some();
            self.pending_actions
                .push_back(Action::FetchPlaylist(super::action::PlaylistRequest {
                    url: self.playlist_url.clone(),
                    blocking,
                    skip,
                }));
            if blocking.is_none() {
                self.pending_actions
                    .push_back(Action::Wait(reload_wait(playlist.target_duration)));
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
                .push_back(Action::Wait(reload_wait(delta.target_duration)));
        }
        self.full_reload_requested = true;
        self.pending_actions
            .push_back(Action::FetchPlaylist(super::action::PlaylistRequest {
                url: self.playlist_url.clone(),
                blocking: None,
                skip: false,
            }));
        Error::DeltaNotMergeable {
            msn: delta.msn,
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
            .any(|a| matches!(a, Action::FetchPlaylist(_)))
        {
            return false;
        }
        self.pending_actions
            .push_back(Action::FetchPlaylist(super::action::PlaylistRequest {
                url: self.playlist_url.clone(),
                blocking: None,
                skip: false,
            }));
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
    ///
    /// The loss is published as [`Output::Damage`]
    /// ([`broadcast_core::damage::DamageKind::MediaUnitDropped`]) because the
    /// content that resource carried never reaches the player; the cause is
    /// the caller's [`SkipReason`].
    ///
    /// # Errors
    /// [`Error::Damage`] when the caller's policy says this report means stop.
    pub fn skip_resource(&mut self, id: ResourceId) -> Result<()> {
        self.skip_resource_due_to(id, SkipReason::Unusable)
    }

    /// [`Self::skip_resource`] with the cause named, so the report says why
    /// the segment or part was lost (a 404, a 410, a timeout).
    ///
    /// # Errors
    /// [`Error::Damage`] when the caller's
    /// [`damage_policy`](HlsClientConfig::damage_policy) says this report
    /// means stop.
    pub fn skip_resource_due_to(&mut self, id: ResourceId, reason: SkipReason) -> Result<()> {
        if !matches!(id, ResourceId::Init) {
            let (what, msn) = match id {
                ResourceId::Segment { msn } => (reason.segment(), msn),
                ResourceId::Part { msn, .. } => (reason.part(), msn),
                ResourceId::Init => return Ok(()),
            };
            let damage = broadcast_core::damage::Damage::new(
                broadcast_core::damage::DamageKind::MediaUnitDropped,
                broadcast_core::damage::Severity::Degraded,
                what,
            )
            .with_ordinal(msn.value());
            self.publish_damage(damage)?;
        }
        self.forget_resource(id);
        Ok(())
    }

    /// Mark `id` delivered and restart the timeline across the gap it leaves.
    fn forget_resource(&mut self, id: ResourceId) {
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
    fn map_entries(
        &self,
        playlist: &MediaPlaylist,
        text: &str,
        next_msn: broadcast_hls::MediaSequence,
    ) -> Vec<MapEntry> {
        let keyed = keys_at_maps(text);
        let mut entries: Vec<MapEntry> = Vec::new();
        let listed = playlist
            .segments
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.map.as_ref().map(|m| {
                    (
                        m,
                        broadcast_hls::MediaSequence::new(playlist.msn.value() + index_u64(i)),
                    )
                })
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
                    .resolved_url(self.base(), &map.uri)
                    .ok()
                    .and_then(|url| self.init_slot_for(&url, &map.byte_range))
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
    fn init_slot_for(&self, url: &Url, byte_range: &Option<ByteRange>) -> Option<InitId> {
        self.inits
            .iter()
            .find(|(_, slot)| {
                without_query(slot.url.as_str()) == without_query(url.as_str())
                    && slot.map.byte_range == *byte_range
            })
            .map(|(id, _)| *id)
    }

    /// The init slot for `map`, created (and queued for fetching) when the
    /// client has none for that resource yet; a slot whose fetch failed is
    /// queued again.
    fn init_for_map(&mut self, map: &MapTag) -> Result<InitId> {
        let url = self.resolved_url(self.base(), &map.uri)?;
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
            .map_or((None, broadcast_hls::MediaSequence::ZERO), |e| {
                (e.encryption.clone(), e.first_msn)
            });
        let byte_range = self.resolve_byte_range(url.as_str(), &map.byte_range)?;
        let crypto = match key.as_ref().and_then(|k| k.uri.as_deref().map(|u| (k, u))) {
            Some((key, uri)) => {
                let key_id = self.ensure_key_requested(uri)?;
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
                demux: None,
                init_events: Vec::new(),
                demux_error: None,
                specs: Vec::new(),
                tracks_known: false,
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
    /// applies to it (RFC 8216bis §4.4.4.5), creating that init on first use.
    fn bind_init(&mut self, msn: broadcast_hls::MediaSequence, map: Option<&MapTag>) -> Result<()> {
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
    /// [`Error::Demux`] if the streaming MP4 stage refuses `bytes` (a damaged
    /// body is reported through [`Output::FragmentDamaged`], not this error).
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
        if bytes.len() > self.limits.max_segment_bytes {
            self.requested.remove(&id);
            if let Some(slot) = self.init_target.and_then(|t| self.inits.get_mut(&t)) {
                slot.fetch = InitFetch::Failed;
            }
            return Err(Error::ResourceTooLarge {
                id,
                len: bytes.len(),
                max: self.limits.max_segment_bytes,
            });
        }
        if id != ResourceId::Init {
            self.fulfilled.insert(id);
        }
        match id {
            ResourceId::Init => self.accept_init(bytes)?,
            ResourceId::Part { .. } | ResourceId::Segment { .. } => {
                let processed = self.accept_media_resource(id, bytes);
                match processed {
                    Ok(()) => {}
                    Err(e @ Error::PendingKeyedOverflow { .. })
                    | Err(e @ Error::PendingDemuxOverflow { .. }) => return Err(e),
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
        if slot.crypto.is_none()
            && isobmff::box_types::find_box(bytes, &isobmff::init_segment::MOOV).is_none()
        {
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
        self.install_demux(target, plain);
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
            Ok(plain)
                if isobmff::box_types::find_box(&plain, &isobmff::init_segment::MOOV).is_none() =>
            {
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
        // Samples this resource's bytes produce are attributed to it; a
        // segment is only complete once all of them have been seen, which is
        // where its declared `EXTINF` is checked.
        let previous = core::mem::replace(&mut self.draining_msn, resource_msn(&id));
        let outcome = if let ResourceId::Segment { msn } = id
            && let Some(crypto) = self.segment_crypto.get(&msn).copied()
        {
            match self.key_material.get(&crypto.key_id).copied() {
                Some(KeyBytes(key)) => {
                    let plain = decrypt_segment(id, crypto, &key, bytes)?;
                    self.route_media_resource(id, &plain)
                }
                None => self.buffer_until_keyed(id, bytes),
            }
        } else {
            self.route_media_resource(id, bytes)
        };
        self.draining_msn = previous;
        outcome?;
        self.check_declared_duration(id)
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
            self.buffer_until_init(id, bytes)
        } else {
            self.finish_media_resource(id, bytes)
        }
    }

    /// Hold a Part/Segment body until its init becomes ready, within
    /// [`MAX_PENDING_DEMUX_BYTES`]: the oldest waiting bodies are dropped (and
    /// become requestable again) to make room, reported as
    /// [`Error::PendingDemuxOverflow`].
    fn buffer_until_init(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_PENDING_DEMUX_BYTES {
            self.fulfilled.remove(&id);
            self.requested.remove(&id);
            return Err(Error::PendingDemuxOverflow { dropped: id });
        }
        let mut held: usize = self.pending_demux.iter().map(|(_, b)| b.len()).sum();
        let mut dropped: Option<ResourceId> = None;
        while held.saturating_add(bytes.len()) > MAX_PENDING_DEMUX_BYTES {
            let Some((old_id, old)) = self.pending_demux.pop_front() else {
                break;
            };
            held = held.saturating_sub(old.len());
            self.fulfilled.remove(&old_id);
            self.requested.remove(&old_id);
            dropped.get_or_insert(old_id);
        }
        self.pending_demux.push_back((id, bytes.to_vec()));
        match dropped {
            Some(dropped) => Err(Error::PendingDemuxOverflow { dropped }),
            None => Ok(()),
        }
    }

    /// Feed the key file fetched for an [`Action::FetchKey`]. Encrypted
    /// segments that arrived earlier are decrypted and demuxed in Media
    /// Sequence Number order.
    ///
    /// # Errors
    /// [`Error::UnrequestedKey`] for an unknown id or a key already held;
    /// [`Error::BadKeyLength`] unless `bytes` is exactly 16 octets;
    /// [`Error::DecryptFailed`] if a waiting segment does not decrypt, and
    /// [`Error::InitUndecodable`]/[`Error::DecryptFailed`] for a waiting init
    /// that does not decode; the init's failure never stops the queued
    /// segments from being decrypted, they wait for the refetched init.
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
        let mut init_failure = None;
        for (init, crypto) in waiting_inits {
            let cipher = self
                .inits
                .get_mut(&init)
                .and_then(|slot| slot.cipher.take());
            if let Some(cipher) = cipher
                && let Err(e) = self.complete_encrypted_init(init, crypto, &key, &cipher)
            {
                init_failure.get_or_insert(e);
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
                return Err(init_failure.unwrap_or(e));
            }
        }
        self.maybe_emit_end_of_stream()?;
        init_failure.map_or(Ok(()), Err)
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
    fn ensure_key_requested(&mut self, uri: &str) -> Result<KeyId> {
        let url = self.resolved_url(self.base(), uri)?;
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
        Ok(id)
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
    fn forget_keys_below(&mut self, first_msn: broadcast_hls::MediaSequence) {
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
        msn: broadcast_hls::MediaSequence,
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
        let id = self.ensure_key_requested(uri)?;
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
    /// (`ftyp`/`styp`/`moof`), never [`TS_SYNC_BYTE`] — the classic TS
    /// form has no `EXT-X-MAP` at all.
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

    /// Classify a `broadcast_hls` parse failure: a bound the playlist broke is
    /// permanent ([`Error::PlaylistTooLarge`]), never a retryable parse error.
    fn playlist_error(error: broadcast_hls::Error) -> Error {
        match error {
            broadcast_hls::Error::TooLarge { what, len, max } => {
                Error::PlaylistTooLarge { what, len, max }
            }
            other => Error::PlaylistParse(other),
        }
    }

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
        if playlist.msn < prev.msn {
            return playlist;
        }
        let Ok(prefix_start) = usize::try_from(playlist.msn.value() - prev.msn.value()) else {
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
        msn: broadcast_hls::MediaSequence,
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
            // second time. Caught by `multimux/tests/it/glass_to_glass.rs`
            // (issue #717 slice 5): every sample was double-delivered for
            // the first two segments of a real, live-paced run.
            if self.has_delivered_part(msn) {
                self.begin_ts_resource(ResourceId::Segment { msn }, msn)?;
                self.retry_undelivered_parts(msn);
                if self.undelivered_parts(msn).is_empty() {
                    self.delivered_segments.insert(msn);
                }
                return Ok(());
            }
            let id = ResourceId::Segment { msn };
            if !self.requested.contains(&id) {
                let Some(url) = self.segment_url(msn, &seg.uri) else {
                    return Ok(());
                };
                self.bind_init(msn, seg.map.as_ref())?;
                let byte_range = self.request_byte_range(id, url.as_str(), &seg.byte_range)?;
                self.request_resource(id, url, byte_range);
            }
            return Ok(());
        }

        let mut fully_accounted = true;
        for (i, part) in seg.parts.iter().enumerate() {
            let part_index = broadcast_hls::PartIndex::new(u32::try_from(i).unwrap_or(u32::MAX));
            if part.gap || self.delivered_parts.contains(&(msn, part_index)) {
                continue;
            }
            fully_accounted = false;
            let id = ResourceId::Part {
                msn,
                part: part_index,
            };
            if !self.requested.contains(&id) {
                let Some(url) = self.segment_url(msn, &part.uri) else {
                    return Ok(());
                };
                self.bind_init(msn, seg.map.as_ref())?;
                let byte_range = self.request_byte_range(id, url.as_str(), &part.byte_range)?;
                self.request_resource(id, url, byte_range);
            }
        }
        if fully_accounted {
            self.delivered_segments.insert(msn);
        }
        Ok(())
    }

    fn undelivered_parts(&self, msn: broadcast_hls::MediaSequence) -> Vec<ResourceId> {
        self.resource_requests
            .range(
                ResourceId::Part {
                    msn,
                    part: broadcast_hls::PartIndex::ZERO,
                }..=ResourceId::Part {
                    msn,
                    part: broadcast_hls::PartIndex::MAX,
                },
            )
            .map(|(id, _)| *id)
            .filter(|id| match id {
                ResourceId::Part { part, .. } => !self.delivered_parts.contains(&(msn, *part)),
                _ => false,
            })
            .collect()
    }

    fn retry_undelivered_parts(&mut self, msn: broadcast_hls::MediaSequence) {
        for id in self.undelivered_parts(msn) {
            self.retry_resource(id);
        }
    }

    fn has_delivered_part(&self, msn: broadcast_hls::MediaSequence) -> bool {
        self.delivered_parts
            .range((msn, broadcast_hls::PartIndex::ZERO)..=(msn, broadcast_hls::PartIndex::MAX))
            .next()
            .is_some()
    }

    fn covered_by_parts(&self, msn: broadcast_hls::MediaSequence) -> bool {
        self.has_delivered_part(msn) && self.undelivered_parts(msn).is_empty()
    }

    fn process_open_segment(
        &mut self,
        msn: broadcast_hls::MediaSequence,
        open: &OpenSegment,
    ) -> Result<()> {
        for (i, part) in open.parts.iter().enumerate() {
            let part_index = broadcast_hls::PartIndex::new(u32::try_from(i).unwrap_or(u32::MAX));
            if part.gap || self.delivered_parts.contains(&(msn, part_index)) {
                continue;
            }
            let id = ResourceId::Part {
                msn,
                part: part_index,
            };
            if !self.requested.contains(&id) {
                let Some(url) = self.segment_url(msn, &part.uri) else {
                    return Ok(());
                };
                self.bind_init(msn, open.map.as_ref())?;
                let byte_range = self.request_byte_range(id, url.as_str(), &part.byte_range)?;
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

    /// The URL `uri` resolves to against `base`, or `None` (with the typed
    /// [`Error::UnresolvableReference`] pushed as this client's pending error)
    /// when the guarded resolver in `broadcast_common::url_ref` refuses it:
    /// a reference carrying an ASCII control character is never sanitised into
    /// a different URL and never requested (issue #2458).
    fn fetch_url(&mut self, base: &Url, uri: &str) -> Option<String> {
        match self.resolved_url(base, uri) {
            Ok(url) => Some(url.to_string()),
            Err(report) => {
                self.push_pending_error(report);
                None
            }
        }
    }

    /// The resolved URL, or the [`Error::UnresolvableReference`] to report for
    /// it — without reporting it, so a caller that fails the whole playlist
    /// (an unusable init) can return the error directly.
    fn resolved_url(&self, base: &Url, uri: &str) -> core::result::Result<Url, Error> {
        url_ref::resolve_http(base, uri).map_err(|source| Error::UnresolvableReference {
            uri: uri.to_string(),
            source,
        })
    }

    /// The URL segment URI `uri` (with its query, which the byte-range cursor
    /// keeps) resolves to, or `None` (with the report queued) when the guarded
    /// resolver refuses it: the segment stays unrequested instead of being
    /// silently renamed (issue #2458).
    fn segment_url(&mut self, msn: broadcast_hls::MediaSequence, uri: &str) -> Option<Url> {
        match url_ref::resolve_http(self.base(), uri) {
            Ok(url) => Some(url),
            Err(source) => {
                self.report_unresolvable(msn, uri, source);
                None
            }
        }
    }

    /// Report an unresolvable reference naming segment `msn`, and drop the
    /// segment's decryption parameters so a key it would have fetched is not
    /// left stranded.
    fn report_unresolvable(
        &mut self,
        msn: broadcast_hls::MediaSequence,
        uri: &str,
        source: broadcast_common::url_ref::Error,
    ) {
        self.segment_crypto.remove(&msn);
        self.push_pending_error(Error::UnresolvableReference {
            uri: uri.to_string(),
            source,
        });
    }

    /// Queue an error for [`Self::next_error`] within [`MAX_PENDING_ERRORS`].
    /// Once the real slots are full the last one becomes a counted
    /// [`Error::UnresolvableReferences`] and every further report increments
    /// it, so the queue is bounded whether or not the host drains it.
    fn push_pending_error(&mut self, error: Error) {
        if self.pending_errors.len() < MAX_PENDING_ERRORS - 1 {
            self.pending_errors.push_back(error);
            return;
        }
        match self.pending_errors.back_mut() {
            Some(Error::UnresolvableReferences { count }) => *count += 1,
            _ => self
                .pending_errors
                .push_back(Error::UnresolvableReferences { count: 1 }),
        }
    }

    /// The errors [`Self::poll`] and [`Self::next_output`] raise on the
    /// caller's behalf: every playlist reference the guarded resolver refused
    /// (issue #2458), in playlist order. Reported once, in order, and never
    /// turned into a fetch.
    pub fn next_error(&mut self) -> Option<Error> {
        self.pending_errors.pop_front()
    }

    fn request_resource(&mut self, id: ResourceId, url: Url, byte_range: Option<ByteSpan>) {
        self.requested.insert(id);
        self.resource_requests.insert(id, (url.clone(), byte_range));
        self.outstanding_fetches += 1;
        self.pending_actions.push_back(Action::FetchResource {
            id,
            url,
            byte_range,
        });
    }

    /// The range to fetch for `id`: the one already resolved when it was first
    /// requested, so a re-request after a failure cannot continue from a
    /// cursor the first request advanced.
    fn request_byte_range(
        &mut self,
        id: ResourceId,
        url: &str,
        br: &Option<ByteRange>,
    ) -> Result<Option<ByteSpan>> {
        if let Some((_, stored)) = self.resource_requests.get(&id) {
            return Ok(*stored);
        }
        self.resolve_byte_range(url, br)
    }

    /// Resolve a `PartSpec`/`MediaSegment`/`MapTag` `BYTERANGE` into an
    /// absolute [`ByteSpan`] (`offset`, `length`), honouring the "omitted
    /// offset continues the previous sub-range of the same resource" rule
    /// (tracked per resolved URL).
    ///
    /// # Errors
    /// [`Error::ByteRangeOverflow`] if `offset + length` (both taken
    /// straight from the untrusted remote playlist) overflows `u64` — see
    /// that variant's doc for why this is rejected rather than saturated.
    fn resolve_byte_range(
        &mut self,
        url: &str,
        br: &Option<ByteRange>,
    ) -> Result<Option<ByteSpan>> {
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
        match core::num::NonZeroU64::new(br.length) {
            Some(len) => Ok(Some(ByteSpan::new(offset, len))),
            None => Ok(None),
        }
    }

    /// Same overflow contract as [`Self::resolve_byte_range`] — see
    /// [`Error::ByteRangeOverflow`].
    fn resolve_hint_byte_range(
        &mut self,
        url: &str,
        ll: &broadcast_hls::LowLatencyConfig,
    ) -> Result<Option<ByteSpan>> {
        let Some(length) = ll.preload_hint_byte_range_length else {
            return Ok(None);
        };
        let br = ByteRange {
            length,
            offset: ll.preload_hint_byte_range_start,
        };
        self.resolve_byte_range(url, &Some(br))
    }

    /// Start a fresh streaming MP4 stage on `plain` for `slot`, replaying the
    /// track announcements a re-read of the init makes.
    fn install_demux(&mut self, slot: InitId, plain: &[u8]) {
        let Some(state) = self.inits.get_mut(&slot) else {
            return;
        };
        state.demux = None;
        state.demux_error = None;
        state.init_events.clear();
        match start_demux(plain) {
            Ok(mut demux) => {
                while let Some(event) = demux.poll() {
                    if !matches!(event, DemuxEvent::InputDegraded { .. }) {
                        state.init_events.push(event);
                    }
                }
                state.demux = Some(Mp4DemuxState(demux));
            }
            Err(err) => state.demux_error = Some(err),
        }
    }

    /// Replace the stage of `slot` with a fresh one over its held init, so a
    /// segment that could not be demuxed costs only itself.
    fn restart_demux(&mut self, slot: InitId) {
        let Some(plain) = self.inits.get(&slot).and_then(|s| s.bytes.clone()) else {
            return;
        };
        self.install_demux(slot, &plain);
    }

    /// Feed one Part/Segment body to `slot`'s stage and close the unit: the
    /// body goes in [`DEMUX_FEED_BYTES`] pieces, so the stage's buffer holds a
    /// piece plus the one fragment it is resolving, never the whole body.
    fn feed_segment(
        &mut self,
        slot: InitId,
        bytes: &[u8],
    ) -> core::result::Result<(), transmux::Error> {
        let Some(state) = self.inits.get_mut(&slot) else {
            return Err(transmux::Error::InvalidInput("no init slot for the stage"));
        };
        if let Some(source) = state.demux_error.clone() {
            return Err(source);
        }
        let Some(demux) = state.demux.as_mut() else {
            return Err(transmux::Error::InvalidInput(
                "no streaming MP4 stage for the init",
            ));
        };
        for chunk in bytes.chunks(DEMUX_FEED_BYTES) {
            demux.0.feed(Mp4Input::Next(chunk), Timestamp::ZERO)?;
        }
        demux.0.feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
    }

    fn demux_and_emit(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        let slot = self.init_of(&id).filter(|s| self.init_ready(Some(*s)));
        let Some(slot) = slot else {
            return Err(Error::InitNotYetAvailable { id });
        };
        self.announce_init(slot);
        if segment::progressive_movie(bytes) {
            return Err(Error::NotFragmented { id });
        }
        let fed = self.feed_segment(slot, bytes);
        if let Err(source) = fed {
            self.restart_demux(slot);
            return Err(Error::Demux { id, source });
        }
        let (specs, damaged, abandoned, samples, damage_reports) = {
            let state = self
                .inits
                .get_mut(&slot)
                .ok_or(Error::InitNotYetAvailable { id })?;
            let mut events = core::mem::take(&mut state.init_events);
            if let Some(demux) = state.demux.as_mut() {
                while let Some(event) = demux.0.poll() {
                    events.push(event);
                }
            }
            let mut samples: BTreeMap<TrackId, Vec<Sample>> = BTreeMap::new();
            let mut damaged = 0usize;
            let mut damage_reports: Vec<broadcast_core::damage::Damage> = Vec::new();
            let mut abandoned: Vec<(Option<TrackId>, AbandonReason)> = Vec::new();
            for event in events {
                match event {
                    DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                        state.tracks_known = true;
                        if let Some(reason) = encrypted_data_track(&spec) {
                            state
                                .specs
                                .retain(|existing| existing.track_id != spec.track_id);
                            abandoned.push((Some(spec.track_id), reason));
                            continue;
                        }
                        match state
                            .specs
                            .iter_mut()
                            .find(|existing| existing.track_id == spec.track_id)
                        {
                            Some(existing) => *existing = spec,
                            None => state.specs.push(spec),
                        }
                    }
                    DemuxEvent::TrackAbandoned {
                        track_id, reason, ..
                    } => {
                        state.tracks_known = true;
                        abandoned.push((track_id, reason));
                    }
                    DemuxEvent::Sample {
                        track_id, sample, ..
                    } => {
                        samples.entry(track_id).or_default().push(sample);
                    }
                    DemuxEvent::InputDegraded { kind, .. } => {
                        if matches!(
                            kind,
                            InputDegradation::MalformedUnit { .. }
                                | InputDegradation::TruncatedUnit { .. }
                        ) {
                            damaged = damaged.saturating_add(1);
                        }
                    }
                    // The one report the reader publishes, forwarded as it
                    // arrived so its location, track and cause survive.
                    DemuxEvent::Damage(damage) => damage_reports.push(damage),
                    _ => {}
                }
            }
            let mut specs = state.specs.clone();
            specs.sort_by_key(|spec| spec.track_id);
            samples.retain(|track_id, _| specs.iter().any(|spec| spec.track_id == *track_id));
            (specs, damaged, abandoned, samples, damage_reports)
        };
        self.note_durations(&samples, &specs);
        for damage in damage_reports {
            self.publish_damage(damage)?;
        }
        if damaged > 0 {
            self.pending_outputs.push_back(Output::FragmentDamaged {
                id,
                skipped: damaged,
            });
        }
        for (track_id, reason) in abandoned {
            let (fourcc, reason) = abandoned_track(reason);
            let key = (track_id.map(TrackId::value), fourcc.clone());
            let first = self
                .inits
                .get_mut(&slot)
                .is_some_and(|s| s.skipped_reported.insert(key));
            if first {
                self.pending_outputs.push_back(Output::TrackSkipped {
                    track_id,
                    fourcc,
                    reason,
                });
            }
        }
        if self.inits.get(&slot).is_some_and(|s| s.tracks_known)
            && self.last_fmp4_specs.as_ref() != Some(&specs)
        {
            self.pending_outputs
                .push_back(Output::Tracks(specs.clone()));
            self.last_fmp4_specs = Some(specs);
        }
        for (track_id, samples) in samples {
            if !samples.is_empty() {
                self.pending_outputs
                    .push_back(Output::Samples { track_id, samples });
            }
        }
        Ok(())
    }

    /// Emit the pending `EXT-X-DISCONTINUITY` for `msn` and, when one fires,
    /// restart the TS demuxer's timeline: the timestamps after a
    /// discontinuity are a new timeline (RFC 8216 §4.3.2.3), so the previous
    /// 33-bit unwrap anchor must not carry across it. The old timeline's
    /// held-back final access units are flushed *before* the marker. The
    /// track set is kept ([`StreamingTsDemux::restart_timeline`]): a fresh
    /// demuxer would have to re-probe every PID from the new timeline alone
    /// and loses any PID whose parameter sets are not in it (a segment that
    /// opens mid-GOP).
    fn begin_ts_resource(
        &mut self,
        id: ResourceId,
        msn: broadcast_hls::MediaSequence,
    ) -> Result<()> {
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
        demux
            .restart_timeline()
            .map_err(|source| super::Error::Demux { id, source })?;
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
    /// byte-for-byte demuxable by `transmux::mp4_demux::Mp4Demux` like any other init
    /// segment, so callers built against the fMP4 path (e.g. `multimux`'s
    /// `HlsPull`) need no TS-specific handling. It is emitted before the
    /// first samples and again whenever the track set (a new PID, a codec
    /// change) makes the re-synthesized init differ from the last one.
    fn demux_and_emit_ts(&mut self, id: ResourceId, bytes: &[u8]) -> Result<()> {
        let max_pes_unit_bytes = self.max_pes_unit_bytes;
        let TsDemuxState(demux) = self.ts_demux.get_or_insert_with(|| {
            TsDemuxState(StreamingTsDemux::new().with_max_pes_unit_bytes(max_pes_unit_bytes))
        });
        demux
            .feed(bytes)
            .map_err(|source| super::Error::Demux { id, source })?;
        self.drain_ts_demux(id)
    }

    /// A whole media segment has been fed: a PID its PMT declares that sent
    /// nothing in it is absent, so it must not hold the rest of its program
    /// back until `EXT-X-ENDLIST` (a live playlist has none).
    fn release_ts_segment_end(&mut self, id: ResourceId) -> Result<()> {
        let Some(TsDemuxState(demux)) = self.ts_demux.as_mut() else {
            return Ok(());
        };
        demux
            .release_silent_heads()
            .map_err(|source| super::Error::Demux { id, source })?;
        self.drain_ts_demux(id)
    }

    /// Flush the TS demuxer's held-back access units (no more input on this
    /// timeline) and emit them.
    fn flush_ts_demux(&mut self, id: ResourceId) -> Result<()> {
        let Some(TsDemuxState(demux)) = self.ts_demux.as_mut() else {
            return Ok(());
        };
        demux
            .finish()
            .map_err(|source| super::Error::Demux { id, source })?;
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
        let mut damage_reports: Vec<broadcast_core::damage::Damage> = Vec::new();
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    if spec.config.is_muxable_in_bmff() {
                        self.ts_specs.insert(spec.track_id, spec);
                    } else {
                        self.ts_specs.remove(&spec.track_id);
                        let stream_type = match &spec.config {
                            transmux::CodecConfig::Data {
                                identity: transmux::ir::DataIdentity::Ts { stream_type, .. },
                            } => Some(*stream_type),
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
                DemuxEvent::InputDegraded { .. } => {}
                // The TS reader's own report: a resynchronisation, a dropped
                // PES unit, a truncated tail, a continuity gap.
                DemuxEvent::Damage(damage) => damage_reports.push(damage),
                _ => {}
            }
        }
        for damage in damage_reports {
            self.publish_damage(damage)?;
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
        {
            let specs: Vec<TrackSpec> = self.ts_specs.values().cloned().collect();
            self.note_durations(&per_track, &specs);
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
    fn emit_discontinuity_if_needed(&mut self, msn: broadcast_hls::MediaSequence) -> bool {
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
            let msn = broadcast_hls::MediaSequence::new(playlist.msn.value() + index_u64(i));
            self.skip_segment(msn, seg)?;
        }
        Ok(())
    }

    /// Mark a segment skipped at join time as delivered, advancing the
    /// byte-range cursor exactly as fetching it would have (a segment with
    /// parts is addressed by its parts' ranges, a plain one by its own).
    fn skip_segment(
        &mut self,
        msn: broadcast_hls::MediaSequence,
        seg: &MediaSegment,
    ) -> Result<()> {
        if self.delivered_segments.contains(&msn) {
            return Ok(());
        }
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
        let first = playlist.msn;
        let below = |id: &ResourceId| resource_msn(id).is_some_and(|m| m < first);
        let fulfilled = &self.fulfilled;
        self.requested
            .retain(|id| !(below(id) && fulfilled.contains(id)));
        self.resource_requests.retain(|id, _| !below(id));
        self.fulfilled.retain(|id| !below(id));
        self.pending_keyed.retain(|(id, _)| !below(id));
        self.pending_demux.retain(|(id, _)| !below(id));
        self.segment_crypto.retain(|&m, _| m >= first);
        self.delivered_parts.retain(|&(m, _)| m >= first);
        self.delivered_segments.retain(|&m| m >= first);
        self.discontinuous_msns.retain(|&m| m >= first);
        self.discontinuity_emitted.retain(|&m| m >= first);
        self.msn_init.retain(|&m, _| m >= first);
        self.segment_durations.retain(|&m, _| m >= first);
        self.duration_spans.retain(|(m, _), _| *m >= first);
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

    /// Drop every held init no segment still being fetched refers to and no
    /// map of the current playlist names, keeping the newest; then enforce
    /// [`MAX_INIT_SLOTS`], evicting the oldest slot no segment refers to (and,
    /// if every remaining slot is referred to, the oldest one anyway, so the
    /// bound always holds).
    fn prune_inits(&mut self) {
        let newest = self.inits.keys().next_back().copied();
        let bound: BTreeSet<InitId> = self.msn_init.values().copied().collect();
        let live: Vec<&MapTag> = self.playlist_maps.iter().map(|e| &e.map).collect();
        self.inits.retain(|id, slot| {
            Some(*id) == newest
                || bound.contains(id)
                || live.contains(&&slot.map)
                || slot.fetch == InitFetch::Ready
        });
        while self.inits.len() > MAX_INIT_SLOTS {
            let unreferenced = self
                .inits
                .iter()
                .find(|(id, _)| Some(**id) != newest && !bound.contains(id))
                .map(|(id, _)| *id);
            let victim = match unreferenced {
                Some(id) => id,
                None => match self.inits.keys().find(|id| Some(**id) != newest) {
                    Some(id) => *id,
                    None => break,
                },
            };
            self.inits.remove(&victim);
            self.msn_init.retain(|_, id| *id != victim);
            if self.init_target == Some(victim) {
                self.init_target = None;
            }
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
            return if self.last_media_sequence.is_some_and(|p| new.msn < p) {
                Continuity::Restart
            } else {
                Continuity::Continues
            };
        };
        let (p0, n0) = (prev.msn, new.msn);
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
                .checked_distance_from(p0)
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
        self.resource_requests.clear();
        self.last_media_sequence = None;
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
            let msn = broadcast_hls::MediaSequence::new(playlist.msn.value() + i);
            self.delivered_segments.contains(&msn)
                || self.fulfilled.contains(&ResourceId::Segment { msn })
                || self.covered_by_parts(msn)
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
    use alloc::vec;
    use transmux::sample_aes::aes128_encrypt_segment;

    fn config(url: &str) -> HlsClientConfig {
        HlsClientConfig::new(Url::parse(url).expect("test URL"))
    }

    // Audit 20 finding 52 (#2863): the reload cadence is derived from the
    // untrusted EXT-X-TARGETDURATION, so `MAX_RELOAD_WAIT_MS` clamps it.
    #[test]
    fn reload_wait_is_clamped_for_a_hostile_target_duration() {
        assert_eq!(
            reload_wait(u32::MAX),
            Duration::from_millis(MAX_RELOAD_WAIT_MS)
        );
        assert_eq!(reload_wait(4), Duration::from_millis(2_000));
        assert_eq!(reload_wait(0), Duration::from_millis(500));
    }

    const EMPTY_FRAGMENT: &[u8] = &[
        0, 0, 0, 8, b'm', b'o', b'o', b'f', 0, 0, 0, 8, b'm', b'd', b'a', b't',
    ];

    #[test]
    fn decrypt_segment_strips_pkcs7_padding_to_the_exact_plaintext() {
        let key = [0x5Au8; KEY_LEN];
        let iv = [0xC3u8; BLOCK_LEN];
        let id = ResourceId::Segment {
            msn: broadcast_hls::MediaSequence::new(9),
        };
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
        let iv = media_sequence_iv(broadcast_hls::MediaSequence::new(0x0102_0304_0506_0708));
        assert_eq!(iv[..8], [0u8; 8]);
        assert_eq!(iv[8..], [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            media_sequence_iv(broadcast_hls::MediaSequence::new(0))[..],
            [0u8; 16]
        );
        assert_eq!(
            media_sequence_iv(broadcast_hls::MediaSequence::new(0x1FF))[14..],
            [1, 0xFF]
        );
    }

    // Regression: `on_resource` documents (see `Error::UnrequestedResource`)
    // that it rejects a `ResourceId` the client never requested, but
    // previously never actually checked — any bytes for any id (a
    // caller/driver bug, or a stale/duplicate delivery) were silently
    // accepted. Must FAIL if that check is ever removed.
    #[test]
    fn on_resource_rejects_a_never_requested_id() {
        let mut client = HlsClient::new(config("http://example.com/playlist.m3u8"));
        let id = ResourceId::Segment {
            msn: broadcast_hls::MediaSequence::new(0),
        };

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
        let mut client = HlsClient::new(config("http://example.com/playlist.m3u8"));
        let id = ResourceId::Segment {
            msn: broadcast_hls::MediaSequence::new(0),
        };
        client.request_resource(
            id,
            Url::parse("http://example.com/seg0.m4s").expect("URL"),
            None,
        );

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
        let client = HlsClient::new(config("http://example.com/playlist.m3u8"));
        assert!(client.is_ts_segment(&[TS_SYNC_BYTE, 0x40, 0x11, 0x00]));
    }

    // ...but say no to an ISOBMFF (fMP4/CMAF) resource even when no map has
    // been seen yet — the content itself is never TS, so it must fall
    // through to the ordinary init-buffering path rather than being
    // misrouted into `TsDemux` (which would reject it as malformed TS).
    #[test]
    fn is_ts_segment_false_for_an_isobmff_resource_with_no_map_seen() {
        let client = HlsClient::new(config("http://example.com/playlist.m3u8"));
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
        let mut client = HlsClient::new(config("http://example.com/playlist.m3u8"));
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
        let mut client = HlsClient::new(config("http://example.com/playlist.m3u8"));
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
        let mut client = HlsClient::new(config("http://example.com/playlist.m3u8"));
        let url = "http://example.com/seg0.m4s";
        // Seed the cursor near the top of the u64 range with an explicit
        // offset, then the next omitted-offset range pushes it over.
        let seed = Some(ByteRange {
            length: u64::MAX - 10,
            offset: Some(5),
        });
        let span = client
            .resolve_byte_range(url, &seed)
            .expect("seed range does not itself overflow")
            .expect("Some for a Some(ByteRange)");
        assert_eq!((span.offset, span.len.get()), (5, u64::MAX - 10));

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
        let mut client = HlsClient::new(config("http://example.com/playlist.m3u8"));
        let prev = MediaPlaylist {
            msn: broadcast_hls::MediaSequence::new(0),
            segments: vec![MediaSegment::default(); 2],
            ..Default::default()
        };
        // `last_full_playlist` is set directly (private field, same module)
        // rather than driven through a full `on_playlist` round-trip — the
        // point under test is purely `merge_delta`'s own arithmetic guard.
        client.last_full_playlist = Some(prev);

        let delta = MediaPlaylist {
            msn: broadcast_hls::MediaSequence::new(10), // prefix_start == 10 - 0 == 10
            skip: Some(broadcast_hls::SkipInfo::new(u64::MAX - 5)),
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
        // (RFC 8216 §4.3.2.2), so the per-URL cursor matters.
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
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client
            .on_playlist(byte_range_playlist(0, "a.ts").as_bytes())
            .unwrap();
        assert_eq!(
            client.byte_range_cursor.iter().collect::<Vec<_>>(),
            vec![(&"http://example.com/a.ts".to_string(), &150u64)]
        );
        // Deliver msn 0 (buffered: no init), leave msn 1 in flight.
        client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(0),
                },
                EMPTY_FRAGMENT,
            )
            .unwrap();

        client
            .on_playlist(byte_range_playlist(1000, "b.ts").as_bytes())
            .unwrap();

        let requested: Vec<ResourceId> = client.requested.iter().copied().collect();
        assert_eq!(
            requested,
            vec![
                // in flight when the window moved: kept
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(1),
                },
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(1000),
                },
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(1001),
                },
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
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let pl = |first: u64| {
            format!(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                 #EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-DISCONTINUITY\n#EXTINF:2.0,\ns.ts\n"
            )
        };
        client.on_playlist(pl(5).as_bytes()).unwrap();
        client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(5),
                },
                EMPTY_FRAGMENT,
            )
            .unwrap();
        assert_eq!(
            client
                .discontinuous_msns
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![broadcast_hls::MediaSequence::new(5)]
        );
        client.on_playlist(pl(9).as_bytes()).unwrap();
        assert_eq!(
            client
                .discontinuous_msns
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![broadcast_hls::MediaSequence::new(9)]
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

    fn pending_demux_bytes(client: &HlsClient) -> usize {
        client.pending_demux.iter().map(|(_, b)| b.len()).sum()
    }

    fn map_playlist(first_msn: u64, count: u64) -> String {
        let mut text = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n\
             #EXT-X-MEDIA-SEQUENCE:{first_msn}\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n"
        );
        for msn in first_msn..first_msn + count {
            text.push_str(&format!("#EXTINF:2.0,\nseg{msn}.m4s\n"));
        }
        text
    }

    fn media_fragment_of_len(len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        bytes[..EMPTY_FRAGMENT.len()].copy_from_slice(EMPTY_FRAGMENT);
        bytes
    }

    // Audit 20 finding 3 (#2814): a media segment that arrives before its
    // init is buffered, and the queue was never bounded while the init never
    // became ready. Past the cap the oldest waiting body is dropped with a
    // typed error and the queue stays bounded.
    #[test]
    fn pending_demux_is_capped_and_drops_the_oldest_with_a_typed_error() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client
            .on_playlist(map_playlist(0, 2).as_bytes())
            .expect("playlist parses");
        let big = media_fragment_of_len(40 * 1024 * 1024);
        client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(0),
                },
                &big,
            )
            .expect("first body is buffered");
        let err = client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(1),
                },
                &big,
            )
            .expect_err("the second 40 MiB body exceeds the 64 MiB cap");
        assert!(
            matches!(err, Error::PendingDemuxOverflow { dropped } if dropped == ResourceId::Segment {
                msn: broadcast_hls::MediaSequence::new(0),
            }),
            "{err:?}"
        );
        assert!(
            pending_demux_bytes(&client) <= MAX_PENDING_DEMUX_BYTES,
            "{} bytes held",
            pending_demux_bytes(&client)
        );
        assert_eq!(client.pending_demux.len(), 1);
    }

    // ...and a body below the current window can never be played, so prune
    // must drop it with the other MSN-keyed records.
    #[test]
    fn pending_demux_is_pruned_with_the_window() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client
            .on_playlist(map_playlist(0, 2).as_bytes())
            .expect("playlist parses");
        client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(0),
                },
                EMPTY_FRAGMENT,
            )
            .expect("body is buffered pending the init");
        assert_eq!(client.pending_demux.len(), 1);
        client
            .on_playlist(map_playlist(1, 2).as_bytes())
            .expect("playlist parses");
        assert!(
            client.pending_demux.is_empty(),
            "a body below the window must be pruned: {:?}",
            client
                .pending_demux
                .iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        );
    }

    // Audit 20 finding 18 (#2829): the overflow check must run before any
    // state mutation, or one hostile `EXT-X-MEDIA-SEQUENCE` sets the
    // high-water mark to ~u64::MAX and drops every key, so the very next
    // legitimate playlist looks regressed and the session wedges.
    #[test]
    fn media_sequence_overflow_is_rejected_before_it_poisons_state() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client
            .on_playlist(keyed_playlist(0, 2, "key.bin").as_bytes())
            .unwrap();
        let high_water = client.last_media_sequence;
        let keys = client.key_urls.len();
        assert_eq!(high_water, Some(broadcast_hls::MediaSequence::new(0)));
        assert_eq!(keys, 1, "the playlist's key must be tracked");

        let hostile = format!(
            "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n\
             #EXT-X-MEDIA-SEQUENCE:{}\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n\
             #EXTINF:2.0,\nseg.ts\n",
            u64::MAX
        );
        let err = client
            .on_playlist(hostile.as_bytes())
            .expect_err("u64::MAX + one segment overflows");
        assert!(
            matches!(err, Error::MediaSequenceOverflow { .. }),
            "{err:?}"
        );
        assert_eq!(
            client.last_media_sequence, high_water,
            "a rejected playlist must not move the high-water mark"
        );
        assert_eq!(
            client.key_urls.len(),
            keys,
            "a rejected playlist must not drop key material"
        );
    }

    // Audit 20 finding 19 (#2830): a URI containing `{$name}` must never be
    // fetched literally. The client refuses a playlist that declares
    // `EXT-X-DEFINE` variables instead of resolving them.
    #[test]
    fn playlist_with_ext_x_define_is_refused_not_fetched_literally() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let text = "#EXTM3U\n#EXT-X-VERSION:8\n#EXT-X-TARGETDURATION:2\n\
                    #EXT-X-DEFINE:NAME=\"host\",VALUE=\"cdn.example\"\n\
                    #EXTINF:2.0,\nhttps://{$host}/seg0.m4s\n";
        assert_eq!(
            client
                .on_playlist(text.as_bytes())
                .expect_err("EXT-X-DEFINE is refused"),
            Error::UnsupportedDefine
        );
        for _ in 0..16 {
            match client.poll() {
                Some(Action::FetchResource { id, .. }) => {
                    panic!("a refused playlist must not request {id:?}")
                }
                Some(_) => {}
                None => break,
            }
        }

        let mut master_client = HlsClient::new(config("http://example.com/master.m3u8"));
        let master = "#EXTM3U\n\
                      #EXT-X-DEFINE:IMPORT=\"host\"\n\
                      #EXT-X-STREAM-INF:BANDWIDTH=1000\n\
                      https://{$host}/v.m3u8\n";
        assert_eq!(
            master_client
                .on_playlist(master.as_bytes())
                .expect_err("a master EXT-X-DEFINE is refused"),
            Error::UnsupportedDefine
        );
    }

    // Audit 20 finding 42 (#2853): a reference to a non-web scheme must never
    // become a fetch. Before the fix `ftp:` was resolved and requested like
    // any other URI.
    #[test]
    fn non_web_fetch_schemes_are_refused() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
                    #EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXTINF:2.0,\nftp://cdn/seg0.ts\n#EXT-X-ENDLIST\n";
        client
            .on_playlist(text.as_bytes())
            .expect("playlist parses");
        match client.next_error() {
            Some(Error::UnresolvableReference { source, .. }) => {
                assert_eq!(source, broadcast_common::url_ref::Error::ForbiddenScheme)
            }
            other => panic!("expected a scheme refusal, got {other:?}"),
        }
        for _ in 0..16 {
            match client.poll() {
                Some(Action::FetchResource { id, .. }) => {
                    panic!("a non-web reference must not be requested: {id:?}")
                }
                Some(_) => {}
                None => break,
            }
        }
    }

    // ...and an `https` stream must not have a key fetch downgraded into the
    // clear.
    #[test]
    fn an_https_stream_refuses_an_http_key_uri() {
        let mut client = HlsClient::new(config("https://example.com/p.m3u8"));
        let text = "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n\
                    #EXT-X-MEDIA-SEQUENCE:0\n\
                    #EXT-X-KEY:METHOD=AES-128,URI=\"http://cdn/key.bin\"\n\
                    #EXTINF:2.0,\nseg0.ts\n#EXT-X-ENDLIST\n";
        match client.on_playlist(text.as_bytes()) {
            Err(Error::UnresolvableReference { source, .. }) => {
                assert_eq!(source, broadcast_common::url_ref::Error::ForbiddenScheme)
            }
            other => panic!("expected an https->http downgrade refusal, got {other:?}"),
        }
        assert!(
            client.key_urls.is_empty(),
            "no key URL may be retained for a downgraded fetch"
        );
    }

    // Audit 20 finding 41 (#2852): the sans-IO client has no own playlist or
    // resource bound, so `HlsClientConfig::limits` is enforced here.
    #[test]
    fn client_limits_bound_playlist_and_resource_bodies() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nseg.ts\n\
                    #EXT-X-ENDLIST\n";
        let mut limits = Limits::default();
        limits.max_playlist_bytes = 16;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        assert!(matches!(
            client.on_playlist(text.as_bytes()),
            Err(Error::PlaylistTooLarge {
                what: "playlist bytes",
                ..
            })
        ));

        let two = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nseg0.ts\n\
                   #EXTINF:2.0,\nseg1.ts\n#EXT-X-ENDLIST\n";
        let mut limits = Limits::default();
        limits.max_segments = 1;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        assert!(matches!(
            client.on_playlist(two.as_bytes()),
            Err(Error::PlaylistTooLarge {
                what: "media segments",
                ..
            })
        ));

        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n";
        let mut limits = Limits::default();
        limits.max_playlist_bytes = 16;
        let mut client =
            HlsClient::new(config("http://example.com/master.m3u8").with_limits(limits));
        assert!(matches!(
            client.on_playlist(master.as_bytes()),
            Err(Error::PlaylistTooLarge { .. })
        ));

        let mut limits = Limits::default();
        limits.max_segment_bytes = 4;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        client
            .on_playlist(map_playlist(0, 1).as_bytes())
            .expect("playlist parses");
        let err = client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(0),
                },
                EMPTY_FRAGMENT,
            )
            .expect_err("a 16-byte body exceeds the 4-byte cap");
        assert!(
            matches!(err, Error::ResourceTooLarge { id: ResourceId::Segment { msn }, .. } if msn == broadcast_hls::MediaSequence::new(0)),
            "{err:?}"
        );
        assert!(
            client.pending_demux.is_empty(),
            "an over-size body must be refused before it is buffered"
        );
    }

    // Audit 20 finding 33 (#2844): `pending_errors` is drained by a separate
    // API a host can skip, so a playlist naming many bad references must not
    // grow the queue without bound. The excess coalesces into one counted
    // error.
    #[test]
    fn pending_errors_are_capped_and_coalesced() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let total = MAX_PENDING_ERRORS + 44;
        let mut text = String::from(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n",
        );
        for i in 0..total {
            text.push_str(&format!("#EXTINF:2.0,\nbad seg{i}.ts\n"));
        }
        text.push_str("#EXT-X-ENDLIST\n");
        client
            .on_playlist(text.as_bytes())
            .expect("an unresolvable-segment playlist still parses");
        assert_eq!(
            client.pending_errors.len(),
            MAX_PENDING_ERRORS,
            "the report queue is capped at MAX_PENDING_ERRORS"
        );

        let mut real = 0usize;
        let mut coalesced = 0usize;
        for _ in 0..MAX_PENDING_ERRORS + 2 {
            match client.next_error() {
                Some(Error::UnresolvableReferences { count }) => coalesced += count,
                Some(Error::UnresolvableReference { .. }) => real += 1,
                Some(other) => panic!("unexpected error: {other:?}"),
                None => break,
            }
        }
        assert_eq!(real, MAX_PENDING_ERRORS - 1);
        assert_eq!(real + coalesced, total);
        assert!(client.next_error().is_none());
    }

    fn insert_init_slot(client: &mut HlsClient, id: InitId, fetch: InitFetch) {
        client.inits.insert(
            id,
            InitSlot {
                map: MapTag {
                    uri: format!("init{id}.mp4"),
                    ..Default::default()
                },
                url: Url::parse(&format!("http://example.com/init{id}.mp4")).expect("test URL"),
                byte_range: None,
                encryption: None,
                crypto: None,
                fetch,
                cipher: None,
                bytes: None,
                skipped_reported: BTreeSet::new(),
                demux: None,
                init_events: Vec::new(),
                demux_error: None,
                specs: Vec::new(),
                tracks_known: false,
            },
        );
    }

    // Audit 20 finding 4 (#2815): a failing or never-completing EXT-X-MAP
    // fetch leaves a non-Ready slot, and the old retain kept every one of
    // them. A slot no segment and no current map refers to must be dropped.
    #[test]
    fn prune_inits_drops_unreferenced_non_ready_slots() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        for id in 0..10 {
            insert_init_slot(&mut client, id, InitFetch::Failed);
        }
        client.prune_inits();
        assert_eq!(
            client.inits.len(),
            1,
            "only the newest unreferenced slot may survive, not every Failed one"
        );
        assert!(client.inits.contains_key(&9));
    }

    // ...and a playlist naming more maps than MAX_INIT_SLOTS must not grow
    // the slot map past the cap.
    #[test]
    fn init_slots_are_capped() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let total = MAX_INIT_SLOTS + 36;
        for id in 0..total as u32 {
            insert_init_slot(&mut client, id, InitFetch::Wanted);
            client
                .msn_init
                .insert(broadcast_hls::MediaSequence::new(u64::from(id)), id);
        }
        client.prune_inits();
        assert_eq!(client.inits.len(), MAX_INIT_SLOTS);
        assert!(
            client
                .msn_init
                .values()
                .all(|id| client.inits.contains_key(id)),
            "no segment may stay bound to an evicted slot"
        );
        assert!(client.inits.contains_key(&(total as u32 - 1)));
    }

    #[test]
    fn pending_keyed_is_pruned_with_the_window() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client
            .on_playlist(keyed_playlist(0, 1, "key.bin").as_bytes())
            .unwrap();
        client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(0),
                },
                &[0u8; 32],
            )
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
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client
            .on_playlist(keyed_playlist(0, 2, "key.bin").as_bytes())
            .unwrap();
        let big = vec![0u8; 40 * 1024 * 1024];
        client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(0),
                },
                &big,
            )
            .unwrap();
        let err = client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(1),
                },
                &big,
            )
            .expect_err("the second 40 MiB segment exceeds the 64 MiB cap");
        assert!(
            matches!(err, Error::PendingKeyedOverflow { dropped } if dropped == ResourceId::Segment {
                msn: broadcast_hls::MediaSequence::new(0),
            }),
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
            vec![ResourceId::Segment {
                msn: broadcast_hls::MediaSequence::new(1),
            }]
        );
    }

    #[test]
    fn key_maps_do_not_grow_with_per_reload_tokenised_key_urls() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
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
        let mut c = HlsClient::new(config("http://h/p.m3u8"));
        assert_eq!(c.next_wait(), None, "the first queued action is a fetch");
        let _ = c.poll();
        c.on_playlist(
            b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n",
        )
        .unwrap();
        while c.next_wait().is_none() {
            c.poll()
                .expect("a Wait must be queued after a non-blocking live playlist");
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
        let t0 = Timestamp::ZERO;
        let first = c.poll_timeout(t0).expect("a wait is at the front");
        assert_eq!(first, Timestamp::from_millis(1000));
        for later in [250u64, 600, 999, 5000] {
            assert_eq!(
                c.poll_timeout(Timestamp::from_millis(later)),
                Some(first),
                "re-query at +{later} ms must not slide the deadline"
            );
        }
        // Draining the wait supersedes it: the next wait is anchored afresh.
        assert!(
            matches!(c.poll(), Some(Action::Wait(wait)) if wait == Duration::from_millis(1000))
        );
        assert_eq!(c.poll_timeout(Timestamp::from_millis(9_000)), None);
    }

    /// What the client reads from `EXT-X-PROGRAM-DATE-TIME`: the production parser, in Unix
    /// milliseconds.
    fn program_date_ms(text: &str) -> Option<i64> {
        broadcast_hls::ProgramDateTime::parse(text)
            .ok()
            .map(|pdt| pdt.timestamp().as_millisecond())
    }

    /// The spellings the playlist parser accepts beyond strict RFC 3339: a zone-less value is UTC
    /// (RFC 8216bis) and surrounding whitespace is ignored.
    #[test]
    fn program_date_time_accepts_a_zone_less_value_as_utc_and_ignores_whitespace() {
        assert_eq!(
            program_date_ms("2026-10-02T10:00:00"),
            Some(1_790_935_200_000)
        );
        assert_eq!(
            program_date_ms(" 2026-10-02T10:00:00Z "),
            Some(1_790_935_200_000)
        );
        assert_eq!(program_date_ms("not a date"), None);
    }

    #[test]
    fn program_date_time_accepts_the_common_spellings() {
        assert_eq!(
            program_date_ms("2026-10-02 10:00:00Z"),
            Some(1_790_935_200_000),
            "space separator"
        );
        assert_eq!(
            program_date_ms("2026-10-02t10:00:00z"),
            Some(1_790_935_200_000),
            "lower-case t and z"
        );
        assert_eq!(
            program_date_ms("2026-10-02T10:00:00.123456789Z"),
            Some(1_790_935_200_123),
            "extra digits truncated"
        );
        assert_eq!(
            program_date_ms("1969-12-31T23:59:59Z"),
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
            assert_eq!(program_date_ms(text), want, "{text}");
        }
        // An hour-only offset: whatever `jiff` decides, it must never be a guess.
        let hour_only = program_date_ms("2026-10-02T12:00:00+02");
        assert!(hour_only == want || hour_only.is_none(), "{hour_only:?}");
    }

    /// A leap second is clamped to :59 (jiff), where the old parser added 60 s to the minute.
    #[test]
    fn a_leap_second_is_clamped_to_the_last_second_of_the_minute() {
        assert_eq!(
            program_date_ms("2016-12-31T23:59:60Z"),
            Some(1_483_228_799_000)
        );
    }

    #[test]
    fn rfc3339_parses_to_literal_epoch_milliseconds() {
        // 2026-10-02T10:00:00Z = 1_790_935_200 s (independently: `date -u -d`).
        assert_eq!(
            program_date_ms("2026-10-02T10:00:00Z"),
            Some(1_790_935_200_000)
        );
        assert_eq!(
            program_date_ms("2026-10-02T10:00:00.250Z"),
            Some(1_790_935_200_250)
        );
        assert_eq!(
            program_date_ms("2026-10-02T10:00:00.5Z"),
            Some(1_790_935_200_500)
        );
        assert_eq!(program_date_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            program_date_ms("2000-02-29T23:59:59Z"),
            Some(951_868_799_000)
        );
        // The same instant with an offset, both spellings.
        assert_eq!(
            program_date_ms("2026-10-02T12:00:00+02:00"),
            Some(1_790_935_200_000)
        );
        assert_eq!(
            program_date_ms("2026-10-02T05:00:00-0500"),
            Some(1_790_935_200_000)
        );
        // A zone-less date-time is UTC (RFC 8216bis); a bare date is not a date-time
        // (RFC 8216 §4.3.2.6) and everything else that is not RFC 3339 is `None`, never a guess.
        assert_eq!(program_date_ms("2026-10-02"), None, "a bare date");
        for bad in [
            "",
            "2026-13-02T10:00:00Z",
            "2026-10-02T25:00:00Z",
            "2026-10-02T10:00:00Zjunk",
            "2026-10-02T10:00:00.Z",
            "2026-10-02T10:00:00+2:00",
            "xxxx-10-02T10:00:00Z",
        ] {
            assert_eq!(program_date_ms(bad), None, "{bad:?}");
        }
    }

    // Audit 21 finding 54 (#2949): the largest legal resource body is
    // accepted; one byte more is refused with the observed length and the id
    // is requestable again.
    #[test]
    fn resource_body_cap_boundary_is_inclusive() {
        let mut limits = Limits::default();
        limits.max_segment_bytes = 400;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        client
            .on_playlist(map_playlist(0, 1).as_bytes())
            .expect("playlist parses");
        let id = ResourceId::Segment {
            msn: broadcast_hls::MediaSequence::new(0),
        };
        client
            .on_resource(id, &media_fragment_of_len(400))
            .expect("a body of exactly max_segment_bytes is accepted");
        assert_eq!(client.pending_demux.len(), 1);

        let mut limits = Limits::default();
        limits.max_segment_bytes = 400;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        client
            .on_playlist(map_playlist(0, 1).as_bytes())
            .expect("playlist parses");
        let err = client
            .on_resource(id, &media_fragment_of_len(401))
            .expect_err("one byte over the cap is refused");
        assert!(
            matches!(err, Error::ResourceTooLarge { id: refused, len: 401, max: 400 } if refused == id),
            "{err:?}"
        );
        assert_eq!(
            client.pending_demux.len(),
            0,
            "an over-size body is not held"
        );
        assert!(
            client.retry_resource(id),
            "a refused resource must be fetchable again"
        );
        assert!(client.requested.contains(&id));
    }

    // ...and the ciphertext holding queue treats the cap as a total budget:
    // exact fit holds, one byte over drops the OLDEST with a typed error, and
    // a single body over the cap is refused outright.
    #[test]
    fn pending_keyed_cap_boundary_and_oldest_drop() {
        let id = |msn: u64| ResourceId::Segment {
            msn: broadcast_hls::MediaSequence::new(msn),
        };
        let mut limits = Limits::default();
        limits.max_segment_bytes = MAX_PENDING_KEYED_BYTES + 1;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        client
            .on_playlist(keyed_playlist(0, 3, "key.bin").as_bytes())
            .expect("playlist parses");
        client
            .on_resource(id(0), &vec![7u8; MAX_PENDING_KEYED_BYTES])
            .expect("a body of exactly the cap is held");
        assert_eq!(pending_keyed_bytes(&client), MAX_PENDING_KEYED_BYTES);
        let err = client
            .on_resource(id(1), &[7u8; 1])
            .expect_err("one byte over the cap drops the oldest");
        assert!(
            matches!(err, Error::PendingKeyedOverflow { dropped } if dropped == id(0)),
            "{err:?}"
        );
        assert_eq!(
            client
                .pending_keyed
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            vec![id(1)]
        );
        let err = client
            .on_resource(id(2), &vec![7u8; MAX_PENDING_KEYED_BYTES + 1])
            .expect_err("a single body over the cap is refused");
        assert!(
            matches!(err, Error::PendingKeyedOverflow { dropped } if dropped == id(2)),
            "{err:?}"
        );

        let mut limits = Limits::default();
        limits.max_segment_bytes = MAX_PENDING_KEYED_BYTES + 1;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        client
            .on_playlist(keyed_playlist(0, 3, "key.bin").as_bytes())
            .expect("playlist parses");
        client
            .on_resource(id(0), &vec![7u8; MAX_PENDING_KEYED_BYTES / 2])
            .expect("first half");
        client
            .on_resource(id(1), &vec![7u8; MAX_PENDING_KEYED_BYTES / 2])
            .expect("an exact fit of two halves is held");
        assert_eq!(client.pending_keyed.len(), 2);
        let err = client
            .on_resource(id(2), &vec![7u8; MAX_PENDING_KEYED_BYTES / 2 + 1])
            .expect_err("one byte over an exact fit drops the oldest");
        assert!(
            matches!(err, Error::PendingKeyedOverflow { dropped } if dropped == id(0)),
            "{err:?}"
        );
    }

    #[test]
    fn pending_demux_cap_boundary_and_oldest_drop() {
        let id = |msn: u64| ResourceId::Segment {
            msn: broadcast_hls::MediaSequence::new(msn),
        };
        let mut limits = Limits::default();
        limits.max_segment_bytes = MAX_PENDING_DEMUX_BYTES + 1;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        client
            .on_playlist(map_playlist(0, 3).as_bytes())
            .expect("playlist parses");
        client
            .on_resource(id(0), &media_fragment_of_len(MAX_PENDING_DEMUX_BYTES))
            .expect("a body of exactly the cap is held");
        assert_eq!(pending_demux_bytes(&client), MAX_PENDING_DEMUX_BYTES);
        let err = client
            .on_resource(id(1), &media_fragment_of_len(EMPTY_FRAGMENT.len() + 1))
            .expect_err("one byte over the cap drops the oldest");
        assert!(
            matches!(err, Error::PendingDemuxOverflow { dropped } if dropped == id(0)),
            "{err:?}"
        );
        assert_eq!(
            client
                .pending_demux
                .iter()
                .map(|(i, _)| *i)
                .collect::<Vec<_>>(),
            vec![id(1)]
        );
        let err = client
            .on_resource(id(2), &media_fragment_of_len(MAX_PENDING_DEMUX_BYTES + 1))
            .expect_err("a single body over the cap is refused");
        assert!(
            matches!(err, Error::PendingDemuxOverflow { dropped } if dropped == id(2)),
            "{err:?}"
        );

        let mut limits = Limits::default();
        limits.max_segment_bytes = MAX_PENDING_DEMUX_BYTES + 1;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8").with_limits(limits));
        client
            .on_playlist(map_playlist(0, 3).as_bytes())
            .expect("playlist parses");
        client
            .on_resource(id(0), &media_fragment_of_len(MAX_PENDING_DEMUX_BYTES / 2))
            .expect("first half");
        client
            .on_resource(id(1), &media_fragment_of_len(MAX_PENDING_DEMUX_BYTES / 2))
            .expect("an exact fit of two halves is held");
        assert_eq!(client.pending_demux.len(), 2);
    }

    // Audit 21 finding 54 (#2949): the init-slot bound evicts the oldest
    // UNREFERENCED slot, never the newest and never one a live segment is
    // bound to; when every slot is referenced it falls back to the oldest
    // non-newest one, and a surviving init_target is untouched.
    #[test]
    fn prune_inits_prefers_unreferenced_victims_and_keeps_referenced_ones() {
        let msn = broadcast_hls::MediaSequence::new;

        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        for slot in [0, 1, 2] {
            insert_init_slot(&mut client, slot, InitFetch::Wanted);
            client.msn_init.insert(msn(u64::from(slot)), slot);
        }
        for slot in 3..=65 {
            insert_init_slot(&mut client, slot, InitFetch::Ready);
        }
        client.prune_inits();
        assert_eq!(client.inits.len(), MAX_INIT_SLOTS);
        assert!(client.inits.contains_key(&65), "the newest slot survives");
        for slot in [0, 1, 2] {
            assert!(
                client.inits.contains_key(&slot),
                "slot {slot} is bound to a live segment"
            );
        }
        assert!(
            !client.inits.contains_key(&3),
            "the oldest unreferenced slot is the victim"
        );
        assert!(!client.inits.contains_key(&4));
        assert!(client.inits.contains_key(&5));

        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        for slot in 0..70u32 {
            insert_init_slot(&mut client, slot, InitFetch::Wanted);
            client.msn_init.insert(msn(u64::from(slot)), slot);
        }
        client.init_target = Some(69);
        client.prune_inits();
        assert_eq!(client.inits.len(), MAX_INIT_SLOTS);
        assert!(client.inits.contains_key(&69), "the newest slot survives");
        assert!(
            !client.inits.contains_key(&0) && !client.inits.contains_key(&1),
            "with every slot referenced the oldest non-newest ones are evicted"
        );
        assert!(
            client
                .msn_init
                .values()
                .all(|id| client.inits.contains_key(id)),
            "no segment may stay bound to an evicted slot"
        );
        assert_eq!(
            client.msn_init.len(),
            MAX_INIT_SLOTS,
            "evicting a slot drops only its own segment binding"
        );
        assert_eq!(client.msn_init.get(&msn(69)), Some(&69));
        assert_eq!(client.msn_init.get(&msn(6)), Some(&6));
        assert_eq!(
            client.init_target,
            Some(69),
            "an evicted slot was not the init_target"
        );
    }

    // Audit 21 finding 54 (#2949): the outstanding-fetch counter counts
    // REQUESTS, so the end-of-stream gate can never see a false zero.
    #[test]
    fn outstanding_fetch_counter_counts_requests() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client
            .on_playlist(map_playlist(0, 2).as_bytes())
            .expect("playlist parses");
        assert_eq!(
            client.outstanding_fetches, 3,
            "one init and two segment requests"
        );
        client
            .on_resource(
                ResourceId::Segment {
                    msn: broadcast_hls::MediaSequence::new(0),
                },
                EMPTY_FRAGMENT,
            )
            .expect("held pending the init");
        assert_eq!(client.outstanding_fetches, 2);

        let mut keyed = HlsClient::new(config("http://example.com/p.m3u8"));
        keyed
            .on_playlist(keyed_playlist(0, 1, "key.bin").as_bytes())
            .expect("playlist parses");
        assert_eq!(
            keyed.outstanding_fetches, 2,
            "one key and one segment request"
        );
    }

    // Audit 21 finding 55 (#2950): every continuity signal on its own, since
    // a single mis-classification restarts a healthy stream or delivers one
    // backwards.
    fn pdt(secs: u64) -> broadcast_hls::ProgramDateTime {
        broadcast_hls::ProgramDateTime::parse(&format!(
            "2026-10-02T{:02}:{:02}:{:02}Z",
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        ))
        .expect("test PDT")
    }

    fn playlist_of(first_msn: u64, segs: Vec<MediaSegment>) -> MediaPlaylist {
        MediaPlaylist {
            msn: broadcast_hls::MediaSequence::new(first_msn),
            segments: segs,
            ..Default::default()
        }
    }

    fn seg(uri: &str) -> MediaSegment {
        MediaSegment {
            uri: uri.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn continuity_identity_and_regression_signals() {
        let msn = broadcast_hls::MediaSequence::new;
        let client_with = |prev: Option<MediaPlaylist>, last: u64| {
            let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
            client.last_full_playlist = prev;
            client.last_media_sequence = Some(msn(last));
            client
        };

        // No previous full playlist: only an older media sequence restarts.
        let client = client_with(None, 5);
        assert_eq!(
            client.continuity(&playlist_of(3, vec![seg("a.ts")])),
            Continuity::Restart
        );
        assert_eq!(
            client.continuity(&playlist_of(5, vec![seg("a.ts")])),
            Continuity::Continues,
            "an equal media sequence is not a regression"
        );

        let prev = playlist_of(5, vec![seg("a.ts"), seg("b.ts")]);
        let client = client_with(Some(prev.clone()), 6);
        assert_eq!(
            client.continuity(&playlist_of(6, vec![seg("b.ts"), seg("c.ts")])),
            Continuity::Continues,
            "the shared number names the same segment"
        );

        let mut different_uri = seg("b.ts");
        different_uri.uri = "other.ts".to_string();
        assert_eq!(
            client.continuity(&playlist_of(6, vec![different_uri, seg("c.ts")])),
            Continuity::Restart,
            "a shared number naming different media is a restart"
        );

        let mut different_duration = seg("b.ts");
        different_duration.duration = broadcast_hls::DecimalSeconds::new(9.0).expect("duration");
        assert_eq!(
            client.continuity(&playlist_of(6, vec![different_duration, seg("c.ts")])),
            Continuity::Restart
        );

        let mut different_range = seg("b.ts");
        different_range.byte_range = Some(broadcast_hls::ByteRange {
            length: 10,
            offset: Some(0),
        });
        assert_eq!(
            client.continuity(&playlist_of(6, vec![different_range, seg("c.ts")])),
            Continuity::Restart
        );

        let mut pdt_a = seg("b.ts");
        pdt_a.program_date_time = Some(pdt(100));
        let mut prev_pdt = prev.clone();
        prev_pdt.segments[1].program_date_time = Some(pdt(100));
        let client = client_with(Some(prev_pdt), 6);
        assert_eq!(
            client.continuity(&playlist_of(6, vec![pdt_a.clone(), seg("c.ts")])),
            Continuity::Continues,
            "the same wall-clock time at a shared number"
        );
        let mut pdt_b = seg("b.ts");
        pdt_b.program_date_time = Some(pdt(102));
        assert_eq!(
            client.continuity(&playlist_of(6, vec![pdt_b, seg("c.ts")])),
            Continuity::Restart,
            "a different wall-clock time at a shared number"
        );

        // A CDN re-signing every shared URI is still the same stream; one
        // re-signed URI among unchanged ones is not.
        let client = client_with(
            Some(playlist_of(5, vec![seg("a.ts?t=1"), seg("b.ts?t=1")])),
            6,
        );
        assert_eq!(
            client.continuity(&playlist_of(6, vec![seg("b.ts?t=2"), seg("c.ts?t=2")])),
            Continuity::Continues,
            "a wholly re-signed window"
        );
        let client = client_with(
            Some(playlist_of(
                5,
                vec![seg("a.ts?t=1"), seg("b.ts?t=1"), seg("c.ts")],
            )),
            6,
        );
        assert_eq!(
            client.continuity(&playlist_of(
                6,
                vec![seg("b.ts?t=2"), seg("c.ts"), seg("d.ts")]
            )),
            Continuity::Restart,
            "only some shared URIs re-signed"
        );
    }

    #[test]
    fn continuity_regression_without_overlap() {
        let msn = broadcast_hls::MediaSequence::new;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client.last_full_playlist =
            Some(playlist_of(100, vec![seg("seg100.ts"), seg("seg101.ts")]));
        client.last_media_sequence = Some(msn(100));

        let mut new = playlist_of(10, vec![seg("seg10.ts"), seg("seg11.ts")]);
        assert_eq!(client.continuity(&new), Continuity::Restart);

        // The same URIs under different numbers: names reused.
        let reused = playlist_of(10, vec![seg("seg100.ts")]);
        assert_eq!(client.continuity(&reused), Continuity::Restart);

        // PROGRAM-DATE-TIME decides an older copy from a restart.
        let mut prev = playlist_of(100, vec![seg("seg100.ts")]);
        prev.segments[0].program_date_time = Some(pdt(12 * 3600));
        client.last_full_playlist = Some(prev);
        new.segments[1].program_date_time = Some(pdt(10 * 3600 + 2));
        assert_eq!(
            client.continuity(&new),
            Continuity::OlderWindow,
            "a window ending before the previous one began is a lagging copy"
        );
        new.segments[1].program_date_time = Some(pdt(12 * 3600 + 2));
        assert_eq!(
            client.continuity(&new),
            Continuity::Restart,
            "a window ending inside or after the previous one is a restart"
        );
        new.segments[1].program_date_time = Some(pdt(12 * 3600));
        assert_eq!(
            client.continuity(&new),
            Continuity::Restart,
            "a window ending exactly where the previous one began is not wholly older"
        );

        // Overlapping but regressed is a lagging copy, as is one whose
        // discontinuity sequence decreased while regressed.
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let mut prev = playlist_of(5, vec![seg("a.ts"), seg("b.ts")]);
        prev.discontinuity_sequence = 3;
        client.last_full_playlist = Some(prev);
        client.last_media_sequence = Some(msn(6));
        let mut lagging = playlist_of(5, vec![seg("a.ts"), seg("b.ts")]);
        lagging.discontinuity_sequence = 3;
        assert_eq!(client.continuity(&lagging), Continuity::Lagging);
        lagging.discontinuity_sequence = 5;
        assert_eq!(
            client.continuity(&lagging),
            Continuity::Lagging,
            "a regressed window is not a restart for a higher discontinuity sequence"
        );
        lagging.discontinuity_sequence = 2;
        assert_eq!(
            client.continuity(&lagging),
            Continuity::Lagging,
            "a regressed window is a lagging copy whatever its discontinuity sequence"
        );

        // ...but a decreasing discontinuity sequence at a non-regressed
        // number is a restart.
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let mut prev = playlist_of(5, vec![seg("a.ts"), seg("b.ts")]);
        prev.discontinuity_sequence = 3;
        client.last_full_playlist = Some(prev);
        client.last_media_sequence = Some(msn(5));
        let mut decreased = playlist_of(6, vec![seg("b.ts"), seg("c.ts")]);
        decreased.discontinuity_sequence = 2;
        assert_eq!(client.continuity(&decreased), Continuity::Restart);
        decreased.discontinuity_sequence = 3;
        assert_eq!(client.continuity(&decreased), Continuity::Continues);
    }

    #[test]
    fn an_encrypted_init_without_a_moov_is_undecodable() {
        let key = [0x11u8; KEY_LEN];
        let iv = [0x22u8; BLOCK_LEN];
        let crypto = SegmentCrypto {
            key_id: KeyId(0),
            iv,
        };
        let moov = [0u8, 0, 0, 8, b'm', b'o', b'o', b'v'];

        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        insert_init_slot(&mut client, 0, InitFetch::InFlight);
        let cipher = aes128_encrypt_segment(&key, &iv, EMPTY_FRAGMENT);
        let err = client
            .complete_encrypted_init(0, crypto, &key, &cipher)
            .expect_err("a fragment without a moov is not an init section");
        assert!(matches!(err, Error::InitUndecodable), "{err:?}");
        assert_eq!(client.inits[&0].fetch, InitFetch::Failed);

        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        insert_init_slot(&mut client, 0, InitFetch::InFlight);
        let cipher = aes128_encrypt_segment(&key, &iv, &moov);
        client
            .complete_encrypted_init(0, crypto, &key, &cipher)
            .expect("a moov-bearing init section is accepted");
        assert_eq!(client.inits[&0].fetch, InitFetch::Ready);
        assert_eq!(client.inits[&0].bytes.as_deref(), Some(&moov[..]));
    }

    #[test]
    fn merge_delta_splices_only_an_advancing_or_equal_delta() {
        let prev = MediaPlaylist {
            msn: broadcast_hls::MediaSequence::new(5),
            segments: vec![seg("a.ts"), seg("b.ts"), seg("c.ts"), seg("d.ts")],
            ..Default::default()
        };
        let client = {
            let mut c = HlsClient::new(config("http://example.com/p.m3u8"));
            c.last_full_playlist = Some(prev);
            c
        };

        let delta = |first: u64, uri: &str| MediaPlaylist {
            msn: broadcast_hls::MediaSequence::new(first),
            skip: Some(broadcast_hls::SkipInfo::new(2)),
            segments: vec![seg(uri)],
            ..Default::default()
        };

        let merged = client.merge_delta(delta(7, "e.ts"));
        assert_eq!(merged.skip, None, "the delta is merged into a full view");
        assert_eq!(
            merged
                .segments
                .iter()
                .map(|s| s.uri.as_str())
                .collect::<Vec<_>>(),
            vec!["c.ts", "d.ts", "e.ts"]
        );

        let equal = client.merge_delta(delta(5, "e.ts"));
        assert_eq!(
            equal.skip, None,
            "a delta starting at the previous window's first number is mergeable"
        );
        assert_eq!(
            equal
                .segments
                .iter()
                .map(|s| s.uri.as_str())
                .collect::<Vec<_>>(),
            vec!["a.ts", "b.ts", "e.ts"]
        );

        let behind = client.merge_delta(delta(3, "e.ts"));
        assert!(
            behind.skip.is_some(),
            "a delta below the previous window cannot be spliced"
        );
    }

    #[test]
    fn covered_by_parts_needs_a_part_and_no_undelivered_part() {
        let msn = broadcast_hls::MediaSequence::new;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let url = Url::parse("http://example.com/p0.m4s").expect("test URL");
        client.resource_requests.insert(
            ResourceId::Part {
                msn: msn(0),
                part: broadcast_hls::PartIndex::new(0),
            },
            (url.clone(), None),
        );
        client.resource_requests.insert(
            ResourceId::Part {
                msn: msn(0),
                part: broadcast_hls::PartIndex::new(1),
            },
            (url, None),
        );
        client
            .delivered_parts
            .insert((msn(0), broadcast_hls::PartIndex::new(0)));
        assert!(
            !client.covered_by_parts(msn(0)),
            "one part is still undelivered"
        );
        client
            .delivered_parts
            .insert((msn(0), broadcast_hls::PartIndex::new(1)));
        assert!(client.covered_by_parts(msn(0)));
        assert!(
            !client.covered_by_parts(msn(1)),
            "a segment with no parts is never covered by parts"
        );
    }

    #[test]
    fn skipping_a_segment_ignores_its_gap_parts() {
        let msn = broadcast_hls::MediaSequence::new;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let gap = MediaSegment {
            uri: "s0.m4s".to_string(),
            parts: vec![
                broadcast_hls::PartSpec::new(
                    "gap.m4s".to_string(),
                    broadcast_hls::DecimalSeconds::new(0.5).expect("duration"),
                )
                .with_gap(true)
                .with_byte_range(broadcast_hls::ByteRange {
                    length: 100,
                    offset: Some(0),
                }),
                broadcast_hls::PartSpec::new(
                    "real.m4s".to_string(),
                    broadcast_hls::DecimalSeconds::new(0.5).expect("duration"),
                )
                .with_byte_range(broadcast_hls::ByteRange {
                    length: 50,
                    offset: Some(0),
                }),
            ],
            ..Default::default()
        };
        client.skip_segment(msn(0), &gap).expect("skip");
        assert!(client.delivered_segments.contains(&msn(0)));
        assert_eq!(
            client
                .byte_range_cursor
                .iter()
                .map(|(u, c)| (u.as_str(), *c))
                .collect::<Vec<_>>(),
            vec![("http://example.com/real.m4s", 50)],
            "a GAP part is never fetched, so its byte range is not consumed"
        );
    }

    #[test]
    fn prune_keeps_the_current_window_entries() {
        let msn = broadcast_hls::MediaSequence::new;
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        let part = broadcast_hls::PartIndex::new(0);
        client.delivered_parts.insert((msn(4), part));
        client.delivered_segments.insert(msn(4));
        client.discontinuous_msns.insert(msn(4));
        client.discontinuity_emitted.insert(msn(4));
        client.segment_crypto.insert(
            msn(4),
            SegmentCrypto {
                key_id: KeyId(0),
                iv: [0u8; BLOCK_LEN],
            },
        );
        client.delivered_parts.insert((msn(5), part));
        client.delivered_segments.insert(msn(5));
        client.discontinuous_msns.insert(msn(5));
        client.discontinuity_emitted.insert(msn(5));

        let playlist = playlist_of(5, vec![seg("s5.ts")]);
        client.prune(&playlist);
        assert_eq!(
            client.delivered_parts.iter().copied().collect::<Vec<_>>(),
            vec![(msn(5), part)],
            "entries below the window go, entries in it stay"
        );
        assert_eq!(
            client
                .delivered_segments
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![msn(5)]
        );
        assert_eq!(
            client
                .discontinuous_msns
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![msn(5)]
        );
        assert_eq!(
            client
                .discontinuity_emitted
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![msn(5)]
        );
        assert!(client.segment_crypto.is_empty());
    }

    // Audit 21 finding 55 (#2950): a regression withdraws the old
    // numbering's queued key fetches (their ids name the old stream).
    #[test]
    fn restart_withdraws_queued_key_and_resource_fetches() {
        let mut client = HlsClient::new(config("http://example.com/p.m3u8"));
        client.pending_actions.push_back(Action::FetchKey {
            id: KeyId(0),
            url: Url::parse("http://example.com/key.bin").expect("test URL"),
        });
        client.pending_actions.push_back(Action::FetchResource {
            id: ResourceId::Segment {
                msn: broadcast_hls::MediaSequence::new(5),
            },
            url: Url::parse("http://example.com/s5.ts").expect("test URL"),
            byte_range: None,
        });
        client.restart_after_regression().expect("restart");
        assert!(
            !client
                .pending_actions
                .iter()
                .any(|a| matches!(a, Action::FetchKey { .. } | Action::FetchResource { .. })),
            "the old numbering's fetches are withdrawn: {:?}",
            client.pending_actions
        );
        assert!(matches!(client.next_output(), Some(Output::Discontinuity)));
    }
}
