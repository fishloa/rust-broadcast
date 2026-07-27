//! SRT (Secure Reliable Transport) ingest source (issue #739): an
//! [`srt_runtime::io`] socket (listener or caller mode) carrying an MPEG-2
//! Transport Stream, feeding transmux's incremental
//! [`transmux::StreamingTsDemux`] — the SRT analogue of
//! [`crate::source::ts_udp`]'s UDP-over-`StreamingTsDemux` pull loop, driven
//! identically: `feed` + a `while let Some(ev) = demux.poll_event()` drain.
//! multimux owns only the SRT socket; all PAT/PMT/PES demuxing and
//! codec-config recovery is transmux's.
//!
//! SRT itself has two connection modes ([`draft-sharabayko-srt-01`] §3):
//!
//! - **Listener** ([`SrtSource::new_listener`]): binds a UDP port and accepts
//!   inbound Callers — a push input, mirroring
//!   [`crate::source::rtmp::RtmpSource`]'s bind-once-reuse-forever pattern
//!   (the listener is bound lazily via a [`tokio::sync::OnceCell`] and its
//!   `accept()` — `&mut self` on [`srt_runtime::io::SrtListener`] — is
//!   guarded by a [`tokio::sync::Mutex`] so it can be shared across
//!   reconnects without re-binding the port).
//! - **Caller** ([`SrtSource::new_caller`]): dials out to a remote SRT
//!   listener on every `connect()`/reconnect — a pull input, mirroring every
//!   other source in this crate.
//!
//! Since SRT carries an MPEG-2 TS payload in-band, the track set comes from
//! the stream's own PMT — exactly like [`crate::source::ts_udp`] —
//! so [`SrtSource::connect`] reads payloads into a [`StreamingTsDemux`] until
//! [`transmux::DemuxEvent::TracksResolved`] fires (bounded by
//! [`IngestTimeouts::connect`]), and [`SrtSession::next_samples`] reads and
//! demuxes bounded by [`IngestTimeouts::read`] — see `ts_udp`'s module doc
//! for why both bounds exist (issue #663 P5.2, audit-ingest #3).
//!
//! Encrypted SRT (the SEK-wrapped payload encryption `draft-sharabayko-srt-01`
//! §6 negotiates) is **out of scope**: [`srt_runtime::io`] does not yet apply
//! the SEK to decrypt DATA payloads, so this source carries no passphrase
//! field — see [`crate::config::InputSpec::Srt`]'s doc.
//!
//! [`draft-sharabayko-srt-01`]: https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01

use std::collections::BTreeSet;
use std::sync::Arc;

use srt_runtime::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};
use tokio::sync::{Mutex, OnceCell};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingTsDemux};

use crate::error::{MultimuxError, Result};
use crate::source::{IngestTimeouts, Source};

