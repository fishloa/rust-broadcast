//! Microsoft Smooth Streaming (MS-SSTR) pull ingest source (issue #759):
//! fetches a remote client Manifest via `transmux::smooth_parse::SmoothManifest`,
//! resolves each selected `StreamIndex`'s fragment-URL template, and demuxes
//! the fetched fragments via [`Fmp4Demux`] — the last Wave-0 ingest input,
//! completing the trio started by [`crate::source::hls_pull`] (#663/#760) and
//! [`crate::source::dash_pull`] (#758).
//!
//! # No init segment on the wire
//!
//! Unlike DASH/CMAF, Smooth has no bootstrapping init segment: a
//! `QualityLevel@CodecPrivateData` IS the codec config. [`SmoothPullSource::connect`]
//! therefore *synthesizes* one per stream via
//! `transmux::smooth_parse::track_spec_from_quality_level` +
//! [`build_init_segment`] (T1, issue #759) before any fragment can be
//! demuxed — see `StreamState::init_bytes`.
//!
//! # Discovering each stream's wire track id
//!
//! An MS-SSTR manifest carries no `track_id` anywhere (unlike a DASH init
//! segment's own `tkhd`), yet every fetched fragment's `moof`/`tfhd@track_ID`
//! must match a `trak` in the synthesized init segment's `moov` for
//! [`Fmp4Demux::unpackage`] to absorb its samples at all — get this wrong and
//! every sample for that stream is silently dropped (exactly the class of
//! bug issue #758's fixture caught for DASH's colliding `tkhd@track_ID=1`
//! representations). [`SmoothPullSource::connect`] resolves this by fetching
//! each stream's *first* fragment during connect and peeking its
//! `moof`/`tfhd@track_ID` directly (`discover_moof_track_id`, no `moov`
//! needed for that), then builds that stream's synthesized init segment with
//! the *same* track id — so every later fragment already matches. The
//! samples that come out of each per-stream demux are then remapped from
//! that discovered "local" id to a session-wide unique "global" id
//! (`StreamState::global_track_id`) before being surfaced, mirroring
//! `dash_pull`'s own local->global remap (there, discovered from each
//! Representation's *init segment*; here, from each stream's *first
//! fragment*, since Smooth has no init segment to read one from).
//!
//! # Video sample-duration clock (v1-scope convention)
//!
//! A `QualityLevel`'s audio attributes give an unambiguous sample-duration
//! clock (`SamplingRate`), but MS-SSTR has **no equivalent field for video**
//! anywhere in the manifest. [`SmoothPullSource`] assumes the same 90 kHz
//! clock this crate already uses for every other H.264 ingest path
//! (`transmux::VIDEO_CLOCK_RATE`, shared with `source::rtp_udp`/
//! `source::rtsp`'s RTP timestamps) — a documented v1-scope simplification,
//! not a spec-mandated value (real origins are free to use a different
//! video sample-duration clock, most commonly the manifest's own declared
//! `TimeScale`; supporting that is deferred).
//!
//! # Fragment time anchor
//!
//! A Smooth fragment's `moof` carries a `tfxd`/`tfrf` (PIFF) box, not a
//! standard `tfdt`; `transmux::movie_fragment::TrackFragmentBox::parse_body`
//! ignores unrecognised UUID boxes, so [`Fmp4Demux`] never recovers an
//! absolute decode time from one. This is harmless here: exactly like
//! `dash_pull`, every fragment is demuxed via a *fresh* `Fmp4Demux` seeded
//! only with the stream's own init bytes, so the pipeline's segmenter always
//! derives its running timeline from each sample's own relative `duration`
//! (never an absolute decode time) — the manifest's `c@t` is used only to
//! resolve each fragment's *URL*, never fed into a sample's timing.
//!
//! # Live (`IsLive="TRUE"`) manifests
//!
//! Once every stream's known chunk plan is exhausted,
//! [`SmoothPullSession::next_samples`] re-fetches and re-parses the manifest
//! no more often than `MANIFEST_REFRESH_INTERVAL` (MS-SSTR has no
//! `MPD@minimumUpdatePeriod` analogue — `LookAheadFragmentCount`/
//! `DVRWindowLength` are hints about how much look-ahead/history a manifest
//! carries, not a refresh cadence, so a fixed interval is used instead) and
//! extends each stream's plan with any `c` entries beyond its low-water
//! mark. A static (`IsLive` absent/`"FALSE"`) manifest instead treats plan
//! exhaustion as true end-of-stream.
//!
//! # PlayReady / PIFF sample encryption is NOT supported
//!
//! Real legacy Smooth Streaming origins commonly protect content with
//! PlayReady (PIFF sample encryption). This crate has no PlayReady/PIFF
//! decrypt path, so rather than silently demuxing garbage (encrypted)
//! sample bytes into the pipeline, [`SmoothPullSource::connect`]/
//! [`SmoothPullSession::next_samples`] detect it and fail with a clear
//! typed [`crate::MultimuxError::Encrypted`]:
//! - a manifest carrying a `<Protection>` element (`manifest_declares_protection`
//!   — a coarse, deliberately dependency-free tag-boundary text scan: T1's
//!   `SmoothManifest::parse` silently skips unrecognised child elements
//!   including `<Protection>`, so this crate cannot see it in the parsed
//!   structure at all);
//! - a fragment whose bytes carry a `senc`/`saiz`/`saio` box or the PIFF
//!   `UUID Sample Encryption Box` extended type (`fragment_looks_encrypted`
//!   — a coarse raw byte-pattern scan, not a full box parse: encryption
//!   support is explicitly out of scope, so this only needs to detect it,
//!   never decode it).
//!
//! # v1 scope
//!
//! One `QualityLevel` per `StreamIndex` (the first — bitrate/quality
//! selection is out of scope, matching `dash_pull`'s own "first
//! Representation" simplification); `StreamType::Text` (timed-text)
//! `StreamIndex`es are skipped (not carriable in this crate's fMP4 mux path
//! — see `track_spec_from_quality_level`'s own `UnsupportedCodec` for that
//! variant).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use broadcast_auth::Credentials;
use broadcast_common::Unpackage;
use reqwest::{Client, StatusCode};
use transmux::box_types::parse_box;
use transmux::media::Fmp4Demux;
use transmux::movie_fragment::MovieFragmentBox;
use transmux::pipeline::{Sample, TrackSpec, build_init_segment};
use transmux::smooth_parse::{
    SmoothManifest, StreamIndex, StreamType, track_spec_from_quality_level,
};
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::IngestTimeouts;
use crate::source::Source;
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};

