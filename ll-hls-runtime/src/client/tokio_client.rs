//! `TokioClient` — a tokio + reqwest IO adapter driving
//! [`crate::client::LlHlsClient`] over real HTTP (issue #717 slice 5).
//!
//! Feature-gated behind `tokio` (NOT default): the sans-IO core in
//! `engine.rs` has zero dependency on tokio, reqwest, a socket, or a clock —
//! this module is a thin async shell that performs the actual HTTP GETs the
//! core's [`crate::client::Action`]s describe (playlist reload, incl. blocking
//! `_HLS_msn`/`_HLS_part`; resource fetch, incl. `Range` byte-ranges) and
//! feeds the responses back into the core, looping until the caller stops
//! polling or the stream ends.
//!
//! # Auth
//!
//! [`TokioClientConfig::auth`] supports HTTP Basic (RFC 7617) and Bearer
//! (RFC 6750) via reqwest's own request-builder helpers
//! (`RequestBuilder::basic_auth`/`bearer_auth`).
//!
//! **TODO** (tracked against issue #717's follow-ups): once the workspace
//! grows a shared multi-scheme HTTP auth crate (Basic/Digest/Bearer,
//! alongside `rtsp-runtime`'s own `http-auth`-based negotiation), replace
//! this ad hoc `Auth` enum with that shared implementation rather than
//! reqwest's built-in helpers — Digest auth in particular is not something
//! reqwest supports natively.
//!
//! # Error recovery
//!
//! - A **resource** (init/part/segment) fetch that keeps failing is retried
//!   up to [`TokioClientConfig::max_resource_retries`] times with capped
//!   exponential backoff, then [`crate::client::LlHlsClient::on_error`] is
//!   called and the adapter moves on — the sans-IO core un-marks that
//!   resource as "requested", so the *next* playlist reload naturally
//!   re-requests it (see `engine.rs`'s `on_error` docs). One flaky fetch
//!   never stalls the whole client.
//! - A **playlist** reload has no such fallback in the sans-IO core — unlike
//!   a resource, [`crate::client::LlHlsClient::on_error`] with `None` does not
//!   re-queue anything (there is no "next reload" to fall back to; the
//!   *current* reload IS the mechanism that discovers what to fetch next).
//!   So a playlist fetch is retried indefinitely with capped backoff
//!   ([`TokioClientConfig::retry_backoff`]/[`TokioClientConfig::max_retry_backoff`])
//!   rather than ever giving up — a caller wanting a hard ceiling on how long
//!   [`TokioClient::next_output`] may block should wrap it in
//!   `tokio::time::timeout` itself.

use std::time::Duration;

use reqwest::Client;

use super::{Action, LlHlsClient, Output, ResourceId};

/// Authentication attached to every request [`TokioClient`] makes. See the
/// module docs' "Auth" section for the planned replacement.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Auth {
    /// HTTP Basic (RFC 7617).
    Basic {
        /// Username.
        username: String,
        /// Password, if any.
        password: Option<String>,
    },
    /// HTTP Bearer (RFC 6750).
    Bearer {
        /// The bearer token.
        token: String,
    },
}

/// Errors from the tokio IO adapter itself — distinct from
/// [`crate::client::Error`], the sans-IO core's own parse/demux error type
/// (wrapped here via
/// [`TokioError::Client`]).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TokioError {
    /// The underlying HTTP request failed outright (connect/timeout/TLS/
    /// transport error) — never a non-2xx response, see [`Self::Status`].
    #[error("HTTP request to {url} failed: {source}")]
    Http {
        /// The request URL.
        url: String,
        /// The underlying reqwest error.
        #[source]
        source: reqwest::Error,
    },
    /// The server returned a non-success HTTP status.
    #[error("HTTP {status} fetching {url}")]
    Status {
        /// The request URL.
        url: String,
        /// The response status.
        status: reqwest::StatusCode,
    },
    /// The sans-IO core rejected the fetched playlist/resource — see
    /// [`crate::client::Error`] for the underlying reason.
    #[error(transparent)]
    Client(#[from] super::Error),
}

