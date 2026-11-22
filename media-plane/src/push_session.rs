//! Sans-IO decision core for a push output: [`PushSession`] owns the connected
//! [`PushEgress`] and answers one [`PushDecision`] per host input.
//!
//! This is the decision half of `multimux`'s `drive_push`, lifted out so the
//! rows can be unit-tested without sockets or a runtime
//! (`docs/dvr-runtime/DESIGN.md` §6, issue #3927/#3687). The host keeps every
//! await: `T::connect`, `setup()`, the bounded flush, the reconnect backoff,
//! the `select!`s, logging and metrics. This module has no clock, no `async`,
//! no tokio and no `tracing`.
//!
//! Every [`PushEgress`] handed to the session comes back inside a decision
//! (`Reconnect`, `Backoff`, `Redundant` or `Stop`) or stays held by the
//! session until one does; each returned egress is the caller's to close,
//! the session itself never closes one. The host closes it with its
//! transport's own shutdown procedure — SRT's shutdown handshake
//! (`private/specs-verified/ietf/specs/ietf_draft_sharabayko_srt_01.txt`
//! §3.2.7) or RTSP `TEARDOWN`
//! (`private/specs-verified/rtp/rtsp-runtime/methods-and-status.md`). The
//! session tracks four things: the connected egress, an egress whose
//! transport setup the host is still awaiting, the track generation last
//! negotiated, and the counters.
//!
//! The decisions mirror, row for row, the table in
//! `docs/dvr-runtime/DESIGN.md` §6 (today's `drive_push`):
//!
//! - a `TracksLagged { skipped }` item counts `skipped` dropped samples and
//!   then behaves exactly as `TracksChanged`;
//! - a `TracksChanged` at or below the negotiated generation, or while not
//!   connected, is a no-op; above it, the egress renegotiates;
//! - `Lagged`/`Degraded` count `skipped` dropped samples and are still handed
//!   to a connected egress, so an output can mark the discontinuity;
//! - `Timed`/`Sparse` while disconnected are dropped one per item, everything
//!   else while disconnected is ignored;
//! - a send failure classified [`PushErrorClass::DropItem`] counts one dropped
//!   sample, a [`PushErrorClass::Reconnect`] one tears the connection down and
//!   hands the egress back;
//! - a flush `Cancelled` and a give-up both stop the session and hand the
//!   egress back.

use transmux::TrackSpec;

use crate::egress::{NegotiationOutcome, PushEgress};
use crate::trunk::SampleCursorItem;

/// Classifies a [`PushEgress`] failure for [`PushSession`]: whether the item
/// can be dropped while the connection stays usable, or the connection itself
/// must be torn down.
///
/// Implemented by the output's own error type (in `multimux`,
/// `SendMediaError`: a mux failure is [`PushErrorClass::DropItem`], a
/// transport failure is [`PushErrorClass::Reconnect`]). The bound is on
/// [`PushSession`]'s impl block only, so [`PushEgress`] and every existing
/// implementation stay unchanged.
pub trait PushErrorKind {
    /// How the session must react to this failure.
    fn kind(&self) -> PushErrorClass;
}

/// The two reactions to a failed send — see [`PushErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PushErrorClass {
    /// Drop the item and keep sending: the failure is in framing this one
    /// item, not in the connection.
    DropItem,
    /// Tear the connection down and reconnect: the transport itself failed.
    Reconnect,
}

impl PushErrorClass {
    /// The stable lowercase token for logs and metrics.
    pub fn name(&self) -> &'static str {
        match self {
            PushErrorClass::DropItem => "drop-item",
            PushErrorClass::Reconnect => "reconnect",
        }
    }
}

broadcast_common::impl_spec_display!(PushErrorClass);

/// What a [`PushSession`] has counted since it was created: samples handed to
/// a connected egress and accepted, samples dropped (disconnected items, loss
/// reports, mux failures), and successful connections.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PushCounters {
    /// Samples whose send succeeded while connected.
    pub samples_sent: u64,
    /// Samples or loss reports accounted as dropped.
    pub samples_dropped: u64,
    /// Connections that negotiated and completed setup.
    pub connects: u32,
}

/// How the host's bounded egress flush ended — see [`PushSession::on_flush`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlushResult {
    /// Everything queued was written.
    Flushed,
    /// The transport reported a write failure.
    Failed,
    /// The write did not finish within the host's limit.
    TimedOut,
    /// The host's cancellation fired first.
    Cancelled,
}