/// The synthesized per-stream init segment's `mvhd` timescale — arbitrary
/// (ISO/IEC 14496-12 §8.2.2: the movie header's timescale only paces
/// `mvhd.duration`, never any track's own sample timing, which lives
/// entirely in that track's own `TrackSpec::timescale`/`mdhd`). Reuses
/// [`transmux::VIDEO_CLOCK_RATE`] purely so this module doesn't invent a
/// second arbitrary constant.
const SYNTHETIC_MOVIE_TIMESCALE: u32 = transmux::VIDEO_CLOCK_RATE;

/// Fixed live-manifest refresh interval — see the module doc's "Live
/// (`IsLive="TRUE"`) manifests". Matches `dash_pull`'s own
/// `DEFAULT_MPD_REFRESH_INTERVAL` default.
const MANIFEST_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// The PIFF "UUID Sample Encryption Box" extended type
/// (`A2394F52-5A9B-4F14-A244-6C427C648DF4`) — the pre-CENC PIFF fallback
/// some legacy Smooth Streaming origins wrap sample encryption metadata in,
/// checked by [`fragment_looks_encrypted`] alongside the standard CENC
/// `senc`/`saiz`/`saio` box types.
const PIFF_SAMPLE_ENCRYPTION_UUID: [u8; 16] = [
    0xA2, 0x39, 0x4F, 0x52, 0x5A, 0x9B, 0x4F, 0x14, 0xA2, 0x44, 0x6C, 0x42, 0x7C, 0x64, 0x8D, 0xF4,
];

