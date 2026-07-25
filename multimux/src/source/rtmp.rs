//! RTMP push ingest source (issue #738): binds a listen port and accepts
//! inbound RTMP publishers (`rtmp_runtime::io::AsyncRtmpServer`), demuxing
//! each accepted publisher's FLV byte stream via
//! [`transmux::StreamingFlvDemux`] — the FLV analogue of [`crate::source::ts_http`]'s
//! `StreamingTsDemux` pull loop, driven identically: `feed` + a
//! `while let Some(ev) = demux.poll_event()` drain.
//!
//! Unlike every other source in this module, [`RtmpSource`] is push, not
//! pull: nothing is dialled out to. [`RtmpSource::connect`] instead binds
//! the listen socket **once** (lazily, via a [`tokio::sync::OnceCell`]) and
//! reuses it across reconnects — so a publisher disconnecting only costs
//! the next `accept()`, never a `:1935` re-bind race with
//! [`crate::origin::supervisor::supervise`]'s backoff loop.
//!
//! FLV carries no PMT-style "this many tracks are coming" declaration (see
//! [`transmux::flv_stream`]'s module doc, "No `TracksResolved`"), so
//! [`RtmpSource::connect`] instead waits for the first
//! [`rtmp_runtime::server::ServerEvent::Media`] batch that resolves *any*
//! track, then returns — exactly mirroring [`crate::source::ts_http`]'s
//! per-chunk PMT wait, just without a `TracksResolved` flag to check.
//! [`RtmpSession::next_samples`] ignores samples for any track that
//! resolves later (an audio sequence header arriving in a subsequent
//! read), same "unrouted track -> ignored" policy as
//! [`crate::source::ts_http::TsHttpSession::next_samples`] applies to a
//! post-connect PMT version bump.

use std::collections::BTreeSet;
use std::sync::Arc;

use rtmp_runtime::io::{AsyncRtmpServer, RtmpConnection};
use rtmp_runtime::server::{ServerConfig, ServerEvent};
use tokio::sync::OnceCell;
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingFlvDemux};

use crate::error::{MultimuxError, Result};
use crate::source::Source;

