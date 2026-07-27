//! `Dialer`/`Listener`/`IngestSession` — the ingress traits, and the generic
//! `run_dial`/`run_listen` drivers that pump them into a [`crate::Trunk`]
//! (plan step 3c;
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §2).
//!
//! Traits and generic drivers only — no real protocol is ported here (that
//! is plan step 5, "port the 9 sources"). This module's job is to define a
//! shape those nine `multimux::source::*` implementations can all actually
//! be squeezed into, and to own the feed/poll/deadline/dispatch loop so no
//! protocol has to reimplement it (every one of the nine today hand-rolls
//! its own `while let Some(event) = demux.poll_event() { match event { .. } }`
//! drain — see e.g. `multimux::source::ts_udp::TsUdpSession::next_samples`).
//!
//! `#[cfg(feature = "std")]`: like [`crate::trunk`], this module wires
//! straight to [`crate::Trunk`] (`Arc`/`HashMap`), and every real consumer is
//! `std`+`tokio` per the architecture (see `crate::trunk`'s module docs).
//!
//! # `IngestSession` is a `Stage`, matching `ByteStage`'s precedent
//!
//! Exactly like [`crate::ByteStage`], `IngestSession` is not a new drive
//! trait — it is a blanket specialisation of [`broadcast_common::Stage`]
//! whose input is a borrowed byte slice and whose output is [`SessionEvent`]:
//!
//! ```text
//! pub trait IngestSession: for<'a> Stage<In<'a> = &'a [u8], Out = SessionEvent> + Send { .. }
//! ```
//!
//! This buys the same three things `ByteStage` documents: one drive model
//! for the whole plane, `finish()` for a clean end-of-input flush, and
//! `demand()` for back-pressure — plus it means [`run_dial`]/[`run_listen`]
//! (below) can drive an `IngestSession` with the exact same "feed, drain
//! `poll()`, repeat" loop [`crate::byte_stage`]'s own tests already validate
//! against a real `Stage`. The one addition over the bare blanket ([`ByteStage`]
//! adds nothing) is [`IngestSession::poll_transmit`], with a `None`-returning
//! default so the blanket impl still applies to every `Stage` for free — see
//! [Why `poll_transmit` exists](#why-poll_transmit-exists-and-most-sources-will-never-override-it) below.
//!
//! [`ByteStage`]: crate::ByteStage
//!
//! ## Why `poll_transmit` exists, and most sources will never override it
//!
//! `RtspSession` (`multimux/src/source/rtsp.rs`) is driven over an
//! interleaved TCP connection where the client is also expected to send
//! (RTCP receiver reports, periodic keepalive) — a pure `feed(bytes) ->
//! poll() -> SessionEvent` consumer has nowhere to hand that back. Rather
//! than invent a second trait for the two or three sources that need it,
//! [`IngestSession::poll_transmit`] is a plain method with a `None` default:
//! a driver (this module's [`IngestDriver`]/[`ListenDriver`], or a future
//! Step 5 adapter) drains it after every `feed`/`on_deadline` exactly like
//! [`Stage::poll`], and a session with nothing to send simply never
//! overrides it.
//!
//! # The program dimension (B5) — `SessionEvent::NewProgram` at any time
//!
//! Rev 1 of the architecture assumed one connection maps to exactly one
//! timeline; the audit's finding B5 is that MPTS and T2-MI multi-PLP break
//! this outright (`parse_pat` flattens every program; `program_number`
//! appears nowhere in the demuxed IR). [`SessionEvent::NewProgram`] is how an
//! `IngestSession` announces one: [`IngestDriver`]/[`ListenDriver`] mint a
//! **fresh [`Trunk`]** for every [`ProgramId`] the moment it is announced —
//! including the second, third, ... program on the *same* connection, and
//! including one announced only after other programs (or samples for them)
//! have already been flowing. There is no "known programs" list supplied up
//! front and no special first-poll path: `NewProgram` is just another
//! [`SessionEvent`] variant, driven through the exact same `poll()` drain as
//! every `Sample`, so "a program appears mid-session" is not a distinct code
//! path from "a program was there from the start" — it is the *only* path.
//!
//! This is deliberately more general than architecture §1.3's steady-state
//! design (program-splitting as a `ByteStage` upstream of demux, so each
//! `IngestSession` only ever sees one already-known program). That design is
//! still the right Step 5 target for MPTS — it lets each program's demux run
//! independently — but it presupposes the program table has already been
//! read once to know how many `ByteStage`s to build, which is exactly the
//! chicken-and-egg `multimux::source::ts_udp::TsUdpSession::next_samples`
//! hits *today*: a PMT version bump that adds a **track** after `connect()`'s
//! one-shot `track_specs()` snapshot is already only handled by logging a
//! warning and dropping it (see that function's `DemuxEvent::TrackAdded`
//! arm) — there is no live wiring for "new track" today, let alone "new
//! program". `SessionEvent::NewProgram` is what closes that gap generically:
//! whether a program is split upstream (known before the session starts) or
//! discovered while demuxing an MPTS in one session, the driver's reaction is
//! identical — mint a `Trunk`, keep going.
//!
//! # Supervision: EOF is not an error (the `HealthState` fix)
//!
//! Today's bug, concretely: `multimux::origin::supervisor::supervise` treats
//! `run_pipeline`'s `Ok(())` (clean source EOF) and `Err(_)` (a real failure)
//! identically — both fall into the same `set_health(HealthState::Reconnecting)`
//! arm (`multimux/src/origin/supervisor.rs`) — and
//! `ll_hls_runtime::server::store::HealthState::Failed`'s own doc comment
//! admits *"the loop here does not currently produce it"*. Nothing
//! distinguishes "the stream ended" from "the stream broke" because both are
//! folded into one `Result<(), Error>` before the health state is even set.
//!
//! [`HealthState`] here is not a copy of that enum — it is what the fold
//! above should have been: an `IngestSession`'s [`Stage::finish`] returning
//! `Ok(())` (clean end of input, no error ever raised) drives
//! [`HealthState::Ended`]; its [`Stage::feed`]/[`finish`](Stage::finish)
//! returning `Err` drives [`HealthState::Failed`] carrying that concrete error, generically
//! (`HealthState<E>`, `E = S::Error`) rather than losing it to a formatted
//! string. Both are reachable and observed via [`IngestDriver::health`]/
//! [`ListenDriver::health`] — see this module's tests for a mutation-checked
//! proof that ending cleanly is never mistaken for failing.
//!
//! # `Listener` and `max_sessions`: enforced by the driver, not by convention
//!
//! `max_sessions` lives on the [`Listener`] trait as a fixed accessor, but
//! **[`ListenDriver::poll_accept`] is the only place it is checked** — a
//! concrete `Listener` cannot forget to enforce it (there is nothing for it
//! to enforce; `poll_accept` just hands back whatever the transport
//! accepted). Once `max_sessions` live sessions are admitted, every further
//! accepted connection is dropped **immediately, before being fed a single
//! byte** — this project has already shipped four unbounded-allocation
//! vectors, and an unbounded listener is exactly that class of bug, so the
//! bound is structural (checked in one generic place) rather than a
//! per-protocol discipline.
//!
//! # `Dialer`/`Listener` are a synchronous boundary, not a byte-level sans-IO
//! handshake (a rejected alternative, recorded here)
//!
//! `broadcast_common::Stage` and `IngestSession` are genuinely sans-IO
//! (byte-in, typed-out, no I/O performed by the trait). `Dialer::dial` and
//! `Listener::poll_accept` are **not** — they return `Result<Session, Error>`
//! directly, and a real implementation is free to block or run an async
//! multi-round-trip handshake underneath (`RtspSource::connect` is DESCRIBE →
//! SETUP × N → PLAY; `TsUdpSource::connect` reads datagrams until a PMT
//! resolves; `RtmpSource::connect` accepts then waits for a first sample).
//!
//! A byte-level sans-IO handshake trait for `Dialer` (mirroring `Stage`:
//! `feed`/`poll_transmit`/`poll -> Option<Session>`) was considered and
//! rejected for this step: every real dial in the workspace today already
//! *is* an async function doing exactly that multi-round-trip wait, so a
//! byte-level state machine here would not remove any work — it would only
//! relocate the same logic into a hand-rolled state machine inside each
//! implementor, for a layer (`Dialer|Listener`) that sits *above* the byte
//! layer in the architecture diagram specifically because connection
//! establishment is not byte-processing. The real cost this defers to Step 5
//! is documented in this crate's `.../scratchpad/3c-report.md`: four of the
//! nine sources model "listen" as "accept one session serially per
//! `connect()` call" with no concurrent-session concept at all (`RtmpSource`,
//! `SrtSource` listener mode), and every dial-based source performs its
//! handshake as an `async fn` that a synchronous `Dialer::dial` cannot call
//! without an executor bridge — both are real adaptation costs, named here
//! rather than hidden.
//!
//! # Reconnect: caller-chosen backoff, never a hardcoded sleep
//!
//! [`DialSupervisor`] bounds *how many times* [`Dialer::dial`] is retried
//! (`ReconnectPolicy::max_attempts`) but never sleeps, blocks, or otherwise
//! decides *how long* to wait between attempts — that stays entirely with
//! the caller (an async `sleep`, a `tokio::time::sleep`, nothing at all in a
//! test), matching every other bounded-but-caller-driven knob in this crate
//! ([`crate::byte_merge::MergePolicy::Failover`]'s `silence_timeout`, driven
//! by the caller's own `on_deadline` calls, not an internal timer). Once
//! [`ReconnectPolicy::max_attempts`] is exhausted, every further
//! [`DialSupervisor::try_dial`] call is an `O(1)` no-op
//! ([`DialAttempt::Exhausted`]) — it does not call [`Dialer::dial`] again,
//! so a permanently-failing dialer cannot spin the attempt count or allocate
//! per call, however many times it is polled.