/// Tunables for [`TokioClient`]. [`Default`] gives sane values for a
/// well-behaved LL-HLS origin reachable over a real (or loopback) network.
#[derive(Debug, Clone)]
pub struct TokioClientConfig {
    /// Per-request timeout for a plain (non-blocking) playlist GET, or a
    /// resource (init/part/segment) GET.
    pub request_timeout: Duration,
    /// Per-request timeout for a **blocking** Playlist Reload
    /// (`_HLS_msn`/`_HLS_part`, RFC 8216bis §6.2.5.2) — must exceed the
    /// origin's own blocking hold time (e.g. `multimux`'s `LlHlsOutput` caps
    /// at 5s) with headroom, since the origin is expected to hold the
    /// response open until new content exists or its own cap elapses.
    pub blocking_timeout: Duration,
    /// How many times a **resource** fetch is retried (capped exponential
    /// backoff) before the adapter gives up on that specific fetch and moves
    /// on (see the module docs' "Error recovery" section). A playlist reload
    /// is never subject to this cap — it retries indefinitely.
    pub max_resource_retries: u32,
    /// Initial backoff between retry attempts; doubles per attempt up to
    /// [`Self::max_retry_backoff`].
    pub retry_backoff: Duration,
    /// Ceiling the doubled [`Self::retry_backoff`] is capped at.
    pub max_retry_backoff: Duration,
    /// Optional auth attached to every request (see the module docs).
    pub auth: Option<Auth>,
}

impl Default for TokioClientConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(5),
            blocking_timeout: Duration::from_secs(10),
            max_resource_retries: 3,
            retry_backoff: Duration::from_millis(200),
            max_retry_backoff: Duration::from_secs(2),
            auth: None,
        }
    }
}

/// Diagnostic counters for what [`TokioClient`] has actually done —
/// distinguishing "parsed an LL-HLS tag" from "acted on it", the same bar
/// this crate's own acceptance tests hold the adapter to (issue #717's
/// "blocking-reload + preload-hint prefetch actually exercised" acceptance
/// item). Also useful to a real caller for observability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokioClientStats {
    /// Playlist GETs performed (blocking + non-blocking).
    pub playlist_fetches: u64,
    /// Of those, how many carried `_HLS_msn`/`_HLS_part` — a Blocking
    /// Playlist Reload (RFC 8216bis §6.2.5.2).
    pub blocking_reloads: u64,
    /// Resource (init/part/segment) GETs performed.
    pub resource_fetches: u64,
    /// Of those, how many were for the exact URL most recently named by the
    /// playlist's `#EXT-X-PRELOAD-HINT` (RFC 8216bis §4.4.5.3) — i.e.
    /// fetched ahead of that resource's own numbered (`#EXT-X-PART`)
    /// appearance, not merely alongside it.
    pub preload_hint_resource_fetches: u64,
}

/// An async shell driving [`crate::client::LlHlsClient`] over real HTTP.
///
/// ```no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use ll_hls_runtime::client::tokio_client::TokioClient;
///
/// let mut client = TokioClient::new("http://127.0.0.1:8080/live/media.m3u8")?;
/// while let Some(output) = client.next_output().await? {
///     // Hand `output` (Init / Samples / Discontinuity / EndOfStream) to a decoder.
///     let _ = output;
/// }
/// # Ok(())
/// # }
/// ```
pub struct TokioClient {
    core: LlHlsClient,
    http: Client,
    config: TokioClientConfig,
    playlist_url: String,
    stats: TokioClientStats,
    last_preload_hint_url: Option<String>,
    ended: bool,
}

