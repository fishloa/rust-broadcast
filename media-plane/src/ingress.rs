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
//! # Establishment is ordinary driving — `dial()` performs no I/O
//!
//! **[`Dialer::dial`] does not connect anything.** It *constructs* a session
//! in a not-yet-established state, along with whatever first bytes that
//! session wants sent (queued for [`IngestSession::poll_transmit`]). The
//! handshake then completes through the **same feed/poll pump as everything
//! else**: the driver writes [`IngestSession::poll_transmit`]'s bytes to the
//! socket, reads the peer's reply, hands it to [`Stage::feed`], and the
//! session either queues the next request or announces
//! [`SessionEvent::Established`]. No I/O happens inside any trait method, so
//! the plane stays genuinely sans-IO and tokio stays out of this layer.
//!
//! This is deliberately **the same pattern `rtsp-runtime` already uses**, not
//! a second invention: `rtsp_runtime::client::ClientSession` is a sans-IO
//! engine whose request builders (`describe`/`setup`/`play`) *return the
//! outbound bytes to send* and whose `handle_data` consumes inbound bytes and
//! returns typed `ClientEvent`s, with the RFC 2326 Appendix A.1 state machine
//! held internally and exposed via `state()`. `IngestSession` is that shape
//! expressed through `Stage`: `poll_transmit` ≙ "the bytes to send",
//! `feed` ≙ `handle_data`, `poll` ≙ the returned events, and
//! [`IngestDriver::health`] ≙ `state()`. `ll-hls-runtime` splits its client
//! and server engines the same way.
//!
//! An earlier revision of this module had `dial()` "perform the whole
//! connect/handshake" and return an already-live session. That was wrong and
//! is recorded here rather than quietly changed: a sans-IO trait cannot do
//! I/O, so such a `dial()` only ever fits sources whose "connect" is a purely
//! local operation (binding a UDP socket). Every genuinely multi-round-trip
//! source — RTSP (DESCRIBE → SETUP × N → PLAY), an SRT caller handshake,
//! TS-UDP's read-until-PMT-resolves — would have needed an executor bridge
//! (`block_on`, or a handshake thread) to be callable at all, dragging tokio
//! back into the layer that was kept free of it on purpose.
//!
//! ## One session type, not a separate `PendingSession`
//!
//! A distinct `PendingSession` type that `poll()`s into an `IngestSession`
//! was considered and rejected. It would have to duplicate
//! `feed`/`poll_transmit`/`next_deadline`/`on_deadline` (a handshake needs
//! every one of them — that is the whole point), doubling the trait surface
//! for one bit of state; it would force each driver to hold a
//! `Pending | Established` enum and re-dispatch every call through it; and
//! `rtsp-runtime` — the in-repo precedent this mirrors — deliberately does
//! *not* do it either: `ClientSession` is one type from `Init` through
//! `Playing`, with the phase readable via `state()`. One type, with the phase
//! visible as [`HealthState::Establishing`] vs [`HealthState::Live`], keeps
//! establishment on exactly the code path everything else already uses, which
//! was the goal.
//!
//! # The handshake is bounded by a caller-supplied deadline
//!
//! A peer that opens a connection and then goes quiet must not pin a session
//! forever. [`HandshakePolicy::establish_by`] is an **absolute
//! [`Timestamp`]** by which [`SessionEvent::Established`] must have arrived;
//! past it, a still-establishing session terminates as
//! [`HealthState::HandshakeTimedOut`] — and in a [`ListenDriver`] is reaped,
//! freeing its `max_sessions` slot, so a flood of half-open connections
//! cannot squat the bound.
//!
//! **Why a deadline rather than an attempt or pump-iteration cap** (the two
//! alternatives, both rejected): the failure mode is wall-clock — "the peer
//! stopped talking" — which is exactly what `multimux`'s already-proven
//! `IngestTimeouts::connect` (`DEFAULT_CONNECT_TIMEOUT`, 10 s) bounds today,
//! so this matches a shape known to work on real cameras and encoders. An
//! iteration cap would be a proxy that misfires in both directions: a real
//! RTSP handshake is DESCRIBE + SETUP × N + PLAY where **N comes from the
//! SDP and is not knowable when the cap would have to be chosen**, so any
//! fixed number is either too small for an 8-track presentation (breaking a
//! legitimate handshake) or too large to bound a stalled one usefully. The
//! deadline also costs no new parameter — [`Timestamp`] is already threaded
//! through [`Stage::feed`]/[`Stage::on_deadline`] — and [`IngestDriver::next_deadline`]
//! surfaces it, so a real driver knows when to fire the check without
//! polling. Per this crate's sans-IO rule there is no internal timer: the
//! deadline is only observed on a `feed`/`on_deadline` the caller makes,
//! exactly like [`crate::byte_merge::MergePolicy::Failover`]'s
//! `silence_timeout`.
//!
//! *Memory* during a handshake is bounded separately and deliberately not
//! here: it is the session's own `demand()`/internal-buffer bound (the
//! `FixedFramer` precedent in [`crate::byte_stage`]'s tests), because only
//! the session knows how much partial handshake state it is legitimately
//! holding.
//!
//! # Known seam: pull sources (HLS/DASH/Smooth) are request-driven, not
//! stream-driven
//!
//! Recorded, not solved — it is a Step 5 decision and belongs in front of a
//! human. `multimux`'s `hls_pull`/`dash_pull`/`smooth_pull` sources are not
//! continuous byte streams: they fetch a playlist/manifest on a reload timer,
//! compute segment URLs, and GET whole objects. Two observations:
//!
//! - **`feed(&[u8])` itself is *not* the problem.** `Stage`'s contract says
//!   nothing about chunk size and explicitly decouples `poll` from `feed`, so
//!   handing one whole downloaded segment body to `feed` is entirely within
//!   contract — no different from a 1316-byte UDP datagram except in size.
//! - **`poll_transmit() -> Option<Bytes>` *is* the real gap.** It expresses
//!   "send these bytes on the connection you already have", which is right for
//!   RTSP/RTMP/SRT but cannot express "issue a GET for *this URL*". A pull
//!   source's scheduling need is otherwise already covered by the machinery
//!   above: [`Stage::next_deadline`]/[`Stage::on_deadline`] *is* a playlist
//!   reload timer, and the establishment pump *is* a request/response loop.
//!
//! So the missing piece looks like a request-*addressing* type (a
//! `Transmit`-style enum carrying either raw bytes or a URL + method), not a
//! second drive model or a pull-shaped sibling trait. Adding one now would be
//! speculative — no caller exists until Step 5 ports those three sources —
//! so this module states the seam and stops.
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
/// variant; this step only builds the three events a driver needs to tell
/// handshaking from live and to route samples into the right [`Trunk`], and
/// does not add a variant it has no correct producer for yet (this crate's
/// own precedent — [`crate::byte_merge`]'s `Hitless2022_7` note).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SessionEvent {
    /// The handshake finished: this session's transport is usable and it is
    /// now live. Exactly one of these per session lifetime — see
    /// [Establishment is ordinary driving](self#establishment-is-ordinary-driving--dial-performs-no-io).
    /// Until it arrives the driver reports [`HealthState::Establishing`];
    /// after it, [`HealthState::Live`].
    ///
    /// # Why not called `TracksResolved`
    ///
    /// `transmux::DemuxEvent::TracksResolved { generation }` already owns that
    /// name for the analogous *demux-side* question, and this is deliberately
    /// not a synonym for it: **tracks here are a per-program fact** carried by
    /// [`SessionEvent::NewProgram`] (finding B5 — one connection, N programs),
    /// whereas establishment is a per-*connection* fact. Folding tracks into
    /// this variant would force a session ingesting an MPTS to nominate some
    /// arbitrary program as "the one that resolved the connection", and would
    /// leave a session that has finished its handshake but not yet seen a PAT
    /// with no way to say so. It carries no payload for the same
    /// "no field without a correct producer" reason.
    ///
    /// The two events do line up on the awkward case, though, and this is
    /// where that lands: `DemuxEvent::TracksResolved`'s own docs note that a
    /// container with no up-front track declaration (FLV/RTMP) legitimately
    /// never emits it, and that the asymmetry "is for the media plane's
    /// ingress layer to handle explicitly (e.g. gating on the first
    /// `DemuxEvent::Sample`)". This variant is that explicit handling: an
    /// RTMP session decides for itself when it is ready — gating on the first
    /// sample, exactly as those docs suggest — and says so here.
    Established,
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

