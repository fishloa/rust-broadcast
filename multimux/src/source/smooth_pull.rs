//! Microsoft Smooth Streaming (MS-SSTR) pull ingest source (issue #759;
//! re-ported onto the media-plane ingress traits at plan step 5a round 3): a
//! sans-IO [`SmoothIngestSession`] plus [`run_smooth_pull`], the tokio drive
//! loop that performs the real GETs — mirrors `dash_pull`'s own round-3
//! shape (own `SmoothAction`/`SmoothResourceId` request/response identity,
//! `next_deadline`/`on_deadline`-driven live-manifest refresh, no internal
//! sleep).
//!
//! # No init segment on the wire
//!
//! Unlike DASH/CMAF, Smooth has no bootstrapping init segment: a
//! `QualityLevel@CodecPrivateData` IS the codec config.
//! [`SmoothIngestSession`] therefore *synthesizes* one per stream via
//! `transmux::smooth_parse::track_spec_from_quality_level` +
//! `build_init_segment` (T1, issue #759) once its first fragment resolves
//! `local_track_id` — see "Discovering each stream's wire track id" below.
//!
//! # Discovering each stream's wire track id
//!
//! An MS-SSTR manifest carries no `track_id` anywhere, yet every fetched
//! fragment's `moof`/`tfhd@track_ID` must match a `trak` in the synthesized
//! init segment's `moov` for [`Fmp4Demux::unpackage`] to absorb its samples
//! at all. [`SmoothIngestSession`] resolves this exactly as the pre-port
//! module did: fetch each stream's *first* fragment and peek its
//! `moof`/`tfhd@track_ID` directly (no `moov` needed), then build that
//! stream's synthesized init segment with the same track id. Unlike the
//! pre-port module, that first fragment's samples are demuxed and emitted
//! immediately once every stream's first fragment has resolved, rather than
//! being cached and replayed on the caller's first poll — a
//! simplification the sans-IO restructure enables, not a duplicate of
//! anything the `Trunk` holds (see the module's DASH counterpart for that
//! judgement, repeated verbatim here).
//!
//! # Round 3: the in-read-path sleep is gone
//!
//! Exactly like `dash_pull`'s own round-3 fix: `Stage::next_deadline`/
//! `Stage::on_deadline` report/act on when a live-manifest refresh is due;
//! [`run_smooth_pull`] is the only place a clock is read or a sleep awaited.
//!
//! # Video sample-duration clock (v1-scope convention)
//!
//! Unchanged: assumes `transmux::VIDEO_CLOCK_RATE` (90 kHz) for video, since
//! MS-SSTR has no per-stream video sample-duration field.
//!
//! # PlayReady / PIFF sample encryption is NOT supported
//!
//! Unchanged: a manifest `<Protection>` element or a fragment carrying
//! CENC/PIFF sample-encryption boxes fails with a typed
//! [`crate::error::MultimuxError::Encrypted`] rather than silently demuxing
//! garbage.
//!
//! # v1 scope
//!
//! Unchanged: one `QualityLevel` per `StreamIndex` (the first);
//! `StreamType::Text` `StreamIndex`es are skipped.

use std::collections::VecDeque;
use std::time::Duration;

use broadcast_auth::Credentials;
use broadcast_common::{Demand, Stage, Timestamp, Unpackage};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestSession, ProgramId, SessionEvent, run_dial,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};
use reqwest::{Client as HttpClient, StatusCode};
use tokio::task::JoinSet;
use transmux::box_types::parse_box;
use transmux::media::Fmp4Demux;
use transmux::movie_fragment::MovieFragmentBox;
use transmux::pipeline::build_init_segment;
use transmux::smooth_parse::{
    SmoothManifest, StreamIndex, StreamType, track_spec_from_quality_level,
};
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};
use crate::source::{IngestTimeouts, Source, may_spawn_fetch};

/// The synthesized per-stream init segment's `mvhd` timescale — arbitrary
/// (ISO/IEC 14496-12 §8.2.2). Reuses [`transmux::VIDEO_CLOCK_RATE`] purely so
/// this module doesn't invent a second arbitrary constant.
const SYNTHETIC_MOVIE_TIMESCALE: u32 = transmux::VIDEO_CLOCK_RATE;

/// Fixed live-manifest refresh interval (MS-SSTR has no
/// `MPD@minimumUpdatePeriod` analogue). Matches `dash_pull`'s own
/// `DEFAULT_MPD_REFRESH_INTERVAL`.
const MANIFEST_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// How long [`run_smooth_pull`] waits before retrying a tolerated `404` for a
/// live-edge fragment — see `dash_pull`'s `SEGMENT_RETRY_DELAY`.
const FRAGMENT_RETRY_DELAY: Duration = Duration::from_millis(500);

/// The PIFF "UUID Sample Encryption Box" extended type
/// (`A2394F52-5A9B-4F14-A244-6C427C648DF4`).
const PIFF_SAMPLE_ENCRYPTION_UUID: [u8; 16] = [
    0xA2, 0x39, 0x4F, 0x52, 0x5A, 0x9B, 0x4F, 0x14, 0xA2, 0x44, 0x6C, 0x42, 0x7C, 0x64, 0x8D, 0xF4,
];

/// How long a `run_*_pull` drive loop parks when its session has, momentarily,
/// neither an outbound request queued nor a fetch in flight — and has not
/// ended. A bare `continue` there would spin the loop with no `.await` in it,
/// which on a current-thread runtime starves every other task on the executor
/// (including the in-flight fetches this loop is waiting for). Short enough
/// that it costs no observable latency, long enough that it is not a spin.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// A remote MS-SSTR client Manifest to pull: its URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct SmoothPullRoute {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    auth: Option<Credentials>,
}

impl std::fmt::Debug for SmoothPullRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmoothPullRoute")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl SmoothPullRoute {
    /// Build a route descriptor. `url` is the client Manifest URL to pull.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        SmoothPullRoute {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }
}

impl Source for SmoothPullRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Identifies one selected `StreamIndex` by its position in this session's
/// resolved-stream order — stable for the session's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamIdx(pub usize);

/// This session's own request/response identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmoothResourceId {
    Manifest,
    FirstFragment(StreamIdx),
    Fragment(StreamIdx, u64),
}

/// One unit of IO [`run_smooth_pull`] must perform.
#[derive(Debug, Clone)]
pub enum SmoothAction {
    FetchManifest {
        url: String,
    },
    FetchFirstFragment {
        stream: StreamIdx,
        url: String,
    },
    FetchFragment {
        stream: StreamIdx,
        t: u64,
        d: u64,
        url: String,
        tolerate_404: bool,
    },
}