impl TokioClient {
    /// Build a client for the Media Playlist at `playlist_url`, with
    /// [`TokioClientConfig::default`] tunables.
    ///
    /// # Errors
    /// Only if the underlying `reqwest::Client` fails to build (e.g. TLS
    /// backend initialization failure) — not a network error, since no
    /// request has been made yet.
    pub fn new(playlist_url: impl Into<String>) -> Result<Self, TokioError> {
        Self::with_config(playlist_url, TokioClientConfig::default())
    }

    /// Build a client with explicit [`TokioClientConfig`] tunables.
    ///
    /// # Errors
    /// See [`Self::new`].
    pub fn with_config(
        playlist_url: impl Into<String>,
        config: TokioClientConfig,
    ) -> Result<Self, TokioError> {
        let playlist_url = playlist_url.into();
        let http = Client::builder()
            .build()
            .map_err(|source| TokioError::Http {
                url: playlist_url.clone(),
                source,
            })?;
        Ok(Self {
            core: LlHlsClient::new(playlist_url.clone()),
            http,
            config,
            playlist_url,
            stats: TokioClientStats::default(),
            last_preload_hint_url: None,
            ended: false,
        })
    }

    /// Diagnostic counters for requests actually made so far — see
    /// [`TokioClientStats`].
    pub fn stats(&self) -> TokioClientStats {
        self.stats
    }

