//! Behaviour pin for the shared ingest session drive loop (issue #2373): the
//! ordered sequence of [`SessionEvent`]s one WHIP publish and one RTMP publish
//! produce, rendered as text so it can be compared against a committed golden
//! under `fixtures/golden/ingest/`.
//!
//! The drive loops in `crate::source::whip` and `crate::source::rtmp` are
//! collapsed into one by #2373; this recorder is the oracle that move must not
//! disturb. It records at the *session* boundary — the [`IngestSession`]
//! trait, exactly where the two loops diverge today and where the shared
//! driver will sit — rather than at the transport, which is already shared.
//!
//! `test-seams` only: it exists for tests, never for a served route.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{
    HandshakePolicy, HealthState, IngestDriver, IngestSession, ListenDriver, Listener, ProgramId,
    SessionEvent, SessionId,
};
use media_plane::trunk::TrunkConfig;
use transmux::TrackSpec;
use transmux::pipeline::CodecConfig;

/// One recorded [`SessionEvent`], reduced to the fields the drive loop's
/// behaviour actually depends on (the program/track identity it routes on and
/// the per-sample shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    event: String,
}

impl Recorded {
    /// This event, rendered as one golden line.
    pub fn event(&self) -> &str {
        &self.event
    }

    fn of(event: &SessionEvent) -> Self {
        let event = match event {
            SessionEvent::Established => "Established".to_string(),
            SessionEvent::NewProgram { program, tracks } => format!(
                "NewProgram program={} tracks=[{}]",
                program.0,
                render_tracks(tracks)
            ),
            SessionEvent::TracksChanged { program, tracks } => format!(
                "TracksChanged program={} tracks=[{}]",
                program.0,
                render_tracks(tracks)
            ),
            SessionEvent::Sample {
                program,
                track_id,
                retention,
                sample,
            } => format!(
                "Sample program={} track={} retention={:?} data={} dts={:?} pts={:?} duration={:?} sync={}",
                program.0,
                track_id,
                retention,
                sample.data.len(),
                sample.timing.and_then(|t| t.dts()),
                sample.timing.and_then(|t| t.pts()),
                sample.timing.and_then(|t| t.duration()),
                sample.flags.is_sync,
            ),
            other => format!("{other:?}"),
        };
        Recorded { event }
    }
}

/// One track's identity as it matters to a downstream consumer: which track,
/// what its clock is, and the codec (plus dimensions, for a video codec — a
/// wrong SPS/PPS capture shows up as a wrong size).
pub fn render_track(t: &TrackSpec) -> String {
    match &t.config {
        CodecConfig::Avc { width, height, .. } | CodecConfig::Hevc { width, height, .. } => {
            format!(
                "{{track={} timescale={} codec={} {}x{}}}",
                t.track_id,
                t.timescale.get(),
                t.config.name(),
                width,
                height
            )
        }
        CodecConfig::Aac {
            channel_count,
            sample_rate,
            ..
        } => format!(
            "{{track={} timescale={} codec={} {}ch {}Hz}}",
            t.track_id,
            t.timescale.get(),
            t.config.name(),
            channel_count,
            sample_rate
        ),
        other => format!(
            "{{track={} timescale={} codec={}}}",
            t.track_id,
            t.timescale.get(),
            other.name()
        ),
    }
}