/// A `StreamIndex` resolved from the manifest but not yet initialized (its
/// first-fragment fetch, which discovers `local_track_id`, is outstanding).
struct PendingStream {
    stream: StreamIndex,
    stream_type: StreamType,
    bitrate: u64,
    /// Remaining plan *after* the first `(t, d)` pair (that one is what
    /// [`SmoothAction::FetchFirstFragment`] fetches).
    plan: VecDeque<(u64, u64)>,
    last_time: u64,
    first_bytes: Option<Vec<u8>>,
}

/// One selected stream's live state.
struct StreamState {
    stream: StreamIndex,
    bitrate: u64,
    init_bytes: Vec<u8>,
    local_track_id: u32,
    global_track_id: u32,
    plan: VecDeque<(u64, u64)>,
    last_time: u64,
    in_flight: bool,
}

struct LiveState {
    manifest_url: Url,
    is_live: bool,
    last_manifest_fetch: Timestamp,
    streams: Vec<StreamState>,
    manifest_refresh_in_flight: bool,
}

impl LiveState {
    fn all_idle_and_exhausted(&self) -> bool {
        self.streams
            .iter()
            .all(|s| !s.in_flight && s.plan.is_empty())
    }
}

enum Phase {
    AwaitingManifest,
    /// Every stream resolved from the initial manifest, plus that manifest's
    /// own `IsLive` — carried through this phase because [`LiveState`] needs
    /// it the moment the last first-fragment resolves.
    AwaitingFirstFragments {
        streams: Vec<PendingStream>,
        is_live: bool,
    },
    Live(LiveState),
}

/// The sans-IO Smooth-pull [`IngestSession`]. [`run_smooth_pull`] performs
/// every real GET and feeds responses in.
pub struct SmoothIngestSession {
    manifest_url: Url,
    phase: Phase,
    pending_requests: VecDeque<SmoothAction>,
    pending_events: VecDeque<SessionEvent>,
}

impl SmoothIngestSession {
    /// Construct a fresh session — performs no I/O.
    pub fn new(manifest_url: Url) -> Self {
        let mut pending_requests = VecDeque::new();
        pending_requests.push_back(SmoothAction::FetchManifest {
            url: manifest_url.to_string(),
        });
        SmoothIngestSession {
            manifest_url,
            phase: Phase::AwaitingManifest,
            pending_requests,
            pending_events: VecDeque::new(),
        }
    }

    /// True once every stream's plan is empty, none is in flight, and the
    /// manifest is static (`IsLive` absent/`"FALSE"`).
    pub fn ended(&self) -> bool {
        matches!(&self.phase, Phase::Live(live) if !live.is_live && live.all_idle_and_exhausted())
    }

    fn on_manifest(&mut self, bytes: &[u8]) -> Result<()> {
        let text = std::str::from_utf8(bytes).map_err(|e| MultimuxError::Connect {
            reason: format!("smooth-pull: manifest is not valid UTF-8: {e}"),
        })?;
        if manifest_declares_protection(text) {
            return Err(MultimuxError::Encrypted {
                reason: "smooth-pull: manifest declares a <Protection> element (PlayReady/PIFF \
                         sample encryption) — decrypting Smooth-protected content is not \
                         supported"
                    .into(),
            });
        }
        let manifest = SmoothManifest::parse(text).map_err(|e| MultimuxError::Connect {
            reason: format!("smooth-pull: manifest parse: {e}"),
        })?;

        match std::mem::replace(&mut self.phase, Phase::AwaitingManifest) {
            Phase::Live(mut live) => {
                live.is_live = manifest.is_live;
                live.manifest_refresh_in_flight = false;
                for stream in &mut live.streams {
                    let Some(found) = manifest
                        .streams
                        .iter()
                        .find(|s| s.stream_type == stream.stream.stream_type)
                    else {
                        continue;
                    };
                    let chunks = found
                        .enumerate_chunks()
                        .map_err(|e| MultimuxError::Connect {
                            reason: format!(
                                "smooth-pull: manifest refresh: chunk enumeration: {e}"
                            ),
                        })?;
                    for (t, d) in chunks {
                        if t >= stream.last_time {
                            stream.last_time = t.saturating_add(d).max(stream.last_time);
                            stream.plan.push_back((t, d));
                        }
                    }
                    stream.stream = found.clone();
                }
                self.phase = Phase::Live(live);
                self.pump_fragment_fetches();
                Ok(())
            }
            other @ Phase::AwaitingFirstFragments { .. } => {
                // A second manifest delivery while the first round of
                // first-fragment fetches is still outstanding — a driver bug
                // or a duplicate delivery. Ignored rather than restarting
                // resolution, which would re-queue a `FetchFirstFragment` for
                // every stream on top of the ones already in flight (see
                // `dash_pull`'s identical case).
                self.phase = other;
                Ok(())
            }
            Phase::AwaitingManifest => self.on_initial_manifest(manifest),
        }
    }

    fn on_initial_manifest(&mut self, manifest: SmoothManifest) -> Result<()> {
        let mut pending = Vec::new();
        for si in &manifest.streams {
            let stream_type = si.stream_type;
            if matches!(stream_type, StreamType::Text) {
                continue;
            }
            let quality = si.qualities.first().ok_or_else(|| MultimuxError::Connect {
                reason: format!(
                    "smooth-pull: StreamIndex {:?} ({stream_type}) has no QualityLevel",
                    si.name
                ),
            })?;
            let chunks = si.enumerate_chunks().map_err(|e| MultimuxError::Connect {
                reason: format!("smooth-pull: chunk enumeration: {e}"),
            })?;
            let Some(&(first_t, first_d)) = chunks.first() else {
                return Err(MultimuxError::Connect {
                    reason: format!(
                        "smooth-pull: StreamIndex {:?} ({stream_type}) has no known fragments yet",
                        si.name
                    ),
                });
            };
            let last_time = chunks
                .last()
                .map(|&(t, d)| t.saturating_add(d))
                .unwrap_or_else(|| first_t.saturating_add(first_d));
            let mut plan: VecDeque<(u64, u64)> = chunks.into();
            plan.pop_front();

            let rel = si.resolve_fragment_url(quality.bitrate, first_t);
            let url = self
                .manifest_url
                .join(&rel)
                .map_err(|e| MultimuxError::Connect {
                    reason: format!("smooth-pull: bad fragment URL {rel:?}: {e}"),
                })?;

            let idx = StreamIdx(pending.len());
            self.pending_requests
                .push_back(SmoothAction::FetchFirstFragment {
                    stream: idx,
                    url: url.to_string(),
                });
            pending.push(PendingStream {
                stream: si.clone(),
                stream_type,
                bitrate: quality.bitrate,
                plan,
                last_time,
                first_bytes: None,
            });
        }
        if pending.is_empty() {
            return Err(MultimuxError::Connect {
                reason: "smooth-pull: manifest resolved no usable stream (video/audio)".into(),
            });
        }
        self.phase = Phase::AwaitingFirstFragments {
            streams: pending,
            is_live: manifest.is_live,
        };
        self.pending_events.push_back(SessionEvent::Established);
        Ok(())
    }