use std::collections::HashMap;
use std::sync::Arc;

use broadcast_common::{Stage, Timestamp};
use bytes::Bytes;
use transmux::{Sample, TrackSpec};

use crate::trunk::{RetentionClass, Trunk, TrunkConfig, TrunkWriter};

/// Identifies one program within one ingest connection — see
/// [the program dimension](self#the-program-dimension-b5-sessionevent-newprogram-at-any-time).
///
/// Meaningless outside the [`IngestSession`] that assigned it: two sessions
/// each reporting `ProgramId(1)` are two unrelated programs, each getting
/// its own [`Trunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramId(pub u32);

/// What an [`IngestSession`]'s [`Stage::poll`] hands back to a driver.
///
/// `#[non_exhaustive]`: a later step (egress track-set negotiation, §1.3's
/// upstream program-split) may need a `ProgramEnded`/`TrackSetChanged`
/// variant; this step only builds the two events a driver needs to route
/// samples into the right [`Trunk`], and does not add a variant it has no
/// correct producer for yet (this crate's own precedent —
/// [`crate::byte_merge`]'s `Hitless2022_7` note).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SessionEvent {
    /// A new program was discovered — mint a fresh [`Trunk`] for it. May be
    /// the very first event a session ever produces, or arrive after many
    /// samples for other programs already have — both are the same case;
    /// see [the module docs](self#the-program-dimension-b5-sessionevent-newprogram-at-any-time).
    NewProgram {
        /// Identifies this program for every subsequent
        /// [`SessionEvent::Sample`] carrying it.
        program: ProgramId,
        /// The demuxed track specs for this program, so far. Reuses
        /// [`transmux::TrackSpec`] rather than inventing a parallel type —
        /// this crate's established pattern (see `crate::trunk::SegmentEntry`'s
        /// module doc for the same reuse-don't-duplicate reasoning).
        tracks: Vec<TrackSpec>,
    },
    /// One decoded sample for `track_id`, belonging to `program` (must have
    /// been announced via a prior `NewProgram` with this `program` — a
    /// `Sample` for an unannounced program is a contract violation by the
    /// `IngestSession` implementor, and a driver drops it rather than
    /// panicking; see [`IngestDriver`]'s docs).
    Sample {
        /// Which program this sample belongs to.
        program: ProgramId,
        /// Track id within that program, matching
        /// [`TrunkWriter::publish`]'s own `track_id`.
        track_id: u32,
        /// Which ring this sample's track publishes into — the publisher
        /// (ultimately, the `IngestSession` implementor) decides this, same
        /// as any other [`TrunkWriter::publish`] caller.
        retention: RetentionClass,
        /// The decoded sample itself.
        sample: Sample,
    },
}