/// Why the session wants a reconnect; carries the failure when there was one.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReconnectCause<Er> {
    /// A send failed with [`PushErrorClass::Reconnect`].
    Send(Er),
    /// A mid-stream renegotiation failed.
    Renegotiate(Er),
    /// The bounded flush reported a write failure.
    Flush,
    /// The bounded flush did not finish in time.
    FlushTimeout,
    /// The transport's own setup failed.
    Setup,
}

/// What the host does next after one [`PushSession`] input.
#[non_exhaustive]
pub enum PushDecision<E: PushEgress> {
    /// Nothing to do; keep draining the cursor on this connection (or wait
    /// for one).
    Continue,
    /// A mid-stream track change was accepted; `carried` of `proposed` tracks
    /// are selected.
    Renegotiated {
        /// Tracks the egress selected.
        carried: usize,
        /// Tracks proposed to it.
        proposed: usize,
    },
    /// A mid-stream track change was truthfully refused; the connection
    /// continues on its previous selection.
    RenegotiateRefused {
        /// The egress's static reason.
        reason: &'static str,
    },
    /// This item could not be framed; it was dropped and the connection
    /// stays up.
    MuxDropped(E::Error),
    /// Connect-time negotiation succeeded: await the transport's setup on
    /// [`PushSession::pending_setup_mut`], then call
    /// [`PushSession::on_setup`].
    NeedsSetup,
    /// Setup completed; the session is connected and negotiated.
    Connected {
        /// Tracks the egress selected.
        carried: usize,
        /// Tracks proposed to it.
        proposed: usize,
    },
    /// Tear this egress down (the caller closes it) and reconnect.
    Reconnect {
        /// The egress, handed back for the caller to close.
        egress: E,
        /// Why the reconnect is needed.
        cause: ReconnectCause<E::Error>,
    },
    /// Connect-time negotiation failed or was refused; close the egress and
    /// back off before dialling again.
    Backoff {
        /// The egress that failed to negotiate, handed back for the caller to
        /// close.
        egress: E,
        /// The failure, when there was one to report.
        cause: Option<E::Error>,
    },
    /// [`PushSession::on_connected`] was called while an egress was already
    /// connected or pending setup; the session is unchanged and the
    /// redundant egress is handed back for the caller to close.
    Redundant {
        /// The redundant egress.
        egress: E,
    },
    /// Stop the push permanently; close the egress if one is handed back.
    Stop {
        /// The connected or pending egress, handed back for the caller to
        /// close.
        egress: Option<E>,
    },
}

impl<E> core::fmt::Debug for PushDecision<E>
where
    E: PushEgress + core::fmt::Debug,
    E::Error: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PushDecision::Continue => f.write_str("Continue"),
            PushDecision::Renegotiated { carried, proposed } => f
                .debug_struct("Renegotiated")
                .field("carried", carried)
                .field("proposed", proposed)
                .finish(),
            PushDecision::RenegotiateRefused { reason } => f
                .debug_struct("RenegotiateRefused")
                .field("reason", reason)
                .finish(),
            PushDecision::MuxDropped(error) => f.debug_tuple("MuxDropped").field(error).finish(),
            PushDecision::NeedsSetup => f.write_str("NeedsSetup"),
            PushDecision::Connected { carried, proposed } => f
                .debug_struct("Connected")
                .field("carried", carried)
                .field("proposed", proposed)
                .finish(),
            PushDecision::Reconnect { egress, cause } => f
                .debug_struct("Reconnect")
                .field("egress", egress)
                .field("cause", cause)
                .finish(),
            PushDecision::Backoff { egress, cause } => f
                .debug_struct("Backoff")
                .field("egress", egress)
                .field("cause", cause)
                .finish(),
            PushDecision::Redundant { egress } => {
                f.debug_struct("Redundant").field("egress", egress).finish()
            }
            PushDecision::Stop { egress } => {
                f.debug_struct("Stop").field("egress", egress).finish()
            }
        }
    }
}

struct PendingSetup<E> {
    egress: E,
    generation: u64,
    carried: usize,
    proposed: usize,
}

/// The sans-IO state machine of one push output — see the module docs.
///
/// One session per output task. The host feeds it every item it drains from
/// its [`crate::SampleCursor`] and every asynchronous outcome, and acts on the
/// returned [`PushDecision`].
#[non_exhaustive]
pub struct PushSession<E> {
    connected: Option<E>,
    pending: Option<PendingSetup<E>>,
    negotiated: Option<u64>,
    counters: PushCounters,
}