/// A remote MS-SSTR client Manifest to pull: its URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct SmoothPullSource {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    /// Config-supplied credentials, taking precedence over any URL userinfo
    /// — see `crate::source::http_auth::resolve_credentials`.
    auth: Option<Credentials>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl std::fmt::Debug for SmoothPullSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmoothPullSource")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl SmoothPullSource {
    /// Build a source descriptor. `url` is the client Manifest URL to pull.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        SmoothPullSource {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo at
    /// [`Self::connect`] time — see
    /// `crate::source::http_auth::resolve_credentials`.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }

    /// Fetches the manifest, rejects a PlayReady/PIFF-protected source (see
    /// the module doc), resolves every video/audio `StreamIndex`'s first
    /// `QualityLevel` (discovering + matching each stream's wire track id —
    /// see the module doc), and builds the initial per-stream chunk plan —
    /// all bounded by [`IngestTimeouts::connect`]. Every expected track is
    /// resolved here, before the session is ever returned — a route never
    /// starts having silently dropped one of its declared streams.
    pub async fn connect(&self) -> Result<SmoothPullSession> {
        let parsed = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad Smooth-pull URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        let credentials = resolve_credentials(self.auth.clone(), credentials_from_url(&parsed)?);
        let clean_manifest_url = strip_userinfo(&parsed)?;

        let client = Client::builder()
            .build()
            .map_err(|e| MultimuxError::Connect {
                reason: format!("reqwest client: {e}"),
            })?;

        let connect_timeout = self.timeouts.connect;
        let outcome = tokio::time::timeout(
            connect_timeout,
            do_connect(&client, credentials.clone(), &clean_manifest_url),
        )
        .await;
        let (is_live, streams, specs) = match outcome {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(MultimuxError::Connect {
                    reason: format!("smooth-pull: connect exceeded {connect_timeout:?}"),
                });
            }
        };

        Ok(SmoothPullSession {
            client,
            auth: credentials,
            manifest_url: clean_manifest_url,
            is_live,
            last_manifest_fetch: Instant::now(),
            streams,
            specs,
            read_timeout: self.timeouts.read,
        })
    }
}

impl Source for SmoothPullSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// One resolved video/audio `StreamIndex`'s live state.
struct StreamState {
    /// The stream's own parsed `StreamIndex` (its `Url` template +
    /// `resolve_fragment_url`/`enumerate_chunks` — refreshed wholesale on a
    /// live-manifest re-fetch, see [`SmoothPullSession::maybe_refresh_manifest`]).
    stream: StreamIndex,
    /// The (first, and in v1 scope only) `QualityLevel`'s bitrate — the
    /// `{bitrate}` token [`StreamIndex::resolve_fragment_url`] substitutes.
    bitrate: u64,
    /// This stream's synthesized init segment (`ftyp`+`moov`), built once at
    /// connect time — concatenated onto every fetched fragment before
    /// [`Fmp4Demux::unpackage`] (see the module doc's "no init segment on
    /// the wire").
    init_bytes: Vec<u8>,
    /// This stream's wire track id, discovered from its first fragment's
    /// `moof`/`tfhd@track_ID` at connect time (see the module doc's
    /// "Discovering each stream's wire track id") — `init_bytes`'s `moov`
    /// carries the exact same id, so every fragment's samples are absorbed.
    local_track_id: u32,
    /// The session-wide unique id this stream's samples are remapped to
    /// before being surfaced.
    global_track_id: u32,
    /// Pending `(t, d)` pairs (in the stream's own tick scale — see
    /// `StreamIndex::enumerate_chunks`) not yet fetched, in presentation
    /// order. The very first pair is fetched during `connect()` (to
    /// discover `local_track_id`) and cached in
    /// [`Self::cached_first_fragment`] rather than being re-fetched here.
    plan: VecDeque<(u64, u64)>,
    /// The first chunk's bytes, already fetched during `connect()` —
    /// consumed (and cleared) by the first [`SmoothPullSession::next_samples`]
    /// call so that fetch is never repeated.
    cached_first_fragment: Option<Vec<u8>>,
    /// The end tick (`t + d`) of the last chunk known for this stream — the
    /// low-water mark a live-manifest refresh extends beyond.
    last_time: u64,
}