/// An SRT-over-MPEG-TS stream to ingest, in either listener (bind + accept)
/// or caller (dial out) mode — see the module doc.
pub struct SrtSource {
    name: String,
    /// Listener bind address (`Some`) — mutually exclusive with `remote`.
    /// Enforced by [`crate::config::InputSpec::Srt`]'s `validate`.
    listen: Option<String>,
    /// Caller dial-out address (`Some`) — mutually exclusive with `listen`.
    remote: Option<String>,
    stream_id: Option<String>,
    latency_ms: Option<u16>,
    timeouts: IngestTimeouts,
    /// Bind-once, reuse-forever (listener mode only) — see the module doc
    /// and [`crate::source::rtmp::RtmpSource`]'s analogous `server` field.
    /// [`SrtListener::accept`] takes `&mut self`, hence the `Mutex` (an
    /// `OnceCell` alone only gives shared access).
    listener: OnceCell<Arc<Mutex<SrtListener>>>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`, mirroring every other
/// source in this module): `listener`'s inner `SrtListener` has no `Debug`
/// impl of its own to derive over.
impl std::fmt::Debug for SrtSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtSource")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("remote", &self.remote)
            .field("stream_id", &self.stream_id)
            .field("latency_ms", &self.latency_ms)
            .finish()
    }
}

impl SrtSource {
    /// Build a listener-mode source: binds `listen` (lazily, on first
    /// `connect()`) and accepts the next Caller on every reconnect — see the
    /// module doc.
    pub fn new_listener(name: impl Into<String>, listen: impl Into<String>) -> Self {
        SrtSource {
            name: name.into(),
            listen: Some(listen.into()),
            remote: None,
            stream_id: None,
            latency_ms: None,
            timeouts: IngestTimeouts::default(),
            listener: OnceCell::new(),
        }
    }

    /// Build a caller-mode source: dials `remote` fresh on every
    /// `connect()`/reconnect.
    pub fn new_caller(name: impl Into<String>, remote: impl Into<String>) -> Self {
        SrtSource {
            name: name.into(),
            listen: None,
            remote: Some(remote.into()),
            stream_id: None,
            latency_ms: None,
            timeouts: IngestTimeouts::default(),
            listener: OnceCell::new(),
        }
    }

    /// Sets the Stream ID to advertise (`draft-sharabayko-srt-01` §3.2.1.3) —
    /// caller mode only; a listener-mode config's `stream_id` is ignored by
    /// the handshake (it's a Caller-advertised field, per
    /// [`HandshakeConfig::stream_id`]'s own doc).
    #[must_use]
    pub fn with_stream_id(mut self, id: Option<String>) -> Self {
        self.stream_id = id;
        self
    }

    /// Overrides the negotiated TSBPD latency (`draft-sharabayko-srt-01`
    /// §4.3.1.2); `None` keeps [`HandshakeConfig::default`]'s value.
    #[must_use]
    pub fn with_latency_ms(mut self, ms: Option<u16>) -> Self {
        self.latency_ms = ms;
        self
    }

    /// Overrides the default [`IngestTimeouts`] — see
    /// `TsUdpSource::with_timeouts` for the pattern this mirrors.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    fn handshake_config(&self) -> HandshakeConfig {
        let mut cfg = HandshakeConfig {
            stream_id: self.stream_id.clone(),
            ..HandshakeConfig::default()
        };
        if let Some(latency) = self.latency_ms {
            cfg.latency_ms = latency;
        }
        cfg
    }

    /// Binds (listener mode, once) or dials (caller mode, fresh) the SRT
    /// socket, then reads payloads into a [`StreamingTsDemux`] until every
    /// currently PMT-declared track has resolved — the SRT/TS analogue of
    /// `TsUdpSource::connect`'s PMT wait, so [`SrtSession::track_specs`] is
    /// always populated before the pipeline builds its segmenter.
    ///
    /// In **caller** mode the entire dial-plus-track-wait is bounded by a
    /// single [`IngestTimeouts::connect`], mirroring
    /// [`crate::source::rtsp::RtspSource::connect`]'s whole-handshake wrap
    /// (issue #739 review): a dead/blackholed remote must be bounded by the
    /// *configured* connect timeout, not left to srt-runtime's own internal
    /// handshake-retry budget, which the [`tokio::time::timeout`] would
    /// otherwise never get a chance to interrupt (the dial future runs to
    /// its own completion before the subsequent track-wait future is even
    /// polled).
    ///
    /// In **listener** mode only the track-wait is bounded: the prior
    /// `bind`+`accept()` waits for an inbound Caller to show up, which is
    /// idle (nothing wrong is happening) rather than stalled, so it stays
    /// unbounded — exactly like `RtmpSource::connect` accepting inbound
    /// publishers.
    pub async fn connect(&self) -> Result<SrtSession> {
        let cfg = self.handshake_config();
        let connect_timeout = self.timeouts.connect;

        let (sock, demux, specs) = if let Some(listen) = &self.listen {
            let listener = self
                .listener
                .get_or_try_init(|| async {
                    SrtListener::bind(listen.as_str(), cfg.clone())
                        .await
                        .map(|l| Arc::new(Mutex::new(l)))
                        .map_err(|e| MultimuxError::Connect {
                            reason: format!("srt: bind {listen}: {e}"),
                        })
                })
                .await?;

            // Guard dropped at the end of this block, before the
            // track-resolution read loop below — an accepted-but-unresolved
            // connection must never hold the listener lock, or a slow/stalled
            // publisher would block every other route reconnect sharing this
            // listener (there is only one per `SrtSource`, but a wedged lock
            // is a bug class worth avoiding on principle, mirroring the
            // module doc's bind-once-reuse-forever contract).
            let mut sock = {
                let mut l = listener.lock().await;
                l.accept().await.map_err(|e| MultimuxError::Connect {
                    reason: format!("srt: accept: {e}"),
                })?
            };

            let mut demux = StreamingTsDemux::new();
            let mut specs: Vec<TrackSpec> = Vec::new();
            match tokio::time::timeout(
                connect_timeout,
                wait_for_tracks(&mut sock, &mut demux, &mut specs),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(MultimuxError::Connect {
                        reason: format!(
                            "srt: no PMT-declared track resolved within {connect_timeout:?}"
                        ),
                    });
                }
            }
            (sock, demux, specs)
        } else {
            let remote = self.remote.as_ref().unwrap_or_else(|| {
                unreachable!(
                    "SrtSource must be constructed via new_listener or new_caller, \
                     exactly one of listen/remote is always Some"
                )
            });

            match tokio::time::timeout(connect_timeout, async {
                let mut sock = SrtSocket::connect(remote.as_str(), cfg)
                    .await
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("srt: connect {remote}: {e}"),
                    })?;
                let mut demux = StreamingTsDemux::new();
                let mut specs: Vec<TrackSpec> = Vec::new();
                wait_for_tracks(&mut sock, &mut demux, &mut specs).await?;
                Ok::<_, MultimuxError>((sock, demux, specs))
            })
            .await
            {
                Ok(Ok(triple)) => triple,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(MultimuxError::Connect {
                        reason: format!(
                            "srt: connect {remote}: no connection/PMT-declared track \
                             resolved within {connect_timeout:?}"
                        ),
                    });
                }
            }
        };

        let known_track_ids: BTreeSet<u32> = specs.iter().map(|s| s.track_id).collect();
        Ok(SrtSession {
            sock,
            demux,
            specs,
            known_track_ids,
            read_timeout: self.timeouts.read,
        })
    }
}

/// Reads payloads from `sock` into `demux` until every currently
/// PMT-declared track has resolved (`specs` non-empty and
/// [`DemuxEvent::TracksResolved`] observed) — shared by both
/// [`SrtSource::connect`] branches (listener and caller), which differ only
/// in how the bound is applied around this loop (see that method's doc).
async fn wait_for_tracks(
    sock: &mut SrtSocket,
    demux: &mut StreamingTsDemux,
    specs: &mut Vec<TrackSpec>,
) -> Result<()> {
    loop {
        let payload = sock.recv().await.map_err(|e| MultimuxError::Connect {
            reason: format!("srt recv: {e}"),
        })?;
        let Some(bytes) = payload else {
            return Err(MultimuxError::Connect {
                reason: "srt: connection ended before any track resolved".into(),
            });
        };
        demux.feed(&bytes);
        let mut resolved = false;
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) => specs.push(spec),
                DemuxEvent::TracksResolved { .. } => resolved = true,
                _ => {}
            }
        }
        if resolved && !specs.is_empty() {
            return Ok(());
        }
    }
}

impl Source for SrtSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// A live SRT session: a connected [`SrtSocket`] (either role), feeding a
/// [`StreamingTsDemux`].
pub struct SrtSession {
    sock: SrtSocket,
    demux: StreamingTsDemux,
    specs: Vec<TrackSpec>,
    /// Track ids known at connect time — a `Sample` for any later-discovered
    /// track (e.g. a PMT version bump after `connect` returned) is dropped
    /// rather than surfaced for a track the segmenter was never built with,
    /// mirroring `TsUdpSession::next_samples`'s "unrouted track -> ignored"
    /// handling.
    known_track_ids: BTreeSet<u32>,
    /// Bound on each [`Self::next_samples`] read — see
    /// [`IngestTimeouts::read`].
    read_timeout: std::time::Duration,
}

impl SrtSession {
    /// The `TrackSpec`s resolved during [`SrtSource::connect`]'s PMT wait.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Receives the next SRT-delivered payload (one or more 188-byte TS
    /// packets) and feeds it to the demuxer, returning every completed
    /// sample it yields for a track known at connect time.
    ///
    /// Returns `Ok(None)` once [`SrtSocket::recv`] reports the peer has shut
    /// down — [`crate::origin::supervisor::supervise`] then reconnects
    /// (accepting the next publisher in listener mode, or redialling in
    /// caller mode) exactly as it does for any other source's end-of-stream.
    ///
    /// Bounded by [`IngestTimeouts::read`] (mirroring `TsUdpSession`/
    /// `RtmpSession`): a source that stops sending (dropped link, wedged
    /// encoder) would otherwise leave this `.await` pending forever — a
    /// timed-out read surfaces as a [`MultimuxError::Connect`], reconnected
    /// by the supervisor exactly like any other read error.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let read_timeout = self.read_timeout;
        let payload = tokio::time::timeout(read_timeout, self.sock.recv())
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: format!("srt recv: no data within {read_timeout:?}"),
            })?
            .map_err(|e| MultimuxError::Connect {
                reason: format!("srt recv: {e}"),
            })?;

        let Some(bytes) = payload else {
            return Ok(None);
        };

        self.demux.feed(&bytes);
        let mut out = Vec::new();
        while let Some(event) = self.demux.poll_event() {
            match event {
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    if self.known_track_ids.contains(&track_id) {
                        out.push((track_id, sample));
                    }
                }
                DemuxEvent::TrackRemoved { track_id, .. } => {
                    // A mid-stream PMT version bump dropped a previously-live
                    // PID (issue #774). Drop it from the known set so no
                    // stale sample is ever forwarded for it (defense in depth
                    // — `DemuxEvent`'s removal semantics already guarantee no
                    // `Sample` for this `track_id` follows), and surface the
                    // change instead of silently swallowing it: the running
                    // pipeline/segmenter was built once from connect-time
                    // `track_specs()` and has no way to learn a track vanished.
                    tracing::warn!(
                        track_id,
                        "srt: track removed mid-stream (PMT no longer lists it); \
                         no further samples will be surfaced for it"
                    );
                    self.known_track_ids.remove(&track_id);
                }
                DemuxEvent::TrackUpdated(spec) => {
                    tracing::debug!(
                        track_id = spec.track_id,
                        "srt: track metadata updated mid-stream (es_info_descriptors/\
                         stream_type); the running pipeline was built from the connect-time \
                         TrackSpec and does not pick this up"
                    );
                }
                DemuxEvent::TrackAdded(spec) => {
                    // A PID declared only after `connect()`'s PMT wait
                    // resolved. The pipeline was built from that connect-time
                    // track set (`SampleSource::track_specs` is a one-shot
                    // snapshot, `pipeline::run_pipeline` calls it exactly
                    // once) — there is no API on this session to add a track
                    // to an already-running segmenter, so this is reported
                    // rather than silently dropped or half-wired in.
                    tracing::warn!(
                        track_id = spec.track_id,
                        "srt: new track declared mid-stream; the running pipeline was built \
                         from the connect-time track set and cannot pick it up without a reconnect"
                    );
                }
                DemuxEvent::TrackAbandoned { reason, .. } => {
                    tracing::warn!(
                        ?reason,
                        "srt: a probing track was abandoned before it resolved"
                    );
                }
                _ => {}
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::supervisor::{Backoff, supervise};
    use crate::store::MediaStore;
    use std::time::Duration;
    use tokio::sync::watch;
    use transmux::TsMux;
    use transmux::media::Track;
    use transmux::pipeline::CodecConfig;

    /// Builds a real (not hand-faked) MPEG-2 TS byte stream carrying one
    /// H.264 video track with `num_frames` access units, by round-tripping
    /// through the workspace's own `transmux::TsMux` packager — mirrors
    /// `ts_udp.rs`'s own `build_ts_bytes` (same real-fixture discipline: a
    /// hand-built TS risks missing real PSI/PES framing quirks a muxed
    /// stream actually has), generalized to a frame count so a store-landing
    /// test can request enough cumulative duration to cross a real LL-HLS
    /// part boundary (`transmux::ll_hls::LlHlsSegmenter::push`: a part
    /// flushes once the anchor track buffers `part_target_ms` worth of
    /// samples, independent of any keyframe).
    fn build_ts_bytes(num_frames: u32) -> Vec<u8> {
        use broadcast_common::Package;
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
        let samples: Vec<Sample> = (0..num_frames)
            .map(|i| {
                let nal = [0x65u8, 0xAA, (i % 256) as u8];
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                Sample::new(
                    data,
                    Some(i64::from(i) * i64::from(frame_dur)),
                    Some(i64::from(i) * i64::from(frame_dur)),
                    Some(frame_dur),
                    i == 0,
                )
            })
            .collect();
        let track = Track::new(spec, samples);
        let media = transmux::media::Media::new(vec![track], 90_000);
        TsMux::default().package(&media).expect("mux to TS")
    }

    /// Polls `f` every millisecond until it returns `true` or `timeout`
    /// elapses — mirrors `rtmp.rs`'s own test helper of the same name.
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

    /// Loopback biting test (issue #739): a real `SrtSocket` caller connects
    /// to a real listener-mode `SrtSource` bound to an ephemeral loopback
    /// port, sends a muxed TS stream in SRT-payload-sized (~1316-byte,
    /// TS-packet-aligned) chunks, driven through the real
    /// `supervise`/`run_pipeline` machinery into a real `MediaStore` —
    /// proving the whole chain (`SrtListener::accept` ->
    /// `StreamingTsDemux` -> segmenter -> `MediaStore`) actually moves real
    /// H.264 samples, not just that `connect()` resolves specs. Mutation
    /// check: if `SrtSession::next_samples` dropped every payload (or never
    /// fed the demux), the store would stay empty and this assertion would
    /// fail.
    #[tokio::test]
    async fn loopback_srt_listener_publish_lands_media_in_the_store() {
        // `SrtSource::connect` binds asynchronously inside the spawned
        // `supervise` task below, so the client needs to know the bind
        // address up front — reserve a real ephemeral UDP port (the same
        // technique `ts_udp.rs`/`rtmp.rs` use), bind `SrtSource` to that
        // exact address, and have the client dial it directly. UDP has no
        // `TIME_WAIT`, so the port is immediately reusable once the
        // reservation socket is dropped.
        let reserved = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        // A short read timeout: `SrtSocket::drop` does not send the peer an
        // explicit SRT Shutdown control packet (see `srt_runtime::io`'s
        // module doc — only the sans-IO `ControlPacket::Shutdown` path sets
        // `peer_shutdown`), so once the client stops sending, this session's
        // `next_samples()` would otherwise wait the full (default 30s)
        // read timeout before `run_pipeline` returns and `supervise`'s
        // shutdown check is next reached — this keeps the test itself fast
        // and provides the whole `supervise().await` a bounded time to end
        // once `shutdown_tx` fires below.
        let source =
            SrtSource::new_listener("cam-srt", addr.to_string()).with_timeouts(IngestTimeouts {
                connect: IngestTimeouts::default().connect,
                read: Duration::from_millis(300),
            });
        // 40 frames @ 30fps / 90kHz = 120,000 ticks, comfortably past the
        // 500ms-part / 90kHz = 45,000-tick part boundary
        // (`LlHlsSegmenter::push`) so a real part actually flushes into the
        // store, not just samples reaching the session.
        let ts_bytes = build_ts_bytes(40);
        let store = Arc::new(MediaStore::new(1.0, 500, 8));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let backoff = Backoff::new(Duration::from_millis(1), Duration::from_millis(20), 2.0);

        let client = tokio::spawn(async move {
            let cfg = HandshakeConfig::default();
            let mut sock = None;
            for _ in 0..200 {
                match SrtSocket::connect(addr, cfg.clone()).await {
                    Ok(s) => {
                        sock = Some(s);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
            let mut sock = sock.expect("connect to SrtSource's listener");
            for chunk in ts_bytes.chunks(7 * 188) {
                sock.send(chunk).await.expect("send TS payload");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // Keep the socket (and its driver task) alive until the test has
            // had a chance to observe the delivered media.
            tokio::time::sleep(Duration::from_secs(2)).await;
        });

        let handle = tokio::spawn(supervise(
            source,
            store.clone(),
            1.0,
            500,
            backoff,
            "test-srt".to_string(),
            shutdown_rx,
        ));

        let landed = wait_until(Duration::from_secs(10), || store.init_bytes().is_some()).await;
        assert!(landed, "SRT publish must land an init segment in the store");

        let landed_media = wait_until(Duration::from_secs(10), || {
            store.latest_progress().1 > 0 || !store.window_segments().is_empty()
        })
        .await;
        assert!(
            landed_media,
            "SRT publish must land at least one real part/segment in the store"
        );

        let specs = store.track_specs();
        assert_eq!(specs.len(), 1, "one video track from the muxed TS");
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "expected an AVC (video) track among {specs:?}"
        );

        shutdown_tx.send(true).ok();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("supervise returns promptly on shutdown")
            .expect("supervise task did not panic");
        client.abort();
    }

    /// Loopback test for caller mode (issue #739): a real listener-mode
    /// `SrtListener` (test-owned, not through `SrtSource`) accepts, and a
    /// caller-mode `SrtSource` dials out to it — proving the caller-mode
    /// `connect()` path (the `remote.is_some()` branch, no `OnceCell`) also
    /// resolves real tracks and delivers real samples.
    #[tokio::test]
    async fn loopback_srt_caller_connects_and_yields_samples() {
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let ts_bytes = build_ts_bytes(10);
        let server = tokio::spawn(async move {
            let mut sock = listener.accept().await.expect("listener accept");
            for chunk in ts_bytes.chunks(7 * 188) {
                sock.send(chunk).await.expect("send TS payload");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        });

        let source = SrtSource::new_caller("cam-srt-caller", bound_addr.to_string());
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");

        let specs = session.track_specs();
        assert_eq!(specs.len(), 1, "one video track from the muxed TS");
        assert_eq!(specs[0].timescale, 90_000);

        let mut samples = Vec::new();
        for _ in 0..20 {
            match tokio::time::timeout(Duration::from_millis(500), session.next_samples()).await {
                Ok(Ok(Some(batch))) => samples.extend(batch),
                _ => break,
            }
        }
        assert!(
            !samples.is_empty(),
            "expected at least one sample from the muxed TS stream over a caller-mode connect"
        );

        drop(session);
        server.abort();
    }

    /// A source that stops sending after `connect()` resolves tracks must
    /// not hang `next_samples()` forever (mirrors
    /// `ts_udp::next_samples_times_out_when_source_goes_silent` /
    /// `rtmp::read_times_out_when_publisher_goes_idle_after_publish`): with a
    /// short configured [`IngestTimeouts::read`], the next call — for which
    /// nothing further ever arrives — must return an `Err` within that
    /// bound, not block indefinitely.
    #[tokio::test]
    async fn next_samples_times_out_when_source_goes_silent() {
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let ts_bytes = build_ts_bytes(10);
        let server = tokio::spawn(async move {
            let mut sock = listener.accept().await.expect("listener accept");
            for chunk in ts_bytes.chunks(7 * 188) {
                sock.send(chunk).await.expect("send TS payload");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // `sock` is kept alive (held open, idle) rather than dropped —
            // models a publisher that stalls without shutting down, not a
            // clean EOS.
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        const READ_TIMEOUT: Duration = Duration::from_millis(200);
        let source = SrtSource::new_caller("cam-srt-silent", bound_addr.to_string()).with_timeouts(
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read: READ_TIMEOUT,
            },
        );

        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");
        assert_eq!(session.track_specs().len(), 1, "one video track resolved");

        // Drain whatever samples were already in flight.
        loop {
            match tokio::time::timeout(Duration::from_millis(100), session.next_samples()).await {
                Ok(Ok(Some(batch))) if !batch.is_empty() => continue,
                _ => break,
            }
        }

        let outcome = tokio::time::timeout(READ_TIMEOUT * 10, session.next_samples())
            .await
            .expect(
                "next_samples must return within a bounded multiple of the read timeout, not hang",
            );
        assert!(
            outcome.is_err(),
            "expected a recoverable read-timeout error once the source goes silent"
        );

        server.abort();
    }

    /// Biting test (issue #739 review, Important finding): a caller-mode
    /// `connect()` dialing a dead/blackholed remote must honor the
    /// *configured* [`IngestTimeouts::connect`], not srt-runtime's own
    /// internal handshake-retry budget. Uses a bound-but-otherwise-idle UDP
    /// socket (never reads, never replies) as the "remote" — loopback-only,
    /// so this needs no external network access and is deterministic in CI —
    /// with a short configured connect timeout. Mutation check: wrapping
    /// only `wait_for_tracks` (not the dial) in the timeout, as the
    /// pre-fix code did, makes `connect()` take as long as srt-runtime's own
    /// handshake retries (several seconds), blowing the outer bound this test
    /// asserts against.
    #[tokio::test]
    async fn caller_connect_times_out_against_an_unreachable_remote() {
        // A real bound UDP socket that never reads/replies to anything sent
        // to it — from the caller's perspective this is indistinguishable
        // from a blackholed remote: every handshake induction packet lands
        // on a live port but nothing ever answers it.
        let dead = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind dead socket");
        let dead_addr = dead.local_addr().expect("dead socket addr");

        const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
        let source = SrtSource::new_caller("cam-srt-dead", dead_addr.to_string()).with_timeouts(
            IngestTimeouts {
                connect: CONNECT_TIMEOUT,
                read: Duration::from_secs(30),
            },
        );

        let started = tokio::time::Instant::now();
        // A generous *outer* bound: if the fix regresses, `connect()` falls
        // back to srt-runtime's own handshake-retry budget (~5s x retries)
        // rather than hanging forever, so this still resolves — just far
        // outside the tight bound asserted below — without wedging the test
        // suite.
        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect(
                "connect() must itself return well within a small multiple of the configured \
                 connect timeout, not rely on this test's own outer bound",
            );
        let elapsed = started.elapsed();

        assert!(
            result.is_err(),
            "connect() against a dead/blackholed remote must fail, not hang or succeed"
        );
        assert!(
            elapsed < CONNECT_TIMEOUT * 4,
            "connect() took {elapsed:?} against a configured connect timeout of \
             {CONNECT_TIMEOUT:?} — the caller-mode dial must be bounded by the configured \
             timeout, not srt-runtime's own internal handshake-retry budget"
        );

        drop(dead);
    }

    /// Listener-mode analogue of the caller-mode "dead remote" test above
    /// (issue #739 review, Minor finding): a real caller completes the SRT
    /// handshake (`SrtListener::accept` returns) but then sends nothing at
    /// all — `connect()`'s subsequent track-resolution wait must still
    /// return within [`IngestTimeouts::connect`], not hang forever waiting
    /// for a PMT that never arrives. (The listener-mode code path already
    /// bounds `wait_for_tracks` correctly pre-#739-review; this closes the
    /// test-coverage gap the review flagged, mirroring
    /// `next_samples_times_out_when_source_goes_silent`'s coverage of the
    /// post-connect read path.)
    #[tokio::test]
    async fn listener_connect_times_out_when_caller_accepts_but_stays_silent() {
        let reserved = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
        let source = SrtSource::new_listener("cam-srt-listener-silent", addr.to_string())
            .with_timeouts(IngestTimeouts {
                connect: CONNECT_TIMEOUT,
                read: Duration::from_secs(30),
            });

        let client = tokio::spawn(async move {
            let cfg = HandshakeConfig::default();
            let mut sock = None;
            for _ in 0..200 {
                match SrtSocket::connect(addr, cfg.clone()).await {
                    Ok(s) => {
                        sock = Some(s);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
            let sock = sock.expect("connect to SrtSource's listener");
            // Hold the connected socket open and idle, sending nothing — a
            // caller whose handshake completes but never actually publishes
            // any TS payload.
            tokio::time::sleep(Duration::from_secs(2)).await;
            drop(sock);
        });

        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect(
                "connect() must return within a small multiple of the connect timeout, not hang",
            );
        let elapsed = started.elapsed();

        assert!(
            result.is_err(),
            "connect() against a silent-after-accept caller must fail, not hang waiting for a \
             PMT that never arrives"
        );
        assert!(
            elapsed < CONNECT_TIMEOUT * 4,
            "connect() took {elapsed:?} against a configured connect timeout of \
             {CONNECT_TIMEOUT:?}"
        );

        client.abort();
    }
}
