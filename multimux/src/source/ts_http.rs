//! MPEG-2 Transport Stream over HTTP ingest source (issue #663 P3c): a
//! streaming HTTP GET (chunked/progressive, `reqwest`) feeding transmux's
//! incremental [`transmux::StreamingTsDemux`] — mirrors
//! [`crate::source::ts_udp`] almost exactly, just over an HTTP byte stream
//! instead of UDP datagrams. Auth (Basic/Digest/Bearer) is answered once via
//! `crate::source::http_auth::authenticated_get`, which delegates to
//! `broadcast-auth` for the Digest challenge/response — `reqwest` itself has
//! no Digest support. Credentials come from [`TsHttpSource::with_auth`]
//! (config-supplied, e.g. a Bearer token — the only way to supply one, since
//! it has no URL-userinfo form) if set, else the connect URL's own userinfo —
//! see `crate::source::http_auth::resolve_credentials`.
//!
//! Unlike UDP (connectionless, never signals end-of-stream),
//! [`TsHttpSession::next_samples`] returns `Ok(None)` once the HTTP body
//! stream ends (server closed the connection, or the response completed
//! normally) — [`crate::origin::supervisor::supervise`] then reconnects with
//! backoff, exactly as it does for any other source's end-of-stream.

use std::collections::BTreeSet;
use std::time::Duration;

use broadcast_auth::Credentials;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use reqwest::Client;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::IngestTimeouts;
use crate::source::Source;
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingTsDemux};

/// An MPEG-2 TS-over-HTTP stream to pull: an `http(s)://` URL, which may
/// carry `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct TsHttpSource {
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
impl std::fmt::Debug for TsHttpSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsHttpSource")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl TsHttpSource {
    /// Build a source descriptor.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        TsHttpSource {
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

    /// Opens a streaming GET to the configured URL (answering a `401`
    /// challenge if the URL carried credentials — see
    /// `crate::source::http_auth::authenticated_get`), then reads response
    /// chunks into a [`StreamingTsDemux`] until every currently PMT-declared
    /// track has resolved (or [`IngestTimeouts::connect`] elapses) — the
    /// TS-over-HTTP analogue of [`crate::source::ts_udp::TsUdpSource::connect`]'s
    /// PMT wait.
    pub async fn connect(&self) -> Result<TsHttpSession> {
        let parsed = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad TS-over-HTTP URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        let credentials = resolve_credentials(self.auth.clone(), credentials_from_url(&parsed)?);
        let clean_url = strip_userinfo(&parsed)?;

        let client = Client::builder()
            .build()
            .map_err(|e| MultimuxError::Connect {
                reason: format!("reqwest client: {e}"),
            })?;
        let response = authenticated_get(&client, clean_url.as_str(), credentials.as_ref()).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
                MultimuxError::Auth {
                    reason: format!("ts/http: {status}"),
                }
            } else {
                MultimuxError::Connect {
                    reason: format!("ts/http: HTTP {status}"),
                }
            });
        }

        let mut stream: BoxStream<'static, reqwest::Result<Vec<u8>>> = response
            .bytes_stream()
            .map(|item| item.map(|b| b.to_vec()))
            .boxed();
        let mut demux = StreamingTsDemux::new();
        let mut specs: Vec<TrackSpec> = Vec::new();

        let wait_for_tracks = async {
            loop {
                let Some(chunk) = stream.next().await else {
                    return Err(MultimuxError::Connect {
                        reason: "ts/http: stream ended before PMT resolved".into(),
                    });
                };
                let chunk = chunk.map_err(|e| MultimuxError::Connect {
                    reason: format!("ts/http stream read: {e}"),
                })?;
                demux.feed(&chunk);
                let mut resolved = false;
                while let Some(event) = demux.poll_event() {
                    match event {
                        DemuxEvent::TrackAdded(track) => specs.push(track.spec.clone()),
                        DemuxEvent::TracksResolved => resolved = true,
                        _ => {}
                    }
                }
                if resolved && !specs.is_empty() {
                    return Ok::<(), MultimuxError>(());
                }
            }
        };

        let connect_timeout = self.timeouts.connect;
        match tokio::time::timeout(connect_timeout, wait_for_tracks).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(MultimuxError::Connect {
                    reason: format!(
                        "ts/http: no PMT-declared track resolved within {connect_timeout:?}"
                    ),
                });
            }
        }

        let known_track_ids: BTreeSet<u32> = specs.iter().map(|s| s.track_id).collect();
        Ok(TsHttpSession {
            stream,
            demux,
            specs,
            known_track_ids,
            read_timeout: self.timeouts.read,
        })
    }
}