/// The sans-IO ingress drive contract: a specialisation of [`Stage`] whose
/// input is a borrowed byte slice and whose output is [`SessionEvent`] — see
/// [the module docs](self#ingestsession-is-a-stage-matching-bytestages-precedent).
///
/// # Why this is explicitly implemented, unlike [`crate::ByteStage`]
///
/// `ByteStage` gets a blanket `impl<T> ByteStage for T where T: Stage<…>`
/// because it adds **nothing** to `Stage` — it is a pure alias, so a blanket
/// impl costs nothing and saves every implementor a line. `IngestSession`
/// adds [`poll_transmit`](Self::poll_transmit), which has a default. A blanket
/// impl here would make that default **impossible to override** (the blanket
/// would already be the one impl for every type, and a second manual impl
/// would collide), quietly breaking the handshake mechanism it exists for.
/// So implementors write one extra line — `impl IngestSession for MySession {}`
/// to accept the default, or a real body to send bytes. This asymmetry is
/// deliberate and is the reason it is documented rather than "fixed".
pub trait IngestSession: for<'a> Stage<In<'a> = &'a [u8], Out = SessionEvent> + Send {
    /// Outbound bytes this session wants written to its peer.
    ///
    /// Two uses, one mechanism: the **handshake** requests that establish the
    /// session (an RTSP `DESCRIBE`, then `SETUP`, then `PLAY` — see
    /// [Establishment is ordinary driving](self#establishment-is-ordinary-driving--dial-performs-no-io)),
    /// and in-session traffic afterwards (RTCP receiver reports, an RTSP
    /// keepalive `OPTIONS`, an SRT ACK). This is exactly what
    /// `rtsp_runtime::client::ClientSession`'s request builders return, only
    /// pulled rather than returned.
    ///
    /// A driver drains this in a loop after every
    /// [`Stage::feed`]/[`Stage::on_deadline`] (and once immediately after
    /// [`Dialer::dial`], to send the first handshake request), exactly like
    /// [`Stage::poll`]. A session with nothing to send never overrides it.
    fn poll_transmit(&mut self) -> Option<Bytes> {
        None
    }
}

