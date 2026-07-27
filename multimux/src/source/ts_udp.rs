//! MPEG-2 Transport Stream over UDP ingest source (issue #663 P3a; ported
//! onto the media-plane ingress traits at plan step 5a): a UDP socket
//! (unicast or multicast) feeding transmux's incremental
//! [`transmux::StreamingTsDemux`] — multimux owns only the socket; all PAT/
//! PMT/PES demuxing and codec-config recovery is transmux's, the same
//! streaming demux core `ts-fix` and every other TS consumer in this
//! workspace drives.
//!
//! # Why this source fits the sans-IO reshape with zero executor bridge
//!
//! UDP is connectionless: binding a local socket is a purely local operation
//! (no peer round-trip at all), so [`TsUdpDialer::dial`] performs **no I/O**
//! — it just constructs a fresh [`TsUdpIngestSession`] and immediately queues
//! [`SessionEvent::Established`] (mirroring `media_plane::ingress`'s own
//! `ScriptedSession` test precedent: "a source whose handshake is a purely
//! local operation with nothing to negotiate"). The actual `UdpSocket::bind`
//! (still real I/O, just never a multi-round-trip *handshake*) happens in
//! [`run_ts_udp`], the multimux-side driver that owns the socket and pumps
//! [`media_plane::ingress::IngestDriver`] — exactly where the crate's own
//! module docs (`docs/superpowers/plans/2026-07-26-media-plane-implementation.md`
//! step 5) say tokio belongs.
//!
//! # B5: the mid-stream `NewProgram` this source used to drop
//!
//! Before this port, a PID declared only *after* `connect()`'s PMT wait
//! resolved was logged and silently dropped (`DemuxEvent::TrackAdded`'s old
//! arm) — cited directly in `media_plane::ingress`'s own module docs as "the
//! gap `NewProgram` generalises". [`ProgramTracker`] closes it: the *first*
//! [`transmux::DemuxEvent::TracksResolved`] mints `ProgramId(0)` from every
//! track collected up to that point, and **any** [`transmux::DemuxEvent::TrackAdded`]
//! arriving after that mints a **new** `ProgramId` (1, 2, ...) instead of
//! being dropped. This is a deliberate simplification, not full MPTS
//! `program_number` support: `transmux::DemuxEvent` does not carry
//! `program_number` today (`media_plane::ingress`'s own docs record this as
//! finding B5's root cause), so "a track declared after the stream's initial
//! program resolved" is treated as a new program rather than being mapped to
//! its real PMT-declared `program_number` — the latter needs
//! `program_number` threaded through `transmux`'s IR first (future work, not
//! this port). What this *does* prove is the mechanism
//! [`media_plane::ingress::IngestDriver`] was built for: a `NewProgram`
//! announced mid-session, on an already-live connection, mints a fresh
//! `Trunk` exactly like one announced at the start.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Duration;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{Dialer, IngestSession, ProgramId, SessionEvent};
use media_plane::trunk::RetentionClass;
use tokio::net::UdpSocket;

use crate::error::{MultimuxError, Result};
use crate::source::IngestTimeouts;
use crate::source::Source;
use crate::source::udp::bind_udp;
use transmux::pipeline::TrackSpec;
use transmux::{DemuxEvent, StreamingTsDemux};

/// Max UDP datagram this source reads in one `recv` — comfortably above a
/// typical 7×188-byte (1316-byte) TS-over-UDP payload and any legal UDP
/// datagram (65 507 bytes over IPv4).
const MAX_UDP_DATAGRAM: usize = 65_536;

/// An MPEG-2 TS-over-UDP route: bind address (+ optional multicast group) —
/// no control plane, no out-of-band SDP (the PMT carries the track set
/// in-band, unlike raw RTP/UDP). Replaces the old (pre-5a) `TsUdpSource`;
/// [`run_ts_udp`] is the new `connect()`+`next_samples()` loop, now driving
/// [`media_plane::ingress::IngestDriver`] instead of hand-rolling its own
/// demux drain.
#[derive(Clone)]
pub struct TsUdpRoute {
    name: String,
    addr: String,
    multicast_group: Option<String>,
    timeouts: IngestTimeouts,
}

impl std::fmt::Debug for TsUdpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsUdpRoute")
            .field("name", &self.name)
            .field("addr", &self.addr)
            .field("multicast_group", &self.multicast_group)
            .finish()
    }
}