/// The sans-IO ingress drive contract: a blanket specialisation of
/// [`Stage`], matching [`crate::ByteStage`]'s own precedent — see
/// [the module docs](self#ingestsession-is-a-stage-matching-bytestages-precedent).
pub trait IngestSession: for<'a> Stage<In<'a> = &'a [u8], Out = SessionEvent> + Send {
    /// Outbound bytes this session wants written back to its peer (RTCP
    /// receiver reports, an RTSP keepalive `OPTIONS`, an SRT ACK) — most
    /// sessions need none. See
    /// [Why `poll_transmit` exists](self#why-poll_transmit-exists-and-most-sources-will-never-override-it).
    fn poll_transmit(&mut self) -> Option<Bytes> {
        None
    }
}

impl<T> IngestSession for T where T: for<'a> Stage<In<'a> = &'a [u8], Out = SessionEvent> + Send {}

/// Outbound connect: RTSP, raw RTP/UDP, TS-over-UDP, an SRT caller, an
/// HLS/DASH/Smooth pull client. See
/// [Dialer/Listener are a synchronous boundary](self#dialerlistener-are-a-synchronous-boundary-not-a-byte-level-sans-io-handshake-a-rejected-alternative-recorded-here).
pub trait Dialer: Send {
    /// The session a successful dial produces.
    type Session: IngestSession;
    /// Why a dial attempt failed.
    type Error;

    /// Perform one dial attempt (the whole connect/handshake), returning a
    /// live session or the reason it failed.
    fn dial(&mut self) -> Result<Self::Session, Self::Error>;
}

/// Identifies one session a [`ListenDriver`] currently has admitted, for
/// every call that needs to name which one ([`ListenDriver::feed`],
/// [`ListenDriver::health`], ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u64);

/// Inbound accept: RTMP push, an SRT listener, a future WHIP. See
/// [`max_sessions` is enforced by the driver](self#listener-and-max_sessions-enforced-by-the-driver-not-by-convention).
pub trait Listener: Send {
    /// The session one accepted connection produces.
    type Session: IngestSession;
    /// Why an accept attempt failed.
    type Error;

    /// The hard bound on concurrently admitted sessions — see the module
    /// docs. A fixed accessor, not a mutable knob: this step has no use case
    /// for changing it mid-flight, and a fixed value is what lets
    /// [`ListenDriver`] reason about it as a real bound rather than a
    /// point-in-time snapshot that might already be stale.
    fn max_sessions(&self) -> usize;

    /// Try to accept the next inbound connection. `Ok(None)` means nothing
    /// is waiting right now (a non-blocking poll, matching [`Stage::poll`]'s
    /// shape) — not an error and not end-of-input; a [`Listener`] has no
    /// "end" the way a single connection does.
    fn poll_accept(&mut self) -> Result<Option<Self::Session>, Self::Error>;
}

/// The outcome of driving one [`IngestSession`] to completion, distinguishing
/// a clean end from a real failure — see
/// [Supervision: EOF is not an error](self#supervision-eof-is-not-an-error-the-healthstate-fix).
///
/// `#[non_exhaustive]`: a later step may add `Connecting`/`Reconnecting` once
/// a driver owns a full redial loop rather than just the bounded initial
/// dial [`DialSupervisor`] covers in this step.
#[derive(Debug)]
#[non_exhaustive]
pub enum HealthState<E> {
    /// Actively driving; no end or error observed yet.
    Live,
    /// An [`IngestSession`]'s [`Stage::finish`] returned `Ok(())` with no
    /// prior error — the source ended on its own; this is not a failure.
    Ended,
    /// An [`IngestSession`]'s [`Stage::feed`] or [`Stage::finish`] returned
    /// `Err`. Carries the concrete error rather than a formatted string, so a
    /// caller that cares can match on it.
    Failed(E),
}

impl<E: PartialEq> PartialEq for HealthState<E> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (HealthState::Live, HealthState::Live) => true,
            (HealthState::Ended, HealthState::Ended) => true,
            (HealthState::Failed(a), HealthState::Failed(b)) => a == b,
            _ => false,
        }
    }
}

/// Drives one connected [`IngestSession`], dispatching every
/// [`SessionEvent`] it yields into a fresh per-[`ProgramId`] [`Trunk`], and
/// tracking [`HealthState`] — the "pump that owns the feed/poll/deadline
/// loop" for a single dialed-out connection (see [`run_dial`]). [`ListenDriver`]
/// embeds one of these per admitted session, so this is also where an
/// accepted connection's per-program `Trunk` bookkeeping actually lives.
pub struct IngestDriver<S: IngestSession> {
    session: S,
    trunk_config: TrunkConfig,
    programs: HashMap<ProgramId, Arc<Trunk>>,
    writers: HashMap<ProgramId, TrunkWriter>,
    health: HealthState<S::Error>,
}

