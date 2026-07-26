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
//! [`RtmpSource::connect`] instead leans on that module's own documented
//! **ordering assumption** (Annex E: a track's AVC/AAC sequence-header tag
//! always precedes that track's media tags — true of every conformant
//! encoder) and waits for the first [`transmux::DemuxEvent::Sample`], not
//! just the first [`transmux::DemuxEvent::TrackAdded`]: every codec config
//! (video *and* audio) is therefore guaranteed to have already resolved by
//! the time any media sample exists, so `connect()` never returns a
//! partial (e.g. video-only) track set even when the two sequence headers
//! land in separate [`rtmp_runtime::io::RtmpConnection::next_events`] reads
//! (#738 T11b review, Important). That first `Sample` (and any others
//! drained in the same batch) is buffered into the returned [`RtmpSession`]
//! rather than discarded, so [`RtmpSession::next_samples`] returns it
//! first, before reading more — no media dropped. Only a *genuinely*
//! later-appearing track (one whose sequence header arrives after the
//! first media sample — non-conformant, but defended against anyway) is
//! ignored post-connect, mirroring
//! [`crate::source::ts_http::TsHttpSession::next_samples`]'s "unrouted
//! track -> ignored" policy for a post-connect PMT version bump.
//!
//! Both `connect()`'s track-resolution wait and `next_samples()`'s live
//! reads are bounded by an [`IngestTimeouts`] (`with_timeouts`, mirroring
//! every other source in this module): a
//! publisher that completes the handshake and then goes idle, or one that
//! connects and never sends `publish`, would otherwise hang `connect()` or
//! `next_samples()` forever — and because `supervise()` awaits `connect()`
//! sequentially against the bind-once, reused-forever listener (see above),
//! that would wedge the *entire* route, never accepting another publisher
//! (#738 T11b review, Critical).

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use rtmp_runtime::io::{AsyncRtmpServer, RtmpConnection};
use rtmp_runtime::server::{ServerConfig, ServerEvent};
use tokio::sync::OnceCell;
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingFlvDemux};

use crate::error::{MultimuxError, Result};
use crate::source::{IngestTimeouts, Source};