/// Performs the manifest fetch + parse + per-stream first-fragment fetch
/// [`SmoothPullSource::connect`] wraps in its connect timeout. Returns the
/// manifest's `IsLive`, the resolved [`StreamState`]s, and every recovered
/// [`TrackSpec`] (track ids already remapped to session-wide unique ids).
async fn do_connect(
    client: &Client,
    credentials: Option<Credentials>,
    manifest_url: &Url,
) -> Result<(bool, Vec<StreamState>, Vec<TrackSpec>)> {
    let manifest_text = fetch_text(
        client,
        manifest_url.as_str(),
        credentials.as_ref(),
        "manifest",
    )
    .await?;
    if manifest_declares_protection(&manifest_text) {
        return Err(MultimuxError::Encrypted {
            reason: "manifest declares a <Protection> element (PlayReady/PIFF sample \
                     encryption) — decrypting Smooth-protected content is not supported"
                .into(),
        });
    }
    let manifest = SmoothManifest::parse(&manifest_text).map_err(|e| MultimuxError::Connect {
        reason: format!("smooth-pull: manifest parse: {e}"),
    })?;

    let mut streams = Vec::new();
    let mut specs = Vec::new();
    let mut next_track_id: u32 = 1;

    for si in &manifest.streams {
        let stream_type = si.stream_type;
        if matches!(stream_type, StreamType::Text) {
            // Not an "expected track" this route ingests (see the module
            // doc's "v1 scope") — skipping it is not a silent drop of
            // anything promised.
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

        let frag_rel = si.resolve_fragment_url(quality.bitrate, first_t);
        let frag_url = manifest_url
            .join(&frag_rel)
            .map_err(|e| MultimuxError::Connect {
                reason: format!("smooth-pull: bad fragment URL {frag_rel:?}: {e}"),
            })?;
        let first_bytes =
            fetch_bytes(client, frag_url.as_str(), credentials.as_ref(), "fragment").await?;
        if fragment_looks_encrypted(&first_bytes) {
            return Err(MultimuxError::Encrypted {
                reason: format!(
                    "smooth-pull: StreamIndex {:?} ({stream_type}) fragment carries PIFF/CENC \
                     sample-encryption boxes — decrypting Smooth-protected content is not \
                     supported",
                    si.name
                ),
            });
        }
        let local_track_id = discover_moof_track_id(&first_bytes)?;

        // See the module doc's "Video sample-duration clock (v1-scope
        // convention)".
        let effective_timescale: u32 = match stream_type {
            StreamType::Video => transmux::VIDEO_CLOCK_RATE,
            StreamType::Audio => quality
                .sampling_rate
                .ok_or_else(|| MultimuxError::Connect {
                    reason: format!(
                        "smooth-pull: StreamIndex {:?} audio QualityLevel has no SamplingRate",
                        si.name
                    ),
                })?,
            StreamType::Text => unreachable!("Text streams are skipped above"),
        };

        let local_spec = track_spec_from_quality_level(
            local_track_id,
            effective_timescale,
            stream_type,
            quality,
        )?;
        let init_bytes =
            build_init_segment(std::slice::from_ref(&local_spec), SYNTHETIC_MOVIE_TIMESCALE)?;

        let global_id = next_track_id;
        next_track_id += 1;
        let mut global_spec = local_spec.clone();
        global_spec.track_id = global_id;
        specs.push(global_spec);

        let last_time = chunks
            .last()
            .map(|&(t, d)| t.saturating_add(d))
            .unwrap_or_else(|| first_t.saturating_add(first_d));

        let mut plan: VecDeque<(u64, u64)> = chunks.into();
        // The first pair's bytes are already fetched above (to discover
        // local_track_id) and cached below — never re-fetched.
        plan.pop_front();

        streams.push(StreamState {
            stream: si.clone(),
            bitrate: quality.bitrate,
            init_bytes,
            local_track_id,
            global_track_id: global_id,
            plan,
            cached_first_fragment: Some(first_bytes),
            last_time,
        });
    }

    if specs.is_empty() {
        return Err(MultimuxError::Connect {
            reason: "smooth-pull: manifest resolved no usable stream (video/audio)".into(),
        });
    }

    Ok((manifest.is_live, streams, specs))
}

/// Peeks a fetched fragment's `moof`/`traf[0]`/`tfhd@track_ID` without
/// needing a `moov` (unlike [`Fmp4Demux::unpackage`], which requires one in
/// the same buffer) — see the module doc's "Discovering each stream's wire
/// track id". Walks top-level boxes exactly like [`Fmp4Demux::unpackage`]
/// does internally, tolerating any leading box (a real fragment response,
/// and this crate's own `SmoothPackager` fixture output, both lead with a
/// `styp`).
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
/// tag (self-closing or not) anywhere in `xml` — see the module doc's
/// "PlayReady / PIFF sample encryption is NOT supported". Matches `<Tag`
/// followed by whitespace, `>`, or `/` (never a longer element name like
/// `<ProtectionFoo`).
fn manifest_declares_protection(xml: &str) -> bool {
    tag_starts_present(xml, "Protection")
}

/// Shared implementation of [`manifest_declares_protection`]'s tag-boundary
/// scan, kept generic over the tag name in case a future caller needs the
/// same check for a different element.
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

/// Coarse raw byte-pattern scan for a CENC (`senc`/`saiz`/`saio`) or PIFF
/// ([`PIFF_SAMPLE_ENCRYPTION_UUID`]) sample-encryption box anywhere in a
/// fetched fragment's bytes — see the module doc's "PlayReady / PIFF sample
/// encryption is NOT supported". Deliberately not a full box walk: this
/// crate never decrypts Smooth content, so detecting the signal is enough.
fn fragment_looks_encrypted(bytes: &[u8]) -> bool {
    contains_subslice(bytes, b"senc")
        || contains_subslice(bytes, b"saiz")
        || contains_subslice(bytes, b"saio")
        || contains_subslice(bytes, &PIFF_SAMPLE_ENCRYPTION_UUID)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Performs `GET url`, returning an error whose `reason` names `what` on any
/// non-2xx status. Mirrors `dash_pull`'s own `fetch_bytes`.
async fn fetch_bytes(
    client: &Client,
    url: &str,
    creds: Option<&Credentials>,
    what: &str,
) -> Result<Vec<u8>> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(status_error(what, status));
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| MultimuxError::Connect {
            reason: format!("smooth-pull {what} read: {e}"),
        })
}

