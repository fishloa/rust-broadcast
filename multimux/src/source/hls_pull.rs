//! HLS-pull ingest source (issue #663 P3c / #717): wraps
//! [`ll_hls_runtime::client::tokio_client::TokioClient`] — the sans-IO
//! Low-Latency HLS (RFC 8216bis) playback client engine, driven over real
//! HTTP — as a multimux [`crate::origin::supervisor::SourceConnector`]/
//! [`crate::pipeline::SampleSource`].
//!
//! No new demux here: `TokioClient` already demuxes fetched CMAF parts/
//! segments into `transmux::Sample`s internally via `transmux::Fmp4Demux`
//! (see `ll_hls_runtime::client::engine`'s "Output adapter" docs) — this
//! module only:
//!
//! 1. Recovers the `TrackSpec`s the local `LlHlsSegmenter`
//!    ([`crate::pipeline::run_pipeline`]) needs to (re-)package the pulled
//!    stream, by feeding the client's first `Output::Init` bytes through
//!    `transmux::Fmp4Demux` once — the *same* demuxer the client itself
//!    already uses to decode samples, not a hand-rolled `moov` parse.
//! 2. Relays every subsequent `Output::Samples` batch straight through.
//!
//! # Auth
//!
//! Credentials (Basic/Digest/Bearer) are passed to `TokioClient` as a
//! `broadcast_auth::Credentials` via `TokioClientConfig::auth` — the same
//! shared model `rtsp-runtime` and [`crate::source::ts_http`] use (issue
//! #663 P3b). [`HlsPullSource::with_auth`] (config-supplied, e.g. a Bearer
//! token — the only way to supply one, since it has no URL-userinfo form)
//! takes precedence over the pull URL's own userinfo, if set — see
//! `crate::source::http_auth::resolve_credentials`. `ll_hls_runtime`'s
//! `TokioClient` performs the actual HTTP fetching (including the Digest
//! challenge/response, delegated to `broadcast-auth` on its own side, and
//! cached across requests — see its own module docs), so this source never
//! builds its own `reqwest::Client` — unlike `ts_http`, which does its own
//! streaming GETs and so uses `http_auth::authenticated_get` directly.
//!
//! # Known limitation
//!
//! A mid-stream `Output::Init` (the client re-emits it only on a codec-
//! parameter change across an `#EXT-X-DISCONTINUITY`) is treated as an empty
//! batch, not fed back into the already-built local segmenter's track set —
//! matching [`crate::pipeline::SampleSource::track_specs`]'s "called once,
//! before the first sample" contract. A pulled origin that changes codec
//! parameters mid-stream is not yet supported; the pipeline keeps running
//! against the original track specs.

use std::time::Duration;

use broadcast_auth::Credentials;
use broadcast_common::Unpackage;
use ll_hls_runtime::client::Output as HlsOutput;
use ll_hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig};
use transmux::Fmp4Demux;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::IngestTimeouts;
use crate::source::Source;
use crate::source::http_auth::{credentials_from_url, resolve_credentials, strip_userinfo};
use transmux::pipeline::{Sample, TrackSpec};