impl Source for TsHttpSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live TS-over-HTTP session: a streaming response body, feeding a
/// [`StreamingTsDemux`].
pub struct TsHttpSession {
    stream: BoxStream<'static, reqwest::Result<Vec<u8>>>,
    demux: StreamingTsDemux,
    specs: Vec<TrackSpec>,
    /// Track ids known at connect time — a `Sample` for any later-discovered
    /// track (e.g. a PMT version bump after `connect` returned) is dropped
    /// rather than surfaced for a track the segmenter was never built with,
    /// mirroring [`crate::source::ts_udp::TsUdpSession::next_samples`]'s
    /// "unrouted track -> ignored" handling.
    known_track_ids: BTreeSet<u32>,
    /// Bound on each [`Self::next_samples`] read — see
    /// [`IngestTimeouts::read`].
    read_timeout: Duration,
}

impl TsHttpSession {
    /// The `TrackSpec`s resolved during [`TsHttpSource::connect`]'s PMT
    /// wait.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Reads the next chunk of the HTTP body and feeds it to the demuxer,
    /// returning every completed sample it yields for a track known at
    /// connect time.
    ///
    /// Returns `Ok(None)` once the body stream ends — unlike
    /// [`crate::source::ts_udp::TsUdpSession::next_samples`] (UDP has no
    /// transport-level end-of-stream signal), an HTTP response body *does*
    /// end, and that end is exactly the "reconnect" signal
    /// [`crate::origin::supervisor::supervise`] is built to act on.
    ///
    /// Bounded by [`IngestTimeouts::read`] (issue #663 P5, audit-ingest #3):
    /// a server that stops sending chunks without closing the connection
    /// (wedged origin) would otherwise leave this `.await` pending forever —
    /// a timed-out read surfaces as a [`MultimuxError::Connect`], reconnected
    /// by the supervisor exactly like any other read error.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let read_timeout = self.read_timeout;
        let Some(chunk) = tokio::time::timeout(read_timeout, self.stream.next())
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: format!("ts/http stream read: no data within {read_timeout:?}"),
            })?
        else {
            return Ok(None);
        };
        let chunk = chunk.map_err(|e| MultimuxError::Connect {
            reason: format!("ts/http stream read: {e}"),
        })?;
        self.demux.feed(&chunk);
        let mut out = Vec::new();
        while let Some(event) = self.demux.poll_event() {
            if let DemuxEvent::Sample { track_id, sample } = event {
                if self.known_track_ids.contains(&track_id) {
                    out.push((track_id, sample));
                }
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use broadcast_common::Package;
    use transmux::TsMux;
    use transmux::media::Track;
    use transmux::pipeline::CodecConfig;

    /// Builds a real (not hand-faked) MPEG-2 TS byte stream carrying one
    /// H.264 video track with a handful of access units, by round-tripping
    /// through the workspace's own `transmux::TsMux` packager — mirrors
    /// `source::ts_udp`'s own `build_ts_bytes` test helper exactly.
    fn build_ts_bytes() -> Vec<u8> {
        let avc = transmux::avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        let spec = TrackSpec::new(
            1,
            90_000,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        );
        let frame_dur = 90_000 / 30;
        let samples: Vec<Sample> = (0..10u32)
            .map(|i| {
                let nal = [0x65u8, 0xAA, i as u8];
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                Sample::new(data, frame_dur, i == 0, 0)
            })
            .collect();
        let track = Track::new(spec, samples);
        let media = transmux::media::Media::new(vec![track], 90_000);
        TsMux::default().package(&media).expect("mux to TS")
    }

    /// Starts a tiny axum server streaming `body` in fixed-size chunks (a
    /// real chunked-transfer HTTP response, not a single buffered body) at
    /// `/stream.ts`, returning its base URL. `auth`, if given, gates every
    /// request behind that scheme (see `crate::testutil::require_auth`) —
    /// used by the auth-scheme biting tests below; `None` (the plain
    /// `start_chunked_ts_server` case) mirrors the original no-auth server.
    async fn start_chunked_ts_server_with_auth(
        body: Vec<u8>,
        auth: Option<crate::testutil::MockAuthScheme>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(body: axum::extract::State<Vec<u8>>) -> AxumResponse {
            // Stream in small chunks so the client genuinely reads the body
            // incrementally (biting on `bytes_stream()`, not just a single
            // buffered read).
            let chunks: Vec<std::result::Result<Vec<u8>, std::io::Error>> =
                body.0.chunks(7 * 188).map(|c| Ok(c.to_vec())).collect();
            let stream = futures_util::stream::iter(chunks);
            let body = Body::from_stream(stream);
            ([(axum::http::header::CONTENT_TYPE, "video/mp2t")], body).into_response()
        }
        let mut app = Router::new()
            .route("/stream.ts", get(handler))
            .with_state(body);
        if let Some(scheme) = auth {
            app = crate::testutil::require_auth(app, scheme);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/stream.ts"), server)
    }

    /// Starts a plain (no-auth) chunked TS server — see
    /// [`start_chunked_ts_server_with_auth`].
    async fn start_chunked_ts_server(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        start_chunked_ts_server_with_auth(body, None).await
    }

    /// Loopback biting test: a real axum server streams a real muxed TS
    /// fixture over chunked HTTP; asserts `TsHttpSource` resolves the track
    /// set and yields real depayloaded samples.
    #[tokio::test]
    async fn loopback_http_ts_yields_samples_after_pmt_resolves() {
        let ts_bytes = build_ts_bytes();
        let (url, server) = start_chunked_ts_server(ts_bytes).await;

        let source = TsHttpSource::new("cam-ts-http", url);
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");

        let specs = session.track_specs();
        assert_eq!(specs.len(), 1, "one video track from the muxed TS");
        assert_eq!(specs[0].timescale, 90_000);

        let mut samples = Vec::new();
        while let Ok(Ok(Some(batch))) =
            tokio::time::timeout(Duration::from_millis(500), session.next_samples()).await
        {
            samples.extend(batch);
        }
        assert!(
            !samples.is_empty(),
            "expected at least one sample from the muxed TS stream over HTTP"
        );

        server.abort();
    }

    /// A `404` (or any non-2xx) must fail `connect`, not silently proceed as
    /// if a track set would eventually resolve.
    #[tokio::test]
    async fn connect_fails_on_non_success_status() {
        let app = Router::new().route(
            "/nope.ts",
            get(|| async { axum::http::StatusCode::NOT_FOUND }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });

        let source = TsHttpSource::new("cam-ts-http", format!("http://{addr}/nope.ts"));
        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out");
        assert!(result.is_err(), "a 404 must fail connect()");

        server.abort();
    }

    /// Biting test (issue #663 P5, audit-ingest #3): a server that resolves
    /// the track set and then goes silent — never sends another chunk, never
    /// closes the connection — must fail `next_samples()` within the
    /// configured [`IngestTimeouts::read`], not hang forever (the exact
    /// wedged/half-open failure mode a "no read timeout" `TsHttpSession`
    /// used to hang on). A raw TCP listener (not the axum chunked helper
    /// above, which always closes at end-of-body) plays the "accepts then
    /// stalls mid-body" server: valid headers + the PMT-resolving TS bytes,
    /// promised via a `Content-Length` far larger than what's ever actually
    /// written, so the client's body stream genuinely waits for more.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_mid_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let ts_bytes = build_ts_bytes();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");

        let _server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await; // drain the request, unparsed
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\n\r\n",
                ts_bytes.len() * 10 // promise far more than will ever be sent
            );
            stream
                .write_all(header.as_bytes())
                .await
                .expect("write header");
            stream
                .write_all(&ts_bytes)
                .await
                .expect("write body prefix");
            // Go silent forever: never write again, never close — the
            // stalled/wedged-server failure mode.
            std::future::pending::<()>().await;
        });

        let source = TsHttpSource::new("stalled", format!("http://{addr}/stream.ts"))
            .with_timeouts(IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_millis(100),
            });
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect resolves tracks from the already-sent prefix");

        let result = tokio::time::timeout(Duration::from_secs(5), session.next_samples())
            .await
            .expect(
                "next_samples() must return on its own via IngestTimeouts::read, \
                 not hang until this test's own backstop timeout",
            );
        assert!(
            result.is_err(),
            "a server that goes silent mid-body must fail next_samples(), not hang forever"
        );
    }

    // --- issue #663 "Finish client-side multi-scheme auth": Basic/Digest/
    // Bearer/wrong-creds against a real mock auth server ---

    const AUTH_USER: &str = "cam-user";
    const AUTH_PASS: &str = "cam-pass";
    const DIGEST_REALM: &str = "mock realm";
    const BEARER_TOKEN: &str = "ts-http-bearer-token";

    /// Drives a `TsHttpSource` to connect and pull every sample the server
    /// serves, returning the sample count — the common "auth worked, real
    /// media came out" assertion shared by the Basic/Digest/Bearer tests
    /// below.
    async fn connect_and_drain(source: TsHttpSource) -> Result<usize> {
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: "connect timed out".into(),
            })??;
        let mut total = 0usize;
        while let Ok(Some(batch)) =
            tokio::time::timeout(Duration::from_millis(500), session.next_samples())
                .await
                .unwrap_or(Ok(None))
        {
            total += batch.len();
        }
        Ok(total)
    }

    /// Basic (RFC 7617), credentials from URL userinfo: the server issues a
    /// Basic challenge, `TsHttpSource` answers it via
    /// `source::http_auth::authenticated_get`'s retry path, and real samples
    /// come out.
    #[tokio::test]
    async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes();
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Basic {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
            }),
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);

        let source = TsHttpSource::new("cam-basic", credentialed);
        let total = connect_and_drain(source)
            .await
            .expect("Basic auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Basic auth");

        server.abort();
    }

    /// Digest (RFC 7616), credentials from URL userinfo: the server issues a
    /// real Digest challenge (nonce/realm/qop=auth) via a real
    /// `broadcast_auth::Verifier` (`crate::testutil::require_auth`) that
    /// independently recomputes the expected response — a client that can't
    /// answer it gets nothing back, so this proves `TsHttpSource` actually
    /// computed a correct Digest response via `broadcast-auth`, not just
    /// echoed something Digest-shaped.
    #[tokio::test]
    async fn digest_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes();
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);

        let source = TsHttpSource::new("cam-digest", credentialed);
        let total = connect_and_drain(source)
            .await
            .expect("Digest auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Digest auth");

        server.abort();
    }

    /// Bearer (RFC 6750), config-supplied (the only way to supply one — it
    /// has no URL-userinfo form): `TsHttpSource::with_auth` overrides the
    /// (bare, no-userinfo) connect URL.
    #[tokio::test]
    async fn bearer_auth_config_supplied_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes();
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Bearer {
                token: BEARER_TOKEN.into(),
            }),
        )
        .await;

        let source =
            TsHttpSource::new("cam-bearer", url).with_auth(Some(Credentials::bearer(BEARER_TOKEN)));
        let total = connect_and_drain(source)
            .await
            .expect("config-supplied Bearer token must authenticate");
        assert!(total > 0, "expected real samples after Bearer auth");

        server.abort();
    }

    /// Config-supplied auth takes precedence over URL userinfo: the URL
    /// carries a *wrong* password, but `TsHttpSource::with_auth` supplies the
    /// correct one — connect must succeed on the config auth, proving
    /// `resolve_credentials` really overrides rather than merely falling
    /// back.
    #[tokio::test]
    async fn config_auth_overrides_wrong_url_userinfo() {
        let ts_bytes = build_ts_bytes();
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let wrong_url_creds = url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);

        let source = TsHttpSource::new("cam-override", wrong_url_creds)
            .with_auth(Some(Credentials::new(AUTH_USER, AUTH_PASS)));
        let total = connect_and_drain(source)
            .await
            .expect("config auth must override the URL's wrong userinfo password");
        assert!(
            total > 0,
            "expected real samples via config-overridden auth"
        );

        server.abort();
    }

    /// Wrong credentials must fail `connect()` (stay `401`), not hang or
    /// silently proceed — the negative counterpart to the three tests above,
    /// proving they actually bite (a client answering with the wrong
    /// password gets rejected exactly like a client with none).
    #[tokio::test]
    async fn wrong_credentials_stay_401_and_connect_errors() {
        let ts_bytes = build_ts_bytes();
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let wrong_creds = url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);

        let source = TsHttpSource::new("cam-wrong", wrong_creds);
        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect must not hang against a persistent 401");
        assert!(
            result.is_err(),
            "wrong credentials must fail connect(), not silently proceed"
        );

        server.abort();
    }
}