    fn on_first_fragment(&mut self, idx: StreamIdx, bytes: &[u8]) -> Result<()> {
        let Phase::AwaitingFirstFragments {
            streams: pending, ..
        } = &mut self.phase
        else {
            return Ok(());
        };
        let Some(p) = pending.get_mut(idx.0) else {
            return Ok(());
        };
        if fragment_looks_encrypted(bytes) {
            return Err(MultimuxError::Encrypted {
                reason: format!(
                    "smooth-pull: stream {:?} fragment carries PIFF/CENC sample-encryption \
                     boxes — decrypting Smooth-protected content is not supported",
                    p.stream.name
                ),
            });
        }
        p.first_bytes = Some(bytes.to_vec());
        if pending.iter().all(|p| p.first_bytes.is_some()) {
            self.finish_awaiting_first_fragments()?;
        }
        Ok(())
    }

    fn finish_awaiting_first_fragments(&mut self) -> Result<()> {
        let Phase::AwaitingFirstFragments {
            streams: pending,
            is_live,
        } = std::mem::replace(&mut self.phase, Phase::AwaitingManifest)
        else {
            unreachable!("caller only invokes this from Phase::AwaitingFirstFragments");
        };
        let mut specs = Vec::new();
        let mut streams = Vec::new();
        let mut first_emits: Vec<(u32, Vec<u8>)> = Vec::new();

        for (global_id, p) in (1_u32..).zip(pending) {
            let first_bytes = p.first_bytes.expect("checked all-Some by the caller");
            let local_track_id = discover_moof_track_id(&first_bytes)?;
            let effective_timescale: u32 = match p.stream_type {
                StreamType::Video => transmux::VIDEO_CLOCK_RATE,
                StreamType::Audio => p
                    .stream
                    .qualities
                    .first()
                    .and_then(|q| q.sampling_rate)
                    .ok_or_else(|| MultimuxError::Connect {
                        reason: format!(
                            "smooth-pull: StreamIndex {:?} audio QualityLevel has no \
                                 SamplingRate",
                            p.stream.name
                        ),
                    })?,
                StreamType::Text => unreachable!("Text streams are skipped at manifest parse"),
            };
            let quality = p.stream.qualities.first().expect("checked above");
            let local_spec = track_spec_from_quality_level(
                local_track_id,
                effective_timescale,
                p.stream_type,
                quality,
            )?;
            let init_bytes =
                build_init_segment(std::slice::from_ref(&local_spec), SYNTHETIC_MOVIE_TIMESCALE)?;

            let mut global_spec = local_spec.clone();
            global_spec.track_id = global_id;
            specs.push(global_spec);

            first_emits.push((global_id, first_bytes));
            streams.push(StreamState {
                stream: p.stream,
                bitrate: p.bitrate,
                init_bytes,
                local_track_id,
                global_track_id: global_id,
                plan: p.plan,
                last_time: p.last_time,
                in_flight: false,
            });
        }

        self.phase = Phase::Live(LiveState {
            manifest_url: self.manifest_url.clone(),
            // Carried through `Phase::AwaitingFirstFragments` from the
            // initial manifest — getting this wrong would make an
            // `IsLive="TRUE"` manifest report `ended()` the moment its first
            // chunk plan drained, ending a live route after one pass.
            is_live,
            last_manifest_fetch: Timestamp::ZERO,
            streams,
            manifest_refresh_in_flight: false,
        });
        self.pending_events.push_back(SessionEvent::NewProgram {
            program: ProgramId(0),
            tracks: specs,
        });

        // Emit each stream's already-fetched first fragment's samples now —
        // see the module doc's "Discovering each stream's wire track id".
        for (global_id, bytes) in first_emits {
            self.emit_fragment_samples(global_id, &bytes)?;
        }
        self.pump_fragment_fetches();
        Ok(())
    }

    fn emit_fragment_samples(&mut self, global_track_id: u32, bytes: &[u8]) -> Result<()> {
        let Phase::Live(live) = &self.phase else {
            return Ok(());
        };
        let Some(stream) = live
            .streams
            .iter()
            .find(|s| s.global_track_id == global_track_id)
        else {
            return Ok(());
        };
        let mut combined = Vec::with_capacity(stream.init_bytes.len() + bytes.len());
        combined.extend_from_slice(&stream.init_bytes);
        combined.extend_from_slice(bytes);
        let local_track_id = stream.local_track_id;
        let media = Fmp4Demux::new().unpackage(combined.as_slice())?;
        for track in media.tracks {
            if track.spec.track_id == local_track_id {
                for sample in track.samples {
                    self.pending_events.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id: global_track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
            }
        }
        Ok(())
    }

    fn pump_fragment_fetches(&mut self) {
        let Phase::Live(live) = &mut self.phase else {
            return;
        };
        let is_live = live.is_live;
        let manifest_url = live.manifest_url.clone();
        for (i, stream) in live.streams.iter_mut().enumerate() {
            if stream.in_flight {
                continue;
            }
            let Some((t, d)) = stream.plan.pop_front() else {
                continue;
            };
            let rel = stream.stream.resolve_fragment_url(stream.bitrate, t);
            let Ok(url) = manifest_url.join(&rel) else {
                continue;
            };
            stream.in_flight = true;
            self.pending_requests
                .push_back(SmoothAction::FetchFragment {
                    stream: StreamIdx(i),
                    t,
                    d,
                    url: url.to_string(),
                    tolerate_404: is_live,
                });
        }
    }

    fn on_fragment(&mut self, idx: StreamIdx, bytes: &[u8]) -> Result<()> {
        let global_id = {
            let Phase::Live(live) = &mut self.phase else {
                return Ok(());
            };
            let Some(stream) = live.streams.get_mut(idx.0) else {
                return Ok(());
            };
            stream.in_flight = false;
            stream.global_track_id
        };
        if fragment_looks_encrypted(bytes) {
            return Err(MultimuxError::Encrypted {
                reason: "smooth-pull: fragment carries PIFF/CENC sample-encryption boxes — \
                         decrypting Smooth-protected content is not supported"
                    .into(),
            });
        }
        self.emit_fragment_samples(global_id, bytes)?;
        self.pump_fragment_fetches();
        Ok(())
    }
}

impl Stage for SmoothIngestSession {
    type In<'a> = (SmoothResourceId, &'a [u8]);
    type Out = SessionEvent;
    type Error = MultimuxError;