/// Outbound connect: RTSP, raw RTP/UDP, TS-over-UDP, an SRT caller, an
/// HLS/DASH/Smooth pull client.
pub trait Dialer: Send {
    /// The session a dial produces.
    type Session: IngestSession;
    /// Why constructing the session failed.
    type Error;

    /// **Construct** a session — performing no I/O and completing no
    /// handshake.
    ///
    /// The returned session is *not yet established*: it starts in
    /// [`HealthState::Establishing`], and the handshake completes through the
    /// ordinary pump ([`IngestSession::poll_transmit`] out,
    /// [`Stage::feed`] in) until it emits [`SessionEvent::Established`] — see
    /// [Establishment is ordinary driving](self#establishment-is-ordinary-driving--dial-performs-no-io).
    /// An implementation should queue its first handshake request for
    /// `poll_transmit` here.
    ///
    /// The `Err` path is for purely local construction failures — a URL that
    /// will not parse, contradictory config — **not** for connect failures,
    /// which this method never attempts and therefore cannot observe. A peer
    /// that refuses or never answers surfaces later, as
    /// [`HealthState::Failed`] or [`HealthState::HandshakeTimedOut`].
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

/// A session's phase, distinguishing "still handshaking" from "live", and a
/// clean end from a real failure — see
/// [Supervision: EOF is not an error](self#supervision-eof-is-not-an-error-the-healthstate-fix).
///
/// [`Establishing`](Self::Establishing) and [`Live`](Self::Live) are the two
/// running states; the other three are terminal (in a [`ListenDriver`],
/// reaching any of them reaps the session and frees its `max_sessions` slot).
///
/// `#[non_exhaustive]`: a later step may add `Reconnecting` once a driver owns
/// a full redial loop rather than just the bounded initial dial
/// [`DialSupervisor`] covers in this step.
#[derive(Debug)]
#[non_exhaustive]
pub enum HealthState<E> {
    /// Constructed by [`Dialer::dial`] (or accepted by a [`Listener`]) but
    /// the handshake has not finished: the session has not yet emitted
    /// [`SessionEvent::Established`]. Bounded by
    /// [`HandshakePolicy::establish_by`] — see
    /// [the handshake is bounded](self#the-handshake-is-bounded-by-a-caller-supplied-deadline).
    Establishing,
    /// Established and actively driving; no end or error observed yet.
    Live,
    /// An [`IngestSession`]'s [`Stage::finish`] returned `Ok(())` with no
    /// prior error — the source ended on its own; this is not a failure.
    Ended,
    /// An [`IngestSession`]'s [`Stage::feed`] or [`Stage::finish`] returned
    /// `Err`. Carries the concrete error rather than a formatted string, so a
    /// caller that cares can match on it.
    Failed(E),
    /// [`HandshakePolicy::establish_by`] passed while the session was still
    /// [`Establishing`](Self::Establishing) — the peer opened a connection and
    /// never completed the handshake.
    ///
    /// Deliberately **not** folded into [`Failed`](Self::Failed): that variant
    /// carries the *session's* own error type, and a handshake that simply
    /// never progressed produced no session error to carry — the session did
    /// nothing wrong, it was starved of input. Inventing an `E` to put here
    /// would mean either fabricating one or forcing every implementor's error
    /// type to grow a timeout variant it cannot itself raise.
    HandshakeTimedOut {
        /// The deadline that passed.
        deadline: Timestamp,
    },
}

impl<E: PartialEq> PartialEq for HealthState<E> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (HealthState::Establishing, HealthState::Establishing) => true,
            (HealthState::Live, HealthState::Live) => true,
            (HealthState::Ended, HealthState::Ended) => true,
            (HealthState::Failed(a), HealthState::Failed(b)) => a == b,
            (
                HealthState::HandshakeTimedOut { deadline: a },
                HealthState::HandshakeTimedOut { deadline: b },
            ) => a == b,
            _ => false,
        }
    }
}

impl<E> HealthState<E> {
    /// `true` while this session is still being driven —
    /// [`Establishing`](Self::Establishing) or [`Live`](Self::Live). `false`
    /// once it has reached a terminal state.
    pub fn is_running(&self) -> bool {
        matches!(self, HealthState::Establishing | HealthState::Live)
    }
}

