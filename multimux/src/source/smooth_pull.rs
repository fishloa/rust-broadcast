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
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::{Client as HttpClient, StatusCode};
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
use crate::source::{IngestTimeouts, Source};

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

/// How many consecutive tolerated-`404` retries of one live-edge fragment
/// [`run_smooth_pull`] makes before abandoning it (audit W14a). Mirrors
/// `dash_pull`'s own bound: an encoder restart that resets numbering, or a
/// fragment that never appears, must not retry forever and pin the stream
/// `in_flight` so the manifest is never refreshed.
const MAX_TOLERATED_404_ATTEMPTS: u32 = 240;

/// How many consecutive manifest refreshes may lack a `StreamIndex` matching
/// a live stream's full identity before that stream is abandoned (audit W14e).
/// A one-off miss (a transient encoder hiccup) is tolerated; a stream that
/// never reappears stops the session from stalling silently.
const MAX_REFRESH_MISSES: u32 = 5;

/// The PIFF "UUID Sample Encryption Box" extended type
/// (`A2394F52-5A9B-4F14-A244-6C427C648DF4`).
const PIFF_SAMPLE_ENCRYPTION_UUID: [u8; 16] = [
    0xA2, 0x39, 0x4F, 0x52, 0x5A, 0x9B, 0x4F, 0x14, 0xA2, 0x44, 0x6C, 0x42, 0x7C, 0x64, 0x8D, 0xF4,
];

/// Local name of the Smooth client Manifest's content-protection element
/// (PlayReady/PIFF sample encryption).
const PROTECTION_ELEMENT: &str = "Protection";

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
#[non_exhaustive]
pub enum SmoothResourceId {
    Manifest,
    FirstFragment(StreamIdx),
    Fragment(StreamIdx, u64),
    /// The drive loop's explicit "abandon this stream's outstanding fragment"
    /// signal (audit W14): a fragment whose tolerated-`404` retries ran out is
    /// dropped so the stream goes idle and the manifest refreshes. An explicit
    /// variant, not an empty body through `Fragment`, so a genuine empty HTTP
    /// `200` fragment is never mistaken for an abandon.
    AbandonFragment(StreamIdx),
}