impl<S: IngestSession> IngestDriver<S> {
    /// Wrap an already-connected `session`, ready to be fed. Every program
    /// it later announces gets a fresh [`Trunk`] built from `trunk_config`.
    pub fn new(session: S, trunk_config: TrunkConfig) -> Self {
        IngestDriver {
            session,
            trunk_config,
            programs: HashMap::new(),
            writers: HashMap::new(),
            health: HealthState::Live,
        }
    }

    /// Feed more bytes read from this session's connection. A no-op once
    /// [`Self::health`] is no longer [`HealthState::Live`] — a terminated
    /// session is never fed again.
    pub fn feed(&mut self, input: &[u8], now: Timestamp) {
        if !matches!(self.health, HealthState::Live) {
            return;
        }
        match self.session.feed(input, now) {
            Ok(()) => self.drain(),
            Err(e) => self.health = HealthState::Failed(e),
        }
    }

    /// Let the session act on the passage of time (rate-scheduled
    /// re-emission, a keepalive interval) — see [`Stage::on_deadline`]. A
    /// no-op once terminated, matching [`Self::feed`].
    pub fn on_deadline(&mut self, now: Timestamp) {
        if !matches!(self.health, HealthState::Live) {
            return;
        }
        self.session.on_deadline(now);
        self.drain();
    }

    /// Signal clean end-of-input. `Ok(())` from the session drives
    /// [`HealthState::Ended`] (not a failure); `Err` drives
    /// [`HealthState::Failed`] — this is the method the mutation-checked
    /// EOF-vs-failure test in this module drives directly. A no-op once
    /// already terminated.
    pub fn finish(&mut self) {
        if !matches!(self.health, HealthState::Live) {
            return;
        }
        match self.session.finish() {
            Ok(()) => {
                self.drain();
                self.health = HealthState::Ended;
            }
            Err(e) => self.health = HealthState::Failed(e),
        }
    }

    /// Drain outbound bytes the session wants sent to its peer — see
    /// [`IngestSession::poll_transmit`].
    pub fn poll_transmit(&mut self) -> Option<Bytes> {
        self.session.poll_transmit()
    }

    /// The next point in time this session has scheduled work at, if any.
    pub fn next_deadline(&self) -> Option<Timestamp> {
        self.session.next_deadline()
    }

    /// This session's current health.
    pub fn health(&self) -> &HealthState<S::Error> {
        &self.health
    }

    /// The [`Trunk`] for `program`, if it has been announced yet.
    pub fn trunk(&self, program: ProgramId) -> Option<&Arc<Trunk>> {
        self.programs.get(&program)
    }

    /// Every program this session has announced so far.
    pub fn programs(&self) -> impl Iterator<Item = ProgramId> + '_ {
        self.programs.keys().copied()
    }

    /// Drain every ready [`SessionEvent`], dispatching each into its
    /// program's `Trunk`. A `Sample` for a program never announced via
    /// `NewProgram` is dropped (documented `IngestSession` contract
    /// violation, not a panic — see [`SessionEvent::Sample`]'s docs).
    fn drain(&mut self) {
        while let Some(event) = self.session.poll() {
            match event {
                SessionEvent::NewProgram { program, .. } => {
                    let trunk = Trunk::new(self.trunk_config);
                    let writer = trunk
                        .writer()
                        .expect("a freshly constructed Trunk always has an unclaimed writer");
                    self.programs.insert(program, trunk);
                    self.writers.insert(program, writer);
                }
                SessionEvent::Sample {
                    program,
                    track_id,
                    retention,
                    sample,
                } => {
                    if let Some(writer) = self.writers.get(&program) {
                        writer.publish(track_id, retention, sample);
                    }
                }
            }
        }
    }
}

/// Dial once and wrap the result for driving — see [`IngestDriver`]. This is
/// the whole of `run_dial`'s "happy path"; [`DialSupervisor`] adds bounded
/// retry on top for a `Dialer` that fails outright.
pub fn run_dial<D: Dialer>(
    dialer: &mut D,
    trunk_config: TrunkConfig,
) -> Result<IngestDriver<D::Session>, D::Error> {
    let session = dialer.dial()?;
    Ok(IngestDriver::new(session, trunk_config))
}

/// Bounded retry policy for [`DialSupervisor`] — how many times
/// [`Dialer::dial`] is retried before giving up, never how long to wait
/// between attempts (that stays with the caller; see
/// [Reconnect](self#reconnect-caller-chosen-backoff-never-a-hardcoded-sleep)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReconnectPolicy {
    /// Maximum number of consecutive [`Dialer::dial`] attempts before
    /// [`DialSupervisor::try_dial`] gives up.
    pub max_attempts: u32,
}

impl ReconnectPolicy {
    /// Build a policy bounded to `max_attempts` consecutive dial failures.
    ///
    /// Panics if `max_attempts == 0` — a policy that never even tries once
    /// is a construction mistake, not a real bound.
    pub fn new(max_attempts: u32) -> Self {
        assert!(max_attempts > 0, "ReconnectPolicy max_attempts must be > 0");
        ReconnectPolicy { max_attempts }
    }
}