/// Text-body counterpart of [`fetch_bytes`], for the manifest itself.
async fn fetch_text(
    client: &Client,
    url: &str,
    creds: Option<&Credentials>,
    what: &str,
) -> Result<String> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(status_error(what, status));
    }
    response.text().await.map_err(|e| MultimuxError::Connect {
        reason: format!("smooth-pull {what} read: {e}"),
    })
}

/// Fetches one fragment. When `tolerate_404` (dynamic/live manifests only),
/// a `404` is reported as `Ok(None)` ("not yet available, retry later")
/// rather than an error; any other non-2xx status is always a hard error.
/// Mirrors `dash_pull`'s own `fetch_segment`.
async fn fetch_segment(
    client: &Client,
    url: &str,
    creds: Option<&Credentials>,
    tolerate_404: bool,
) -> Result<Option<Vec<u8>>> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if tolerate_404 && status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(status_error("fragment", status));
    }
    response
        .bytes()
        .await
        .map(|b| Some(b.to_vec()))
        .map_err(|e| MultimuxError::Connect {
            reason: format!("smooth-pull fragment read: {e}"),
        })
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

/// A live Smooth-pull session: every resolved stream's live state, plus the
/// connect-recovered [`TrackSpec`]s.
pub struct SmoothPullSession {
    client: Client,
    auth: Option<Credentials>,
    /// The (userinfo-stripped) manifest URL — also the base every relative
    /// fragment URL resolves against, and what a live-manifest refresh
    /// re-fetches.
    manifest_url: Url,
    is_live: bool,
    last_manifest_fetch: Instant,
    streams: Vec<StreamState>,
    specs: Vec<TrackSpec>,
    /// Bound on each network step in [`Self::next_samples`] — see
    /// [`IngestTimeouts::read`].
    read_timeout: Duration,
}

impl SmoothPullSession {
    /// The [`TrackSpec`]s recovered during [`SmoothPullSource::connect`],
    /// track ids already remapped to session-wide unique values.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Fetches the next pending fragment for every stream that still has one
    /// queued (or, for the very first call, consumes each stream's
    /// already-fetched-at-connect-time first fragment), demuxes each
    /// (concatenated onto that stream's synthesized init bytes — see the
    /// module doc), and returns every recovered sample, track ids remapped
    /// to this session's global ids.
    ///
    /// When every stream's plan is empty: a live manifest triggers a
    /// (rate-limited) refresh and returns an empty batch (not
    /// end-of-stream) so the caller keeps polling; a static manifest returns
    /// `Ok(None)` (true end-of-stream).
    ///
    /// Bounded by [`IngestTimeouts::read`): a stalled/unreachable server must
    /// not wedge the route — a timed-out fetch surfaces as an `Err`,
    /// reconnected by [`crate::origin::supervisor::supervise`] like any
    /// other read error.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let read_timeout = self.read_timeout;
        let mut out = Vec::new();
        let mut any_pending = false;