/// One unit of IO [`run_smooth_pull`] must perform.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    /// Consecutive manifest refreshes with no `StreamIndex` matching this
    /// stream's full identity. Dropped (abandoned) after
    /// [`MAX_REFRESH_MISSES`] — a renamed/retemplated stream stops silently
    /// stalling (audit W14e).
    refresh_misses: u32,
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
                // Streams to drop (their `StreamIndex` vanished from the
                // refreshed manifest); removed after the loop so the index
                // stays valid while iterating.
                let mut abandoned: Vec<StreamIdx> = Vec::new();
                for (stream_idx, stream) in live.streams.iter_mut().enumerate() {
                    // Match the refreshed `StreamIndex` to this stream's own
                    // one by its **full identity** — `Type` *and* `Name`
                    // *and* `Url` template (MS-SSTR §2.2.3) — never by
                    // `Type` alone: with two audio `StreamIndex`es a
                    // type-only `find` returned the *first* audio index for
                    // both, so the second stream adopted the first's URL
                    // template and timeline and fetched the wrong fragments
                    // under a track id that no longer matched (audit run 7,
                    // W14).
                    let Some(found) = manifest
                        .streams
                        .iter()
                        .find(|s| same_stream_index(s, &stream.stream))
                    else {
                        // No StreamIndex with this stream's full identity in
                        // the refreshed manifest: the encoder renamed it or
                        // changed its `Url` template. Count the consecutive
                        // miss and drop the stream once it exceeds
                        // [`MAX_REFRESH_MISSES`], so a renamed stream does
                        // not stall silently for the session's life (audit
                        // W14e).
                        stream.refresh_misses = stream.refresh_misses.saturating_add(1);
                        metrics::counter!(crate::prometheus::PULL_STREAM_REFRESH_MISS_TOTAL)
                            .increment(1);
                        if stream.refresh_misses >= MAX_REFRESH_MISSES {
                            tracing::warn!(
                                name = ?stream.stream.name,
                                stream_type = %stream.stream.stream_type,
                                misses = stream.refresh_misses,
                                "smooth-pull: refreshed manifest has no matching StreamIndex; \
                                 abandoning this stream rather than stalling silently"
                            );
                            abandoned.push(StreamIdx(stream_idx));
                        }
                        continue;
                    };
                    stream.refresh_misses = 0;
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
                // Drop the abandoned streams, highest index first so the
                // remaining indices stay valid.
                abandoned.sort_by_key(|s| std::cmp::Reverse(s.0));
                for idx in abandoned {
                    if idx.0 < live.streams.len() {
                        live.streams.remove(idx.0);
                    }
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
            // Only Video/Audio StreamIndexes are muxed; Text is explicitly
            // out of scope, and `StreamType` is `#[non_exhaustive]` -- any
            // future variant this code doesn't yet know how to mux is
            // skipped the same way rather than reaching the exhaustive
            // Video/Audio match in `finish_awaiting_first_fragments` below.
            if !matches!(stream_type, StreamType::Video | StreamType::Audio) {
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
        match fragment_state(bytes) {
            FragmentState::Clear => {}
            FragmentState::Encrypted => {
                return Err(MultimuxError::Encrypted {
                    reason: format!(
                        "smooth-pull: stream {:?} fragment carries PIFF/CENC sample-encryption \
                         boxes — decrypting Smooth-protected content is not supported",
                        p.stream.name
                    ),
                });
            }
            FragmentState::Undetermined => {
                return Err(MultimuxError::Connect {
                    reason: format!(
                        "smooth-pull: stream {:?} fragment is malformed/truncated; cannot \
                         determine whether it is encrypted",
                        p.stream.name
                    ),
                });
            }
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
                _ => unreachable!(
                    "only Video/Audio StreamIndexes reach here -- \
                     Text and any other stream type are skipped at manifest parse"
                ),
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
                refresh_misses: 0,
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
        match fragment_state(bytes) {
            FragmentState::Clear => {}
            FragmentState::Encrypted => {
                return Err(MultimuxError::Encrypted {
                    reason: "smooth-pull: fragment carries PIFF/CENC sample-encryption boxes — \
                             decrypting Smooth-protected content is not supported"
                        .into(),
                });
            }
            FragmentState::Undetermined => {
                return Err(MultimuxError::Connect {
                    reason: "smooth-pull: fragment is malformed/truncated; cannot determine \
                             whether it is encrypted"
                        .into(),
                });
            }
        }
        self.emit_fragment_samples(global_id, bytes)?;
        self.pump_fragment_fetches();
        Ok(())
    }

    /// Abandon `idx`'s outstanding fragment fetch: clear its in-flight state
    /// and pump the next planned fragment (audit W14). An explicit signal, so
    /// an empty HTTP `200` fragment body is still demuxed normally.
    fn abandon_fragment(&mut self, idx: StreamIdx) {
        let Phase::Live(live) = &mut self.phase else {
            return;
        };
        let Some(stream) = live.streams.get_mut(idx.0) else {
            return;
        };
        stream.in_flight = false;
        self.pump_fragment_fetches();
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
            SmoothResourceId::AbandonFragment(idx) => self.abandon_fragment(idx),
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

/// Whether two `StreamIndex` elements denote the **same** stream across a
/// manifest refresh (audit run 7, W14): matching on `Type` alone collapsed
/// two audio `StreamIndex`es onto the first, so the second adopted the
/// first's URL template and timeline and fetched the wrong fragments under a
/// track id that no longer matched. Identity is `Type` + `Name` + `Url`
/// template (MS-SSTR §2.2.3 — the three attributes that name a stream).
fn same_stream_index(a: &StreamIndex, b: &StreamIndex) -> bool {
    a.stream_type == b.stream_type && a.name == b.name && a.url == b.url
}

/// Whether the manifest carries a `<Protection>` element (PlayReady/PIFF
/// sample encryption) anywhere — matched by local name on a real `quick-xml`
/// pull loop, so a prefixed `<ms:Protection>` counts, while a comment, an
/// attribute value, or a longer name such as `<ProtectionFoo>` does not.
///
/// A manifest the reader cannot get through is reported `false`: the caller
/// goes on to [`SmoothManifest::parse`], which rejects the same malformed input
/// with its own error, so nothing unreadable is ever ingested.
fn manifest_declares_protection(xml: &str) -> bool {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e))
                if e.local_name().as_ref() == PROTECTION_ELEMENT =>
            {
                return true;
            }
            Ok(Event::Eof) | Err(_) => return false,
            Ok(_) => {}
        }
    }
}