impl<E: PushEgress> Default for PushSession<E>
where
    E::Error: PushErrorKind,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E: PushEgress> PushSession<E>
where
    E::Error: PushErrorKind,
{
    /// A session with no connection and zeroed counters.
    pub fn new() -> Self {
        PushSession {
            connected: None,
            pending: None,
            negotiated: None,
            counters: PushCounters::default(),
        }
    }

    /// One item drained from the sample cursor.
    pub fn on_item(&mut self, item: &SampleCursorItem) -> PushDecision<E> {
        if let SampleCursorItem::TracksChanged { generation, tracks }
        | SampleCursorItem::TracksLagged {
            generation, tracks, ..
        } = item
        {
            if let SampleCursorItem::TracksLagged { skipped, .. } = item {
                self.counters.samples_dropped =
                    self.counters.samples_dropped.saturating_add(*skipped);
            }
            return self.on_track_change(*generation, tracks);
        }
        if let SampleCursorItem::Lagged { skipped } | SampleCursorItem::Degraded { skipped } = item
        {
            self.counters.samples_dropped = self.counters.samples_dropped.saturating_add(*skipped);
        }
        let Some(egress) = self.connected.as_mut() else {
            if is_sample(item) {
                self.counters.samples_dropped = self.counters.samples_dropped.saturating_add(1);
            }
            return PushDecision::Continue;
        };
        match egress.send(item) {
            Ok(()) => {
                if is_sample(item) {
                    self.counters.samples_sent = self.counters.samples_sent.saturating_add(1);
                }
                PushDecision::Continue
            }
            Err(error) => match error.kind() {
                PushErrorClass::DropItem => {
                    self.counters.samples_dropped = self.counters.samples_dropped.saturating_add(1);
                    PushDecision::MuxDropped(error)
                }
                PushErrorClass::Reconnect => {
                    let Some(egress) = self.connected.take() else {
                        return PushDecision::Continue;
                    };
                    self.negotiated = None;
                    PushDecision::Reconnect {
                        egress,
                        cause: ReconnectCause::Send(error),
                    }
                }
            },
        }
    }

    /// The host dialled `T::connect` and now hands the egress over.
    ///
    /// A call while an egress is already connected or pending setup is a
    /// caller error (gate on [`is_connected`](Self::is_connected) and
    /// [`pending_setup_mut`](Self::pending_setup_mut)); the redundant egress
    /// comes back in [`PushDecision::Redundant`] and the session continues
    /// unchanged. A failed or refused negotiation hands the egress back in
    /// [`PushDecision::Backoff`].
    pub fn on_connected(
        &mut self,
        mut egress: E,
        generation: u64,
        tracks: &[TrackSpec],
    ) -> PushDecision<E> {
        if self.connected.is_some() || self.pending.is_some() {
            return PushDecision::Redundant { egress };
        }
        match egress.negotiate(tracks) {
            NegotiationOutcome::Accepted(selection) => {
                self.pending = Some(PendingSetup {
                    egress,
                    generation,
                    carried: selection.track_ids.len(),
                    proposed: tracks.len(),
                });
                PushDecision::NeedsSetup
            }
            NegotiationOutcome::Error(error) => PushDecision::Backoff {
                egress,
                cause: Some(error),
            },
            _ => PushDecision::Backoff {
                egress,
                cause: None,
            },
        }
    }

    /// The pending egress, for the host to await the transport's setup on.
    pub fn pending_setup_mut(&mut self) -> Option<&mut E> {
        self.pending.as_mut().map(|pending| &mut pending.egress)
    }

    /// The transport's setup finished: `ok` connects the session, otherwise
    /// the egress is handed back for the caller to close.
    pub fn on_setup(&mut self, ok: bool) -> PushDecision<E> {
        let Some(pending) = self.pending.take() else {
            return PushDecision::Continue;
        };
        if !ok {
            return PushDecision::Reconnect {
                egress: pending.egress,
                cause: ReconnectCause::Setup,
            };
        }
        self.connected = Some(pending.egress);
        self.negotiated = Some(pending.generation);
        self.counters.connects = self.counters.connects.saturating_add(1);
        PushDecision::Connected {
            carried: pending.carried,
            proposed: pending.proposed,
        }
    }

    /// The connected egress, for the host's bounded flush.
    pub fn egress_mut(&mut self) -> Option<&mut E> {
        self.connected.as_mut()
    }

    /// The host's bounded flush ended — see [`FlushResult`].
    pub fn on_flush(&mut self, outcome: FlushResult) -> PushDecision<E> {
        match outcome {
            FlushResult::Flushed => PushDecision::Continue,
            FlushResult::Cancelled => PushDecision::Stop {
                egress: self.take_egress(),
            },
            FlushResult::Failed | FlushResult::TimedOut => {
                let Some(egress) = self.connected.take() else {
                    return PushDecision::Continue;
                };
                self.negotiated = None;
                let cause = match outcome {
                    FlushResult::Failed => ReconnectCause::Flush,
                    _ => ReconnectCause::FlushTimeout,
                };
                PushDecision::Reconnect { egress, cause }
            }
        }
    }

    /// The reconnect policy gave up (the cursor has been drained): stop and
    /// hand the egress back, if any.
    pub fn on_give_up(&mut self) -> PushDecision<E> {
        PushDecision::Stop {
            egress: self.take_egress(),
        }
    }

    /// Whether an egress is connected and set up.
    pub fn is_connected(&self) -> bool {
        self.connected.is_some()
    }

    /// The counters as of now.
    pub fn counters(&self) -> PushCounters {
        self.counters
    }

    fn on_track_change(&mut self, generation: u64, tracks: &[TrackSpec]) -> PushDecision<E> {
        let Some(egress) = self.connected.as_mut() else {
            return PushDecision::Continue;
        };
        if self
            .negotiated
            .is_some_and(|negotiated| generation <= negotiated)
        {
            return PushDecision::Continue;
        }
        match egress.renegotiate(tracks) {
            NegotiationOutcome::Accepted(selection) => {
                self.negotiated = Some(generation);
                PushDecision::Renegotiated {
                    carried: selection.track_ids.len(),
                    proposed: tracks.len(),
                }
            }
            NegotiationOutcome::Refused { reason } => {
                self.negotiated = Some(generation);
                PushDecision::RenegotiateRefused { reason }
            }
            NegotiationOutcome::Error(error) => {
                let Some(egress) = self.connected.take() else {
                    return PushDecision::Continue;
                };
                self.negotiated = None;
                PushDecision::Reconnect {
                    egress,
                    cause: ReconnectCause::Renegotiate(error),
                }
            }
        }
    }

    fn take_egress(&mut self) -> Option<E> {
        self.connected
            .take()
            .or_else(|| self.pending.take().map(|pending| pending.egress))
    }
}