        for i in 0..self.streams.len() {
            let fetched: Option<Vec<u8>> =
                if let Some(cached) = self.streams[i].cached_first_fragment.take() {
                    any_pending = true;
                    Some(cached)
                } else {
                    let Some((t, d)) = self.streams[i].plan.pop_front() else {
                        continue;
                    };
                    any_pending = true;

                    let rel = self.streams[i]
                        .stream
                        .resolve_fragment_url(self.streams[i].bitrate, t);
                    let frag_url =
                        self.manifest_url
                            .join(&rel)
                            .map_err(|e| MultimuxError::Connect {
                                reason: format!("smooth-pull: bad fragment URL {rel:?}: {e}"),
                            })?;
                    let stream_name = self.streams[i].stream.name.clone();

                    let fetch = fetch_segment(
                        &self.client,
                        frag_url.as_str(),
                        self.auth.as_ref(),
                        self.is_live,
                    );
                    match tokio::time::timeout(read_timeout, fetch).await {
                        Ok(Ok(Some(bytes))) => Some(bytes),
                        Ok(Ok(None)) => {
                            // Live-edge fragment not yet available: retry next round.
                            self.streams[i].plan.push_front((t, d));
                            None
                        }
                        Ok(Err(e)) => return Err(e),
                        Err(_) => {
                            return Err(MultimuxError::Connect {
                                reason: format!(
                                    "smooth-pull: fragment read (stream {stream_name:?}) timed out \
                                 after {read_timeout:?}"
                                ),
                            });
                        }
                    }
                };

            let Some(bytes) = fetched else { continue };

            if fragment_looks_encrypted(&bytes) {
                let stream_name = self.streams[i].stream.name.clone();
                return Err(MultimuxError::Encrypted {
                    reason: format!(
                        "smooth-pull: stream {stream_name:?} fragment carries PIFF/CENC \
                         sample-encryption boxes — decrypting Smooth-protected content is not \
                         supported"
                    ),
                });
            }

            let stream = &self.streams[i];
            let mut combined = Vec::with_capacity(stream.init_bytes.len() + bytes.len());
            combined.extend_from_slice(&stream.init_bytes);
            combined.extend_from_slice(&bytes);
            let media = Fmp4Demux::new().unpackage(combined.as_slice())?;
            for track in media.tracks {
                if track.spec.track_id == stream.local_track_id {
                    for sample in track.samples {
                        out.push((stream.global_track_id, sample));
                    }
                }
            }
        }

        if !any_pending {
            if self.is_live {
                self.maybe_refresh_manifest().await?;
                return Ok(Some(Vec::new()));
            }
            return Ok(None);
        }