    /// Drive the sans-IO core — performing whatever HTTP its next
    /// [`Action`] needs, retrying transient failures per
    /// [`TokioClientConfig`] — until at least one [`Output`] is available.
    ///
    /// Returns `Ok(None)` once, right after [`Output::EndOfStream`] has
    /// already been yielded by a previous call, signalling the caller to
    /// stop polling. A live stream that never sends `#EXT-X-ENDLIST` simply
    /// never returns `Ok(None)` — the caller drives this in a loop (or a
    /// `tokio::select!` alongside its own shutdown signal) for as long as it
    /// wants to keep playing.
    ///
    /// # Errors
    /// [`TokioError::Client`] if the sans-IO core rejects a fetched
    /// playlist/resource (malformed playlist, demux failure). Resource fetch
    /// failures are retried/recovered internally (see the module docs) and
    /// never surface here; a playlist fetch failure retries indefinitely
    /// rather than ever returning an [`TokioError::Http`]/[`TokioError::Status`]
    /// — see the module docs' "Error recovery" section.
    pub async fn next_output(&mut self) -> Result<Option<Output>, TokioError> {
        loop {
            if let Some(out) = self.core.next_output() {
                if matches!(out, Output::EndOfStream) {
                    self.ended = true;
                }
                return Ok(Some(out));
            }
            if self.ended {
                return Ok(None);
            }

            match self.core.poll() {
                Some(action @ Action::FetchPlaylist { .. }) => {
                    let request_url = action
                        .playlist_request_url()
                        .expect("Action::FetchPlaylist always has a playlist_request_url");
                    let is_blocking = matches!(
                        action,
                        Action::FetchPlaylist {
                            blocking: Some(_),
                            ..
                        }
                    );
                    let timeout = if is_blocking {
                        self.config.blocking_timeout
                    } else {
                        self.config.request_timeout
                    };
                    let bytes = self.fetch_playlist_resilient(&request_url, timeout).await;
                    self.stats.playlist_fetches += 1;
                    if is_blocking {
                        self.stats.blocking_reloads += 1;
                    }
                    self.note_preload_hint(&bytes);
                    self.core.on_playlist(&bytes)?;
                }
                Some(Action::FetchResource {
                    id,
                    url,
                    byte_range,
                }) => match self.fetch_resource_bounded(&url, byte_range).await {
                    Ok(bytes) => {
                        self.stats.resource_fetches += 1;
                        if matches!(id, ResourceId::Part { .. })
                            && self.last_preload_hint_url.as_deref() == Some(url.as_str())
                        {
                            self.stats.preload_hint_resource_fetches += 1;
                        }
                        self.core.on_resource(id, &bytes)?;
                    }
                    Err(_source) => {
                        // Retries exhausted: un-mark as requested so the
                        // next playlist reload naturally re-requests it,
                        // rather than stalling the whole client on one bad
                        // fetch.
                        self.core.on_error(Some(id));
                    }
                },
                Some(Action::WaitMs(ms)) => {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                None => {
                    // Defensive-only: in normal operation `on_playlist`
                    // always re-queues a reload (or `WaitMs`) before
                    // returning, for as long as the stream hasn't ended, so
                    // this should never actually spin. Guard against it
                    // anyway with a short sleep rather than a hot loop.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Re-parse a just-fetched playlist purely to note its
    /// `#EXT-X-PRELOAD-HINT` URL for [`TokioClientStats`] bookkeeping — the
    /// sans-IO core does its own, authoritative parse independently inside
    /// [`crate::client::LlHlsClient::on_playlist`]; this is a deliberate, small,
    /// side-channel duplication (stats only, never fed back into scheduling)
    /// rather than growing an API on the core to expose its parsed state.
    fn note_preload_hint(&mut self, playlist_bytes: &[u8]) {
        self.last_preload_hint_url = core::str::from_utf8(playlist_bytes)
            .ok()
            .and_then(|text| transmux::hls::MediaPlaylist::parse(text).ok())
            .and_then(|pl| pl.low_latency)
            .and_then(|ll| ll.preload_hint_part)
            .map(|hint| super::url::resolve(&self.playlist_url, &hint));
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.config.auth {
            Some(Auth::Basic { username, password }) => req.basic_auth(username, password.clone()),
            Some(Auth::Bearer { token }) => req.bearer_auth(token),
            None => req,
        }
    }

    async fn fetch_bytes(
        &self,
        url: &str,
        byte_range: Option<(u64, u64)>,
        timeout: Duration,
    ) -> Result<Vec<u8>, TokioError> {
        let mut req = self.http.get(url).timeout(timeout);
        if let Some((offset, length)) = byte_range {
            let end = offset + length.saturating_sub(1);
            req = req.header(reqwest::header::RANGE, format!("bytes={offset}-{end}"));
        }
        let req = self.apply_auth(req);
        let resp = req.send().await.map_err(|source| TokioError::Http {
            url: url.to_string(),
            source,
        })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(TokioError::Status {
                url: url.to_string(),
                status,
            });
        }
        let bytes = resp.bytes().await.map_err(|source| TokioError::Http {
            url: url.to_string(),
            source,
        })?;
        Ok(bytes.to_vec())
    }

    /// Retry a playlist fetch indefinitely (capped exponential backoff) —
    /// see the module docs' "Error recovery" section for why a playlist
    /// reload, unlike a resource fetch, has no bounded-retry fallback.
    async fn fetch_playlist_resilient(&self, url: &str, timeout: Duration) -> Vec<u8> {
        let mut backoff = self.config.retry_backoff;
        loop {
            match self.fetch_bytes(url, None, timeout).await {
                Ok(bytes) => return bytes,
                Err(_source) => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(self.config.max_retry_backoff);
                }
            }
        }
    }

    /// Retry a resource fetch up to [`TokioClientConfig::max_resource_retries`]
    /// times (capped exponential backoff), then give up.
    async fn fetch_resource_bounded(
        &self,
        url: &str,
        byte_range: Option<(u64, u64)>,
    ) -> Result<Vec<u8>, TokioError> {
        let mut backoff = self.config.retry_backoff;
        let mut last_err = None;
        for _ in 0..self.config.max_resource_retries.max(1) {
            match self
                .fetch_bytes(url, byte_range, self.config.request_timeout)
                .await
            {
                Ok(bytes) => return Ok(bytes),
                Err(source) => {
                    last_err = Some(source);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(self.config.max_retry_backoff);
                }
            }
        }
        Err(last_err.expect("loop runs at least once (max_resource_retries.max(1))"))
    }
}