/// Bounds how long a session may stay in [`HealthState::Establishing`] — see
/// [the handshake is bounded](self#the-handshake-is-bounded-by-a-caller-supplied-deadline)
/// for why this is a wall-clock deadline rather than an attempt or
/// pump-iteration cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct HandshakePolicy {
    /// Absolute [`Timestamp`], on the same driver-chosen epoch as
    /// [`Stage::feed`]'s `now`, by which [`SessionEvent::Established`] must
    /// have arrived.
    pub establish_by: Timestamp,
}

impl HandshakePolicy {
    /// Require the handshake to complete by the absolute timestamp
    /// `establish_by`.
    pub fn establish_by(establish_by: Timestamp) -> Self {
        HandshakePolicy { establish_by }
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
    handshake: HandshakePolicy,
    programs: HashMap<ProgramId, Arc<Trunk>>,
    writers: HashMap<ProgramId, TrunkWriter>,
    health: HealthState<S::Error>,
}

impl<S: IngestSession> IngestDriver<S> {
    /// Wrap a freshly-constructed (**not yet established**) `session`, ready
    /// to be pumped: it starts in [`HealthState::Establishing`] and reaches
    /// [`HealthState::Live`] when it emits [`SessionEvent::Established`],
    /// bounded by `handshake`. Every program it later announces gets a fresh
    /// [`Trunk`] built from `trunk_config`.
    pub fn new(session: S, trunk_config: TrunkConfig, handshake: HandshakePolicy) -> Self {
        IngestDriver {
            session,
            trunk_config,
            handshake,
            programs: HashMap::new(),
            writers: HashMap::new(),
            health: HealthState::Establishing,
        }
    }

    /// Feed more bytes read from this session's connection — a handshake
    /// response while [`HealthState::Establishing`], media once
    /// [`HealthState::Live`]; the same call either way. A no-op once the
    /// session has reached a terminal state — it is never fed again.
    pub fn feed(&mut self, input: &[u8], now: Timestamp) {
        if !self.health.is_running() {
            return;
        }
        match self.session.feed(input, now) {
            Ok(()) => {
                self.drain();
                self.check_handshake_deadline(now);
            }
            Err(e) => self.health = HealthState::Failed(e),
        }
    }

    /// Let the session act on the passage of time (an in-flight handshake
    /// retransmit, rate-scheduled re-emission, a keepalive interval) — see
    /// [`Stage::on_deadline`]. Also where a blown
    /// [`HandshakePolicy::establish_by`] is observed for a peer that has gone
    /// silent mid-handshake and so is producing no `feed` calls at all. A
    /// no-op once terminated, matching [`Self::feed`].
    pub fn on_deadline(&mut self, now: Timestamp) {
        if !self.health.is_running() {
            return;
        }
        self.session.on_deadline(now);
        self.drain();
        self.check_handshake_deadline(now);
    }