        Ok(Some(out))
    }

    /// Re-fetches and re-parses the manifest (no more often than
    /// [`MANIFEST_REFRESH_INTERVAL`], sleeping out the remainder — capped by
    /// the read timeout — when called too soon) and extends every
    /// still-matching stream's plan with any `c` entries beyond its low-water
    /// mark. A stream no longer present in the refreshed manifest (matched by
    /// `StreamType` — v1 scope assumes at most one video and one audio
    /// `StreamIndex`, same as connect-time resolution) is left as-is (its
    /// plan simply stays empty). Rejects a manifest that has since started
    /// declaring `<Protection>` exactly like [`SmoothPullSource::connect`]
    /// does.
    async fn maybe_refresh_manifest(&mut self) -> Result<()> {
        let elapsed = self.last_manifest_fetch.elapsed();
        if elapsed < MANIFEST_REFRESH_INTERVAL {
            let remaining = (MANIFEST_REFRESH_INTERVAL - elapsed).min(self.read_timeout);
            tokio::time::sleep(remaining).await;
            return Ok(());
        }

        let manifest_text = fetch_text(
            &self.client,
            self.manifest_url.as_str(),
            self.auth.as_ref(),
            "manifest refresh",
        )
        .await?;
        if manifest_declares_protection(&manifest_text) {
            return Err(MultimuxError::Encrypted {
                reason: "smooth-pull: manifest refresh declares a <Protection> element \
                         (PlayReady/PIFF sample encryption) — decrypting Smooth-protected \
                         content is not supported"
                    .into(),
            });
        }
        let manifest =
            SmoothManifest::parse(&manifest_text).map_err(|e| MultimuxError::Connect {
                reason: format!("smooth-pull: manifest refresh parse: {e}"),
            })?;
        self.last_manifest_fetch = Instant::now();
        self.is_live = manifest.is_live;

        for stream in &mut self.streams {
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
                    reason: format!("smooth-pull: manifest refresh: chunk enumeration: {e}"),
                })?;
            for (t, d) in chunks {
                if t >= stream.last_time {
                    stream.last_time = t.saturating_add(d).max(stream.last_time);
                    stream.plan.push_back((t, d));
                }
            }
            stream.stream = found.clone();
        }
        Ok(())
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
    use std::collections::HashMap;
    use std::sync::Arc;
    use transmux::pipeline::CodecConfig;
    use transmux::{Media, SmoothOutput, SmoothPackager, TsDemux};

    // -- fixture generation (runtime, hermetic — see the brief's "OR generate
    // it in the test at runtime if that's cleaner and hermetic") ------------

    /// The same real, committed h264/AAC TS fixture `transmux/tests/smooth.rs`
    /// (T1) demuxes to build its own Smooth output — reused here rather than
    /// committing a second, binary Smooth-specific fixture: generating it at
    /// runtime via the crate's own real `SmoothPackager` is both hermetic and
    /// exercises the exact writer/reader round-trip a real deployment would.
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

    /// Extracts `(kind, start_time)` from a resolved fragment path shaped
    /// like `QualityLevels(500000)/Fragments(video=1234)` — the bitrate
    /// segment is intentionally ignored (the client always echoes back
    /// exactly the bitrate its own fetched manifest declared, so the fixture
    /// server doesn't need to separately validate it).
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

    /// Starts a real axum server (loopback, ephemeral port) serving a
    /// runtime-generated Smooth manifest + its fragments, keyed by
    /// `(kind, start_time)`; `stall`, if given, makes that one
    /// `(kind, start_time)` hang forever instead of responding (the
    /// read-timeout biting test).
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

    /// Serves a fixed, caller-supplied manifest string at `/Manifest` only
    /// (no fragments) — for the encrypted-manifest test, which must fail at
    /// `connect()` before ever fetching a fragment.
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

    /// Independent oracle: demuxes every fixture fragment directly (each
    /// stream's own per-stream init bytes + every one of its fragments,
    /// exactly as the source itself must) to know the real expected sample
    /// counts, without going through `SmoothPullSource` at all.
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

    /// Biting loopback test: a real axum server serves a runtime-generated
    /// Smooth manifest + fragments; asserts `SmoothPullSource` resolves BOTH
    /// the AVC and AAC `TrackSpec`s (from `CodecPrivateData` alone — no init
    /// segment ever crossed the wire) with unique track ids, and, driven to
    /// end-of-stream, yields exactly the independently-demuxed oracle's
    /// sample counts for both tracks — proving real samples for both tracks
    /// land, not just that `connect()` succeeds (the #758-lesson "audio
    /// silently dropped" class of bug this guards against).
    #[tokio::test]
    async fn loopback_smooth_pull_resolves_both_tracks_and_matches_oracle_sample_count() {
        let (url, server, media) = start_fixture_server(None).await;
        let (_media2, out) = build_smooth_output();

        let source = SmoothPullSource::new("smooth-cam", url);
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");

        let specs = session.track_specs();
        assert_eq!(specs.len(), 2, "one video + one audio track");
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "must recover the fixture's AVC video track: {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Aac { .. })),
            "must recover the fixture's AAC audio track: {specs:?}"
        );
        let mut ids: Vec<u32> = specs.iter().map(|s| s.track_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 2, "track ids must be unique global ids");

        let want_total: usize = specs
            .iter()
            .map(|s| oracle_sample_count(&media, &out, s.track_id, s.timescale))
            .sum();
        assert!(want_total > 0, "sanity: fixture must carry real samples");

        let mut got_total = 0usize;
        while let Some(batch) = tokio::time::timeout(Duration::from_secs(5), session.next_samples())
            .await
            .expect("next_samples timed out")
            .expect("next_samples must not error")
        {
            got_total += batch.len();
        }
        assert_eq!(
            got_total, want_total,
            "must pull every real sample from both Smooth streams, no gaps/duplicates"
        );

        server.abort();
    }

    /// The brief's headline acceptance check: driven through the *real*
    /// pipeline (not just `next_samples()`), a Smooth-pull route lands a
    /// real init segment and at least one real segment/part in a real
    /// `MediaStore`, with both track kinds present in the store's own
    /// recorded `TrackSpec`s.
    #[tokio::test]
    async fn pipeline_lands_init_and_segment_in_media_store() {
        let (url, server, _media) = start_fixture_server(None).await;

        let source = SmoothPullSource::new("smooth-cam", url);
        let session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");

        let store = Arc::new(crate::store::MediaStore::new(4.0, 500, 8));
        tokio::time::timeout(
            Duration::from_secs(10),
            crate::pipeline::run_pipeline(store.clone(), 4.0, 500, session, "smooth-cam"),
        )
        .await
        .expect("pipeline timed out")
        .expect("pipeline must not error");

        assert!(
            store.init_bytes().is_some(),
            "init segment must land in the store"
        );
        assert!(
            !store.window_segments().is_empty(),
            "at least one real segment must land in the store"
        );
        let specs = store.track_specs();
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "video track kind must be recorded: {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Aac { .. })),
            "audio track kind must be recorded: {specs:?}"
        );

        server.abort();
    }

    /// Biting test (issue #663 P5 / #738-#739 ingest-hardening lesson): a
    /// server that resolves the manifest + both streams' first fragments but
    /// then stalls on a later fragment must fail `next_samples()` within
    /// [`IngestTimeouts::read`], not hang forever.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_on_a_later_fragment() {
        let (_media, out) = build_smooth_output();
        let audio_id = audio_track_id(&_media);
        let audio_frags: Vec<_> = out
            .fragments
            .iter()
            .filter(|f| f.track_id == audio_id)
            .collect();
        assert!(
            audio_frags.len() >= 2,
            "fixture must produce at least 2 audio fragments for this test to stall a \
             non-cached one: got {}",
            audio_frags.len()
        );
        let stall_time = audio_frags[1].start_time;

        let (url, server, _media2) = start_fixture_server(Some(("audio", stall_time))).await;

        let source = SmoothPullSource::new("smooth-stalled", url).with_timeouts(IngestTimeouts {
            connect: Duration::from_secs(5),
            read: Duration::from_millis(150),
        });
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect must succeed (only a later fragment stalls)");

        // First call consumes each stream's cached first fragment — no
        // network fetch needed yet, so it must succeed.
        let first = tokio::time::timeout(Duration::from_secs(5), session.next_samples())
            .await
            .expect("first next_samples call must not hang")
            .expect("first next_samples call must succeed");
        assert!(first.is_some());

        // Second call fetches the stalled fragment and must time out via
        // IngestTimeouts::read, not this test's own backstop.
        let result = tokio::time::timeout(Duration::from_secs(5), session.next_samples())
            .await
            .expect(
                "next_samples() must return on its own via IngestTimeouts::read, not hang \
                 until this test's own backstop timeout",
            );
        assert!(
            result.is_err(),
            "a server that stalls on a later fragment fetch must fail next_samples(), not \
             hang forever"
        );

        server.abort();
    }

    /// Biting test: a manifest declaring `<Protection>` (PlayReady/PIFF
    /// sample encryption) must fail `connect()` with a clear typed error,
    /// never silently succeed into a session that would go on to demux
    /// garbage (encrypted) sample bytes.
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

        let source = SmoothPullSource::new("smooth-drm", url);
        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out");
        match result {
            Err(MultimuxError::Encrypted { .. }) => {}
            Ok(_) => panic!("expected MultimuxError::Encrypted, got Ok(session)"),
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

    /// Real-fixture bite: discovering the wire track id from an actual
    /// `SmoothPackager`-produced fragment must recover the exact track id
    /// the writer used (the demuxed source's own, per the module doc — not
    /// a hardcoded `1`).
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