/// Whether a fetched fragment carries CENC/PIFF sample-encryption signalling.
///
/// Walks the **box tree** and inspects only the `moof` box and its `traf`
/// children — `senc`/`saiz`/`saio` are `traf` children (ISO/IEC 14496-12:2015
/// §8.8.7/§8.8.8) and the PIFF sample-encryption `uuid` box is a `traf` child
/// too, so they can only legitimately appear there. A byte scan of the whole
/// fragment (the pre-fix behaviour) false-positived on a random four-byte run
/// in clear `mdat` payload.
///
/// **A malformed/truncated fragment is treated as *not* reliably clear**: if
/// the box walk fails *after* a `moof` has been seen (or the top-level walk
/// itself fails mid-fragment), this returns `true` so the caller fails closed
/// rather than silently demuxing bytes that may be encrypted under a
/// truncated structure. Only a fragment whose walk completes with no
/// encryption box anywhere is reported clear.
/// How a fragment's box walk classified it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FragmentState {
    /// No encryption box anywhere; safe to demux.
    Clear,
    /// A `senc`/`saiz`/`saio` (or PIFF `uuid`) box inside a `moof`/`traf`.
    Encrypted,
    /// A malformed/truncated `moof` (or a top-level box that does not parse):
    /// cannot be proven clear, and is not provably encrypted either.
    Undetermined,
}

impl FragmentState {
    #[cfg(test)]
    fn is_clear(self) -> bool {
        matches!(self, FragmentState::Clear)
    }
}

/// Convenience bool for tests: `true` for anything not proven clear
/// (encrypted **or** undetermined) — the fail-closed predicate.
#[cfg(test)]
fn fragment_looks_encrypted(bytes: &[u8]) -> bool {
    !fragment_state(bytes).is_clear()
}

/// Classify a fragment, distinguishing a genuinely encrypted one from a
/// malformed/truncated one (audit W15) so the session's error message is
/// accurate.
fn fragment_state(bytes: &[u8]) -> FragmentState {
    // Walk the top level by peeking each box's 4-byte type *before* parsing
    // it — a `moof` truncated so badly that its own header no longer parses
    // must still fail closed, which a plain `box_iter` error cannot signal
    // (its type is only known once the header parses).
    let mut offset = 0usize;
    let mut saw_moof = false;
    while offset + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0; 4]));
        let box_type = &bytes[offset + 4..offset + 8];
        let is_moof = box_type == b"moof";
        if is_moof {
            saw_moof = true;
        }
        // A truncated box (declared size past the buffer, or < header): if it
        // is a `moof` (or we have already seen one), it cannot be proven
        // clear — fail closed.
        let size_usize = size as usize;
        if size_usize < 8 || offset + size_usize > bytes.len() {
            return if is_moof || saw_moof {
                FragmentState::Undetermined
            } else {
                FragmentState::Clear
            };
        }
        if is_moof {
            match moof_is_encrypted(&bytes[offset + 8..offset + size_usize]) {
                Some(true) => return FragmentState::Encrypted,
                // A truncated `moof` body: cannot prove clear → undetermined.
                None => return FragmentState::Undetermined,
                Some(false) => {}
            }
        }
        offset += size_usize;
    }
    // Trailing bytes too short to be a box header: fine unless they follow a
    // `moof` we could not finish walking.
    if saw_moof && offset < bytes.len() {
        FragmentState::Undetermined
    } else {
        FragmentState::Clear
    }
}

/// Whether a `moof` body's `traf` children carry encryption boxes: `Some(true)`
/// if one is present, `Some(false)` if the walk completed cleanly, `None` if
/// the body is malformed/truncated (cannot be proven clear).
fn moof_is_encrypted(moof_body: &[u8]) -> Option<bool> {
    for result in transmux::box_iter(moof_body) {
        let Ok((child, _)) = result else {
            return None;
        };
        if !child.header.box_type.is(b"traf") {
            continue;
        }
        for result in transmux::box_iter(child.body) {
            let Ok((grandchild, _)) = result else {
                return None;
            };
            let t = grandchild.header.box_type;
            if t.is(b"senc") || t.is(b"saiz") || t.is(b"saio") {
                return Some(true);
            }
            if t.is(b"uuid")
                && grandchild.header.usertype.as_ref() == Some(&PIFF_SAMPLE_ENCRYPTION_UUID)
            {
                return Some(true);
            }
        }
    }
    Some(false)
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

/// What one Smooth fetch observed, before mapping to a [`FetchResult`].
enum FetchOne {
    /// The body bytes.
    Bytes(Vec<u8>),
    /// A tolerated `404`: the live-edge fragment is not ready yet.
    NotReady,
}

/// Fetches one resource, reporting a tolerated `404` as
/// [`FetchOne::NotReady`] (a typed retry signal) rather than an error.
async fn fetch_one_bytes(
    client: &HttpClient,
    url: &str,
    creds: Option<&Credentials>,
    what: &str,
    tolerate_404: bool,
) -> Result<FetchOne> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if tolerate_404 && status == StatusCode::NOT_FOUND {
        return Ok(FetchOne::NotReady);
    }
    if !status.is_success() {
        return Err(status_error(what, status));
    }
    crate::source::read_body_capped(response, crate::source::MAX_HTTP_BODY_BYTES, what)
        .await
        .map(FetchOne::Bytes)
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
        .redirect(crate::source::redirect_policy())
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    Ok((http, clean_url, credentials))
}