    /// Signal clean end-of-input. `Ok(())` from the session drives
    /// [`HealthState::Ended`] (not a failure); `Err` drives
    /// [`HealthState::Failed`] — this is the method the mutation-checked
    /// EOF-vs-failure test in this module drives directly. A no-op once
    /// already terminated.
    pub fn finish(&mut self) {
        if !self.health.is_running() {
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

    /// Drain outbound bytes the session wants sent to its peer — handshake
    /// requests included. See [`IngestSession::poll_transmit`].
    pub fn poll_transmit(&mut self) -> Option<Bytes> {
        self.session.poll_transmit()
    }

    /// The next point in time this driver has work to do: the earlier of the
    /// session's own [`Stage::next_deadline`] and — while still
    /// [`HealthState::Establishing`] — [`HandshakePolicy::establish_by`], so a
    /// caller driving off this value alone still learns about a stalled
    /// handshake at the right moment rather than never.
    pub fn next_deadline(&self) -> Option<Timestamp> {
        let session = self.session.next_deadline();
        let handshake =
            matches!(self.health, HealthState::Establishing).then_some(self.handshake.establish_by);
        match (session, handshake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// This session's current health.
    pub fn health(&self) -> &HealthState<S::Error> {
        &self.health
    }

    /// Terminate a still-`Establishing` session whose deadline has passed.
    ///
    /// Called *after* the session has been fed/drained, never before, so a
    /// handshake response that arrives exactly at the deadline and completes
    /// the handshake still establishes rather than being rejected by a
    /// millisecond.
    fn check_handshake_deadline(&mut self, now: Timestamp) {
        if matches!(self.health, HealthState::Establishing) && now >= self.handshake.establish_by {
            self.health = HealthState::HandshakeTimedOut {
                deadline: self.handshake.establish_by,
            };
        }
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
                SessionEvent::Established => {
                    // Only ever a promotion out of Establishing: a duplicate
                    // Established from a misbehaving session must not resurrect
                    // an already-terminal driver, and must not be treated as a
                    // fresh start for a live one.
                    if matches!(self.health, HealthState::Establishing) {
                        self.health = HealthState::Live;
                    }
                }
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

/// Construct a session via [`Dialer::dial`] and wrap it for driving — see
/// [`IngestDriver`]. Performs **no I/O and completes no handshake**: the
/// returned driver starts in [`HealthState::Establishing`], and the caller
/// pumps it ([`IngestDriver::poll_transmit`] out, [`IngestDriver::feed`] in)
/// until it reports [`HealthState::Live`]. [`DialSupervisor`] adds bounded
/// retry on top for a `Dialer` whose local construction fails outright.
pub fn run_dial<D: Dialer>(
    dialer: &mut D,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
) -> Result<IngestDriver<D::Session>, D::Error> {
    let session = dialer.dial()?;
    Ok(IngestDriver::new(session, trunk_config, handshake))
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
    /// A session was constructed — a driveable [`IngestDriver`], starting in
    /// [`HealthState::Establishing`]. Named `Connected` for the caller's
    /// mental model of "the dial step succeeded"; nothing is connected yet in
    /// the I/O sense (see [`Dialer::dial`]).
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
    /// [`IngestDriver`] (starting in [`HealthState::Establishing`], bounded by
    /// `handshake`; every program it later announces gets a `Trunk` built from
    /// `trunk_config`). Never sleeps — see the module docs.
    pub fn try_dial(
        &mut self,
        trunk_config: TrunkConfig,
        handshake: HandshakePolicy,
    ) -> DialAttempt<D::Session, D::Error> {
        if self.exhausted {
            return DialAttempt::Exhausted;
        }
        self.attempts += 1;
        match self.dialer.dial() {
            Ok(session) => {
                self.attempts = 0;
                DialAttempt::Connected(IngestDriver::new(session, trunk_config, handshake))
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
    handshake: HandshakePolicy,
    sessions: HashMap<SessionId, IngestDriver<L::Session>>,
    next_id: u64,
}

impl<L: Listener> ListenDriver<L> {
    /// Build a driver over `listener`. Every admitted session starts in
    /// [`HealthState::Establishing`] bounded by `handshake` — which is what
    /// stops a flood of half-open inbound connections from squatting the
    /// `max_sessions` bound indefinitely — and every program any of them
    /// announces gets a `Trunk` built from `trunk_config`.
    pub fn new(listener: L, trunk_config: TrunkConfig, handshake: HandshakePolicy) -> Self {
        ListenDriver {
            listener,
            trunk_config,
            handshake,
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
                    self.sessions.insert(
                        id,
                        IngestDriver::new(session, self.trunk_config, self.handshake),
                    );
                    AcceptOutcome::Admitted(id)
                }
            }
            Err(e) => AcceptOutcome::Error(e),
        }
    }

    /// Feed bytes read for session `id`. Returns `Some(health)` exactly when
    /// this call caused the session to terminate (`Ended`, `Failed`, or
    /// `HandshakeTimedOut`) — at which point it is removed from this driver
    /// (its slot is free for a future [`Self::poll_accept`]); returns `None`
    /// for an unknown `id` or a session still running (`Establishing` or
    /// `Live`) after this call.
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

    /// Runs `op` against session `id`'s driver, then — if that call left it in
    /// a terminal state ([`HealthState::is_running`] `== false`) — removes it
    /// and returns that final state. This is the one place a session leaves
    /// `self.sessions`, which is what keeps this driver's resident memory
    /// bounded to [`Listener::max_sessions`] rather than accumulating every
    /// session that has ever ended, failed, or timed out mid-handshake.
    fn drive(
        &mut self,
        id: SessionId,
        op: impl FnOnce(&mut IngestDriver<L::Session>),
    ) -> Option<HealthState<<L::Session as Stage>::Error>> {
        let driver = self.sessions.get_mut(&id)?;
        op(driver);
        if driver.health().is_running() {
            None
        } else {
            self.sessions.remove(&id).map(|d| d.health)
        }
    }
}

/// Build a [`ListenDriver`] over `listener` — the whole of `run_listen`.
pub fn run_listen<L: Listener>(
    listener: L,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
) -> ListenDriver<L> {
    ListenDriver::new(listener, trunk_config, handshake)
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

    /// A handshake deadline far enough out that it never fires — for the
    /// tests that are not about the handshake bound. The two that *are* about
    /// it set their own tight deadline explicitly.
    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
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
    ///
    /// Starts with [`SessionEvent::Established`] already queued — modelling a
    /// source whose handshake is a purely local operation with nothing to
    /// negotiate (binding a UDP socket), which is a real case, not a shortcut.
    /// The genuinely multi-round-trip case has its own session type below
    /// ([`HandshakeSession`]).
    struct ScriptedSession {
        script: VecDeque<FeedOutcome>,
        pending: VecDeque<SessionEvent>,
        finish_outcome: Result<(), FakeError>,
    }

    impl ScriptedSession {
        fn new(script: Vec<FeedOutcome>) -> Self {
            ScriptedSession {
                script: script.into(),
                pending: VecDeque::from(vec![SessionEvent::Established]),
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

    /// Nothing to send: takes `poll_transmit`'s default.
    impl IngestSession for ScriptedSession {}

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

        let mut driver =
            run_dial(&mut dialer, trunk_config(), handshake()).expect("fake dial succeeds");
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
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
        assert!(
            matches!(driver.health(), HealthState::Establishing),
            "a freshly dialled session has not established yet"
        );

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
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());

        driver.feed(b"garbage", Timestamp::ZERO);

        match driver.health() {
            HealthState::Failed(FakeError(reason)) => assert_eq!(*reason, "bad continuity"),
            other => panic!("expected Failed(\"bad continuity\"), got {other:?}"),
        }
    }

    #[test]
    fn erroring_finish_yields_failed_not_ended() {
        let session = ScriptedSession::new(vec![]).failing_finish(FakeError("truncated tail"));
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());

        driver.finish();

        match driver.health() {
            HealthState::Failed(FakeError(reason)) => assert_eq!(*reason, "truncated tail"),
            other => panic!("expected Failed(\"truncated tail\"), got {other:?}"),
        }
    }

    #[test]
    fn terminated_driver_ignores_further_feed_and_finish() {
        let session = ScriptedSession::new(vec![FeedOutcome::Err(FakeError("boom"))]);
        let mut driver = IngestDriver::new(session, trunk_config(), handshake());
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
        let mut driver = run_dial(&mut dialer, trunk_config(), handshake()).unwrap();

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
        let mut driver = run_listen(
            FloodingListener { max_sessions },
            trunk_config(),
            handshake(),
        );

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
        let mut driver = run_listen(
            FloodingListener { max_sessions: 1 },
            trunk_config(),
            handshake(),
        );
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
            supervisor.try_dial(trunk_config(), handshake()),
            DialAttempt::Retry(_)
        ));
        assert_eq!(supervisor.attempts(), 1);
        assert!(matches!(
            supervisor.try_dial(trunk_config(), handshake()),
            DialAttempt::Retry(_)
        ));
        assert_eq!(supervisor.attempts(), 2);
        assert!(matches!(
            supervisor.try_dial(trunk_config(), handshake()),
            DialAttempt::GaveUp(_)
        ));
        assert_eq!(supervisor.attempts(), 3);
        assert!(supervisor.is_exhausted());

        // Flood: however many more times this is called, it must never dial
        // again (no growth in `attempts`) and must always report Exhausted,
        // not spin back into Retry/GaveUp.
        for _ in 0..10_000 {
            assert!(matches!(
                supervisor.try_dial(trunk_config(), handshake()),
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
        match supervisor.try_dial(trunk_config(), handshake()) {
            DialAttempt::Connected(_) => {}
            DialAttempt::Retry(_) => panic!("expected Connected, got Retry"),
            DialAttempt::GaveUp(_) => panic!("expected Connected, got GaveUp"),
            DialAttempt::Exhausted => panic!("expected Connected, got Exhausted"),
        }
        assert_eq!(supervisor.attempts(), 0);
        assert!(!supervisor.is_exhausted());
    }

    // --- Establishment: a real multi-round-trip handshake, no I/O ----------

    /// A genuinely multi-round-trip [`IngestSession`], shaped exactly like
    /// `rtsp_runtime::client::ClientSession`'s DESCRIBE → SETUP → PLAY
    /// sequence: it emits one request at a time via `poll_transmit`, consumes
    /// the peer's reply via `feed`, and only announces
    /// [`SessionEvent::Established`] after the third exchange completes.
    ///
    /// **It performs no I/O of any kind** — it has no socket, no runtime, and
    /// no `async fn`; it only moves bytes between its own two queues. The test
    /// below owns the "wire" itself, which is what makes the no-I/O claim an
    /// observable property rather than a promise.
    struct HandshakeSession {
        /// How many peer replies have been consumed so far.
        step: usize,
        outbound: VecDeque<Bytes>,
        pending: VecDeque<SessionEvent>,
    }

    /// The three requests `HandshakeSession` sends, in order — named after
    /// the RTSP sequence they stand in for.
    const HANDSHAKE_REQUESTS: [&[u8]; 3] = [b"DESCRIBE", b"SETUP", b"PLAY"];

    impl HandshakeSession {
        /// Queues the *first* request only. Note this is all `dial()` does —
        /// no connection, no negotiation.
        fn new() -> Self {
            HandshakeSession {
                step: 0,
                outbound: VecDeque::from(vec![Bytes::from_static(HANDSHAKE_REQUESTS[0])]),
                pending: VecDeque::new(),
            }
        }
    }

    impl Stage for HandshakeSession {
        type In<'a> = &'a [u8];
        type Out = SessionEvent;
        type Error = FakeError;

        fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), FakeError> {
            if self.step >= HANDSHAKE_REQUESTS.len() {
                // Post-handshake media.
                self.pending.push_back(SessionEvent::Sample {
                    program: ProgramId(1),
                    track_id: 7,
                    retention: RetentionClass::Timed,
                    sample: sample(0xAB),
                });
                return Ok(());
            }
            // Each reply must be the 200 for the request we actually sent —
            // a real state machine correlates, so this fake does too.
            let expected = format!(
                "200 {}",
                String::from_utf8_lossy(HANDSHAKE_REQUESTS[self.step])
            );
            if input != expected.as_bytes() {
                return Err(FakeError("handshake reply out of sequence"));
            }
            self.step += 1;
            match HANDSHAKE_REQUESTS.get(self.step) {
                // More handshake to do: queue the next request.
                Some(next) => self.outbound.push_back(Bytes::from_static(next)),
                // Final reply consumed: now established, and the track set is
                // known (it came from the DESCRIBE-equivalent).
                None => {
                    self.pending.push_back(SessionEvent::Established);
                    self.pending.push_back(SessionEvent::NewProgram {
                        program: ProgramId(1),
                        tracks: vec![opaque_track(7)],
                    });
                }
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<SessionEvent> {
            self.pending.pop_front()
        }

        fn finish(&mut self) -> Result<(), FakeError> {
            Ok(())
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn demand(&self) -> Demand {
            Demand::new(4096)
        }
    }

    /// The handshake's outbound side: this is the *only* way a request leaves
    /// the session — it has no socket to write to.
    impl IngestSession for HandshakeSession {
        fn poll_transmit(&mut self) -> Option<Bytes> {
            self.outbound.pop_front()
        }
    }

    /// A [`Dialer`] over [`HandshakeSession`] — `dial()` constructs and
    /// returns immediately, connecting nothing.
    struct HandshakeDialer;

    impl Dialer for HandshakeDialer {
        type Session = HandshakeSession;
        type Error = FakeError;

        fn dial(&mut self) -> Result<HandshakeSession, FakeError> {
            Ok(HandshakeSession::new())
        }
    }

    #[test]
    fn multi_round_trip_handshake_completes_through_feed_and_poll_transmit_only() {
        let mut dialer = HandshakeDialer;
        let mut driver =
            run_dial(&mut dialer, trunk_config(), handshake()).expect("dial constructs a session");

        // `dial()` did no I/O and did not establish anything.
        assert!(
            matches!(driver.health(), HealthState::Establishing),
            "dial() must not establish the session: {:?}",
            driver.health()
        );

        // The whole "network" is this Vec — every byte the session sends is
        // recorded here by the test, and every reply is handed back through
        // `feed`. Nothing else can move bytes, so a session that tried to do
        // its own I/O simply would not progress.
        let mut wire: Vec<Bytes> = Vec::new();
        let mut now = 0u64;

        // Pump: drain poll_transmit, answer each request, feed the reply.
        // Three round trips, driven entirely by this loop.
        for _ in 0..HANDSHAKE_REQUESTS.len() {
            let req = driver
                .poll_transmit()
                .expect("the session has a handshake request to send");
            assert!(
                driver.poll_transmit().is_none(),
                "one request in flight at a time"
            );
            wire.push(req.clone());

            let reply = format!("200 {}", String::from_utf8_lossy(&req));
            now += 1;
            driver.feed(reply.as_bytes(), Timestamp::from_nanos(now));
        }

        // The exact request sequence went out, in order, through
        // poll_transmit — nowhere else.
        let sent: Vec<&[u8]> = wire.iter().map(|b| b.as_ref()).collect();
        assert_eq!(sent, HANDSHAKE_REQUESTS, "handshake request sequence");

        // MUTATION-CHECKED: the promotion out of Establishing lives in
        // `drain()`'s `Established` arm.
        assert!(
            matches!(driver.health(), HealthState::Live),
            "after the final handshake reply the session must be Live: {:?}",
            driver.health()
        );

        // And it is genuinely usable: the program announced with Established
        // has a Trunk, and post-handshake media lands in it.
        let trunk = driver
            .trunk(ProgramId(1))
            .cloned()
            .expect("the handshake announced program 1");
        let mut cursor = trunk.subscribe();
        driver.feed(b"media", Timestamp::from_nanos(now + 1));
        match cursor.poll().expect("post-handshake sample on the ring") {
            crate::SampleCursorItem::Timed { track_id, .. } => assert_eq!(track_id, 7),
            other => panic!("expected Timed, got {other:?}"),
        }
    }

    /// A session that sends its first request and then never establishes,
    /// whatever it is fed — the stalled/half-open peer.
    struct StallingSession {
        outbound: VecDeque<Bytes>,
    }

    impl Stage for StallingSession {
        type In<'a> = &'a [u8];
        type Out = SessionEvent;
        type Error = FakeError;

        fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), FakeError> {
            Ok(()) // never errors, never establishes — just silence
        }

        fn poll(&mut self) -> Option<SessionEvent> {
            None
        }

        fn finish(&mut self) -> Result<(), FakeError> {
            Ok(())
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn demand(&self) -> Demand {
            Demand::new(4096)
        }
    }

    impl IngestSession for StallingSession {
        fn poll_transmit(&mut self) -> Option<Bytes> {
            self.outbound.pop_front()
        }
    }

    struct StallingListener {
        max_sessions: usize,
    }

    impl Listener for StallingListener {
        type Session = StallingSession;
        type Error = FakeError;

        fn max_sessions(&self) -> usize {
            self.max_sessions
        }

        fn poll_accept(&mut self) -> Result<Option<StallingSession>, FakeError> {
            // Queues its first request, exactly like a real session would —
            // so this models "we sent our opening request and the peer went
            // silent", not "nothing ever happened".
            Ok(Some(StallingSession {
                outbound: VecDeque::from(vec![Bytes::from_static(HANDSHAKE_REQUESTS[0])]),
            }))
        }
    }

    #[test]
    fn never_completing_handshake_is_bounded_and_reported_not_leaked() {
        const DEADLINE: Timestamp = Timestamp::from_nanos(1_000);
        let mut driver = run_listen(
            StallingListener { max_sessions: 1 },
            trunk_config(),
            HandshakePolicy::establish_by(DEADLINE),
        );

        let AcceptOutcome::Admitted(id) = driver.poll_accept() else {
            panic!("expected admission");
        };
        assert!(matches!(driver.health(id), Some(HealthState::Establishing)));
        // The one slot is taken, so nothing else gets in while this peer
        // stalls — which is exactly why the bound below must exist.
        assert!(matches!(driver.poll_accept(), AcceptOutcome::Refused));

        // Before the deadline, feeding it more silence must NOT terminate it:
        // a slow-but-progressing handshake is legitimate.
        assert!(
            driver
                .feed(id, b"...", Timestamp::from_nanos(DEADLINE.as_nanos() - 1))
                .is_none(),
            "must not time out before the deadline"
        );
        assert!(matches!(driver.health(id), Some(HealthState::Establishing)));
        assert_eq!(driver.session_count(), 1);

        // At the deadline, with the handshake still incomplete, it terminates
        // — reported, with the deadline that was blown.
        // MUTATION-CHECKED: `check_handshake_deadline`.
        let health = driver
            .on_deadline(id, DEADLINE)
            .expect("the blown deadline must terminate the session");
        assert_eq!(
            health,
            HealthState::HandshakeTimedOut { deadline: DEADLINE },
            "a never-completing handshake must be reported as HandshakeTimedOut"
        );

        // And it is REAPED, not leaked: the slot is free again, so a flood of
        // half-open connections cannot squat max_sessions forever.
        assert_eq!(
            driver.session_count(),
            0,
            "a timed-out session must be reaped, not left pinning its slot"
        );
        assert!(driver.health(id).is_none());
        assert!(matches!(driver.poll_accept(), AcceptOutcome::Admitted(_)));
    }

    #[test]
    fn handshake_completing_exactly_at_the_deadline_still_establishes() {
        const DEADLINE: Timestamp = Timestamp::from_nanos(500);
        // A locally-established session (Established already queued), fed at
        // exactly the deadline: the deadline check runs *after* the feed is
        // drained, so this must be Live, not HandshakeTimedOut.
        let session = ScriptedSession::new(vec![]);
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            HandshakePolicy::establish_by(DEADLINE),
        );
        driver.feed(b"reply", DEADLINE);
        assert!(
            matches!(driver.health(), HealthState::Live),
            "a handshake completing exactly at the deadline must establish, \
             not be rejected by a nanosecond: {:?}",
            driver.health()
        );
    }

    #[test]
    fn next_deadline_surfaces_the_handshake_bound_while_establishing() {
        const DEADLINE: Timestamp = Timestamp::from_nanos(9_000);
        let mut dialer = HandshakeDialer;
        let mut driver = run_dial(
            &mut dialer,
            trunk_config(),
            HandshakePolicy::establish_by(DEADLINE),
        )
        .unwrap();

        // The session itself has no deadline of its own, so a caller driving
        // purely off next_deadline() would never fire the timeout check
        // unless the driver contributes the handshake bound here.
        assert_eq!(
            driver.next_deadline(),
            Some(DEADLINE),
            "while Establishing, next_deadline must surface the handshake bound"
        );

        // Once established it drops out again (the session's own None wins).
        for _ in 0..HANDSHAKE_REQUESTS.len() {
            let req = driver.poll_transmit().expect("handshake request");
            let reply = format!("200 {}", String::from_utf8_lossy(&req));
            driver.feed(reply.as_bytes(), Timestamp::ZERO);
        }
        assert!(matches!(driver.health(), HealthState::Live));
        assert_eq!(
            driver.next_deadline(),
            None,
            "the handshake bound must not linger after establishment"
        );
    }
}