    fn feed(&mut self, (id, bytes): (SmoothResourceId, &[u8]), now: Timestamp) -> Result<()> {
        let was_manifest = matches!(id, SmoothResourceId::Manifest);
        match id {
            SmoothResourceId::Manifest => self.on_manifest(bytes)?,
            SmoothResourceId::FirstFragment(idx) => self.on_first_fragment(idx, bytes)?,
            SmoothResourceId::Fragment(idx, _) => self.on_fragment(idx, bytes)?,
        }
        if let Phase::Live(live) = &mut self.phase {
            // See `dash_pull`'s identical comment: the *initial* manifest
            // arrives while still `AwaitingFirstFragments`, so the first feed
            // that reaches `Live` must stamp the clock too, or the first live
            // refresh is overdue the instant the session goes live.
            if was_manifest || live.last_manifest_fetch == Timestamp::ZERO {
                live.last_manifest_fetch = now;
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending_events.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        let Phase::Live(live) = &self.phase else {
            return None;
        };
        if !live.is_live || live.manifest_refresh_in_flight || !live.all_idle_and_exhausted() {
            return None;
        }
        Some(
            live.last_manifest_fetch
                .saturating_add(MANIFEST_REFRESH_INTERVAL),
        )
    }

    fn on_deadline(&mut self, now: Timestamp) {
        let Phase::Live(live) = &mut self.phase else {
            return;
        };
        if !live.is_live || live.manifest_refresh_in_flight || !live.all_idle_and_exhausted() {
            return;
        }
        if now
            < live
                .last_manifest_fetch
                .saturating_add(MANIFEST_REFRESH_INTERVAL)
        {
            return;
        }
        live.manifest_refresh_in_flight = true;
        self.pending_requests
            .push_back(SmoothAction::FetchManifest {
                url: live.manifest_url.to_string(),
            });
    }

    fn demand(&self) -> Demand {
        Demand::new(crate::source::MAX_TS_READ)
    }
}

impl IngestSession for SmoothIngestSession {
    type Request = SmoothAction;

    fn poll_transmit(&mut self) -> Option<SmoothAction> {
        self.pending_requests.pop_front()
    }
}

/// Constructs a [`SmoothIngestSession`] — performs **no I/O**.
pub struct SmoothPullDialer {
    manifest_url: Url,
}

impl Dialer for SmoothPullDialer {
    type Session = SmoothIngestSession;
    type Error = MultimuxError;

    fn dial(&mut self) -> Result<SmoothIngestSession> {
        Ok(SmoothIngestSession::new(self.manifest_url.clone()))
    }
}

/// Peeks a fetched fragment's `moof`/`traf[0]`/`tfhd@track_ID` without
/// needing a `moov` — see the module doc's "Discovering each stream's wire
/// track id".
fn discover_moof_track_id(fragment_bytes: &[u8]) -> Result<u32> {
    let mut offset = 0usize;
    while offset + 8 <= fragment_bytes.len() {
        let (bx, consumed) =
            parse_box(&fragment_bytes[offset..]).map_err(|e| MultimuxError::Connect {
                reason: format!("smooth-pull: fragment box parse: {e}"),
            })?;
        if &bx.header.box_type.0 == b"moof" {
            let moof = MovieFragmentBox::parse_body(bx.body)?;
            let traf = moof.traf.first().ok_or_else(|| MultimuxError::Connect {
                reason: "smooth-pull: fragment moof has no traf".into(),
            })?;
            return Ok(traf.tfhd.track_id);
        }
        if consumed == 0 {
            break;
        }
        offset += consumed;
    }
    Err(MultimuxError::Connect {
        reason: "smooth-pull: fragment carries no moof box".into(),
    })
}

/// Coarse, dependency-free tag-boundary text scan for a `<Protection` start
/// tag anywhere in `xml`.
fn manifest_declares_protection(xml: &str) -> bool {
    tag_starts_present(xml, "Protection")
}

fn tag_starts_present(xml: &str, tag: &str) -> bool {
    let needle = format!("<{tag}");
    let bytes = xml.as_bytes();
    let mut search_from = 0usize;
    while let Some(pos) = xml.get(search_from..).and_then(|rest| rest.find(&needle)) {
        let abs = search_from + pos;
        let after = abs + needle.len();
        match bytes.get(after) {
            Some(&c) if c.is_ascii_whitespace() || c == b'>' || c == b'/' => return true,
            None => return true,
            _ => {}
        }
        search_from = after;
    }
    false
}

/// Coarse raw byte-pattern scan for a CENC/PIFF sample-encryption box.
fn fragment_looks_encrypted(bytes: &[u8]) -> bool {
    contains_subslice(bytes, b"senc")
        || contains_subslice(bytes, b"saiz")
        || contains_subslice(bytes, b"saio")
        || contains_subslice(bytes, &PIFF_SAMPLE_ENCRYPTION_UUID)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

fn status_error(what: &str, status: StatusCode) -> MultimuxError {
    if status == StatusCode::UNAUTHORIZED {
        MultimuxError::Auth {
            reason: format!("smooth-pull {what}: {status}"),
        }
    } else {
        MultimuxError::Connect {
            reason: format!("smooth-pull {what}: HTTP {status}"),
        }
    }
}

enum FetchOutcome {
    Bytes(Vec<u8>),
    NotReady,
}

async fn fetch_one(
    client: &HttpClient,
    url: &str,
    creds: Option<&Credentials>,
    what: &str,
    tolerate_404: bool,
) -> Result<FetchOutcome> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if tolerate_404 && status == StatusCode::NOT_FOUND {
        return Ok(FetchOutcome::NotReady);
    }
    if !status.is_success() {
        return Err(status_error(what, status));
    }
    response
        .bytes()
        .await
        .map(|b| FetchOutcome::Bytes(b.to_vec()))
        .map_err(|e| MultimuxError::Connect {
            reason: format!("smooth-pull {what} read: {e}"),
        })
}

fn build_client(route: &SmoothPullRoute) -> Result<(HttpClient, Url, Option<Credentials>)> {
    let parsed = Url::parse(&route.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad Smooth-pull URL {}: {e}",
            crate::redact::redact_url(&route.url)
        ),
    })?;
    let credentials = resolve_credentials(route.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;
    let http = HttpClient::builder()
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    Ok((http, clean_url, credentials))
}

struct JoinedFetch {
    id: SmoothResourceId,
    t: u64,
    d: u64,
    url: String,
    tolerate_404: bool,
    outcome: Result<FetchOutcome>,
}

#[allow(clippy::too_many_arguments)]
fn spawn_fetch(
    inflight: &mut JoinSet<JoinedFetch>,
    http: HttpClient,
    creds: Option<Credentials>,
    id: SmoothResourceId,
    t: u64,
    d: u64,
    url: String,
    what: &'static str,
    tolerate_404: bool,
    read_timeout: Duration,
    delay: Duration,
) {
    inflight.spawn(async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let outcome = tokio::time::timeout(
            read_timeout,
            fetch_one(&http, &url, creds.as_ref(), what, tolerate_404),
        )
        .await
        .unwrap_or_else(|_| {
            Err(MultimuxError::Connect {
                reason: format!("smooth-pull {what} ({id:?}) read exceeded {read_timeout:?}"),
            })
        });
        JoinedFetch {
            id,
            t,
            d,
            url,
            tolerate_404,
            outcome,
        }
    });
}