/// `tracks`, one rendered entry each.
pub fn render_tracks(tracks: &[TrackSpec]) -> String {
    tracks
        .iter()
        .map(render_track)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Wraps a real [`IngestSession`], recording every event it emits and every
/// feed it is handed — in order — so a golden pins both halves of the contract
/// the drive loop is written against.
#[derive(Debug, Clone)]
pub struct RecordingSession<S> {
    inner: S,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl<S: IngestSession> Stage for RecordingSession<S> {
    type In<'a> = S::In<'a>;
    type Out = SessionEvent;
    type Error = S::Error;

    fn feed(&mut self, input: Self::In<'_>, now: Timestamp) -> Result<(), Self::Error> {
        self.inner.feed(input, now)
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        let event = self.inner.poll()?;
        if let Ok(mut log) = self.log.lock() {
            log.push(Recorded::of(&event));
        }
        Some(event)
    }

    /// Records everything `finish` leaves ready to poll: a session's flush
    /// (trailing access units, a final sample) is emitted from inside
    /// `finish`, never from a later `poll`, so draining only after `finish`
    /// returns would miss it.
    fn finish(&mut self) -> Result<(), Self::Error> {
        let result = self.inner.finish();
        while let Some(event) = self.inner.poll() {
            if let Ok(mut log) = self.log.lock() {
                log.push(Recorded::of(&event));
            }
        }
        result
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        self.inner.next_deadline()
    }

    fn on_deadline(&mut self, now: Timestamp) {
        self.inner.on_deadline(now);
    }

    fn demand(&self) -> Demand {
        self.inner.demand()
    }
}

impl<S: IngestSession> IngestSession for RecordingSession<S> {
    type Request = S::Request;

    fn poll_transmit(&mut self) -> Option<S::Request> {
        self.inner.poll_transmit()
    }
}

impl<S> RecordingSession<S> {
    /// Wrap `inner`, recording every event it emits into `log`.
    pub fn new(inner: S, log: Arc<Mutex<Vec<Recorded>>>) -> Self {
        Self { inner, log }
    }

    /// The session underneath — for a test that must drive its own read
    /// (taking a protocol batch off the connection) between recorded events.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }
}

/// A [`Listener`] serving sessions a test supplies itself, in order: each
/// accepted session is wrapped in [`RecordingSession`] so its events land in
/// the shared [`Recorder`] — the sequence the shared drive loop acts on.
pub struct RecordingListener<S> {
    pending: std::collections::VecDeque<S>,
    max_sessions: usize,
    log: Arc<Mutex<Vec<Recorded>>>,
    accepted: Arc<AtomicUsize>,
}

impl<S: IngestSession> Listener for RecordingListener<S> {
    type Session = RecordingSession<S>;
    type Error = std::convert::Infallible;

    fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    fn poll_accept(&mut self) -> Result<Option<Self::Session>, Self::Error> {
        let Some(session) = self.pending.pop_front() else {
            return Ok(None);
        };
        self.accepted.fetch_add(1, Ordering::SeqCst);
        Ok(Some(RecordingSession::new(session, Arc::clone(&self.log))))
    }
}

/// The shared event log plus the count of accepted sessions, handed back to
/// the test alongside the driver.
#[derive(Clone)]
pub struct Recorder {
    log: Arc<Mutex<Vec<Recorded>>>,
    accepted: Arc<AtomicUsize>,
}

impl Recorder {
    /// Every event recorded so far, in order, as one rendered line each.
    pub fn lines(&self) -> Vec<String> {
        let log = self
            .log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        log.iter().map(|r| r.event.clone()).collect()
    }

    /// How many events have been recorded.
    pub fn len(&self) -> usize {
        let log = self
            .log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        log.len()
    }

    /// Whether no event has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many sessions the driver has accepted.
    pub fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

/// Build a [`ListenDriver`] over the sessions in `pending`, recording into a
/// fresh [`Recorder`].
pub fn recording_driver<S: IngestSession + 'static>(
    pending: Vec<S>,
    max_sessions: usize,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    max_programs: std::num::NonZeroUsize,
) -> (ListenDriver<RecordingListener<S>>, Recorder) {
    let recorder = Recorder {
        log: Arc::new(Mutex::new(Vec::new())),
        accepted: Arc::new(AtomicUsize::new(0)),
    };
    let listener = RecordingListener {
        pending: pending.into(),
        max_sessions,
        log: Arc::clone(&recorder.log),
        accepted: Arc::clone(&recorder.accepted),
    };
    (
        ListenDriver::new(
            listener,
            media_plane::IngestConfig::new(handshake)
                .with_trunk(trunk_config)
                .with_max_programs(max_programs),
        ),
        recorder,
    )
}

/// The programs `id` has announced, one sorted line each, each carrying the
/// track specs its `Trunk` currently holds — the identity half of a golden.
pub fn render_programs<S: IngestSession>(
    driver: &ListenDriver<RecordingListener<S>>,
    id: SessionId,
) -> Vec<String> {
    match driver.driver(id) {
        Some(session_driver) => render_programs_of(session_driver),
        None => Vec::new(),
    }
}

/// The programs `driver` has announced, one sorted line each — see
/// [`render_programs`].
pub fn render_programs_of<S: IngestSession>(driver: &IngestDriver<S>) -> Vec<String> {
    let mut lines: Vec<String> = driver
        .programs()
        .filter_map(|program: ProgramId| {
            let trunk = driver.trunk(program)?;
            Some(format!(
                "program={} tracks=[{}]",
                program.0,
                render_tracks(&trunk.tracks())
            ))
        })
        .collect();
    lines.sort();
    lines
}

/// One [`SessionEvent`] as a golden line — the same rendering
/// [`RecordingSession`] applies, for a caller observing events on its own.
pub fn render_event(event: &SessionEvent) -> String {
    Recorded::of(event).event().to_string()
}

/// This session's health, as one stable token.
pub fn render_health<S: IngestSession>(
    driver: &ListenDriver<RecordingListener<S>>,
    id: SessionId,
) -> String {
    match driver.health(id) {
        None => "reaped".to_string(),
        Some(HealthState::Establishing) => "establishing".to_string(),
        Some(HealthState::Live) => "live".to_string(),
        Some(HealthState::Ended) => "ended".to_string(),
        Some(HealthState::Failed(_)) => "failed".to_string(),
        Some(HealthState::HandshakeTimedOut { .. }) => "handshake-timed-out".to_string(),
        Some(_) => "other".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_track_set_renders_without_a_dangling_separator() {
        assert!(render_tracks(&[]).is_empty());
    }
}