fn is_sample(item: &SampleCursorItem) -> bool {
    matches!(
        item,
        SampleCursorItem::Timed { .. } | SampleCursorItem::Sparse { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::TrackSelection;
    use broadcast_common::timescale::Timescale;
    use bytes::Bytes;
    use std::collections::VecDeque;
    use std::sync::Arc;
    use transmux::{CodecConfig, Sample, TrackId};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MockError {
        Mux,
        Transport,
    }

    impl PushErrorKind for MockError {
        fn kind(&self) -> PushErrorClass {
            match self {
                MockError::Mux => PushErrorClass::DropItem,
                MockError::Transport => PushErrorClass::Reconnect,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Negotiate { tracks: usize },
        Renegotiate { tracks: usize },
        Send,
    }

    #[derive(Debug)]
    struct MockEgress {
        id: u8,
        calls: Vec<Call>,
        negotiate: VecDeque<NegotiationOutcome<MockError>>,
        renegotiate: VecDeque<NegotiationOutcome<MockError>>,
        send: VecDeque<Result<(), MockError>>,
    }

    impl MockEgress {
        fn new(id: u8) -> Self {
            MockEgress {
                id,
                calls: Vec::new(),
                negotiate: VecDeque::new(),
                renegotiate: VecDeque::new(),
                send: VecDeque::new(),
            }
        }
    }

    fn select(tracks: &[TrackSpec], count: usize) -> TrackSelection {
        TrackSelection::new(tracks.iter().take(count).map(|t| t.track_id).collect())
    }

    impl PushEgress for MockEgress {
        type Error = MockError;

        fn negotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
            self.calls.push(Call::Negotiate {
                tracks: tracks.len(),
            });
            self.negotiate
                .pop_front()
                .unwrap_or_else(|| NegotiationOutcome::Accepted(select(tracks, tracks.len())))
        }

        fn renegotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
            self.calls.push(Call::Renegotiate {
                tracks: tracks.len(),
            });
            self.renegotiate
                .pop_front()
                .unwrap_or_else(|| NegotiationOutcome::Accepted(select(tracks, tracks.len())))
        }

        fn send(&mut self, _item: &SampleCursorItem) -> Result<(), Self::Error> {
            self.calls.push(Call::Send);
            self.send.pop_front().unwrap_or(Ok(()))
        }
    }

    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    fn spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            TrackId::new(track_id),
            timescale(90_000),
            CodecConfig::Vp8 {
                width: 320,
                height: 240,
            },
        )
    }

    fn track_list(count: u32) -> Vec<TrackSpec> {
        (1..=count).map(spec).collect()
    }

    fn shared(count: u32) -> Arc<[TrackSpec]> {
        track_list(count).into()
    }

    fn sample(byte: u8) -> Sample {
        Sample::new(Bytes::from(vec![byte; 4]), Some(0), Some(0), None, true).expect("valid timing")
    }

    fn timed(byte: u8) -> SampleCursorItem {
        SampleCursorItem::Timed {
            track_id: TrackId::new(1),
            sample: sample(byte),
        }
    }

    fn connect(session: &mut PushSession<MockEgress>, generation: u64) {
        let tracks = track_list(2);
        assert!(matches!(
            session.on_connected(MockEgress::new(1), generation, &tracks),
            PushDecision::NeedsSetup
        ));
        assert!(matches!(
            session.on_setup(true),
            PushDecision::Connected {
                carried: 2,
                proposed: 2
            }
        ));
    }

    fn egress_calls(session: &mut PushSession<MockEgress>) -> Vec<Call> {
        session
            .egress_mut()
            .map(|egress| egress.calls.clone())
            .unwrap_or_default()
    }

    fn pending_calls(session: &mut PushSession<MockEgress>) -> Vec<Call> {
        session
            .pending_setup_mut()
            .map(|egress| egress.calls.clone())
            .unwrap_or_default()
    }

    fn queue_send(session: &mut PushSession<MockEgress>, result: Result<(), MockError>) {
        if let Some(egress) = session.egress_mut() {
            egress.send.push_back(result);
        }
    }

    #[test]
    fn tracks_lagged_counts_skipped_then_renegotiates_like_a_track_change() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        if let Some(egress) = session.egress_mut() {
            egress
                .renegotiate
                .push_back(NegotiationOutcome::Accepted(select(&track_list(3), 1)));
        }

        let item = SampleCursorItem::TracksLagged {
            skipped: 3,
            generation: 8,
            tracks: shared(3),
        };
        match session.on_item(&item) {
            PushDecision::Renegotiated { carried, proposed } => {
                assert_eq!((carried, proposed), (1, 3));
            }
            _ => panic!("expected Renegotiated"),
        }
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 3,
                connects: 1,
            }
        );
        assert_eq!(
            egress_calls(&mut session),
            vec![
                Call::Negotiate { tracks: 2 },
                Call::Renegotiate { tracks: 3 },
            ]
        );
    }

    #[test]
    fn tracks_changed_at_or_below_the_negotiated_generation_continues() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);

        let current = SampleCursorItem::TracksChanged {
            generation: 7,
            tracks: shared(3),
        };
        assert!(matches!(session.on_item(&current), PushDecision::Continue));
        let stale = SampleCursorItem::TracksChanged {
            generation: 6,
            tracks: shared(1),
        };
        assert!(matches!(session.on_item(&stale), PushDecision::Continue));
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
        assert_eq!(
            egress_calls(&mut session),
            vec![Call::Negotiate { tracks: 2 }]
        );
    }

    #[test]
    fn tracks_changed_while_disconnected_continues_without_negotiating() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let item = SampleCursorItem::TracksChanged {
            generation: 5,
            tracks: shared(2),
        };
        assert!(matches!(session.on_item(&item), PushDecision::Continue));
        assert_eq!(session.counters(), PushCounters::default());
        assert!(!session.is_connected());
        assert!(session.egress_mut().is_none());
    }

    #[test]
    fn tracks_changed_accepted_reports_the_partial_selection_and_advances() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        if let Some(egress) = session.egress_mut() {
            egress
                .renegotiate
                .push_back(NegotiationOutcome::Accepted(select(&track_list(3), 2)));
        }

        let item = SampleCursorItem::TracksChanged {
            generation: 8,
            tracks: shared(3),
        };
        match session.on_item(&item) {
            PushDecision::Renegotiated { carried, proposed } => {
                assert_eq!((carried, proposed), (2, 3));
            }
            _ => panic!("expected Renegotiated"),
        }
        assert!(matches!(session.on_item(&item), PushDecision::Continue));
        assert_eq!(
            egress_calls(&mut session),
            vec![
                Call::Negotiate { tracks: 2 },
                Call::Renegotiate { tracks: 3 },
            ]
        );
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
    }

    #[test]
    fn tracks_changed_refused_reports_the_reason_and_advances() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        if let Some(egress) = session.egress_mut() {
            egress.renegotiate.push_back(NegotiationOutcome::Refused {
                reason: "sdp already answered",
            });
        }

        let item = SampleCursorItem::TracksChanged {
            generation: 8,
            tracks: shared(3),
        };
        match session.on_item(&item) {
            PushDecision::RenegotiateRefused { reason } => {
                assert_eq!(reason, "sdp already answered");
            }
            _ => panic!("expected RenegotiateRefused"),
        }
        assert!(matches!(session.on_item(&item), PushDecision::Continue));
        assert!(session.is_connected());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
    }

    #[test]
    fn tracks_changed_error_hands_the_egress_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        if let Some(egress) = session.egress_mut() {
            egress
                .renegotiate
                .push_back(NegotiationOutcome::Error(MockError::Transport));
        }

        let item = SampleCursorItem::TracksChanged {
            generation: 8,
            tracks: shared(3),
        };
        match session.on_item(&item) {
            PushDecision::Reconnect { egress, cause } => {
                assert_eq!(egress.id, 1);
                assert!(matches!(
                    cause,
                    ReconnectCause::Renegotiate(MockError::Transport)
                ));
            }
            _ => panic!("expected Reconnect"),
        }
        assert!(!session.is_connected());
        assert!(session.egress_mut().is_none());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
        assert!(matches!(
            session.on_item(&SampleCursorItem::TracksChanged {
                generation: 9,
                tracks: shared(1),
            }),
            PushDecision::Continue
        ));
    }

    #[test]
    fn lagged_and_degraded_count_skipped_and_still_reach_the_egress() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);

        assert!(matches!(
            session.on_item(&SampleCursorItem::Lagged { skipped: 2 }),
            PushDecision::Continue
        ));
        assert!(matches!(
            session.on_item(&SampleCursorItem::Degraded { skipped: 5 }),
            PushDecision::Continue
        ));
        assert!(session.is_connected());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 7,
                connects: 1,
            }
        );
        assert_eq!(
            egress_calls(&mut session),
            vec![Call::Negotiate { tracks: 2 }, Call::Send, Call::Send,]
        );

        let mut offline: PushSession<MockEgress> = PushSession::new();
        assert!(matches!(
            offline.on_item(&SampleCursorItem::Lagged { skipped: 3 }),
            PushDecision::Continue
        ));
        assert!(matches!(
            offline.on_item(&SampleCursorItem::Degraded { skipped: 4 }),
            PushDecision::Continue
        ));
        assert_eq!(
            offline.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 7,
                connects: 0,
            }
        );
    }

    #[test]
    fn timed_and_sparse_while_disconnected_are_dropped_one_each() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        assert!(matches!(session.on_item(&timed(1)), PushDecision::Continue));
        assert!(matches!(
            session.on_item(&SampleCursorItem::Sparse {
                track_id: TrackId::new(2),
                sample: sample(2),
            }),
            PushDecision::Continue
        ));
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 2,
                connects: 0,
            }
        );
    }

    #[test]
    fn other_items_while_disconnected_continue_without_counting() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        assert!(matches!(
            session.on_item(&SampleCursorItem::Discontinuity),
            PushDecision::Continue
        ));
        assert!(matches!(
            session.on_item(&SampleCursorItem::HandoversLagged { skipped: 4 }),
            PushDecision::Continue
        ));
        assert_eq!(session.counters(), PushCounters::default());
    }

    #[test]
    fn a_sent_sample_while_connected_counts_and_continues() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);

        assert!(matches!(session.on_item(&timed(1)), PushDecision::Continue));
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 1,
                samples_dropped: 0,
                connects: 1,
            }
        );
        assert_eq!(
            egress_calls(&mut session),
            vec![Call::Negotiate { tracks: 2 }, Call::Send]
        );
    }

    #[test]
    fn a_drop_item_send_failure_counts_one_dropped_and_stays_connected() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        queue_send(&mut session, Err(MockError::Mux));

        match session.on_item(&timed(1)) {
            PushDecision::MuxDropped(MockError::Mux) => {}
            _ => panic!("expected MuxDropped(Mux)"),
        }
        assert!(session.is_connected());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 1,
                connects: 1,
            }
        );
    }

    #[test]
    fn a_reconnect_send_failure_hands_the_egress_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        queue_send(&mut session, Err(MockError::Transport));

        match session.on_item(&timed(1)) {
            PushDecision::Reconnect { egress, cause } => {
                assert_eq!(egress.id, 1);
                assert!(matches!(cause, ReconnectCause::Send(MockError::Transport)));
            }
            _ => panic!("expected Reconnect"),
        }
        assert!(!session.is_connected());
        assert!(session.egress_mut().is_none());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
    }

    #[test]
    fn on_connected_accepted_needs_setup_and_holds_the_egress_pending() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(2);
        let mut egress = MockEgress::new(4);
        egress
            .negotiate
            .push_back(NegotiationOutcome::Accepted(select(&tracks, 1)));

        assert!(matches!(
            session.on_connected(egress, 7, &tracks),
            PushDecision::NeedsSetup
        ));
        assert!(!session.is_connected());
        assert!(session.egress_mut().is_none());
        assert_eq!(
            pending_calls(&mut session),
            vec![Call::Negotiate { tracks: 2 }]
        );
        assert_eq!(session.counters(), PushCounters::default());
    }

    #[test]
    fn on_connected_negotiation_error_backs_off_with_the_error() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(2);
        let mut egress = MockEgress::new(4);
        egress
            .negotiate
            .push_back(NegotiationOutcome::Error(MockError::Mux));

        match session.on_connected(egress, 7, &tracks) {
            PushDecision::Backoff {
                egress,
                cause: Some(MockError::Mux),
            } => assert_eq!(egress.id, 4),
            _ => panic!("expected Backoff with the negotiation error"),
        }
        assert!(session.pending_setup_mut().is_none());
        assert_eq!(session.counters(), PushCounters::default());
    }

    #[test]
    fn on_connected_refusal_backs_off_without_an_error() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(2);
        let mut egress = MockEgress::new(4);
        egress.negotiate.push_back(NegotiationOutcome::Refused {
            reason: "no prior selection",
        });

        match session.on_connected(egress, 7, &tracks) {
            PushDecision::Backoff {
                egress,
                cause: None,
            } => assert_eq!(egress.id, 4),
            _ => panic!("expected Backoff without an error"),
        }
        assert!(session.pending_setup_mut().is_none());
    }

    #[test]
    fn on_setup_true_connects_with_the_negotiated_counts() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(2);
        let mut egress = MockEgress::new(4);
        egress
            .negotiate
            .push_back(NegotiationOutcome::Accepted(select(&tracks, 1)));
        assert!(matches!(
            session.on_connected(egress, 7, &tracks),
            PushDecision::NeedsSetup
        ));

        match session.on_setup(true) {
            PushDecision::Connected { carried, proposed } => {
                assert_eq!((carried, proposed), (1, 2));
            }
            _ => panic!("expected Connected"),
        }
        assert!(session.is_connected());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
        let current = SampleCursorItem::TracksChanged {
            generation: 7,
            tracks: shared(2),
        };
        assert!(matches!(session.on_item(&current), PushDecision::Continue));
        assert_eq!(
            egress_calls(&mut session),
            vec![Call::Negotiate { tracks: 2 }]
        );
    }

    #[test]
    fn on_setup_false_hands_the_egress_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(1);
        assert!(matches!(
            session.on_connected(MockEgress::new(9), 3, &tracks),
            PushDecision::NeedsSetup
        ));

        match session.on_setup(false) {
            PushDecision::Reconnect { egress, cause } => {
                assert_eq!(egress.id, 9);
                assert!(matches!(cause, ReconnectCause::Setup));
            }
            _ => panic!("expected Reconnect"),
        }
        assert!(!session.is_connected());
        assert!(session.pending_setup_mut().is_none());
        assert_eq!(session.counters(), PushCounters::default());
    }

    #[test]
    fn on_setup_without_a_pending_egress_continues() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        assert!(matches!(session.on_setup(true), PushDecision::Continue));
        assert!(matches!(session.on_setup(false), PushDecision::Continue));
        assert_eq!(session.counters(), PushCounters::default());
    }

    #[test]
    fn a_redundant_connected_egress_is_handed_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);

        let tracks = track_list(2);
        match session.on_connected(MockEgress::new(2), 8, &tracks) {
            PushDecision::Redundant { egress } => {
                assert_eq!(egress.id, 2);
                assert!(egress.calls.is_empty());
            }
            _ => panic!("expected the redundant egress back"),
        }
        assert!(session.is_connected());
        assert!(session.pending_setup_mut().is_none());
        assert_eq!(
            session
                .egress_mut()
                .map(|egress| egress.id)
                .unwrap_or_default(),
            1
        );
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
    }

    #[test]
    fn a_redundant_egress_while_setup_is_pending_is_handed_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(2);
        assert!(matches!(
            session.on_connected(MockEgress::new(1), 7, &tracks),
            PushDecision::NeedsSetup
        ));
        match session.on_connected(MockEgress::new(3), 8, &tracks) {
            PushDecision::Redundant { egress } => assert_eq!(egress.id, 3),
            _ => panic!("expected the redundant egress back"),
        }
        assert_eq!(
            session
                .pending_setup_mut()
                .map(|egress| egress.id)
                .unwrap_or_default(),
            1
        );
    }

    #[test]
    fn a_decision_is_debug_printable() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(2);
        let mut egress = MockEgress::new(5);
        egress
            .negotiate
            .push_back(NegotiationOutcome::Error(MockError::Transport));
        let rendered = format!("{:?}", session.on_connected(egress, 7, &tracks));
        assert!(rendered.starts_with("Backoff"), "{rendered}");
        assert!(rendered.contains("id: 5"), "{rendered}");
        assert!(rendered.contains("Some(Transport)"), "{rendered}");
    }

    #[test]
    fn on_flush_flushed_continues_with_the_egress() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);

        assert!(matches!(
            session.on_flush(FlushResult::Flushed),
            PushDecision::Continue
        ));
        assert!(session.is_connected());
        assert!(session.egress_mut().is_some());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 1,
            }
        );
    }

    #[test]
    fn on_flush_failed_and_timed_out_hand_the_egress_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        match session.on_flush(FlushResult::Failed) {
            PushDecision::Reconnect { egress, cause } => {
                assert_eq!(egress.id, 1);
                assert!(matches!(cause, ReconnectCause::Flush));
            }
            _ => panic!("expected Reconnect"),
        }
        assert!(!session.is_connected());
        assert!(matches!(
            session.on_flush(FlushResult::Failed),
            PushDecision::Continue
        ));

        connect(&mut session, 7);
        match session.on_flush(FlushResult::TimedOut) {
            PushDecision::Reconnect { egress, cause } => {
                assert_eq!(egress.id, 1);
                assert!(matches!(cause, ReconnectCause::FlushTimeout));
            }
            _ => panic!("expected Reconnect"),
        }
        assert!(!session.is_connected());
        assert_eq!(
            session.counters(),
            PushCounters {
                samples_sent: 0,
                samples_dropped: 0,
                connects: 2,
            }
        );
    }

    #[test]
    fn on_flush_cancelled_stops_and_hands_the_egress_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        connect(&mut session, 7);
        match session.on_flush(FlushResult::Cancelled) {
            PushDecision::Stop {
                egress: Some(egress),
            } => assert_eq!(egress.id, 1),
            _ => panic!("expected Stop with the egress"),
        }
        assert!(!session.is_connected());

        let mut pending: PushSession<MockEgress> = PushSession::new();
        let tracks = track_list(1);
        assert!(matches!(
            pending.on_connected(MockEgress::new(9), 3, &tracks),
            PushDecision::NeedsSetup
        ));
        match pending.on_flush(FlushResult::Cancelled) {
            PushDecision::Stop {
                egress: Some(egress),
            } => assert_eq!(egress.id, 9),
            _ => panic!("expected Stop with the pending egress"),
        }
        assert!(pending.pending_setup_mut().is_none());
    }

    #[test]
    fn on_give_up_stops_and_hands_the_egress_back() {
        let mut session: PushSession<MockEgress> = PushSession::new();
        assert!(matches!(
            session.on_give_up(),
            PushDecision::Stop { egress: None }
        ));

        connect(&mut session, 7);
        match session.on_give_up() {
            PushDecision::Stop {
                egress: Some(egress),
            } => assert_eq!(egress.id, 1),
            _ => panic!("expected Stop with the egress"),
        }
        assert!(!session.is_connected());
    }

    #[test]
    fn push_error_class_labels_are_stable() {
        assert_eq!(PushErrorClass::DropItem.name(), "drop-item");
        assert_eq!(PushErrorClass::Reconnect.name(), "reconnect");
        assert_eq!(PushErrorClass::DropItem.to_string(), "drop-item");
        assert_eq!(PushErrorClass::Reconnect.to_string(), "reconnect");
    }
}