impl TsUdpRoute {
    /// Build a route descriptor.
    pub fn new(
        name: impl Into<String>,
        addr: impl Into<String>,
        multicast_group: Option<String>,
    ) -> Self {
        TsUdpRoute {
            name: name.into(),
            addr: addr.into(),
            multicast_group,
            timeouts: IngestTimeouts::default(),
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
}

impl Source for TsUdpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Translates [`transmux::DemuxEvent`]s into [`SessionEvent`]s and tracks
/// which [`ProgramId`] owns each `track_id` — kept as a plain, byte-free
/// state machine (no socket, no demuxer) so the B5 mid-stream-`NewProgram`
/// behaviour is unit-testable by constructing [`DemuxEvent`]s directly (via
/// their own `#[non_exhaustive]` constructors), without needing a hand-built
/// MPTS byte stream this workspace's fixture discipline would otherwise call
/// for.
struct ProgramTracker {
    pending: VecDeque<SessionEvent>,
    /// [`DemuxEvent::TrackAdded`] specs collected before the first
    /// [`DemuxEvent::TracksResolved`] — becomes `ProgramId(0)`'s track set.
    resolving: Vec<TrackSpec>,
    resolved_once: bool,
    track_program: HashMap<u32, ProgramId>,
    next_program_id: u32,
}

impl ProgramTracker {
    /// A session whose handshake is a purely local operation (see the
    /// module doc) starts with `Established` already queued.
    fn new() -> Self {
        ProgramTracker {
            pending: VecDeque::from(vec![SessionEvent::Established]),
            resolving: Vec::new(),
            resolved_once: false,
            track_program: HashMap::new(),
            next_program_id: 0,
        }
    }

    fn handle(&mut self, event: DemuxEvent) {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                if self.resolved_once {
                    // B5: a track declared only after the initial program
                    // resolved — see the module doc.
                    let program = ProgramId(self.next_program_id);
                    self.next_program_id += 1;
                    self.track_program.insert(spec.track_id, program);
                    self.pending.push_back(SessionEvent::NewProgram {
                        program,
                        tracks: vec![spec],
                    });
                } else {
                    self.resolving.push(spec);
                }
            }
            DemuxEvent::TracksResolved { .. } => {
                if !self.resolved_once && !self.resolving.is_empty() {
                    self.resolved_once = true;
                    let program = ProgramId(self.next_program_id);
                    self.next_program_id += 1;
                    let tracks = std::mem::take(&mut self.resolving);
                    for spec in &tracks {
                        self.track_program.insert(spec.track_id, program);
                    }
                    self.pending
                        .push_back(SessionEvent::NewProgram { program, tracks });
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&program) = self.track_program.get(&track_id) {
                    self.pending.push_back(SessionEvent::Sample {
                        program,
                        track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
                // A sample for a track never announced (or since removed)
                // is dropped — mirrors the pre-5a `known_track_ids` check.
            }
            DemuxEvent::TrackRemoved { track_id, .. } => {
                // A mid-stream PMT version bump dropped a previously-live
                // PID (issue #774): stop routing samples for it. No
                // `SessionEvent` for this yet — `SessionEvent` has no
                // `TrackRemoved`/`ProgramEnded` variant (`#[non_exhaustive]`,
                // deliberately not added speculatively; see its own doc).
                self.track_program.remove(&track_id);
            }
            DemuxEvent::TrackUpdated(_) | DemuxEvent::TrackAbandoned { .. } => {
                // Metadata-only / pre-resolution events; nothing routes on
                // them yet (mirrors the pre-5a tracing-only handling).
            }
            _ => {}
        }
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }
}

/// A live TS-over-UDP [`IngestSession`]: no socket, no I/O — just the
/// [`StreamingTsDemux`] plus [`ProgramTracker`]. [`run_ts_udp`] is the
/// multimux-side adapter that owns the real `UdpSocket` and feeds it.
pub struct TsUdpIngestSession {
    demux: StreamingTsDemux,
    tracker: ProgramTracker,
}

impl TsUdpIngestSession {
    fn new() -> Self {
        TsUdpIngestSession {
            demux: StreamingTsDemux::new(),
            tracker: ProgramTracker::new(),
        }
    }
}

impl Stage for TsUdpIngestSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    /// TS-over-UDP demuxing cannot itself fail (mirrors the pre-5a session,
    /// whose `demux.feed` call was never fallible) — every failure mode here
    /// (a dead socket, a read stall) lives at the I/O layer in
    /// [`run_ts_udp`], outside this sans-IO session entirely.
    type Error = Infallible;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> core::result::Result<(), Infallible> {
        self.demux.feed(input);
        while let Some(event) = self.demux.poll_event() {
            self.tracker.handle(event);
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.tracker.poll()
    }

    fn finish(&mut self) -> core::result::Result<(), Infallible> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(MAX_UDP_DATAGRAM)
    }
}

/// Nothing to send back to a UDP peer — takes the default `poll_transmit`.
impl IngestSession for TsUdpIngestSession {}

/// Constructs a [`TsUdpIngestSession`] — performs **no I/O** (see the module
/// doc's "zero executor bridge" section).
#[derive(Clone, Copy, Debug, Default)]
pub struct TsUdpDialer;

impl Dialer for TsUdpDialer {
    type Session = TsUdpIngestSession;
    /// Construction cannot fail — there is no fallible local step (unlike,
    /// say, parsing a URL).
    type Error = Infallible;

    fn dial(&mut self) -> core::result::Result<TsUdpIngestSession, Infallible> {
        Ok(TsUdpIngestSession::new())
    }
}

/// Binds `route`'s UDP socket and returns it, ready for
/// [`recv_and_feed`]/[`run_ts_udp`] — split out so tests can synchronise on
/// the bound address before a synthetic sender starts writing to it (UDP has
/// no connect-then-accept handshake to synchronise on otherwise).
pub async fn bind(route: &TsUdpRoute) -> Result<UdpSocket> {
    bind_udp(&route.addr, route.multicast_group.as_deref()).await
}

/// Reads one datagram from `socket` (bounded by `read_timeout`) and feeds it
/// to `driver`. Returns `Ok(())` on a normal read, or `Err` on a read stall
/// or socket error — UDP is connectionless, so unlike a TCP/HTTP source
/// there is no transport-level clean end-of-stream; every termination here
/// is reported as an I/O-layer error for the caller (production: the route
/// supervisor) to reconnect on, exactly as the pre-5a `next_samples` did.
pub async fn recv_and_feed(
    socket: &UdpSocket,
    buf: &mut [u8],
    driver: &mut media_plane::ingress::IngestDriver<TsUdpIngestSession>,
    read_timeout: Duration,
    now: Timestamp,
) -> Result<()> {
    let n = tokio::time::timeout(read_timeout, socket.recv(buf))
        .await
        .map_err(|_| MultimuxError::Connect {
            reason: format!("ts/udp recv: no data within {read_timeout:?}"),
        })?
        .map_err(|e| MultimuxError::Connect {
            reason: format!("udp recv: {e}"),
        })?;
    driver.feed(&buf[..n], now);
    Ok(())
}

/// Binds `route`'s socket and drives a fresh [`TsUdpIngestSession`] through
/// [`media_plane::ingress::IngestDriver`] until a read stall (bounded by
/// [`IngestTimeouts::read`]) — the new `connect()`+`next_samples()` loop,
/// replacing the pre-5a `TsUdpSource`/`TsUdpSession` pair. Returns the error
/// that ended the loop (always a read-side error — see [`recv_and_feed`]);
/// the caller (the route supervisor) reconnects on it.
pub async fn run_ts_udp(
    route: &TsUdpRoute,
    trunk_config: media_plane::trunk::TrunkConfig,
    handshake: media_plane::ingress::HandshakePolicy,
) -> MultimuxError {
    let socket = match bind(route).await {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut dialer = TsUdpDialer;
    let session = dialer
        .dial()
        .unwrap_or_else(|never: Infallible| match never {});
    let mut driver = media_plane::ingress::IngestDriver::new(session, trunk_config, handshake);
    let mut buf = vec![0u8; MAX_UDP_DATAGRAM];
    let read_timeout = route.timeouts.read;
    let start = std::time::Instant::now();
    loop {
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        if let Err(e) = recv_and_feed(&socket, &mut buf, &mut driver, read_timeout, now).await {
            return e;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::ingress::{HandshakePolicy, HealthState, IngestDriver};
    use media_plane::trunk::TrunkConfig;
    use std::num::NonZeroUsize;
    use transmux::TsMux;
    use transmux::media::Track;
    use transmux::pipeline::{CodecConfig, Sample};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

    fn track_spec(track_id: u32) -> TrackSpec {
        let avc = transmux::avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        )
    }

    fn sample_at(nal: u8) -> Sample {
        Sample::new(vec![0x65, nal], Some(0), Some(0), Some(3000), true)
    }

    // --- Pure ProgramTracker tests: B5 without needing hand-built MPTS bytes ---

    #[test]
    fn first_tracks_resolved_mints_program_zero() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        // Established, then NewProgram(0).
        assert!(matches!(tracker.poll(), Some(SessionEvent::Established)));
        match tracker.poll() {
            Some(SessionEvent::NewProgram { program, tracks }) => {
                assert_eq!(program, ProgramId(0));
                assert_eq!(tracks.len(), 1);
            }
            other => panic!("expected NewProgram(0), got {other:?}"),
        }
    }

    /// The B5 property: a `TrackAdded` arriving *after* the initial program
    /// resolved mints a **second** `ProgramId`, not a dropped/logged event —
    /// this is the exact bug (issue #774's `TrackAdded`-drop) `NewProgram`
    /// was built to close.
    ///
    /// MUTATION-CHECKED: change the `if self.resolved_once` branch's
    /// `ProgramId(self.next_program_id)` to always mint `ProgramId(0)` (i.e.
    /// collapse every late track into the first program) and this test's
    /// `assert_ne!` fails: both programs would compare equal.
    #[test]
    fn late_track_added_mints_a_second_program_not_a_drop() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::TrackAdded(track_spec(1)));
        tracker.handle(DemuxEvent::tracks_resolved(0));
        let _established = tracker.poll();
        let first = match tracker.poll() {
            Some(SessionEvent::NewProgram { program, .. }) => program,
            other => panic!("expected NewProgram, got {other:?}"),
        };

        // A second track declared well after the first program resolved.
        tracker.handle(DemuxEvent::TrackAdded(track_spec(9)));
        let second = match tracker.poll() {
            Some(SessionEvent::NewProgram { program, tracks }) => {
                assert_eq!(tracks[0].track_id, 9);
                program
            }
            other => panic!("expected a second NewProgram, got {other:?}"),
        };
        assert_ne!(
            first, second,
            "a late-declared track must mint a NEW program, not be folded into the first"
        );

        // And a Sample for the late track routes to the second program.
        tracker.handle(DemuxEvent::sample(9, sample_at(0xAA)));
        match tracker.poll() {
            Some(SessionEvent::Sample {
                program, track_id, ..
            }) => {
                assert_eq!(program, second);
                assert_eq!(track_id, 9);
            }
            other => panic!("expected Sample routed to the second program, got {other:?}"),
        }
    }

    /// A `Sample` for a track never announced (e.g. one whose `TrackAdded`
    /// hasn't been seen, or one already `TrackRemoved`) is dropped, not
    /// panicked on or misrouted.
    #[test]
    fn sample_for_unannounced_track_is_dropped() {
        let mut tracker = ProgramTracker::new();
        tracker.handle(DemuxEvent::sample(42, sample_at(0x01)));
        assert!(matches!(tracker.poll(), Some(SessionEvent::Established)));
        assert!(
            tracker.poll().is_none(),
            "no event for an unannounced track's sample"
        );
    }

    // --- B5 note: proven at the ProgramTracker level, not through the real
    // StreamingTsDemux in this suite ------------------------------------
    //
    // An end-to-end variant (feed a real muxed TS stream, then feed a
    // *second* real muxed TS stream carrying a new track, assert a second
    // `Trunk` appears) was attempted and deliberately removed: two
    // independent `TsMux::default().package(&media)` calls each emit PAT/PMT
    // at `version_number` 0 (transmux has no incremental-mux API to bump
    // it), and real MPEG-2 TS PSI semantics treat an unchanged version as
    // "nothing changed" — so `StreamingTsDemux` correctly does *not* fire a
    // second `TrackAdded` for it, and the test failed for the *fixture*
    // being wrong, not the port. `late_track_added_mints_a_second_program_not_a_drop`
    // above proves the actual mechanism (the `ProgramTracker` translation
    // this session's `Stage::feed` drives) against hand-built `DemuxEvent`s,
    // which is the correct level for this property: a genuine version-bumped
    // real-fixture MPTS stream needs `transmux::TsMux` to grow incremental
    // PMT-version control, which is out of this port's scope.

    /// Builds a real (not hand-faked) single-track muxed TS byte stream via
    /// `transmux::TsMux`, mirroring the pre-5a test helper of the same name.
    fn build_ts_bytes(track_id: u32, nal_byte: u8, count: u32) -> Vec<u8> {
        use broadcast_common::Package;
        let spec = track_spec(track_id);
        let frame_dur = 90_000 / 30;
        let samples: Vec<Sample> = (0..count)
            .map(|i| {
                let nal = [0x65u8, nal_byte, (i % 256) as u8];
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

    // --- run_ts_udp: real socket, real end-to-end sample landing in Trunk -

    #[tokio::test]
    async fn loopback_udp_established_and_samples_land_in_trunk() {
        let reserved = UdpSocket::bind("127.0.0.1:0").await.expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        let route = TsUdpRoute::new("cam-ts", addr.to_string(), None);
        // Enough samples (spread over several 7*188-byte datagrams, each
        // paced 5ms apart) that the PMT resolves on an early datagram while
        // later datagrams still carry fresh samples — otherwise the whole
        // stream could fit in the single datagram that also resolves the
        // PMT, and `Trunk::subscribe`'s "starting from now" contract (no
        // backlog) would have nothing left to observe.
        let ts_bytes = build_ts_bytes(1, 0xAB, 60);
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind sender");

        let socket = bind(&route).await.expect("bind route socket");
        let send_task = tokio::spawn(async move {
            for chunk in ts_bytes.chunks(7 * 188) {
                sender.send_to(chunk, addr).await.expect("send TS datagram");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let mut dialer = TsUdpDialer;
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
        assert!(matches!(driver.health(), HealthState::Establishing));

        let mut buf = vec![0u8; MAX_UDP_DATAGRAM];
        let mut cursor = None;
        let mut saw_sample = false;
        for i in 0..200u64 {
            let read = recv_and_feed(
                &socket,
                &mut buf,
                &mut driver,
                Duration::from_millis(500),
                Timestamp::from_nanos(i),
            )
            .await;
            if read.is_err() {
                // A read stall this test's own pacing shouldn't trigger this
                // early, but once the sender has finished it's expected —
                // stop rather than fail.
                break;
            }
            if cursor.is_none() {
                if let Some(t) = driver.trunk(ProgramId(0)) {
                    cursor = Some(t.subscribe());
                }
            }
            if let Some(c) = cursor.as_mut() {
                while let Some(item) = c.poll() {
                    if matches!(item, media_plane::trunk::SampleCursorItem::Timed { .. }) {
                        saw_sample = true;
                    }
                }
            }
            if saw_sample {
                break;
            }
        }
        let _ = send_task.await;
        assert!(matches!(driver.health(), HealthState::Live));
        assert!(
            saw_sample,
            "expected at least one sample published into the Trunk"
        );
    }

    /// A source that stops sending datagrams must fail (not hang) within a
    /// bounded multiple of the configured read timeout — issue #663 P5.2,
    /// preserved from the pre-5a test of the same intent.
    #[tokio::test]
    async fn recv_and_feed_times_out_when_source_goes_silent() {
        let reserved = UdpSocket::bind("127.0.0.1:0").await.expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        const READ_TIMEOUT: Duration = Duration::from_millis(100);
        let route = TsUdpRoute::new("cam-ts", addr.to_string(), None);
        let socket = bind(&route).await.expect("bind");

        let mut dialer = TsUdpDialer;
        let session = dialer.dial().unwrap();
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
        let mut buf = vec![0u8; MAX_UDP_DATAGRAM];

        let outcome = tokio::time::timeout(
            READ_TIMEOUT * 5,
            recv_and_feed(
                &socket,
                &mut buf,
                &mut driver,
                READ_TIMEOUT,
                Timestamp::ZERO,
            ),
        )
        .await
        .expect("recv_and_feed must return within a bounded multiple of the read timeout");
        assert!(
            outcome.is_err(),
            "expected a recoverable read-timeout error"
        );
    }
}
