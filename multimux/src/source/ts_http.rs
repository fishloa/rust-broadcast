//! MPEG-2 Transport Stream over HTTP ingest source (issue #663 P3c): a
//! streaming HTTP GET (chunked/progressive, `reqwest`) feeding transmux's
//! incremental [`transmux::StreamingTsDemux`] — mirrors
//! [`crate::source::ts_udp`] almost exactly, just over an HTTP byte stream
//! instead of UDP datagrams. Auth (Basic/Digest/Bearer, from the URL's
//! userinfo) is answered once via
//! `crate::source::http_auth::authenticated_get`, which delegates to
//! `broadcast-auth` for the Digest challenge/response — `reqwest` itself has
//! no Digest support.
//!
//! Unlike UDP (connectionless, never signals end-of-stream),
//! [`TsHttpSession::next_samples`] returns `Ok(None)` once the HTTP body
//! stream ends (server closed the connection, or the response completed
//! normally) — [`crate::origin::supervisor::supervise`] then reconnects with
//! backoff, exactly as it does for any other source's end-of-stream.

use std::collections::BTreeSet;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use reqwest::Client;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::Source;
use crate::source::http_auth::{authenticated_get, credentials_from_url, strip_userinfo};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingTsDemux};

/// Bound on how long [`TsHttpSource::connect`] waits for the PMT to resolve
/// (every currently-declared track known) before giving up — mirrors
/// `source::ts_udp`'s own `CONNECT_TIMEOUT`, and for the same reason: a
/// source that never sends usable PSI (or a slow/unreachable origin) would
/// otherwise hang `connect` forever, starving
/// [`crate::origin::supervisor::supervise`]'s backoff of a chance to retry.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// An MPEG-2 TS-over-HTTP stream to pull: an `http(s)://` URL, which may
/// carry `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct TsHttpSource {
    name: String,
    url: String,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim.
impl std::fmt::Debug for TsHttpSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsHttpSource")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .finish()
    }
}

impl TsHttpSource {
    /// Build a source descriptor.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        TsHttpSource {
            name: name.into(),
            url: url.into(),
        }
    }

    /// Opens a streaming GET to the configured URL (answering a `401`
    /// challenge if the URL carried credentials — see
    /// `crate::source::http_auth::authenticated_get`), then reads response
    /// chunks into a [`StreamingTsDemux`] until every currently PMT-declared
    /// track has resolved (or `CONNECT_TIMEOUT` elapses) — the
    /// TS-over-HTTP analogue of [`crate::source::ts_udp::TsUdpSource::connect`]'s
    /// PMT wait.
    pub async fn connect(&self) -> Result<TsHttpSession> {
        let parsed = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad TS-over-HTTP URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        let credentials = credentials_from_url(&parsed)?;
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

        match tokio::time::timeout(CONNECT_TIMEOUT, wait_for_tracks).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(MultimuxError::Connect {
                    reason: format!(
                        "ts/http: no PMT-declared track resolved within {CONNECT_TIMEOUT:?}"
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
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let Some(chunk) = self.stream.next().await else {
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
    /// `/stream.ts`, returning its base URL.
    async fn start_chunked_ts_server(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
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
        let app = Router::new()
            .route("/stream.ts", get(handler))
            .with_state(body);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/stream.ts"), server)
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
}