/// An RTMP push-ingest listener: binds `listen` once and accepts publishers
/// against it, gated by an optional `app`/`stream_key` (see
/// [`Self::with_app`]/[`Self::with_stream_key`]).
pub struct RtmpSource {
    name: String,
    listen: String,
    app: Option<String>,
    stream_key: Option<String>,
    timeouts: IngestTimeouts,
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
            timeouts: IngestTimeouts::default(),
            server: OnceCell::new(),
        }
    }

    /// Overrides the default [`IngestTimeouts`] — see
    /// `TsHttpSource::with_timeouts` for the pattern this mirrors: `connect`
    /// bounds the whole track-resolution wait, `read` bounds each
    /// `next_samples` read.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
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
    /// and drives it until the first [`DemuxEvent::Sample`] arrives,
    /// returning a [`RtmpSession`] built from every [`TrackSpec`] resolved
    /// by then (see the module doc's ordering-assumption note: every
    /// conformant sequence header precedes any media, so both tracks are
    /// guaranteed known by the time a sample exists, however many separate
    /// reads their sequence headers land across).
    ///
    /// The whole wait (accept already happened; this bounds only the
    /// handshake/track-resolution reads that follow) is bounded by
    /// [`IngestTimeouts::connect`]; on expiry the accepted connection is
    /// dropped (closing that publisher's socket so the listen port is free
    /// for the next `accept()`) and an `Err` is returned, so
    /// [`crate::origin::supervisor::supervise`] backs off and calls
    /// `connect()` again rather than wedging the whole route on a
    /// stalled/never-publishing client (#738 T11b review, Critical).
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
        let mut pending_samples: VecDeque<(u32, Sample)> = VecDeque::new();

        let wait_for_first_sample = async {
            loop {
                let Some(events) =
                    conn.next_events()
                        .await
                        .map_err(|e| MultimuxError::Connect {
                            reason: format!("rtmp: {e}"),
                        })?
                else {
                    return Err(MultimuxError::Connect {
                        reason: "rtmp: connection ended before any media sample arrived".into(),
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

                let mut saw_sample = false;
                while let Some(ev) = demux.poll_event() {
                    match ev {
                        DemuxEvent::TrackAdded(track) => specs.push(track.spec.clone()),
                        DemuxEvent::Sample { track_id, sample } => {
                            pending_samples.push_back((track_id, sample));
                            saw_sample = true;
                        }
                        _ => {}
                    }
                }

                if saw_sample {
                    return Ok::<(), MultimuxError>(());
                }
            }
        };

        let connect_timeout = self.timeouts.connect;
        match tokio::time::timeout(connect_timeout, wait_for_first_sample).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                drop(conn);
                return Err(e);
            }
            Err(_) => {
                drop(conn);
                return Err(MultimuxError::Connect {
                    reason: format!(
                        "rtmp: no media sample within {connect_timeout:?} \
                         (stalled or never-publishing client)"
                    ),
                });
            }
        }

        let known_track_ids: BTreeSet<u32> = specs.iter().map(|s| s.track_id).collect();
        Ok(RtmpSession {
            conn,
            demux,
            specs,
            known_track_ids,
            pending_samples,
            read_timeout: self.timeouts.read,
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
    /// Sample(s) already drained by [`RtmpSource::connect`]'s wait for the
    /// first [`DemuxEvent::Sample`] (#738 T11b review, Important) — returned
    /// by [`Self::next_samples`] before any further socket read, so nothing
    /// observed during `connect()` is ever silently dropped.
    pending_samples: VecDeque<(u32, Sample)>,
    /// Bound on each [`Self::next_samples`] read — see
    /// [`IngestTimeouts::read`].
    read_timeout: std::time::Duration,
}

impl RtmpSession {
    /// The `TrackSpec`s resolved during [`RtmpSource::connect`]'s wait.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Returns any samples [`RtmpSource::connect`] already buffered, if
    /// present; otherwise reads the next batch of RTMP events (bounded by
    /// [`IngestTimeouts::read`]) and feeds every [`ServerEvent::Media`] tag
    /// run to the FLV demux, returning every completed sample it yields for
    /// a track known at connect time.
    ///
    /// Returns `Ok(None)` once the connection ends (clean EOF, or the
    /// publisher's `deleteStream`/`FCUnpublish`) *and* [`StreamingFlvDemux::finish`]'s
    /// flush of each track's trailing pending sample has already been
    /// drained — [`crate::origin::supervisor::supervise`] then reconnects
    /// (accepting the next publisher) with backoff, exactly as it does for
    /// any other source's end-of-stream.
    ///
    /// A read that produces nothing within [`IngestTimeouts::read`] (a
    /// publisher that goes idle mid-stream without closing the socket)
    /// surfaces as an `Err`, reconnected by the supervisor exactly like any
    /// other read error — instead of hanging forever and wedging the route
    /// (#738 T11b review, Critical).
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        if !self.pending_samples.is_empty() {
            return Ok(Some(self.pending_samples.drain(..).collect()));
        }

        let read_timeout = self.read_timeout;
        let events = tokio::time::timeout(read_timeout, self.conn.next_events())
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: format!("rtmp: no data within {read_timeout:?}"),
            })?
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
    use rtmp_runtime::server::ServerSession;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::watch;
    use transmux::pipeline::CodecConfig;

    /// Replays `fixture` byte-by-byte through a fresh sans-IO `ServerSession`
    /// (mirroring what `RtmpConnection::next_events` does per socket read),
    /// returning the fixture length at which the *first* `ServerEvent::Eof`
    /// fires — i.e. the natural end of the recorded publish session (a real
    /// `deleteStream`/`FCUnpublish`/stream-teardown the capture ends with).
    /// A prefix strictly shorter than this never reaches that teardown, so a
    /// test can play back such a prefix and then go silent to model a
    /// genuine mid-stream stall — as opposed to a legitimate end-of-stream —
    /// without depending on hand-picked byte offsets into the fixture.
    /// Falls back to the whole fixture length if it never emits `Eof`.
    fn fixture_len_before_natural_eof(fixture: &[u8]) -> usize {
        let mut session = ServerSession::new(ServerConfig::default());
        for (i, byte) in fixture.iter().enumerate() {
            let (_, events) = session
                .handle_data(std::slice::from_ref(byte))
                .expect("handle_data must not error while replaying a real capture");
            if events.iter().any(|e| matches!(e, ServerEvent::Eof)) {
                return i;
            }
        }
        fixture.len()
    }

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

        // Strengthened (#738 T11b review, Important): assert BOTH track
        // kinds resolved into the store, not just "some media landed" —
        // `store.set_track_specs` is fed straight from
        // `RtmpSession::track_specs()` at pipeline start
        // (`multimux::pipeline::run_pipeline`), so this directly proves
        // `RtmpSource::connect` resolved video *and* audio rather than
        // returning as soon as the first track's config appeared.
        let specs = store.track_specs();
        assert_eq!(
            specs.len(),
            2,
            "expected both video and audio TrackSpecs in the store, got {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "expected an AVC (video) track among {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Aac { .. })),
            "expected an AAC (audio) track among {specs:?}"
        );

        client.await.expect("client task must not panic");
        shutdown_tx.send(true).ok();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("supervise returns promptly on shutdown")
            .expect("supervise task did not panic");
    }

    /// Biting test (#738 T11b review, Important — the fix-2 regression
    /// test): the real fixture is split into TWO writes so that video's and
    /// audio's sequence headers land in SEPARATE `next_events()` reads (with
    /// a real delay in between, so the server genuinely drains + processes
    /// the first write before the second arrives). `connect()` must still
    /// resolve BOTH tracks, and the audio samples that follow must actually
    /// reach `next_samples()` — not be silently dropped by the old
    /// "break as soon as `specs` is non-empty" logic, which returned
    /// video-only here and then discarded every later audio `Sample` as an
    /// "unrouted track" in `drain_known_samples`.
    ///
    /// The split offset is computed empirically (not hand-picked) by
    /// replaying the fixture byte-by-byte through the same sans-IO
    /// `ServerSession` + `StreamingFlvDemux` this source uses internally,
    /// so this test stays correct even if the fixture is ever swapped for a
    /// different capture.
    #[tokio::test]
    async fn connect_resolves_both_tracks_when_sequence_headers_land_in_separate_reads() {
        let fixture = load_fixture();

        // Find the fixture byte offset at which the FIRST track's
        // `DemuxEvent::TrackAdded` fires, and the offset for the SECOND —
        // by construction (single-byte feed, in cumulative fixture-byte
        // order) the first offset is always strictly before the second.
        let (first_track_offset, second_track_offset) = {
            let mut session = ServerSession::new(ServerConfig::default());
            let mut demux = StreamingFlvDemux::new();
            let mut offsets = Vec::new();
            for (i, byte) in fixture.iter().enumerate() {
                let (_, events) = session
                    .handle_data(std::slice::from_ref(byte))
                    .expect("handle_data must not error while replaying a real capture");
                for event in events {
                    if let ServerEvent::Media { flv } = event {
                        demux
                            .feed(&flv)
                            .expect("feed must not error on a real capture");
                    }
                }
                while let Some(ev) = demux.poll_event() {
                    if matches!(ev, DemuxEvent::TrackAdded(_)) {
                        offsets.push(i + 1);
                    }
                }
                if offsets.len() >= 2 {
                    break;
                }
            }
            assert_eq!(
                offsets.len(),
                2,
                "fixture must resolve exactly two tracks (video+audio) for this test to be meaningful"
            );
            (offsets[0], offsets[1])
        };
        assert!(
            first_track_offset < second_track_offset,
            "the two tracks must resolve at distinct fixture offsets"
        );

        // Split right after the first track resolves and strictly before
        // the second — chunk 1 can therefore only ever yield the first
        // track's `TrackAdded`; chunk 2 supplies the second.
        let split = first_track_offset;
        let (chunk1, chunk2) = fixture.split_at(split);
        let chunk1 = chunk1.to_vec();
        let chunk2 = chunk2.to_vec();

        let reserved = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let source = RtmpSource::new("cam-rtmp-two-chunk", addr.to_string());

        let client = tokio::spawn(async move {
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
                .write_all(&chunk1)
                .await
                .expect("write chunk 1 (through the first track's sequence header)");
            // Real delay (not just a cooperative yield) so the server
            // genuinely reads + processes chunk 1 as its own `next_events()`
            // batch before chunk 2 ever reaches the socket. The handshake +
            // connect/createStream/publish acks are all well under the
            // kernel socket buffer size, so the server's writes never block
            // waiting for us to read in the meantime.
            tokio::time::sleep(Duration::from_millis(150)).await;
            stream
                .write_all(&chunk2)
                .await
                .expect("write chunk 2 (the second track's sequence header onward)");

            let mut sink = [0u8; 8192];
            loop {
                match stream.read(&mut sink).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        let mut session = tokio::time::timeout(Duration::from_secs(10), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");

        let specs = session.track_specs();
        assert_eq!(
            specs.len(),
            2,
            "connect() must resolve BOTH tracks even though their sequence headers \
             landed in separate next_events() reads; got {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "expected an AVC (video) track among {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Aac { .. })),
            "expected an AAC (audio) track among {specs:?}"
        );

        // Drain samples and confirm BOTH track ids' samples actually reach
        // `next_samples()` — not just "some media", proving the buffered
        // first-Sample(s) from `connect()` and every read after are routed,
        // including audio (the track that used to be silently dropped).
        let video_id = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
            .unwrap()
            .track_id;
        let audio_id = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Aac { .. }))
            .unwrap()
            .track_id;

        let mut video_samples = 0usize;
        let mut audio_samples = 0usize;
        while let Ok(Ok(Some(batch))) =
            tokio::time::timeout(Duration::from_millis(500), session.next_samples()).await
        {
            for (track_id, _sample) in batch {
                if track_id == video_id {
                    video_samples += 1;
                } else if track_id == audio_id {
                    audio_samples += 1;
                }
            }
        }
        assert!(video_samples > 0, "expected at least one video sample");
        assert!(
            audio_samples > 0,
            "expected at least one audio sample — this is exactly what the old \
             first-track-break logic dropped when audio's sequence header landed \
             in a later read than video's"
        );

        // Close the server-side socket so the client's reply-drain loop
        // observes EOF and actually finishes — the fixture's own natural
        // teardown may never be reached (the drain loop above only reads
        // until it stalls for 500ms, not necessarily to end of stream), so
        // without an explicit close here the client would otherwise block
        // forever waiting for a FIN nobody sends.
        drop(session);
        client.await.expect("client task must not panic");
    }

    /// Biting test (#738 T11b review, Critical — fix-1b): a client that
    /// completes the TCP connect but never sends any RTMP bytes (no C0/C1
    /// handshake, never mind `publish`) must fail `connect()` within
    /// `IngestTimeouts::connect`, not hang forever — the exact
    /// "connects and never publishes" wedge the review flagged (with the
    /// bind-once `AsyncRtmpServer`, a hung `connect()` would otherwise never
    /// let `supervise()` `accept()` the next publisher).
    #[tokio::test]
    async fn connect_times_out_when_client_never_sends_anything() {
        let reserved = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let source =
            RtmpSource::new("cam-rtmp-silent", addr.to_string()).with_timeouts(IngestTimeouts {
                connect: Duration::from_millis(200),
                read: Duration::from_secs(30),
            });

        let client = tokio::spawn(async move {
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
            // Hold the connection open but never write a byte — never sends
            // the RTMP handshake, let alone `publish`.
            let stream = stream.expect("connect to RtmpSource's listener");
            std::future::pending::<()>().await;
            drop(stream); // unreachable; keeps `stream` alive for the compiler
        });

        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect(
                "connect() must return on its own via IngestTimeouts::connect, \
                 not hang until this test's own backstop timeout",
            );
        assert!(
            result.is_err(),
            "a client that connects and never sends anything must fail connect(), not hang forever"
        );

        client.abort();
    }

    /// Biting test (#738 T11b review, Critical — fix-1a): a publisher that
    /// completes the whole handshake/publish/track-resolution dance and then
    /// goes idle (holds the socket open, sends nothing further) must fail
    /// `next_samples()` within `IngestTimeouts::read`, not hang forever —
    /// otherwise `supervise()` never sees an `Err`, never reconnects, and the
    /// route is wedged even though the publisher is gone in every sense that
    /// matters.
    #[tokio::test]
    async fn read_times_out_when_publisher_goes_idle_after_publish() {
        let reserved = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let source =
            RtmpSource::new("cam-rtmp-stall", addr.to_string()).with_timeouts(IngestTimeouts {
                connect: Duration::from_secs(10),
                read: Duration::from_millis(100),
            });

        let fixture = load_fixture();
        // Send only a prefix that never reaches the fixture's own natural
        // session teardown (`ServerEvent::Eof`) — a real capture ends with
        // one, and sending the whole thing would make the connection end
        // legitimately rather than modelling a mid-stream stall (a genuine
        // "goes idle without closing" client never gets to send its own
        // teardown either). Computed empirically so this stays correct if
        // the fixture is ever swapped for a different capture.
        let prefix_len = fixture_len_before_natural_eof(&fixture);
        assert!(
            prefix_len > 1000,
            "expected substantial media before the fixture's natural end, got {prefix_len} bytes"
        );
        let prefix = fixture[..prefix_len].to_vec();

        let client = tokio::spawn(async move {
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
                .write_all(&prefix)
                .await
                .expect("write the fixture prefix (stops short of its natural teardown)");
            // Drain replies for a bounded window (long enough to observe the
            // handshake/publish acks) but then go idle *without closing the
            // socket* — the "publisher stopped sending but connection stays
            // open" stall this test targets.
            let mut sink = [0u8; 8192];
            let _ = tokio::time::timeout(Duration::from_millis(300), async {
                loop {
                    match stream.read(&mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .await;
            std::future::pending::<()>().await;
            drop(stream); // unreachable; keeps the socket open for the test
        });

        let mut session = tokio::time::timeout(Duration::from_secs(10), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect resolves the real fixture's tracks");
        assert_eq!(session.track_specs().len(), 2, "video+audio resolved");

        // Drain whatever the fixture already delivered (buffered-at-connect
        // samples, plus anything already sitting in the OS socket buffer);
        // once genuinely exhausted, the read timeout must fire rather than
        // hanging past this test's own backstop.
        let mut saw_timeout_err = false;
        for _ in 0..2000 {
            match tokio::time::timeout(Duration::from_secs(2), session.next_samples()).await {
                Ok(Ok(Some(_batch))) => continue,
                Ok(Ok(None)) => panic!(
                    "connection must not report clean EOF while the socket is held open idle"
                ),
                Ok(Err(_)) => {
                    saw_timeout_err = true;
                    break;
                }
                Err(_) => panic!(
                    "next_samples() must return on its own via IngestTimeouts::read, \
                     not hang until this test's own backstop timeout"
                ),
            }
        }
        assert!(
            saw_timeout_err,
            "a publisher that goes idle after publish must fail next_samples() via \
             IngestTimeouts::read, not hang forever"
        );

        client.abort();
    }
}