/// Turns a driver that has reached a terminal [`HealthState`] into this
/// crate's own `Result`, moving the concrete session error out via
/// [`media_plane::ingress::IngestDriver::into_health`].
///
/// **This is why a pull drive loop must check `health()` after every feed at
/// all**: `IngestDriver::feed` records a session error in `health` and
/// returns `()`, so a loop that only ever calls `feed` never observes it. A
/// `smooth_pull` session rejecting a PlayReady-protected manifest, or an
/// `hls_pull` session rejecting a malformed playlist, would otherwise leave
/// the loop spinning against a session that can never make progress.
fn terminal_result<S>(driver: media_plane::ingress::IngestDriver<S>, what: &str) -> Result<()>
where
    S: media_plane::ingress::IngestSession<Error = MultimuxError>,
{
    match driver.into_health() {
        media_plane::ingress::HealthState::Failed(e) => Err(e),
        media_plane::ingress::HealthState::HandshakeTimedOut { deadline } => {
            Err(MultimuxError::Connect {
                reason: format!("{what}: handshake deadline {deadline:?} passed"),
            })
        }
        // `Ended` is a clean finish; the two running states are unreachable
        // here (callers only call this once `health().is_running()` is false)
        // but map to `Ok` rather than panicking on a future variant.
        _ => Ok(()),
    }
}