/// The result of one [`DialSupervisor::try_dial`] call.
///
/// Not `Debug`: [`DialAttempt::Connected`] carries an [`IngestDriver`], which
/// carries a `Trunk`/`TrunkWriter` — neither implements `Debug` (a `Trunk`
/// holds live synchronization primitives, not inspectable state), so this
/// type cannot either without breaking that up.
#[non_exhaustive]
pub enum DialAttempt<S: IngestSession, E> {
    /// This attempt succeeded — a live, driveable [`IngestDriver`].
    Connected(IngestDriver<S>),
    /// This attempt failed, but retries remain: wait (your own chosen
    /// backoff) and call [`DialSupervisor::try_dial`] again.
    Retry(E),
    /// This attempt failed and it was the last one allowed by
    /// [`ReconnectPolicy::max_attempts`] — carries the error that caused
    /// this final attempt to fail.
    GaveUp(E),
    /// [`DialAttempt::GaveUp`] already fired on an earlier call: no new
    /// dial was attempted this time, and none ever will be again from this
    /// [`DialSupervisor`] — see
    /// [Reconnect](self#reconnect-caller-chosen-backoff-never-a-hardcoded-sleep)
    /// for why this is what keeps a permanently-failing dialer from
    /// spinning.
    Exhausted,
}

/// Bounds [`Dialer::dial`] retry — see [`ReconnectPolicy`] and
/// [Reconnect](self#reconnect-caller-chosen-backoff-never-a-hardcoded-sleep).
pub struct DialSupervisor<D: Dialer> {
    dialer: D,
    policy: ReconnectPolicy,
    attempts: u32,
    exhausted: bool,
}

impl<D: Dialer> DialSupervisor<D> {
    /// Build a supervisor over `dialer`, bounded by `policy`.
    pub fn new(dialer: D, policy: ReconnectPolicy) -> Self {
        DialSupervisor {
            dialer,
            policy,
            attempts: 0,
            exhausted: false,
        }
    }

    /// Consecutive failed attempts so far. Reset to `0` on a successful
    /// dial; never exceeds [`ReconnectPolicy::max_attempts`], regardless of
    /// how many times [`Self::try_dial`] is called afterward — see
    /// [`DialAttempt::Exhausted`].
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// `true` once [`ReconnectPolicy::max_attempts`] has been exhausted —
    /// every subsequent [`Self::try_dial`] call returns
    /// [`DialAttempt::Exhausted`] without touching [`Dialer::dial`] again.
    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Try once more to dial, wrapping success into a driveable
    /// [`IngestDriver`] (every program it later announces gets a `Trunk`
    /// built from `trunk_config`). Never sleeps — see the module docs.
    pub fn try_dial(&mut self, trunk_config: TrunkConfig) -> DialAttempt<D::Session, D::Error> {
        if self.exhausted {
            return DialAttempt::Exhausted;
        }
        self.attempts += 1;
        match self.dialer.dial() {
            Ok(session) => {
                self.attempts = 0;
                DialAttempt::Connected(IngestDriver::new(session, trunk_config))
            }
            Err(e) => {
                if self.attempts >= self.policy.max_attempts {
                    self.exhausted = true;
                    DialAttempt::GaveUp(e)
                } else {
                    DialAttempt::Retry(e)
                }
            }
        }
    }
}

/// The outcome of one [`ListenDriver::poll_accept`] call.
#[derive(Debug)]
#[non_exhaustive]
pub enum AcceptOutcome<E> {
    /// A new connection was accepted and admitted under `max_sessions`; use
    /// this id with [`ListenDriver::feed`]/[`ListenDriver::health`]/etc.
    Admitted(SessionId),
    /// Nothing was waiting to be accepted right now — not an error.
    Idle,
    /// A connection was accepted, but `max_sessions` was already reached: it
    /// was dropped immediately, without ever being fed a byte — see
    /// [`max_sessions` is enforced by the driver](self#listener-and-max_sessions-enforced-by-the-driver-not-by-convention).
    Refused,
    /// [`Listener::poll_accept`] itself reported a failure (distinct from a
    /// refusal: the transport-level accept failed, not the admission bound).
    Error(E),
}

/// Drives a [`Listener`], admitting up to its [`Listener::max_sessions`]
/// concurrently and dispatching every admitted session's [`SessionEvent`]s
/// into per-[`ProgramId`] [`Trunk`]s exactly like [`IngestDriver`] (one is
/// embedded per admitted session). See
/// [`max_sessions` is enforced by the driver](self#listener-and-max_sessions-enforced-by-the-driver-not-by-convention).
pub struct ListenDriver<L: Listener> {
    listener: L,
    trunk_config: TrunkConfig,
    sessions: HashMap<SessionId, IngestDriver<L::Session>>,
    next_id: u64,
}

impl<L: Listener> ListenDriver<L> {
    /// Build a driver over `listener`. Every program any admitted session
    /// announces gets a `Trunk` built from `trunk_config`.
    pub fn new(listener: L, trunk_config: TrunkConfig) -> Self {
        ListenDriver {
            listener,
            trunk_config,
            sessions: HashMap::new(),
            next_id: 0,
        }
    }

    /// Currently-admitted (not yet terminated) session count. Never exceeds
    /// [`Listener::max_sessions`].
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// The bound this driver enforces, from the underlying [`Listener`].
    pub fn max_sessions(&self) -> usize {
        self.listener.max_sessions()
    }

    /// Try to accept one more connection, enforcing `max_sessions` (see the
    /// module docs). Calling this in a tight loop with nothing waiting, or
    /// with the bound already reached, is `O(1)` per call and never grows
    /// [`Self::session_count`] past [`Listener::max_sessions`] — flooding it
    /// is exactly the scenario this method exists to make safe.
    pub fn poll_accept(&mut self) -> AcceptOutcome<L::Error> {
        match self.listener.poll_accept() {
            Ok(None) => AcceptOutcome::Idle,
            Ok(Some(session)) => {
                if self.sessions.len() >= self.listener.max_sessions() {
                    // Dropped right here: `session` is never fed, never
                    // polled, never stored.
                    drop(session);
                    AcceptOutcome::Refused
                } else {
                    let id = SessionId(self.next_id);
                    self.next_id += 1;
                    self.sessions
                        .insert(id, IngestDriver::new(session, self.trunk_config));
                    AcceptOutcome::Admitted(id)
                }
            }
            Err(e) => AcceptOutcome::Error(e),
        }
    }