/// An RTMP push-ingest listener: binds `listen` once and accepts publishers
/// against it, gated by an optional `app`/`stream_key` (see
/// [`Self::with_app`]/[`Self::with_stream_key`]).
pub struct RtmpSource {
    name: String,
    listen: String,
    app: Option<String>,
    stream_key: Option<String>,
    /// Bind-once, reuse-forever: see the module doc.
    server: OnceCell<Arc<AsyncRtmpServer>>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`), mirroring
/// [`crate::source::ts_http::TsHttpSource`]'s: `stream_key` is redacted even
/// though it's a shared secret rather than a password, for the same reason
/// — it should never turn up verbatim in a log line.
impl std::fmt::Debug for RtmpSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtmpSource")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("app", &self.app)
            .field("stream_key", &self.stream_key.as_ref().map(|_| "***"))
            .finish()
    }
}

impl RtmpSource {
    /// Build a source descriptor listening on `listen` (e.g. `"0.0.0.0:1935"`,
    /// or `"127.0.0.1:0"` for an ephemeral test port).
    pub fn new(name: impl Into<String>, listen: impl Into<String>) -> Self {
        RtmpSource {
            name: name.into(),
            listen: listen.into(),
            app: None,
            stream_key: None,
            server: OnceCell::new(),
        }
    }

    /// Requires the publisher's `connect` `app` name to match exactly, if
    /// set — enforced in [`Self::connect`] once the first
    /// `Connected { app }` event arrives (`ServerConfig` has no `app`
    /// gate of its own, unlike `expected_stream_key`).
    #[must_use]
    pub fn with_app(mut self, app: Option<String>) -> Self {
        self.app = app;
        self
    }

    /// Requires the publisher's `publish` stream key to match exactly, if
    /// set — passed straight through as
    /// [`ServerConfig::expected_stream_key`], so a mismatched key is
    /// rejected by the sans-IO session itself (`NetStream.Publish.BadName`)
    /// before this source ever sees a `Publish`/`Media` event.
    #[must_use]
    pub fn with_stream_key(mut self, stream_key: Option<String>) -> Self {
        self.stream_key = stream_key;
        self
    }

    /// Binds the listen socket on first call (reused on every subsequent
    /// call/reconnect — see the module doc), accepts the next publisher,
    /// and drives it until the first batch of
    /// [`ServerEvent::Media`] resolves at least one track, returning a
    /// [`RtmpSession`] built from the resolved [`TrackSpec`]s.
    ///
    /// A `Sample` event surfacing before every track in that first batch
    /// has resolved is discarded, not buffered — mirrors
    /// [`crate::source::ts_http::TsHttpSource::connect`]'s PMT wait loop,
    /// which applies the same discipline for the same reason (nothing
    /// downstream can consume a sample for a track whose `TrackSpec` isn't
    /// known yet).
    pub async fn connect(&self) -> Result<RtmpSession> {
        let server = self
            .server
            .get_or_try_init(|| async {
                let config = ServerConfig {
                    expected_stream_key: self.stream_key.clone(),
                    ..ServerConfig::default()
                };
                AsyncRtmpServer::bind(self.listen.as_str(), config)
                    .await
                    .map(Arc::new)
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("rtmp: bind {}: {e}", self.listen),
                    })
            })
            .await?;

        let mut conn = server.accept().await.map_err(|e| MultimuxError::Connect {
            reason: format!("rtmp: accept: {e}"),
        })?;

        let mut demux = StreamingFlvDemux::new();
        let mut specs: Vec<TrackSpec> = Vec::new();

        loop {
            let Some(events) = conn
                .next_events()
                .await
                .map_err(|e| MultimuxError::Connect {
                    reason: format!("rtmp: {e}"),
                })?
            else {
                return Err(MultimuxError::Connect {
                    reason: "rtmp: connection ended before any track resolved".into(),
                });
            };

            for event in events {
                match event {
                    ServerEvent::Connected { app } => {
                        if let Some(expected) = &self.app {
                            if &app != expected {
                                return Err(MultimuxError::Connect {
                                    reason: format!(
                                        "rtmp: app {app:?} does not match configured app {expected:?}"
                                    ),
                                });
                            }
                        }
                    }
                    ServerEvent::Media { flv } => {
                        demux.feed(&flv).map_err(|e| MultimuxError::Depay {
                            reason: format!("rtmp/flv: {e}"),
                        })?;
                    }
                    _ => {}
                }
            }

            while let Some(ev) = demux.poll_event() {
                if let DemuxEvent::TrackAdded(track) = ev {
                    specs.push(track.spec.clone());
                }
            }

            if !specs.is_empty() {
                break;
            }
        }

        let known_track_ids: BTreeSet<u32> = specs.iter().map(|s| s.track_id).collect();
        Ok(RtmpSession {
            conn,
            demux,
            specs,
            known_track_ids,
        })
    }
}

impl Source for RtmpSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live RTMP publish session: an accepted [`RtmpConnection`], feeding a
/// [`StreamingFlvDemux`].
pub struct RtmpSession {
    conn: RtmpConnection,
    demux: StreamingFlvDemux,
    specs: Vec<TrackSpec>,
    /// Track ids known at connect time — a `Sample` for any later-resolved
    /// track is dropped rather than surfaced for a track the segmenter was
    /// never built with. See [`RtmpSource::connect`]'s doc.
    known_track_ids: BTreeSet<u32>,
}

impl RtmpSession {
    /// The `TrackSpec`s resolved during [`RtmpSource::connect`]'s wait.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Reads the next batch of RTMP events and feeds every
    /// [`ServerEvent::Media`] tag run to the FLV demux, returning every
    /// completed sample it yields for a track known at connect time.
    ///
    /// Returns `Ok(None)` once the connection ends (clean EOF, or the
    /// publisher's `deleteStream`/`FCUnpublish`) *and* [`StreamingFlvDemux::finish`]'s
    /// flush of each track's trailing pending sample has already been
    /// drained — [`crate::origin::supervisor::supervise`] then reconnects
    /// (accepting the next publisher) with backoff, exactly as it does for
    /// any other source's end-of-stream.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let events = self
            .conn
            .next_events()
            .await
            .map_err(|e| MultimuxError::Connect {
                reason: format!("rtmp: {e}"),
            })?;

        let Some(events) = events else {
            // Connection ended: flush each track's trailing pending sample
            // (idempotent — a following call, if any, drains nothing new).
            self.demux.finish();
            let out = drain_known_samples(&mut self.demux, &self.known_track_ids);
            return Ok(if out.is_empty() { None } else { Some(out) });
        };

        for event in events {
            if let ServerEvent::Media { flv } = event {
                self.demux.feed(&flv).map_err(|e| MultimuxError::Depay {
                    reason: format!("rtmp/flv: {e}"),
                })?;
            }
        }

        Ok(Some(drain_known_samples(
            &mut self.demux,
            &self.known_track_ids,
        )))
    }
}

/// Drains every pending [`DemuxEvent::Sample`] from `demux`, keeping only
/// samples for a track id in `known_track_ids` — shared by
/// [`RtmpSession::next_samples`]'s live-read and end-of-stream-flush paths.
fn drain_known_samples(
    demux: &mut StreamingFlvDemux,
    known_track_ids: &BTreeSet<u32>,
) -> Vec<(u32, Sample)> {
    let mut out = Vec::new();
    while let Some(ev) = demux.poll_event() {
        if let DemuxEvent::Sample { track_id, sample } = ev {
            if known_track_ids.contains(&track_id) {
                out.push((track_id, sample));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::supervisor::{Backoff, supervise};
    use crate::store::MediaStore;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::watch;

    /// The real ffmpeg-captured RTMP publish (`app=live`, `stream_key=testkey`,
    /// H.264+AAC) — copied into this crate for hermeticity, see
    /// `tests/fixtures/PROVENANCE.md`. Original: `rtmp-runtime/tests/fixtures/obs-publish.bin`.
    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/rtmp-obs-publish.bin"
    );

    fn load_fixture() -> Vec<u8> {
        std::fs::read(FIXTURE).expect("read tests/fixtures/rtmp-obs-publish.bin")
    }

    /// Polls `f` every millisecond until it returns `true` or `timeout`
    /// elapses — mirrors `origin::supervisor`'s own test helper of the same
    /// name.
    async fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if f() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    /// Loopback biting test (issue #738 Task 11b): a real TCP client plays
    /// back the captured ffmpeg publish against a real `RtmpSource` bound to
    /// an ephemeral loopback port, driven through the real
    /// `supervise`/`run_pipeline` machinery into a real `MediaStore` —
    /// proving the whole chain (`AsyncRtmpServer` accept ->
    /// `StreamingFlvDemux` -> `LlHlsSegmenter` -> `MediaStore`) actually
    /// moves real H.264/AAC samples, not just that `connect()` resolves
    /// specs. Mutation check: if `RtmpSource`/`RtmpSession` dropped every
    /// `Media` event (or never fed the demux), the store would stay empty
    /// and this assertion would fail.
    #[tokio::test]
    async fn loopback_rtmp_publish_lands_media_in_the_store() {
        // Reserve an ephemeral port up front (same technique `ts_udp`'s own
        // loopback test uses): `RtmpSource::connect` needs to know its
        // listen address before the client can dial it, but the bound
        // listener isn't observable until `connect()` (blocked on
        // `accept()`) returns. Dropping an unaccepted `TcpListener` frees
        // the port immediately — no `TIME_WAIT` (that only applies to an
        // actually-established connection's 4-tuple, and none was ever
        // accepted here).
        let reserved = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let source = RtmpSource::new("cam-rtmp", addr.to_string());

        let fixture = load_fixture();
        let client = tokio::spawn(async move {
            // The supervised `RtmpSource` binds asynchronously (inside the
            // spawned `supervise` task below), so the listener may not be up
            // yet the instant this task starts — retry the connect for a
            // short window rather than racing it.
            let mut stream = None;
            for _ in 0..200 {
                match TcpStream::connect(addr).await {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
            let mut stream = stream.expect("connect to RtmpSource's listener");
            stream
                .write_all(&fixture)
                .await
                .expect("write fixture publish bytes");
            // Drain replies so the server's writes never block.
            let mut sink = [0u8; 8192];
            loop {
                match stream.read(&mut sink).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        let store = Arc::new(MediaStore::new(1.0, 500, 8));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let backoff = Backoff::new(Duration::from_millis(1), Duration::from_millis(20), 2.0);
        let handle = tokio::spawn(supervise(
            source,
            store.clone(),
            1.0,
            500,
            backoff,
            "test-rtmp".to_string(),
            shutdown_rx,
        ));

        let landed = wait_until(Duration::from_secs(10), || store.init_bytes().is_some()).await;
        assert!(
            landed,
            "RTMP publish must land an init segment in the store"
        );

        // `latest_progress()` returns `(in-progress segment seq, live part
        // count)` straight from the store's own segment/part bookkeeping —
        // unlike sniffing the rendered playlist text, this can't false-
        // positive on the `#EXT-X-PART-INF` header line every playlist
        // carries unconditionally (as distinct from a genuine `#EXT-X-PART:`
        // entry, which only appears once a real part has landed).
        let landed_media = wait_until(Duration::from_secs(10), || {
            store.latest_progress().1 > 0 || !store.window_segments().is_empty()
        })
        .await;
        assert!(
            landed_media,
            "RTMP publish must land at least one real part/segment in the store"
        );

        client.await.expect("client task must not panic");
        shutdown_tx.send(true).ok();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("supervise returns promptly on shutdown")
            .expect("supervise task did not panic");
    }
}