/// Drives `route` to completion — see `dash_pull::run_dash_pull`'s doc for
/// the shared shape (dial → poll_transmit → fetch → feed, bounded fan-out,
/// no session-internal clock/sleep).
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration.
pub async fn run_smooth_pull(
    route: &SmoothPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> Result<()> {
    let (http, clean_url, credentials) = build_client(route)?;
    let mut dialer = SmoothPullDialer {
        manifest_url: clean_url,
    };
    let mut driver = run_dial(
        &mut dialer,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    )?;

    let read_timeout = route.timeouts.read;
    let mut backlog: VecDeque<SmoothAction> = VecDeque::new();
    let mut inflight: JoinSet<JoinedFetch> = JoinSet::new();
    let start = std::time::Instant::now();
    let mut published = std::collections::HashSet::new();
    let mut segmenters = std::collections::HashMap::new();

    loop {
        while let Some(action) = driver.poll_transmit() {
            backlog.push_back(action);
        }

        while may_spawn_fetch(inflight.len()) {
            let Some(action) = backlog.pop_front() else {
                break;
            };
            match action {
                SmoothAction::FetchManifest { url } => spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    SmoothResourceId::Manifest,
                    0,
                    0,
                    url,
                    "manifest",
                    false,
                    read_timeout,
                    Duration::ZERO,
                ),
                SmoothAction::FetchFirstFragment { stream, url } => spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    SmoothResourceId::FirstFragment(stream),
                    0,
                    0,
                    url,
                    "first fragment",
                    false,
                    read_timeout,
                    Duration::ZERO,
                ),
                SmoothAction::FetchFragment {
                    stream,
                    t,
                    d,
                    url,
                    tolerate_404,
                } => spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    SmoothResourceId::Fragment(stream, t),
                    t,
                    d,
                    url,
                    "fragment",
                    tolerate_404,
                    read_timeout,
                    Duration::ZERO,
                ),
            }
        }

        if inflight.is_empty() {
            if driver.session().ended() {
                driver.finish();
                crate::source::segment::drive_program_segmenters(
                    &driver,
                    route_handle,
                    &mut segmenters,
                );
                return terminal_result(driver, "smooth-pull");
            }
            match driver.next_deadline() {
                Some(deadline) => {
                    let now = Timestamp::from_instant(start, std::time::Instant::now());
                    if now < deadline {
                        tokio::time::sleep(deadline.saturating_sub(now)).await;
                    }
                    let now = Timestamp::from_instant(start, std::time::Instant::now());
                    driver.on_deadline(now);
                    crate::source::report_driver_progress(&driver, route_handle, &mut published);
                    crate::source::segment::drive_program_segmenters(
                        &driver,
                        route_handle,
                        &mut segmenters,
                    );
                }
                // See `dash_pull`'s identical arm.
                None => tokio::time::sleep(IDLE_POLL_INTERVAL).await,
            }
            continue;
        }

        let joined = inflight.join_next().await;
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        match joined {
            Some(Ok(JoinedFetch {
                id,
                outcome: Ok(FetchOutcome::Bytes(bytes)),
                ..
            })) => {
                driver.feed((id, bytes.as_slice()), now);
                crate::source::report_driver_progress(&driver, route_handle, &mut published);
                crate::source::segment::drive_program_segmenters(
                    &driver,
                    route_handle,
                    &mut segmenters,
                );
            }
            Some(Ok(JoinedFetch {
                id: SmoothResourceId::Fragment(stream, _),
                t,
                d,
                url,
                tolerate_404,
                outcome: Ok(FetchOutcome::NotReady),
            })) => {
                spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    SmoothResourceId::Fragment(stream, t),
                    t,
                    d,
                    url,
                    "fragment",
                    tolerate_404,
                    read_timeout,
                    FRAGMENT_RETRY_DELAY,
                );
            }
            Some(Ok(JoinedFetch {
                outcome: Ok(FetchOutcome::NotReady),
                ..
            })) => {
                // Only fragment fetches are ever tolerant of 404.
            }
            Some(Ok(JoinedFetch {
                outcome: Err(e), ..
            })) => return Err(e),
            Some(Err(join_err)) => {
                return Err(MultimuxError::Connect {
                    reason: format!("smooth-pull: fetch task failed: {join_err}"),
                });
            }
            None => unreachable!("checked inflight.is_empty() above"),
        }

        if !driver.health().is_running() {
            // The feed above drove the session terminal (a rejected
            // playlist/manifest/resource) — see `terminal_result`. Health is
            // already terminal here, so this call's internal terminal-health
            // check flushes every program's trailing partial segment.
            crate::source::segment::drive_program_segmenters(
                &driver,
                route_handle,
                &mut segmenters,
            );
            return terminal_result(driver, "smooth-pull");
        }

        if driver.session().ended() {
            driver.finish();
            crate::source::segment::drive_program_segmenters(
                &driver,
                route_handle,
                &mut segmenters,
            );
            return terminal_result(driver, "smooth-pull");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::{Path as AxumPath, State};
    use axum::http::StatusCode as AxumStatusCode;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use broadcast_common::Package;
    use media_plane::ingress::HandshakePolicy;
    use media_plane::trunk::{SampleCursor, SampleCursorItem, TrunkConfig};
    use std::collections::HashMap;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use transmux::pipeline::{CodecConfig, TrackSpec};
    use transmux::{Media, SmoothOutput, SmoothPackager, TsDemux};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

    fn drain(cursor: &mut SampleCursor) -> usize {
        let mut n = 0;
        while let Some(item) = cursor.poll() {
            if matches!(item, SampleCursorItem::Timed { .. }) {
                n += 1;
            }
        }
        n
    }

    fn fixture_ts_path() -> std::path::PathBuf {
        std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/ts/h264_aac.ts"
        ))
    }

    fn build_smooth_output() -> (Media, SmoothOutput) {
        let ts = std::fs::read(fixture_ts_path()).expect("h264_aac.ts fixture must exist");
        let media = TsDemux::new()
            .unpackage(ts.as_slice())
            .expect("demux h264_aac.ts");
        let mut pkg = SmoothPackager::default();
        let out = pkg.package(&media).expect("package Smooth");
        (media, out)
    }

    fn video_track_id(media: &Media) -> u32 {
        media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("video track")
            .spec
            .track_id
    }

    fn audio_track_id(media: &Media) -> u32 {
        media
            .tracks
            .iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
            .expect("audio track")
            .spec
            .track_id
    }

    fn parse_kind_time(path: &str) -> Option<(String, u64)> {
        let frag_part = path.split("Fragments(").nth(1)?;
        let inner = frag_part.strip_suffix(')')?;
        let (kind, time_str) = inner.split_once('=')?;
        Some((kind.to_string(), time_str.parse().ok()?))
    }

    #[derive(Clone)]
    struct FixtureState {
        manifest: Arc<String>,
        fragments: Arc<HashMap<(String, u64), Vec<u8>>>,
        stall: Option<(String, u64)>,
    }

    async fn fixture_handler(
        AxumPath(path): AxumPath<String>,
        State(state): State<FixtureState>,
    ) -> AxumResponse {
        if path == "Manifest" {
            return (*state.manifest).clone().into_response();
        }
        let Some((kind, time)) = parse_kind_time(&path) else {
            return AxumStatusCode::NOT_FOUND.into_response();
        };
        if state.stall.as_ref() == Some(&(kind.clone(), time)) {
            std::future::pending::<()>().await;
            unreachable!("pending future never resolves");
        }
        match state.fragments.get(&(kind, time)) {
            Some(bytes) => bytes.clone().into_response(),
            None => AxumStatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn start_fixture_server(
        stall: Option<(&str, u64)>,
    ) -> (String, tokio::task::JoinHandle<()>, Media) {
        let (media, out) = build_smooth_output();
        let video_id = video_track_id(&media);
        let audio_id = audio_track_id(&media);

        let mut fragments = HashMap::new();
        for frag in &out.fragments {
            let kind = if frag.track_id == video_id {
                "video"
            } else if frag.track_id == audio_id {
                "audio"
            } else {
                continue;
            };
            fragments.insert((kind.to_string(), frag.start_time), frag.data.clone());
        }

        let state = FixtureState {
            manifest: Arc::new(out.manifest.clone()),
            fragments: Arc::new(fragments),
            stall: stall.map(|(k, t)| (k.to_string(), t)),
        };
        let app = Router::new()
            .route("/*path", get(fixture_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/Manifest"), server, media)
    }

    async fn start_manifest_only_server(
        manifest: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(
            AxumPath(path): AxumPath<String>,
            State(manifest): State<&'static str>,
        ) -> AxumResponse {
            if path == "Manifest" {
                manifest.into_response()
            } else {
                AxumStatusCode::NOT_FOUND.into_response()
            }
        }
        let app = Router::new()
            .route("/*path", get(handler))
            .with_state(manifest);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/Manifest"), server)
    }

    fn oracle_sample_count(
        media: &Media,
        out: &SmoothOutput,
        track_id: u32,
        timescale: u32,
    ) -> usize {
        let track = media
            .tracks
            .iter()
            .find(|t| t.spec.track_id == track_id)
            .unwrap();
        let stream_type = match track.spec.config {
            CodecConfig::Avc { .. } => StreamType::Video,
            CodecConfig::Aac { .. } => StreamType::Audio,
            _ => panic!("unexpected codec"),
        };
        let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");
        let si = manifest
            .streams
            .iter()
            .find(|s| s.stream_type == stream_type)
            .expect("StreamIndex");
        let quality = &si.qualities[0];
        let local_spec =
            track_spec_from_quality_level(track_id, timescale, stream_type, quality).expect("spec");
        let init = build_init_segment(std::slice::from_ref(&local_spec), SYNTHETIC_MOVIE_TIMESCALE)
            .expect("init");

        let mut total = 0usize;
        for frag in out.fragments.iter().filter(|f| f.track_id == track_id) {
            let mut combined = init.clone();
            combined.extend_from_slice(&frag.data);
            let demuxed = Fmp4Demux::new()
                .unpackage(combined.as_slice())
                .expect("oracle demux");
            total += demuxed
                .tracks
                .iter()
                .map(|t| t.samples.len())
                .sum::<usize>();
        }
        total
    }

    /// Drives the raw [`SmoothIngestSession`] over real HTTP, returning the
    /// `TrackSpec`s it announced and a per-`track_id` `SessionEvent::Sample`
    /// count.
    ///
    /// Counts `SessionEvent`s rather than `SampleCursor` items for the same
    /// reason `hls_pull`'s equivalent helper does: `Trunk::subscribe()` starts
    /// from *now*, and this session emits `NewProgram` **and** every stream's
    /// already-fetched first fragment's samples inside the same
    /// `finish_awaiting_first_fragments` — i.e. the same `feed` — so no cursor
    /// can exist in time to see that first batch. `Trunk` arrival is asserted
    /// separately, below.
    async fn drive_session_and_count(
        route: &SmoothPullRoute,
    ) -> Result<(Vec<TrackSpec>, HashMap<u32, usize>)> {
        let (http, clean_url, credentials) = build_client(route)?;
        let mut session = SmoothIngestSession::new(clean_url);
        let mut backlog: VecDeque<SmoothAction> = VecDeque::new();
        let mut specs: Vec<TrackSpec> = Vec::new();
        let mut per_track: HashMap<u32, usize> = HashMap::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

        loop {
            while let Some(a) = session.poll_transmit() {
                backlog.push_back(a);
            }
            while let Some(event) = session.poll() {
                match event {
                    SessionEvent::NewProgram { tracks, .. } => specs = tracks,
                    SessionEvent::Sample { track_id, .. } => {
                        *per_track.entry(track_id).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
            let Some(action) = backlog.pop_front() else {
                if session.ended() || tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            };
            let now = Timestamp::from_nanos(0);
            match action {
                SmoothAction::FetchManifest { url } => {
                    if let FetchOutcome::Bytes(b) =
                        fetch_one(&http, &url, credentials.as_ref(), "manifest", false).await?
                    {
                        session.feed((SmoothResourceId::Manifest, b.as_slice()), now)?;
                    }
                }
                SmoothAction::FetchFirstFragment { stream, url } => {
                    if let FetchOutcome::Bytes(b) =
                        fetch_one(&http, &url, credentials.as_ref(), "first fragment", false)
                            .await?
                    {
                        session
                            .feed((SmoothResourceId::FirstFragment(stream), b.as_slice()), now)?;
                    }
                }
                SmoothAction::FetchFragment {
                    stream,
                    t,
                    url,
                    tolerate_404,
                    ..
                } => {
                    if let FetchOutcome::Bytes(b) =
                        fetch_one(&http, &url, credentials.as_ref(), "fragment", tolerate_404)
                            .await?
                    {
                        session.feed((SmoothResourceId::Fragment(stream, t), b.as_slice()), now)?;
                    }
                }
            }
        }
        Ok((specs, per_track))
    }

    /// Biting loopback test: a real axum server serves a runtime-generated
    /// Smooth manifest + fragments; asserts the session resolves BOTH the AVC
    /// and AAC `TrackSpec`s (from `CodecPrivateData` alone — no init segment
    /// ever crossed the wire) with unique remapped track ids, and produces
    /// **exactly** the independently-demuxed oracle's per-track sample counts
    /// — the #758-lesson "audio silently dropped" class of bug this guards
    /// against.
    ///
    /// MUTATION-CHECKED: dropping the `Fmp4Demux::unpackage`/emit in
    /// `emit_fragment_samples`, or never advancing `StreamState::plan` in
    /// `pump_fragment_fetches`, makes the counts stay `0`; collapsing the
    /// local->global remap makes `per_track.len()` go 2 -> 1.
    #[tokio::test]
    async fn loopback_smooth_pull_resolves_both_tracks_and_matches_oracle_sample_count() {
        let (url, server, media) = start_fixture_server(None).await;
        let (_media2, out) = build_smooth_output();

        let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");
        let audio_timescale = manifest
            .streams
            .iter()
            .find(|s| s.stream_type == StreamType::Audio)
            .expect("audio StreamIndex")
            .qualities[0]
            .sampling_rate
            .expect("audio SamplingRate");
        let want_video = oracle_sample_count(
            &media,
            &out,
            video_track_id(&media),
            transmux::VIDEO_CLOCK_RATE,
        );
        let want_audio = oracle_sample_count(&media, &out, audio_track_id(&media), audio_timescale);
        assert!(
            want_video > 0 && want_audio > 0,
            "sanity: fixture must carry real samples for both streams"
        );

        let route = SmoothPullRoute::new("smooth-cam", url);
        let (specs, per_track) =
            tokio::time::timeout(Duration::from_secs(20), drive_session_and_count(&route))
                .await
                .expect("drive timed out")
                .expect("drive");

        assert_eq!(specs.len(), 2, "one video + one audio track: {specs:?}");
        let mut ids: Vec<u32> = specs.iter().map(|s| s.track_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 2, "track ids must be unique global ids");

        let video_global = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
            .expect("an AVC track")
            .track_id;
        let audio_global = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Aac { .. }))
            .expect("an AAC track")
            .track_id;
        assert_eq!(
            per_track.len(),
            2,
            "samples must land on exactly 2 distinct track ids: {per_track:?}"
        );
        assert_eq!(
            per_track.get(&video_global).copied().unwrap_or(0),
            want_video,
            "video sample count must match the independent oracle exactly"
        );
        assert_eq!(
            per_track.get(&audio_global).copied().unwrap_or(0),
            want_audio,
            "audio sample count must match the independent oracle exactly"
        );

        server.abort();
    }

    /// The `Trunk`-side counterpart to [`drive_session_and_count`]: drives the
    /// same route through a real [`media_plane::ingress::IngestDriver`] and
    /// asserts real samples land on a real [`SampleCursor`]. `> 0`, not an
    /// exact count, for the reason that helper's doc gives.
    #[tokio::test]
    async fn samples_reach_the_trunk_through_the_ingest_driver() {
        let (url, server, _media) = start_fixture_server(None).await;
        let route = SmoothPullRoute::new("smooth-cam", url);

        let (http, clean_url, credentials) = build_client(&route).expect("build client");
        let mut dialer = SmoothPullDialer {
            manifest_url: clean_url,
        };
        let mut driver = run_dial(
            &mut dialer,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        )
        .expect("dial");

        let mut backlog: VecDeque<SmoothAction> = VecDeque::new();
        let mut cursor: Option<SampleCursor> = None;
        let mut total = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            while let Some(a) = driver.poll_transmit() {
                backlog.push_back(a);
            }
            let Some(action) = backlog.pop_front() else {
                if driver.session().ended() || tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            };
            let now = Timestamp::from_nanos(0);
            match action {
                SmoothAction::FetchManifest { url } => {
                    if let FetchOutcome::Bytes(b) =
                        fetch_one(&http, &url, credentials.as_ref(), "manifest", false)
                            .await
                            .expect("fetch")
                    {
                        driver.feed((SmoothResourceId::Manifest, b.as_slice()), now);
                    }
                }
                SmoothAction::FetchFirstFragment { stream, url } => {
                    if let FetchOutcome::Bytes(b) =
                        fetch_one(&http, &url, credentials.as_ref(), "first fragment", false)
                            .await
                            .expect("fetch")
                    {
                        driver.feed((SmoothResourceId::FirstFragment(stream), b.as_slice()), now);
                    }
                }
                SmoothAction::FetchFragment {
                    stream,
                    t,
                    url,
                    tolerate_404,
                    ..
                } => {
                    if let FetchOutcome::Bytes(b) =
                        fetch_one(&http, &url, credentials.as_ref(), "fragment", tolerate_404)
                            .await
                            .expect("fetch")
                    {
                        driver.feed((SmoothResourceId::Fragment(stream, t), b.as_slice()), now);
                    }
                }
            }
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                total += drain(c);
            }
        }

        assert!(
            total > 0,
            "real samples must reach the Trunk through IngestDriver, got {total}"
        );
        server.abort();
    }

    /// The full `run_smooth_pull` drive loop against the same fixture: a
    /// static (non-`IsLive`) manifest must end cleanly.
    #[tokio::test]
    async fn run_smooth_pull_completes_cleanly_on_a_static_manifest() {
        let (url, server, _media) = start_fixture_server(None).await;
        let route = SmoothPullRoute::new("smooth-cam", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("run_smooth_pull must not hang");
        assert!(
            result.is_ok(),
            "a static manifest must end cleanly: {result:?}"
        );
        server.abort();
    }

    /// Biting test (issue #663 P5 / #738-#739 ingest-hardening lesson): a
    /// server that resolves the manifest + both streams' first fragments but
    /// then stalls on a later fragment must fail within
    /// `IngestTimeouts::read`, not hang forever.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_on_a_later_fragment() {
        let (media, out) = build_smooth_output();
        let audio_id = audio_track_id(&media);
        let audio_frags: Vec<_> = out
            .fragments
            .iter()
            .filter(|f| f.track_id == audio_id)
            .collect();
        assert!(
            audio_frags.len() >= 2,
            "fixture must produce at least 2 audio fragments: got {}",
            audio_frags.len()
        );
        let stall_time = audio_frags[1].start_time;

        let (url, server, _media2) = start_fixture_server(Some(("audio", stall_time))).await;
        let route = SmoothPullRoute::new("smooth-stalled", url).with_timeouts(IngestTimeouts {
            connect: Duration::from_secs(5),
            read: Duration::from_millis(150),
        });
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect(
            "run_smooth_pull must return on its own via IngestTimeouts::read, not hang until \
             this test's own backstop timeout",
        );
        assert!(
            result.is_err(),
            "a server that stalls on a later fragment fetch must fail run_smooth_pull, not \
             hang forever"
        );
        server.abort();
    }

    /// Biting test: a manifest declaring `<Protection>` must fail with a
    /// clear typed error, never silently succeed into a session that would
    /// go on to demux garbage (encrypted) sample bytes.
    #[tokio::test]
    async fn encrypted_manifest_with_protection_element_fails_with_clear_typed_error() {
        const MANIFEST: &str = r#"<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" Duration="10000000" TimeScale="10000000">
            <Protection>
                <ProtectionHeader SystemID="9A04F079-9840-4286-AB92-E65BE0885F95">BASE64==</ProtectionHeader>
            </Protection>
            <StreamIndex Type="video" Url="Fragments(video={start time})">
                <QualityLevel Index="0" Bitrate="1" FourCC="H264" CodecPrivateData="000000016742C01EAB0000000168CE3C80"/>
                <c t="0" d="10000000"/>
            </StreamIndex>
        </SmoothStreamingMedia>"#;
        let (url, server) = start_manifest_only_server(MANIFEST).await;
        let route = SmoothPullRoute::new("smooth-drm", url);
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("must not hang");
        match result {
            Err(MultimuxError::Encrypted { .. }) => {}
            Ok(_) => panic!("expected MultimuxError::Encrypted, got Ok(())"),
            Err(other) => panic!("expected MultimuxError::Encrypted, got {other:?}"),
        }
        server.abort();
    }

    // -- pure-function unit tests --------------------------------------------

    #[test]
    fn manifest_declares_protection_matches_tag_boundary_not_substring() {
        assert!(manifest_declares_protection(
            "<SmoothStreamingMedia><Protection></Protection></SmoothStreamingMedia>"
        ));
        assert!(manifest_declares_protection(
            "<SmoothStreamingMedia><Protection/></SmoothStreamingMedia>"
        ));
        assert!(!manifest_declares_protection(
            "<SmoothStreamingMedia><ProtectionFoo/></SmoothStreamingMedia>"
        ));
        assert!(!manifest_declares_protection(
            "<SmoothStreamingMedia></SmoothStreamingMedia>"
        ));
    }

    #[test]
    fn fragment_looks_encrypted_detects_senc_and_piff_uuid_not_plain_bytes() {
        assert!(fragment_looks_encrypted(b"....senc...."));
        assert!(fragment_looks_encrypted(b"....saiz...."));
        assert!(fragment_looks_encrypted(b"....saio...."));
        assert!(fragment_looks_encrypted(&PIFF_SAMPLE_ENCRYPTION_UUID));
        assert!(!fragment_looks_encrypted(b"stypmoofmdattraftfhdtrun"));
    }

    #[test]
    fn discover_moof_track_id_recovers_real_fixture_track_ids() {
        let (media, out) = build_smooth_output();
        let video_id = video_track_id(&media);
        let audio_id = audio_track_id(&media);

        let video_frag = out
            .fragments
            .iter()
            .find(|f| f.track_id == video_id)
            .unwrap();
        assert_eq!(discover_moof_track_id(&video_frag.data).unwrap(), video_id);

        let audio_frag = out
            .fragments
            .iter()
            .find(|f| f.track_id == audio_id)
            .unwrap();
        assert_eq!(discover_moof_track_id(&audio_frag.data).unwrap(), audio_id);
    }

    #[test]
    fn discover_moof_track_id_errors_not_panics_on_garbage() {
        assert!(discover_moof_track_id(b"not a fragment at all").is_err());
        assert!(discover_moof_track_id(&[]).is_err());
    }
}