    /// Feed bytes read for session `id`. Returns `Some(health)` exactly when
    /// this call caused the session to terminate (`Ended` or `Failed`) — at
    /// which point it is removed from this driver (its slot is free for a
    /// future [`Self::poll_accept`]); returns `None` for an unknown `id` or
    /// a session that is still live after this call.
    pub fn feed(
        &mut self,
        id: SessionId,
        input: &[u8],
        now: Timestamp,
    ) -> Option<HealthState<<L::Session as Stage>::Error>> {
        self.drive(id, |d| d.feed(input, now))
    }

    /// Let session `id` act on the passage of time — see
    /// [`IngestDriver::on_deadline`]. Same removal-on-termination contract
    /// as [`Self::feed`].
    pub fn on_deadline(
        &mut self,
        id: SessionId,
        now: Timestamp,
    ) -> Option<HealthState<<L::Session as Stage>::Error>> {
        self.drive(id, |d| d.on_deadline(now))
    }

    /// Signal clean end-of-input for session `id` — see
    /// [`IngestDriver::finish`]. Same removal-on-termination contract as
    /// [`Self::feed`].
    pub fn finish(&mut self, id: SessionId) -> Option<HealthState<<L::Session as Stage>::Error>> {
        self.drive(id, IngestDriver::finish)
    }

    /// This session's current health, if it is still admitted (a terminated
    /// session is removed by the call that terminated it — see
    /// [`Self::feed`] — so query the return value of that call for the
    /// terminal state).
    pub fn health(&self, id: SessionId) -> Option<&HealthState<<L::Session as Stage>::Error>> {
        self.sessions.get(&id).map(IngestDriver::health)
    }

    /// The `Trunk` for `program` under session `id`, if announced yet.
    pub fn trunk(&self, id: SessionId, program: ProgramId) -> Option<&Arc<Trunk>> {
        self.sessions.get(&id).and_then(|d| d.trunk(program))
    }

    /// Runs `op` against session `id`'s driver, then — if that call left it
    /// no longer [`HealthState::Live`] — removes it and returns its final
    /// state. This is the one place a session leaves `self.sessions`, which
    /// is what keeps this driver's resident memory bounded to
    /// [`Listener::max_sessions`] rather than accumulating every session
    /// that has ever ended.
    fn drive(
        &mut self,
        id: SessionId,
        op: impl FnOnce(&mut IngestDriver<L::Session>),
    ) -> Option<HealthState<<L::Session as Stage>::Error>> {
        let driver = self.sessions.get_mut(&id)?;
        op(driver);
        if matches!(driver.health(), HealthState::Live) {
            None
        } else {
            self.sessions.remove(&id).map(|d| d.health)
        }
    }
}