/// A remote (LL-)HLS Media Playlist to pull: its URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct HlsPullSource {
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
impl std::fmt::Debug for HlsPullSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HlsPullSource")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl HlsPullSource {
    /// Build a source descriptor. `url` is the target Media Playlist URL
    /// (not a Multivariant Playlist — this pulls one rendition directly).
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        HlsPullSource {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`] — see `RtspSource::with_timeouts`
    /// for the pattern this mirrors.
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

    /// Builds a [`TokioClient`] against the configured (userinfo-stripped)
    /// URL, with any extracted credentials attached, and drives it until its
    /// first `Output::Init` arrives — recovering the `TrackSpec`s from it via
    /// [`Fmp4Demux`] — bounded by [`IngestTimeouts::connect`].
    pub async fn connect(&self) -> Result<HlsPullSession> {
        let parsed = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad HLS-pull URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        let credentials = resolve_credentials(self.auth.clone(), credentials_from_url(&parsed)?);
        let clean_url = strip_userinfo(&parsed)?;

        let config = TokioClientConfig {
            auth: credentials,
            ..TokioClientConfig::default()
        };
        let mut client = TokioClient::with_config(clean_url.to_string(), config).map_err(|e| {
            MultimuxError::Connect {
                reason: format!("hls-pull client: {e}"),
            }
        })?;

        let connect_timeout = self.timeouts.connect;
        let specs = match tokio::time::timeout(connect_timeout, wait_for_init(&mut client)).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(MultimuxError::Connect {
                    reason: format!("hls-pull: no init segment within {connect_timeout:?}"),
                });
            }
        };

        Ok(HlsPullSession {
            client,
            specs,
            read_timeout: self.timeouts.read,
        })
    }
}

/// Drives `client` until its first [`HlsOutput::Init`], recovering the
/// pulled stream's [`TrackSpec`]s from it via [`Fmp4Demux`] (the init bytes
/// alone — `ftyp`+`moov`, no `moof`/`mdat` — are enough for `Fmp4Demux` to
/// resolve every track's identity + codec config; it simply finds no
/// fragments to absorb samples from).
async fn wait_for_init(client: &mut TokioClient) -> Result<Vec<TrackSpec>> {
    loop {
        match client.next_output().await {
            Ok(Some(HlsOutput::Init(bytes))) => {
                let media =
                    Fmp4Demux::new()
                        .unpackage(&bytes)
                        .map_err(|e| MultimuxError::Connect {
                            reason: format!("hls-pull: bad init segment: {e}"),
                        })?;
                return Ok(media.tracks.into_iter().map(|t| t.spec).collect());
            }
            Ok(Some(HlsOutput::EndOfStream)) | Ok(None) => {
                return Err(MultimuxError::Connect {
                    reason: "hls-pull: stream ended before an init segment arrived".into(),
                });
            }
            Ok(Some(_other)) => continue,
            Err(e) => {
                return Err(MultimuxError::Connect {
                    reason: format!("hls-pull connect: {e}"),
                });
            }
        }
    }
}

impl Source for HlsPullSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live HLS-pull session: a driven [`TokioClient`] plus the `TrackSpec`s
/// recovered from its first init segment.
pub struct HlsPullSession {
    client: TokioClient,
    specs: Vec<TrackSpec>,
    /// Bound on each [`Self::next_samples`] read — see
    /// [`IngestTimeouts::read`].
    read_timeout: Duration,
}

impl HlsPullSession {
    /// The `TrackSpec`s recovered during [`HlsPullSource::connect`]'s wait
    /// for the first init segment.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Pulls the client's next output, relaying `Output::Samples` straight
    /// through. `Output::Init`/`Output::Discontinuity` (and any future
    /// `#[non_exhaustive]` variant) yield an empty batch rather than
    /// samples — see the module doc's "Known limitation" section for why a
    /// mid-stream re-`Init` doesn't rebuild the track set.
    ///
    /// Returns `Ok(None)` once the client reaches `Output::EndOfStream` (the
    /// origin sent `#EXT-X-ENDLIST` with nothing left outstanding) —
    /// [`crate::origin::supervisor::supervise`] then reconnects, per the
    /// project's uniform "source EOF -> reconnect with backoff" contract.
    ///
    /// Bounded by [`IngestTimeouts::read`] (issue #663 P5, audit-ingest #3):
    /// a pulled origin that stops advancing (wedged/stalled — playlist
    /// requests never complete, or complete but never reveal new media)
    /// would otherwise leave this `.await` pending forever; a timed-out read
    /// surfaces as a [`MultimuxError::Connect`], reconnected by the
    /// supervisor exactly like any other read error.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let read_timeout = self.read_timeout;
        let output = tokio::time::timeout(read_timeout, self.client.next_output())
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: format!("hls-pull: no output within {read_timeout:?}"),
            })?;
        match output {
            Ok(Some(HlsOutput::Samples { track_id, samples })) => {
                Ok(Some(samples.into_iter().map(|s| (track_id, s)).collect()))
            }
            Ok(Some(HlsOutput::EndOfStream)) | Ok(None) => Ok(None),
            Ok(Some(_other)) => Ok(Some(Vec::new())),
            Err(e) => Err(MultimuxError::Connect {
                reason: format!("hls-pull: {e}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::{AppState, router};
    use crate::output::Output as MmOutput;
    use crate::output::llhls::LlHlsOutput;
    use crate::store::MediaStore;
    use std::collections::HashMap;
    use std::sync::Arc;
    use transmux::ll_hls::LlHlsSegmenter;
    use transmux::{
        AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig,
    };

    const TRACK_ID: u32 = 1;
    const MOVIE_TIMESCALE: u32 = 90_000;
    const VIDEO_TIMESCALE: u32 = 90_000;
    const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
    const TARGET_DURATION_SECS: f64 = 1.0;
    const PART_TARGET_MS: u32 = 150;
    const WINDOW_SEGMENTS: usize = 8;
    /// Enough samples to close at least one full segment plus some parts.
    const FRAME_COUNT: u32 = 60;

    fn dummy_avc_config() -> AVCConfigurationBox {
        AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 66,
            profile_compatibility: 0,
            level_indication: 30,
            length_size_minus_one: 3,
            sps: vec![AvcSps(vec![0x67, 66, 0, 30, 0x00])],
            pps: vec![AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        })
    }

    fn video_track_spec() -> TrackSpec {
        TrackSpec::new(
            TRACK_ID,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config: dummy_avc_config(),
                width: 320,
                height: 240,
            },
        )
    }

    /// Feeds a fixed batch of samples into `store` via a real
    /// `LlHlsSegmenter`, then flushes — no real-time pacing needed for this
    /// test (the client polls until it has everything, not "live" latency).
    fn populate_store(store: &MediaStore) {
        let mut seg = LlHlsSegmenter::with_part_target(
            vec![video_track_spec()],
            MOVIE_TIMESCALE,
            TARGET_DURATION_SECS,
            PART_TARGET_MS,
        )
        .expect("segmenter builds");
        store.set_init(seg.init_segment().expect("init segment builds"));

        for i in 0..FRAME_COUNT {
            let is_sync = i % 15 == 0;
            let data = vec![0xABu8.wrapping_add(i as u8); 32];
            let sample = Sample::new(data, FRAME_DUR, is_sync, 0);
            seg.push(TRACK_ID, sample).expect("push succeeds");
            for part in seg.take_ready_parts() {
                store.add_part(part);
            }
            for segment in seg.take_ready_segments() {
                store.add_segment(segment);
            }
        }
        seg.flush().expect("flush succeeds");
        for part in seg.take_ready_parts() {
            store.add_part(part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(segment);
        }
    }

    /// Starts a real `multimux` LL-HLS origin (the same production
    /// `MediaStore` + `LlHlsOutput` + `origin::router` this crate serves in
    /// production — no test double) on an ephemeral loopback port, serving
    /// one already-fully-populated stream named `live`. `auth`, if given,
    /// gates every request behind that scheme (see
    /// `crate::testutil::require_auth`) — used by the auth-scheme biting
    /// tests below; `None` (the plain `start_populated_ll_origin` case)
    /// mirrors the original no-auth origin. Since this is multimux testing
    /// itself (not a separate crate depending back on multimux), there is no
    /// dev-dependency cycle to worry about — see the P3c report for why this
    /// sidesteps the concern the task brief raised.
    async fn start_populated_ll_origin_with_auth(
        auth: Option<crate::testutil::MockAuthScheme>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let store = Arc::new(MediaStore::new(
            TARGET_DURATION_SECS,
            PART_TARGET_MS,
            WINDOW_SEGMENTS,
        ));
        populate_store(&store);

        let mut streams = HashMap::new();
        streams.insert(
            "live".to_string(),
            (
                store,
                vec![Arc::new(LlHlsOutput::default()) as Arc<dyn MmOutput>],
            ),
        );
        let mut app = router(Arc::new(AppState::new(streams)));
        if let Some(scheme) = auth {
            app = crate::testutil::require_auth(app, scheme);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("listener has a local address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/live/media.m3u8"), server)
    }

    /// Starts a plain (no-auth) populated LL-HLS origin — see
    /// [`start_populated_ll_origin_with_auth`].
    async fn start_populated_ll_origin() -> (String, tokio::task::JoinHandle<()>) {
        start_populated_ll_origin_with_auth(None).await
    }

    /// Biting loopback test: a real multimux LL-HLS origin, pulled by
    /// `HlsPullSource` over real HTTP — asserts `connect()` recovers the
    /// right `TrackSpec`s and `next_samples()` yields every sample the
    /// origin actually served, byte-identical.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pulls_real_ll_hls_origin_and_recovers_track_specs_and_samples() {
        let (playlist_url, server) = start_populated_ll_origin().await;

        let source = HlsPullSource::new("pulled-cam", playlist_url);
        let mut session = tokio::time::timeout(Duration::from_secs(10), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");

        let specs = session.track_specs();
        assert_eq!(
            specs.len(),
            1,
            "one video track recovered from the init segment"
        );
        assert_eq!(specs[0].track_id, TRACK_ID);
        assert_eq!(specs[0].timescale, VIDEO_TIMESCALE);
        assert!(
            matches!(specs[0].config, CodecConfig::Avc { .. }),
            "codec config must round-trip as AVC: {:?}",
            specs[0].config
        );

        let mut total_samples = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while total_samples < FRAME_COUNT as usize && tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(5), session.next_samples())
                .await
                .expect("next_samples timed out")
                .expect("next_samples must not error")
            {
                Some(batch) => total_samples += batch.len(),
                None => break,
            }
        }

        assert_eq!(
            total_samples, FRAME_COUNT as usize,
            "must pull every sample the origin served, no gaps/duplicates"
        );

        server.abort();
    }

    /// An unreachable origin must fail `connect()` within `CONNECT_TIMEOUT`
    /// rather than hang forever — proving the bound actually applies (the
    /// underlying `TokioClient` retries a playlist fetch indefinitely on its
    /// own, so without this bound `connect()` would never return).
    #[tokio::test]
    async fn connect_times_out_against_an_unreachable_origin() {
        // Reserve a real ephemeral port, then drop the listener so nothing
        // is actually listening there — guarantees connection refused/
        // timeout rather than accidentally hitting a live server.
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let source = HlsPullSource::new("unreachable", format!("http://{addr}/media.m3u8"));
        let result = tokio::time::timeout(Duration::from_secs(15), source.connect())
            .await
            .expect("connect() must return within its own CONNECT_TIMEOUT, not hang");
        assert!(result.is_err(), "an unreachable origin must fail connect()");
    }

    // --- issue #663 "Finish client-side multi-scheme auth": Basic/Digest/
    // Bearer/wrong-creds against a real mock-auth-gated origin ---

    const AUTH_USER: &str = "cam-user";
    const AUTH_PASS: &str = "cam-pass";
    const DIGEST_REALM: &str = "mock realm";
    const BEARER_TOKEN: &str = "hls-pull-bearer-token";

    /// Drives an `HlsPullSource` to connect and pull every sample the origin
    /// serves, returning the sample count — the common "auth worked, real
    /// media came out" assertion shared by the Basic/Digest/Bearer tests
    /// below.
    async fn connect_and_drain(source: HlsPullSource) -> Result<usize> {
        let mut session = tokio::time::timeout(Duration::from_secs(10), source.connect())
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: "connect timed out".into(),
            })??;
        let mut total = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while total < FRAME_COUNT as usize && tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(5), session.next_samples())
                .await
                .map_err(|_| MultimuxError::Connect {
                    reason: "next_samples timed out".into(),
                })?? {
                Some(batch) => total += batch.len(),
                None => break,
            }
        }
        Ok(total)
    }

    /// Basic (RFC 7617), credentials from URL userinfo: `TokioClient`
    /// pre-applies Basic (RFC 7617, no challenge round-trip needed) via
    /// `apply_auth_preemptive`, and real samples come out.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let (playlist_url, server) =
            start_populated_ll_origin_with_auth(Some(crate::testutil::MockAuthScheme::Basic {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
            }))
            .await;
        let credentialed =
            playlist_url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);

        let source = HlsPullSource::new("pulled-basic", credentialed);
        let total = connect_and_drain(source)
            .await
            .expect("Basic auth from URL userinfo must authenticate");
        assert_eq!(total, FRAME_COUNT as usize, "expected every sample");

        server.abort();
    }

    /// Digest (RFC 7616), credentials from URL userinfo: the origin issues a
    /// real Digest challenge (nonce/realm/qop=auth) via a real
    /// `broadcast_auth::Verifier` (`crate::testutil::require_auth`) that
    /// independently recomputes the expected response — proves `TokioClient`
    /// actually computed a correct Digest response via `broadcast-auth` (and
    /// cached it — see the module's own "Auth" docs), not just echoed
    /// something Digest-shaped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn digest_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let (playlist_url, server) =
            start_populated_ll_origin_with_auth(Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }))
            .await;
        let credentialed =
            playlist_url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);

        let source = HlsPullSource::new("pulled-digest", credentialed);
        let total = connect_and_drain(source)
            .await
            .expect("Digest auth from URL userinfo must authenticate");
        assert_eq!(total, FRAME_COUNT as usize, "expected every sample");

        server.abort();
    }

    /// Bearer (RFC 6750), config-supplied (the only way to supply one — it
    /// has no URL-userinfo form): `HlsPullSource::with_auth` overrides the
    /// (bare, no-userinfo) connect URL.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bearer_auth_config_supplied_authenticates_and_pulls_samples() {
        let (playlist_url, server) =
            start_populated_ll_origin_with_auth(Some(crate::testutil::MockAuthScheme::Bearer {
                token: BEARER_TOKEN.into(),
            }))
            .await;

        let source = HlsPullSource::new("pulled-bearer", playlist_url)
            .with_auth(Some(Credentials::bearer(BEARER_TOKEN)));
        let total = connect_and_drain(source)
            .await
            .expect("config-supplied Bearer token must authenticate");
        assert_eq!(total, FRAME_COUNT as usize, "expected every sample");

        server.abort();
    }

    /// Wrong credentials must fail `connect()` (bounded by
    /// `IngestTimeouts::connect`, since `TokioClient`'s own playlist-fetch
    /// retry never gives up on its own — see the module's "Auth"/"Error
    /// recovery" docs), not hang forever — the negative counterpart to the
    /// three tests above, proving they actually bite (a client answering
    /// with the wrong password never gets past the origin's persistent 401).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrong_credentials_stay_401_and_connect_errors() {
        let (playlist_url, server) =
            start_populated_ll_origin_with_auth(Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }))
            .await;
        let wrong_creds =
            playlist_url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);

        let source =
            HlsPullSource::new("pulled-wrong", wrong_creds).with_timeouts(IngestTimeouts {
                connect: Duration::from_secs(2),
                read: IngestTimeouts::default().read,
            });
        let result = tokio::time::timeout(Duration::from_secs(10), source.connect())
            .await
            .expect("connect() must return within its own CONNECT_TIMEOUT, not hang");
        assert!(
            result.is_err(),
            "wrong credentials must fail connect(), not silently proceed"
        );

        server.abort();
    }
}