/// The scheduler key for one Smooth fetch: everything the join loop needs to
/// feed the result or retry the exact same fragment.
#[derive(Debug, Clone)]
struct SmoothFetch {
    id: SmoothResourceId,
    t: u64,
    d: u64,
    url: String,
    tolerate_404: bool,
    /// How many times this fragment fetch has been attempted (1 on the first
    /// try) — bounds the tolerated-`404` retry loop (audit W14a).
    attempt: u32,
}

/// The scheduler key type for `smooth_pull`'s fetches.
type SmoothPendingFetch = crate::source::pull::PendingFetch<SmoothFetch>;

/// Builds the [`PendingFetch`] for one fetch (after `delay`, for a retry).
/// The delay is applied by [`PullScheduler::pump`], not here.
#[allow(clippy::too_many_arguments)]
fn pending_fetch(
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
    attempt: u32,
) -> SmoothPendingFetch {
    crate::source::pull::PendingFetch {
        delay,
        fut: Box::pin(async move {
            use crate::source::pull::FetchResult;
            let result = match tokio::time::timeout(
                read_timeout,
                fetch_one_bytes(&http, &url, creds.as_ref(), what, tolerate_404),
            )
            .await
            {
                Ok(Ok(FetchOne::Bytes(b))) => FetchResult::Ready(b),
                Ok(Ok(FetchOne::NotReady)) => FetchResult::NotReady(MultimuxError::Connect {
                    reason: format!("smooth-pull {what} ({id:?}) not ready yet"),
                }),
                Ok(Err(e)) => FetchResult::Failed(e),
                Err(_) => FetchResult::Failed(MultimuxError::Connect {
                    reason: format!("smooth-pull {what} ({id:?}) read exceeded {read_timeout:?}"),
                }),
            };
            let key = SmoothFetch {
                id,
                t,
                d,
                url,
                tolerate_404,
                attempt,
            };
            (key, result)
        }),
    }
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
    // The fetch/retry/wait engine (SP6.2) owns the in-flight bound, the retry
    // queue and the idle park; this loop owns only the `Action` translation,
    // the tolerated-`404` retry policy and the `feed`.
    let mut scheduler: crate::source::pull::PullScheduler<SmoothFetch> =
        crate::source::pull::PullScheduler::new(crate::source::MAX_INFLIGHT_FETCHES);
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();

    loop {
        while let Some(action) = driver.poll_transmit() {
            let fetch = match action {
                SmoothAction::FetchManifest { url } => pending_fetch(
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
                    1,
                ),
                SmoothAction::FetchFirstFragment { stream, url } => pending_fetch(
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
                    1,
                ),
                SmoothAction::FetchFragment {
                    stream,
                    t,
                    d,
                    url,
                    tolerate_404,
                } => pending_fetch(
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
                    1,
                ),
            };
            scheduler.push(fetch);
        }

        scheduler.pump();

        if scheduler.is_idle() {
            if driver.session().ended() {
                driver.finish();
                crate::source::advance_route(&driver, route_handle, &mut progress).await;
                return terminal_result(driver, "smooth-pull");
            }
            // A session deadline is the source's own (see `dash_pull`'s
            // identical arm); with none, park briefly via the scheduler.
            match driver.next_deadline() {
                Some(deadline) => {
                    let now = Timestamp::from_instant(start, std::time::Instant::now());
                    if now < deadline {
                        tokio::time::sleep(deadline.saturating_sub(now)).await;
                    }
                    let now = Timestamp::from_instant(start, std::time::Instant::now());
                    driver.on_deadline(now);
                    crate::source::advance_route(&driver, route_handle, &mut progress).await;
                }
                None => {
                    scheduler.next(None, IDLE_POLL_INTERVAL).await;
                }
            }
            continue;
        }

        let joined = scheduler.next(None, IDLE_POLL_INTERVAL).await;
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        match joined {
            Some(crate::source::pull::FetchOutcome::Ready(fetch, bytes)) => {
                driver.feed((fetch.id, bytes.as_slice()), now);
                crate::source::advance_route(&driver, route_handle, &mut progress).await;
            }
            Some(crate::source::pull::FetchOutcome::Failed(_fetch, e)) => {
                // A real fetch failure ends the session.
                return Err(e);
            }
            Some(crate::source::pull::FetchOutcome::NotReady(fetch, e)) => {
                // A tolerated `404` (a live-edge fragment not yet available)
                // is retried directly, without touching the session.
                let SmoothResourceId::Fragment(stream, _) = fetch.id else {
                    // Only fragment fetches are ever tolerant of 404.
                    return Err(e);
                };
                if fetch.attempt >= MAX_TOLERATED_404_ATTEMPTS {
                    // Bound reached: abandon this fragment so the stream goes
                    // idle and the manifest can refresh, rather than retrying
                    // a 404 forever (audit W14a) — the explicit
                    // `AbandonFragment` signal, not an empty body.
                    tracing::warn!(
                        stream = stream.0,
                        t = fetch.t,
                        attempts = fetch.attempt,
                        "smooth-pull: live-edge fragment never became available; abandoning it and refreshing the manifest"
                    );
                    metrics::counter!(crate::prometheus::PULL_FRAGMENT_ABANDONED_TOTAL)
                        .increment(1);
                    driver.feed((SmoothResourceId::AbandonFragment(stream), &[][..]), now);
                    crate::source::advance_route(&driver, route_handle, &mut progress).await;
                } else {
                    scheduler.push_retry(pending_fetch(
                        http.clone(),
                        credentials.clone(),
                        SmoothResourceId::Fragment(stream, fetch.t),
                        fetch.t,
                        fetch.d,
                        fetch.url,
                        "fragment",
                        fetch.tolerate_404,
                        read_timeout,
                        FRAGMENT_RETRY_DELAY,
                        fetch.attempt.saturating_add(1),
                    ));
                }
            }
            Some(crate::source::pull::FetchOutcome::TaskPanic(detail)) => {
                return Err(MultimuxError::Connect {
                    reason: format!("smooth-pull: fetch task failed: {detail}"),
                });
            }
            // Nothing was in flight (a bounded wait elapsed); check the
            // session's health and end condition below.
            None => {}
        }

        if !driver.health().is_running() {
            // The feed above drove the session terminal (a rejected
            // playlist/manifest/resource) — see `terminal_result`. Health is
            // already terminal here, so this call's internal terminal-health
            // check flushes every program's trailing partial segment.
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
            return terminal_result(driver, "smooth-pull");
        }

        if driver.session().ended() {
            driver.finish();
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
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

    fn stream_index(stream_type: StreamType, name: Option<&str>, url: &str) -> StreamIndex {
        StreamIndex {
            stream_type,
            name: name.map(str::to_string),
            subtype: None,
            chunks: None,
            timescale: None,
            url: url.to_string(),
            qualities: Vec::new(),
            chunks_list: Vec::new(),
        }
    }

    /// Audit run 7, W14: a refresh must match a `StreamIndex` by its full
    /// identity, so two audio streams (distinct `Name`, identical `Type` and
    /// default `Url`) do not collapse onto one another.
    #[test]
    fn refresh_matches_stream_index_by_full_identity() {
        let audio_a = stream_index(
            StreamType::Audio,
            Some("audio_eng"),
            "QualityLevels({bitrate})/Fragments(audio={start time})",
        );
        let audio_b = stream_index(
            StreamType::Audio,
            Some("audio_fra"),
            "QualityLevels({bitrate})/Fragments(audio={start time})",
        );
        assert!(
            !same_stream_index(&audio_a, &audio_b),
            "two audio StreamIndexes with different names are different streams"
        );
        assert!(
            same_stream_index(&audio_a, &audio_a.clone()),
            "a stream matches itself across a refresh"
        );
    }

    /// Audit run 7, W14e: the refresh path matches each live stream to its
    /// refreshed `StreamIndex` by full identity. Constructs a two-audio stream
    /// `Live` phase directly, then feeds a refreshed manifest whose audio
    /// `Url` template changed — both streams must still be matched (not
    /// silently dropped), and their plans must keep extending.
    #[test]
    fn refresh_keeps_two_audio_streams_matched_when_template_changes() {
        let url = Url::parse("http://example/Manifest").expect("url");
        let audio_url_a = "QualityLevels({bitrate})/Fragments(audio={start time})";
        let mv = |name: &str, url: &str| StreamState {
            stream: stream_index(StreamType::Audio, Some(name), url),
            bitrate: 1,
            init_bytes: Vec::new(),
            local_track_id: 1,
            global_track_id: 1,
            plan: VecDeque::new(),
            last_time: 0,
            in_flight: false,
            refresh_misses: 0,
        };
        let mut session = SmoothIngestSession::new(url);
        session.phase = Phase::Live(LiveState {
            manifest_url: session.manifest_url.clone(),
            is_live: true,
            last_manifest_fetch: Timestamp::from_nanos(0),
            streams: vec![mv("eng", audio_url_a), mv("fra", audio_url_a)],
            manifest_refresh_in_flight: false,
        });

        // A refreshed manifest with both audio indexes and one extra chunk.
        let refreshed = r#"<?xml version="1.0" encoding="UTF-8"?>
<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" TimeScale="10000000" IsLive="TRUE">
  <StreamIndex Type="audio" Name="eng" Chunks="2" QualityLevels="1" Url="QualityLevels({bitrate})/Fragments(audio={start time})">
    <QualityLevel Index="0" Bitrate="96013" SampleRate="44100" Channels="2" BitsPerSample="16" PacketSize="4" AudioTag="255" FourCC="AACL" CodecPrivateData="1190"/>
    <c t="0" d="20000000"/><c d="20000000"/>
  </StreamIndex>
  <StreamIndex Type="audio" Name="fra" Chunks="3" QualityLevels="1" Url="QualityLevels({bitrate})/Fragments(audio={start time})">
    <QualityLevel Index="0" Bitrate="96013" SampleRate="44100" Channels="2" BitsPerSample="16" PacketSize="4" AudioTag="255" FourCC="AACL" CodecPrivateData="1190"/>
    <c t="0" d="20000000"/><c d="20000000"/><c d="20000000"/>
  </StreamIndex>
</SmoothStreamingMedia>"#;

        session
            .on_manifest(refreshed.as_bytes())
            .expect("refresh must succeed");

        // Both streams matched (not dropped) and each extended its plan.
        let Phase::Live(live) = &session.phase else {
            panic!("still Live");
        };
        assert_eq!(
            live.streams.len(),
            2,
            "both audio streams must survive the refresh"
        );
        // Each stream must have been matched to its OWN StreamIndex: `eng`
        // ends at 40 ms, `fra` at 60 ms. A type-only match would collapse
        // both onto `eng` and give both the same `last_time`.
        let eng = live
            .streams
            .iter()
            .find(|s| s.stream.name.as_deref() == Some("eng"))
            .expect("eng stream");
        let fra = live
            .streams
            .iter()
            .find(|s| s.stream.name.as_deref() == Some("fra"))
            .expect("fra stream");
        assert_eq!(eng.last_time, 40_000_000, "eng extends to its own timeline");
        assert_eq!(fra.last_time, 60_000_000, "fra extends to its own timeline");
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
            .route("/{*path}", get(fixture_handler))
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
            .route("/{*path}", get(handler))
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
        // HANG GUARD (issue #826): ceiling on the whole session-drive loop.
        // Each HTTP fetch (manifest + first fragments + follow-up fragments)
        // completes in ~ms over loopback. Raised to 60s for load tolerance
        // — only job is to fail "never finishes" rather than hang.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);

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
                    let b =
                        fetch_bytes(&http, &url, credentials.as_ref(), "manifest", false).await?;
                    session.feed((SmoothResourceId::Manifest, b.as_slice()), now)?;
                }
                SmoothAction::FetchFirstFragment { stream, url } => {
                    let b = fetch_bytes(&http, &url, credentials.as_ref(), "first fragment", false)
                        .await?;
                    session.feed((SmoothResourceId::FirstFragment(stream), b.as_slice()), now)?;
                }
                SmoothAction::FetchFragment {
                    stream,
                    t,
                    url,
                    tolerate_404,
                    ..
                } => {
                    let b =
                        fetch_bytes(&http, &url, credentials.as_ref(), "fragment", tolerate_404)
                            .await?;
                    session.feed((SmoothResourceId::Fragment(stream, t), b.as_slice()), now)?;
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
            // HANG GUARD (issue #826): backstop around the session drive.
            // The session fetches manifest + fragments from a real axum
            // server; each fetch ~ms, total drive time bounded by the
            // content's fragment count. Raised to 60s for load tolerance
            // since it only exists to fail "never finishes" rather than hang.
            tokio::time::timeout(Duration::from_secs(60), drive_session_and_count(&route))
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
        // HANG GUARD (issue #826): ceiling on the Trunk-drive loop, same
        // reasoning as `drive_session_and_count`'s deadline.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
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
                    let b = fetch_bytes(&http, &url, credentials.as_ref(), "manifest", false)
                        .await
                        .expect("fetch");
                    driver.feed((SmoothResourceId::Manifest, b.as_slice()), now);
                }
                SmoothAction::FetchFirstFragment { stream, url } => {
                    let b = fetch_bytes(&http, &url, credentials.as_ref(), "first fragment", false)
                        .await
                        .expect("fetch");
                    driver.feed((SmoothResourceId::FirstFragment(stream), b.as_slice()), now);
                }
                SmoothAction::FetchFragment {
                    stream,
                    t,
                    url,
                    tolerate_404,
                    ..
                } => {
                    let b =
                        fetch_bytes(&http, &url, credentials.as_ref(), "fragment", tolerate_404)
                            .await
                            .expect("fetch");
                    driver.feed((SmoothResourceId::Fragment(stream, t), b.as_slice()), now);
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
        // HANG GUARD (issue #826): backstop around `run_smooth_pull`.
        // The static manifest is exhausted by the session's fragment-fetch
        // loop; `run_smooth_pull` returns on its own once complete.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
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
            read: Duration::from_secs(2),
        });
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // DISCRIMINATOR (issue #826): must prove the operation returns
        // through the CONFIGURED read timeout, not via any longer system
        // default. Gap widened: configured read timeout raised from 150ms
        // to 2s, assertion window raised from 10s to 15s — 15s is still
        // well below any plausible fallback. MUTATION CHECKED: inflating
        // the configured read timeout to 30s makes the 15s outer timeout
        // fire first, producing `Elapsed`.
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_smooth_pull(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect(
            "run_smooth_pull must return via IngestTimeouts::read, not via \
             the outer assertion window",
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
        // HANG GUARD (issue #826): backstop around `run_smooth_pull` for
        // the DRM manifest test. The manifest parse check returns
        // immediately so `run_smooth_pull` fails within ~ms.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
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
        // A namespace-prefixed element counts; a comment or an attribute value
        // that merely mentions the tag does not.
        assert!(manifest_declares_protection(
            "<SmoothStreamingMedia xmlns:ms=\"urn:x\"><ms:Protection/></SmoothStreamingMedia>"
        ));
        assert!(!manifest_declares_protection(
            "<SmoothStreamingMedia><!-- <Protection> --></SmoothStreamingMedia>"
        ));
        assert!(!manifest_declares_protection(
            "<SmoothStreamingMedia note=\"<Protection>\"/>"
        ));
    }

    /// Serialize one ISOBMFF box (`type` + `body`) using the crate's own
    /// header writer, so the fixtures below are real, parseable boxes.
    fn box_bytes(box_type: [u8; 4], body: &[u8]) -> Vec<u8> {
        use broadcast_common::Serialize;
        let header = transmux::box_types::BoxHeader::new(
            (transmux::box_types::BOX_HEADER_MIN_SIZE + body.len()) as u64,
            transmux::box_types::BoxType::from_bytes(box_type),
            None,
        );
        let mut out = vec![0u8; header.serialized_len() + body.len()];
        let n = header.serialize_into(&mut out).expect("serialize header");
        out[n..].copy_from_slice(body);
        out
    }

    /// Wrap `child` in a `traf`, then that in a `moof`, then that in a
    /// leading `styp` — the MS-SSTR fragment shape.
    fn fragment_with_traf_child(child: Vec<u8>) -> Vec<u8> {
        let traf = box_bytes(*b"traf", &child);
        let moof = box_bytes(*b"moof", &traf);
        let styp = box_bytes(*b"styp", b"msdh");
        let mut frag = styp;
        frag.extend_from_slice(&moof);
        frag
    }

    #[test]
    fn fragment_looks_encrypted_walks_the_moof_tree() {
        // A `senc` inside `moof`/`traf` is encryption.
        assert!(fragment_looks_encrypted(&fragment_with_traf_child(
            box_bytes(*b"senc", &[0u8; 8])
        )));
        assert!(fragment_looks_encrypted(&fragment_with_traf_child(
            box_bytes(*b"saiz", &[0u8; 4])
        )));
        assert!(fragment_looks_encrypted(&fragment_with_traf_child(
            box_bytes(*b"saio", &[0u8; 4])
        )));

        // The bytes `senc`/`saiz`/`saio` appearing in `mdat` payload (or any
        // other box body) are **not** encryption — the pre-fix byte scan
        // false-positived here (audit run 7, W15).
        let moof = box_bytes(
            *b"moof",
            &box_bytes(*b"traf", &box_bytes(*b"tfhd", &[0u8; 8])),
        );
        let mut frag = box_bytes(*b"styp", b"msdh");
        frag.extend_from_slice(&moof);
        frag.extend_from_slice(&box_bytes(*b"mdat", b"...senc...saiz...saio..."));
        assert!(
            !fragment_looks_encrypted(&frag),
            "payload bytes that spell senc/saiz/saio are not encryption boxes"
        );

        // A bare `senc` with no enclosing `moof`/`traf` is not a fragment's
        // encryption signalling.
        assert!(!fragment_looks_encrypted(b"....senc...."));
        assert!(!fragment_looks_encrypted(&PIFF_SAMPLE_ENCRYPTION_UUID));
        // Raw bytes that are not a valid box sequence must fail closed (they
        // read as a malformed top-level box, not a clear fragment).
        assert_eq!(
            fragment_state(b"stypmoofmdattraftfhdtrun"),
            FragmentState::Undetermined
        );

        // The PIFF sample-encryption `uuid` **inside** a `traf` is encryption
        // (the positive case the box walk must still catch).
        let mut uuid_body = PIFF_SAMPLE_ENCRYPTION_UUID.to_vec();
        uuid_body.extend_from_slice(&[0u8; 4]); // FullBox header
        // Build the uuid box with the extended type.
        use broadcast_common::Serialize;
        let hdr = transmux::box_types::BoxHeader::new(
            (transmux::box_types::BOX_HEADER_MIN_SIZE + 16 + uuid_body.len()) as u64,
            transmux::box_types::BoxType::from_bytes(*b"uuid"),
            Some(PIFF_SAMPLE_ENCRYPTION_UUID),
        );
        let mut uuid_box = vec![0u8; hdr.serialized_len() + uuid_body.len()];
        let n = hdr.serialize_into(&mut uuid_box).expect("serialize uuid");
        uuid_box[n..].copy_from_slice(&uuid_body);
        assert!(
            fragment_looks_encrypted(&fragment_with_traf_child(uuid_box)),
            "a PIFF uuid inside traf is encryption"
        );
    }

    /// Audit W15: a **real** PIFF-CBC sample-encrypted fragment (Bento4
    /// `mp4encrypt --method PIFF-CBC`, committed under `tests/fixtures/`) must
    /// be detected as encrypted by the box walk — the positive case, read
    /// from a genuine encrypted fragment rather than a hand-built box.
    #[test]
    fn real_piff_encrypted_fixture_is_detected() {
        let path = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/piff-sample-encrypted.mp4"
        ));
        if !path.exists() {
            eprintln!("SKIP real_piff_encrypted_fixture_is_detected: fixture missing");
            return;
        }
        let bytes = std::fs::read(&path).expect("read fixture");
        assert!(
            fragment_looks_encrypted(&bytes),
            "a real PIFF-CBC encrypted fragment must be detected as encrypted"
        );
    }

    /// Audit W15: a fragment whose box walk fails *after* a `moof` was seen
    /// (a truncated/malformed structure) must be treated as **not reliably
    /// clear** — fail closed, so a truncated encrypted fragment is never
    /// silently demuxed as clear.
    #[test]
    fn truncated_moof_fails_closed() {
        // A `moof` header claims a body larger than the buffer supplies.
        let mut frag = box_bytes(*b"styp", b"msdh");
        // moof with a declared size far past the end.
        frag.extend_from_slice(&8u32.to_be_bytes());
        frag.extend_from_slice(b"moof");
        // No body — the moof parse will fail.
        frag.extend_from_slice(&[0u8; 4]);
        assert_eq!(
            fragment_state(&frag),
            FragmentState::Undetermined,
            "a truncated moof must be undetermined, not clear"
        );

        // A non-fragment (no moof at all) is clear.
        assert_eq!(
            fragment_state(&box_bytes(*b"styp", b"msdh")),
            FragmentState::Clear
        );

        // A `moof` whose own header is truncated so badly its size can no
        // longer be read (fewer than 8 bytes for the header) is also
        // undetermined, not clear.
        let mut header_truncated = box_bytes(*b"styp", b"msdh");
        header_truncated.extend_from_slice(&[0x00, 0x00, 0x00, 0x40]); // size past end
        header_truncated.extend_from_slice(b"moof");
        assert_eq!(
            fragment_state(&header_truncated),
            FragmentState::Undetermined,
            "a top-level moof truncated at its header must fail closed"
        );

        // A genuinely encrypted fragment is classified distinct from
        // undetermined.
        assert_eq!(
            fragment_state(&fragment_with_traf_child(box_bytes(*b"senc", &[0u8; 8]))),
            FragmentState::Encrypted
        );
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

    /// Test-only: `fetch_one_bytes`, unwrapped to the body bytes (tests never
    /// use the `NotReady` signal with `tolerate_404: false`).
    async fn fetch_bytes(
        http: &HttpClient,
        url: &str,
        creds: Option<&Credentials>,
        what: &str,
        tolerate_404: bool,
    ) -> Result<Vec<u8>> {
        match fetch_one_bytes(http, url, creds, what, tolerate_404).await? {
            FetchOne::Bytes(b) => Ok(b),
            FetchOne::NotReady => Err(MultimuxError::Connect {
                reason: "not ready (test)".into(),
            }),
        }
    }
}