/// Build a [`ListenDriver`] over `listener` — the whole of `run_listen`.
pub fn run_listen<L: Listener>(listener: L, trunk_config: TrunkConfig) -> ListenDriver<L> {
    ListenDriver::new(listener, trunk_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Demand;
    use std::collections::VecDeque;
    use transmux::pipeline::{CodecConfig, DataCarriage};

    /// Minimal config every test's `Trunk`s share — capacities are irrelevant
    /// to these tests beyond "large enough that nothing evicts mid-test".
    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(64, 16, 8, 8)
    }

    fn sample(byte: u8) -> Sample {
        Sample::new(Bytes::from(vec![byte; 4]), Some(0), Some(0), Some(1), true)
    }

    fn opaque_track(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Data {
                stream_type: 0x06,
                descriptors: Vec::new(),
                carriage: DataCarriage::Pes,
            },
        )
    }

    /// A fake, `#[cfg(test)]`-only error type for scripted sessions/dialers —
    /// carries a reason string purely for assertion messages.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeError(&'static str);

    /// What one `feed()` call on a [`ScriptedSession`] does.
    enum FeedOutcome {
        /// Succeed, queuing these events for `poll()` to hand back.
        Events(Vec<SessionEvent>),
        /// Fail outright with this error.
        Err(FakeError),
    }

    /// A fully scripted [`IngestSession`]: each `feed()` call consumes the
    /// next entry of `script`, either queuing its events or failing; `finish`
    /// hands back `finish_outcome` (defaults to a clean `Ok(())`).
    struct ScriptedSession {
        script: VecDeque<FeedOutcome>,
        pending: VecDeque<SessionEvent>,
        finish_outcome: Result<(), FakeError>,
    }

    impl ScriptedSession {
        fn new(script: Vec<FeedOutcome>) -> Self {
            ScriptedSession {
                script: script.into(),
                pending: VecDeque::new(),
                finish_outcome: Ok(()),
            }
        }

        fn failing_finish(mut self, err: FakeError) -> Self {
            self.finish_outcome = Err(err);
            self
        }
    }

    impl Stage for ScriptedSession {
        type In<'a> = &'a [u8];
        type Out = SessionEvent;
        type Error = FakeError;

        fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), FakeError> {
            match self.script.pop_front() {
                Some(FeedOutcome::Events(evs)) => {
                    self.pending.extend(evs);
                    Ok(())
                }
                Some(FeedOutcome::Err(e)) => Err(e),
                None => Ok(()),
            }
        }

        fn poll(&mut self) -> Option<SessionEvent> {
            self.pending.pop_front()
        }

        fn finish(&mut self) -> Result<(), FakeError> {
            self.finish_outcome.clone()
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn demand(&self) -> Demand {
            Demand::new(4096)
        }
    }

    /// A fake [`Dialer`] yielding one pre-built session then erroring on
    /// every call after (or always erroring, for the reconnect test).
    struct ScriptedDialer {
        sessions: VecDeque<ScriptedSession>,
        fail_with: FakeError,
    }

    impl Dialer for ScriptedDialer {
        type Session = ScriptedSession;
        type Error = FakeError;

        fn dial(&mut self) -> Result<ScriptedSession, FakeError> {
            self.sessions
                .pop_front()
                .ok_or_else(|| self.fail_with.clone())
        }
    }

    // --- run_dial: happy path, samples land in the Trunk ------------------

    #[test]
    fn run_dial_drives_fake_session_end_to_end_samples_land_in_trunk() {
        let session = ScriptedSession::new(vec![
            FeedOutcome::Events(vec![SessionEvent::NewProgram {
                program: ProgramId(1),
                tracks: vec![opaque_track(7)],
            }]),
            FeedOutcome::Events(vec![SessionEvent::Sample {
                program: ProgramId(1),
                track_id: 7,
                retention: RetentionClass::Timed,
                sample: sample(0xAB),
            }]),
        ]);
        let mut dialer = ScriptedDialer {
            sessions: VecDeque::from(vec![session]),
            fail_with: FakeError("unused"),
        };

        let mut driver = run_dial(&mut dialer, trunk_config()).expect("fake dial succeeds");
        let trunk_before = driver.trunk(ProgramId(1)).cloned();
        assert!(
            trunk_before.is_none(),
            "no Trunk before NewProgram is announced"
        );

        driver.feed(b"pat", Timestamp::ZERO);
        let trunk = driver
            .trunk(ProgramId(1))
            .cloned()
            .expect("NewProgram announced a Trunk for program 1");
        let mut cursor = trunk.subscribe();

        driver.feed(b"pes", Timestamp::from_nanos(1));

        let item = cursor.poll().expect("the published sample is on the ring");
        match item {
            crate::SampleCursorItem::Timed { track_id, sample } => {
                assert_eq!(track_id, 7);
                assert_eq!(sample.data.as_ref(), &[0xAB; 4]);
            }
            other => panic!("expected Timed, got {other:?}"),
        }
    }

    // --- EOF vs failure: the test that makes HealthState::Failed real -----

    #[test]
    fn clean_finish_yields_ended_not_failed() {
        let session = ScriptedSession::new(vec![]);
        let mut driver = IngestDriver::new(session, trunk_config());
        assert!(matches!(driver.health(), HealthState::Live));

        driver.finish();

        // MUTATION-CHECKED: flip this to `Failed` in the impl (or make
        // `finish()`'s `Ok` arm also set `Failed`) and this assertion is the
        // one that catches it.
        assert!(
            matches!(driver.health(), HealthState::Ended),
            "a session that finished cleanly must be Ended, not Failed: {:?}",
            driver.health()
        );
    }

    #[test]
    fn erroring_feed_yields_failed_not_ended() {
        let session = ScriptedSession::new(vec![FeedOutcome::Err(FakeError("bad continuity"))]);
        let mut driver = IngestDriver::new(session, trunk_config());

        driver.feed(b"garbage", Timestamp::ZERO);

        match driver.health() {
            HealthState::Failed(FakeError(reason)) => assert_eq!(*reason, "bad continuity"),
            other => panic!("expected Failed(\"bad continuity\"), got {other:?}"),
        }
    }

    #[test]
    fn erroring_finish_yields_failed_not_ended() {
        let session = ScriptedSession::new(vec![]).failing_finish(FakeError("truncated tail"));
        let mut driver = IngestDriver::new(session, trunk_config());

        driver.finish();

        match driver.health() {
            HealthState::Failed(FakeError(reason)) => assert_eq!(*reason, "truncated tail"),
            other => panic!("expected Failed(\"truncated tail\"), got {other:?}"),
        }
    }

    #[test]
    fn terminated_driver_ignores_further_feed_and_finish() {
        let session = ScriptedSession::new(vec![FeedOutcome::Err(FakeError("boom"))]);
        let mut driver = IngestDriver::new(session, trunk_config());
        driver.feed(b"x", Timestamp::ZERO);
        assert!(matches!(driver.health(), HealthState::Failed(_)));

        // Once Failed, feed/finish must be no-ops: no panic, and health does
        // not flip back to Ended via a stray finish() call.
        driver.finish();
        assert!(matches!(driver.health(), HealthState::Failed(_)));
    }

    // --- Multi-program (B5): two programs -> two Trunks; late program -----

    #[test]
    fn one_connection_two_programs_yields_two_trunks_including_one_announced_late() {
        let session = ScriptedSession::new(vec![
            FeedOutcome::Events(vec![
                SessionEvent::NewProgram {
                    program: ProgramId(1),
                    tracks: vec![opaque_track(1)],
                },
                SessionEvent::Sample {
                    program: ProgramId(1),
                    track_id: 1,
                    retention: RetentionClass::Timed,
                    sample: sample(0x01),
                },
            ]),
            // Nothing new this round: proves NewProgram isn't required on
            // every feed call.
            FeedOutcome::Events(vec![]),
            // Program 2 appears only now, after program 1's samples were
            // already flowing — the exact "announced after ingest started"
            // case B5 requires.
            FeedOutcome::Events(vec![
                SessionEvent::NewProgram {
                    program: ProgramId(2),
                    tracks: vec![opaque_track(9)],
                },
                SessionEvent::Sample {
                    program: ProgramId(2),
                    track_id: 9,
                    retention: RetentionClass::Timed,
                    sample: sample(0x02),
                },
            ]),
        ]);
        let mut dialer = ScriptedDialer {
            sessions: VecDeque::from(vec![session]),
            fail_with: FakeError("unused"),
        };
        let mut driver = run_dial(&mut dialer, trunk_config()).unwrap();

        driver.feed(b"1", Timestamp::from_nanos(0));
        assert!(driver.trunk(ProgramId(1)).is_some());
        assert!(
            driver.trunk(ProgramId(2)).is_none(),
            "program 2 must not exist before it is announced"
        );

        driver.feed(b"2", Timestamp::from_nanos(1));
        assert!(
            driver.trunk(ProgramId(2)).is_none(),
            "a no-op feed must not fabricate a program"
        );

        driver.feed(b"3", Timestamp::from_nanos(2));
        let trunk1 = driver.trunk(ProgramId(1)).cloned().unwrap();
        let trunk2 = driver
            .trunk(ProgramId(2))
            .cloned()
            .expect("program 2 announced mid-session must get its own Trunk");
        assert!(
            !Arc::ptr_eq(&trunk1, &trunk2),
            "each program must get a genuinely distinct Trunk"
        );

        let mut programs: Vec<_> = driver.programs().collect();
        programs.sort();
        assert_eq!(programs, vec![ProgramId(1), ProgramId(2)]);

        // Both Trunks actually carry their own program's sample, subscribed
        // fresh now (after the fact) — proving the two rings are genuinely
        // independent, not aliases of the same one.
        assert_eq!(trunk1.timed_len(), 1);
        assert_eq!(trunk2.timed_len(), 1);
    }

    // --- run_listen: max_sessions is a hard bound --------------------------

    /// A [`Listener`] that always has a fresh session ready to accept —
    /// models an unbounded flood of inbound connections.
    struct FloodingListener {
        max_sessions: usize,
    }

    impl Listener for FloodingListener {
        type Session = ScriptedSession;
        type Error = FakeError;

        fn max_sessions(&self) -> usize {
            self.max_sessions
        }

        fn poll_accept(&mut self) -> Result<Option<ScriptedSession>, FakeError> {
            Ok(Some(ScriptedSession::new(vec![])))
        }
    }

    #[test]
    fn run_listen_admits_up_to_max_sessions_then_refuses_and_stays_bounded() {
        let max_sessions = 3;
        let mut driver = run_listen(FloodingListener { max_sessions }, trunk_config());

        for _ in 0..max_sessions {
            assert!(matches!(driver.poll_accept(), AcceptOutcome::Admitted(_)));
        }
        assert_eq!(driver.session_count(), max_sessions);

        // Flood far beyond the bound: every one of these must be refused,
        // and — the memory-growth assertion — session_count must never
        // exceed max_sessions, checked on every single iteration, not just
        // at the end.
        for _ in 0..10_000 {
            assert!(matches!(driver.poll_accept(), AcceptOutcome::Refused));
            assert!(
                driver.session_count() <= max_sessions,
                "session_count grew past max_sessions under flood"
            );
        }
        assert_eq!(driver.session_count(), max_sessions);
    }

    #[test]
    fn ended_session_is_reaped_freeing_a_slot() {
        let mut driver = run_listen(FloodingListener { max_sessions: 1 }, trunk_config());
        let AcceptOutcome::Admitted(id) = driver.poll_accept() else {
            panic!("expected admission");
        };
        assert_eq!(driver.session_count(), 1);
        assert!(matches!(driver.poll_accept(), AcceptOutcome::Refused));

        let health = driver.finish(id).expect("finish terminates the session");
        assert!(matches!(health, HealthState::Ended));
        assert_eq!(
            driver.session_count(),
            0,
            "a terminated session must be reaped, freeing its slot"
        );
        assert!(
            driver.health(id).is_none(),
            "a reaped session is no longer queryable by id"
        );

        // The freed slot admits a new connection.
        assert!(matches!(driver.poll_accept(), AcceptOutcome::Admitted(_)));
    }

    // --- Reconnect: bounded, caller-configurable, never spins --------------

    #[test]
    fn permanently_failing_dial_is_bounded_and_does_not_spin_or_grow() {
        let dialer = ScriptedDialer {
            sessions: VecDeque::new(),
            fail_with: FakeError("connection refused"),
        };
        let mut supervisor = DialSupervisor::new(dialer, ReconnectPolicy::new(3));

        assert!(matches!(
            supervisor.try_dial(trunk_config()),
            DialAttempt::Retry(_)
        ));
        assert_eq!(supervisor.attempts(), 1);
        assert!(matches!(
            supervisor.try_dial(trunk_config()),
            DialAttempt::Retry(_)
        ));
        assert_eq!(supervisor.attempts(), 2);
        assert!(matches!(
            supervisor.try_dial(trunk_config()),
            DialAttempt::GaveUp(_)
        ));
        assert_eq!(supervisor.attempts(), 3);
        assert!(supervisor.is_exhausted());

        // Flood: however many more times this is called, it must never dial
        // again (no growth in `attempts`) and must always report Exhausted,
        // not spin back into Retry/GaveUp.
        for _ in 0..10_000 {
            assert!(matches!(
                supervisor.try_dial(trunk_config()),
                DialAttempt::Exhausted
            ));
            assert_eq!(
                supervisor.attempts(),
                3,
                "attempts must not grow past max_attempts under flood"
            );
        }
    }

    #[test]
    fn dial_supervisor_succeeds_within_the_bound_and_resets_attempts() {
        let good_session = ScriptedSession::new(vec![]);
        let dialer = ScriptedDialer {
            sessions: VecDeque::from(vec![good_session]),
            fail_with: FakeError("refused"),
        };
        let mut supervisor = DialSupervisor::new(dialer, ReconnectPolicy::new(2));

        // First attempt succeeds immediately (the scripted dialer's one
        // queued session comes out on the very first `dial()` call).
        match supervisor.try_dial(trunk_config()) {
            DialAttempt::Connected(_) => {}
            DialAttempt::Retry(_) => panic!("expected Connected, got Retry"),
            DialAttempt::GaveUp(_) => panic!("expected Connected, got GaveUp"),
            DialAttempt::Exhausted => panic!("expected Connected, got Exhausted"),
        }
        assert_eq!(supervisor.attempts(), 0);
        assert!(!supervisor.is_exhausted());
    }
}
