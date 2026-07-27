//! `ServedEgress`, `PushEgress`, `SegmentEgress` — the three egress shapes
//! that read from a [`crate::Trunk`] (plan step 3d;
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §1/§3).
//!
//! `#[cfg(feature = "std")]`: like [`crate::trunk`]/[`crate::ingress`], this
//! module's types are handed [`transmux::TrackSpec`]s and
//! [`crate::trunk::SampleCursorItem`]/[`crate::trunk::SegmentCursorItem`]
//! values, and every real consumer is `std`+`tokio` per the architecture.
//!
//! # Why three shapes, not one
//!
//! Rev 1 of the architecture treated egress as a single reader class; the
//! architecture-audit finding that corrected it (G6, spec §3) is that the
//! three egress consumers this workspace actually has are structurally
//! different operations, not three configurations of the same one:
//!
//! - [`ServedEgress`] **resolves a request**: given "does part 3.2 of segment
//!   9 exist" (LL-HLS), "here is the current MPD" (DASH), or "give me segment
//!   N from an hour ago" (catch-up), it answers *now*, from whatever window
//!   state it already holds. It never initiates I/O and it never iterates a
//!   stream — a request comes in, an answer goes out (or, for the one
//!   protocol that legitimately blocks on a not-yet-existing byte range,
//!   [`EgressResponse::Await`] goes out — see below).
//! - [`PushEgress`] **streams samples that already exist** at whoever is
//!   listening (WHEP, RTMP-out, SRT-out, a loudness tap): there is no
//!   "request" — it drains one [`crate::trunk::SampleCursor`] forever, in
//!   publish order, until the stream ends.
//! - [`SegmentEgress`] **consumes finished segments** (DVR, MABR, ATSC ROUTE,
//!   Smooth-push): the unit of work is not a sample, and answering "have you
//!   seen segment N" is not the shape of the operation — draining one
//!   [`crate::trunk::SegmentCursor`] to completion, and reacting correctly to
//!   [`crate::trunk::ArchiveOverrun`]'s three-way loss/stall/drop trade, is.
//!
//! Collapsing these into one trait (a single `fn feed(&mut self, item)` that
//! every one of the nine-plus real outputs implements) was tried in spec rev
//! 1 and rejected: a request/response protocol has no "next item" to feed —
//! it has to be able to say "not yet, ask again", which a push/consume
//! interface has no way to express (there is no caller waiting for a
//! return value); conversely a request-answering interface has no method a
//! streaming push side could call once, unprompted, to hand over a sample. A
//! trait that tried to serve both would need every implementor to leave half
//! the interface an unreachable `unimplemented!()`, which is exactly the
//! "no variant you cannot correctly produce" violation this crate's
//! [`crate::byte_merge`] module already rejected once (`Hitless2022_7`).
//! **If a future refactor is tempted to merge these three back into one
//! trait, re-read this section first** — the three shapes above are not an
//! accident of the current call sites, they are the actual difference
//! between resolve/stream/consume, and merging them just moves the
//! unreachable-branch problem into whichever one trait was chosen as the
//! "real" shape.
//!
//! # `ServedEgress::resolve` does not take `&Trunk` — a correction found
//! before Step 4, the same way ingress's `dial()` was
//!
//! The architecture spec's own pseudocode (§3) sketches
//! `fn resolve(req, &Trunk) -> EgressResponse`. Reading
//! `ll-hls-runtime/src/server/` (the engine this trait exists to receive,
//! per the implementation plan's Step 4) before writing this trait surfaced
//! the same class of problem that reading `rtsp-runtime` before writing
//! [`crate::ingress::Dialer::dial`] surfaced: the pseudocode names a
//! parameter that the actual data structure cannot make good on.
//!
//! **What `&Trunk` can answer today, `&self`-shaped, no cursor required:**
//! [`crate::Trunk::events_between`]/[`crate::Trunk::events_in_segment`] (the
//! event log's snapshot queries) and the four `*_len()` diagnostics. That is
//! genuinely a "resolve a request against shared state" shape, and a
//! `ServedEgress` implementation is free to call them.
//!
//! **What it cannot answer:** "does segment 9 exist", "what bytes are
//! part 3.2", or anything else a `ServedEgress` implementing LL-HLS/DASH/
//! catch-up actually needs to resolve most requests. The segment log has
//! exactly one reader shape — [`crate::trunk::SegmentCursor`], a **moving**,
//! **stateful**, single-consumer position — with no companion snapshot query
//! the way the event log has `events_between`. So a `resolve()` that took
//! `&Trunk` would, for every real implementation, immediately have to ignore
//! it and consult a second, self-maintained cache instead — which is exactly
//! what `ll_hls_runtime::server::MediaStore` already does in production: it
//! is fed by `add_segment`/`add_part`/`set_init` (called by whatever drains
//! the segmenter) and *separately* answers `resolve_playlist`/
//! `resolve_resource` from that already-synced state. `MediaStore` is
//! two-sided — a write side and a read side — and no version of `&Trunk`
//! passed into `resolve()` changes that; it would just be a parameter every
//! real implementation quietly declines to use for the one thing it is
//! there for.
//!
//! So this trait states the honest shape: **`resolve` takes no `Trunk` at
//! all.** A `ServedEgress` implementation is constructed with (or otherwise
//! obtains) whatever [`crate::trunk::SegmentCursor`]/[`crate::trunk::SampleCursor`]
//! it needs, keeps its own resolvable window in sync by draining them
//! (Step 4/5's job, not this trait's), and `resolve` only ever reads that
//! already-synced state. This is not a smaller trait than the spec sketched —
//! it is the same trait with a parameter removed that no implementation
//! could have honestly used.
//!
//! **A second gap in the same area, recorded rather than solved:**
//! `MediaStore::listen()` gives its adapter a wakeup future
//! (`event_listener`) so a blocked LL-HLS request does not have to
//! busy-poll — [`Trunk`](crate::Trunk) has no reader-side notify primitive at
//! all today (every cursor is a synchronous, non-blocking `poll()`; the only
//! `Condvar` in `trunk.rs` wakes a **writer** parked on
//! [`crate::trunk::ArchiveOverrun::StallIngest`], not a reader waiting on new
//! data). An adapter built on this trait must therefore poll-with-backoff,
//! bounded by [`AwaitPolicy`], rather than truly sleeping on a wake channel —
//! acceptable (bounded, correct, just less efficient than `event_listener`),
//! but Step 4 should decide deliberately whether to add a `Trunk`-level
//! notify primitive or accept the poll loop, rather than discovering the gap
//! mid-port.
//!
//! # `EgressResponse::Await` is bounded, by construction, not by convention
//!
//! LL-HLS blocking reload (RFC 8216bis §6.2.5.2) is precisely "hold this
//! request open until a part exists" — and precisely that is also a
//! textbook way for a hostile client to park a request forever: ask for a
//! part that will never arrive, and if the origin waits unconditionally,
//! one connection now costs the origin an open handle indefinitely. This
//! project has already shipped four unbounded-allocation vectors, every one
//! in code handling remote input, which is why this is treated as a hard
//! requirement rather than a documented-only convention:
//! [`EgressResponse::Await`] MUST NOT be returned once the caller's own
//! patience ([`AwaitPolicy::deadline`]) has passed. [`AwaitPolicy::expired`]
//! and [`EgressResponse::pending`] are the one correct path through this
//! check — real code in this crate, not merely documented advice — so an
//! implementor gets the bound for free by calling `EgressResponse::pending`
//! rather than hand-rolling the comparison per protocol. The mutation-tested
//! proof is in this module's tests: `EgressResponse::pending` mutated to
//! never expire is exactly the bug this section exists to prevent, and the
//! test below catches it.
//!
//! Once expired, `pending` answers [`EgressResponse::NotFound`] — literally
//! true ("nothing was found within the time I was willing to wait") — not a
//! fabricated [`EgressResponse::Ready`]. An implementation that *can*
//! produce an honest best-effort `Ready` once its patience runs out (e.g.
//! LL-HLS answering with whatever playlist state currently exists, per RFC
//! 8216bis's own "SHOULD respond in a reasonable time" guidance) is free to
//! do that instead of calling `pending` at all — the contract is "never
//! `Await` past the deadline", not "must be `NotFound` specifically".
//!
//! # `PushEgress`: one cursor, `send` takes one item, `poll_transmit` reused
//! from `IngestSession`
//!
//! [`crate::Trunk::subscribe`]'s own docs are the load-bearing citation here:
//! writer cost is **O(N) in cursor count** (`spikes/trunk-bench`), so a
//! `PushEgress` serving many peers (WHEP, any future RTP multicast/SSM
//! egress) must take **exactly one** [`crate::trunk::SampleCursor`] and fan
//! out to its peers itself, at the layer that already holds per-peer state
//! (SRTP context, congestion window, pacing epoch) anyway — never one cursor
//! per peer. This trait does not (and structurally cannot) enforce that by
//! itself, any more than `Trunk::subscribe` enforces "call me only a
//! single-digit number of times" — it is a documented contract a driver
//! (Step 5) must honour, exactly like that method's own docs.
//!
//! `send` takes `&`[`crate::trunk::SampleCursorItem`] — the exact type
//! [`crate::trunk::SampleCursor::poll`] already produces — rather than the
//! architecture spec's sketched `&TrackBatch`. No `TrackBatch` type exists
//! anywhere in this workspace, and inventing one here would be pure
//! speculation: every real drive loop in this crate (`Stage::poll`,
//! `IngestSession`/`SessionEvent`, `SampleCursor::poll`) already hands back
//! **one item per call**, not a batch, and `SampleCursorItem` already
//! carries the `Lagged`/`Degraded` loss reports a push output needs to see
//! in-band, cannot-skip-past, exactly like every other cursor in this crate.
//! Reusing it is the module's established "reuse, don't duplicate" pattern
//! (see `crate::trunk::SegmentEntry`'s own module doc for the same
//! reasoning), not a smaller feature than the spec sketched.
//!
//! [`PushEgress::poll_transmit`] mirrors
//! [`crate::ingress::IngestSession::poll_transmit`] exactly, for the same
//! reason: a WHEP/RTP push output has real outbound bytes to hand a
//! transport (SRTCP receiver reports, an RTP/RTCP control loop), but most
//! `PushEgress` implementations (a loudness tap, an RTMP-out session that
//! only writes what it is fed) never transmit anything of their own accord,
//! so this is a `None`-returning default a real transmitting output
//! overrides, not a method every implementor must stub out.
//!
//! # `renegotiate` and issue #781: a track addition must be expressible, and
//! a refusal must be truthful
//!
//! Issue #781 is the reason `renegotiate` exists at all, restated precisely
//! since this crate does not fix it (Step 5 does): `multimux`'s
//! `SampleSource::track_specs()` is a one-shot, connect-time snapshot, so
//! when `transmux` (issue #774) correctly detects a mid-stream PMT track
//! *addition* and reports it, there is today no call a source can make to
//! tell a running segmenter/output about it — the new track's samples have
//! nowhere to go. [`PushEgress::negotiate`] is the connect-time call;
//! [`PushEgress::renegotiate`] is what #781 needs: a second, later call with
//! the **current** `&[TrackSpec]` (which may add, remove, or update tracks
//! relative to whatever was last negotiated — driven by
//! `SessionEvent::NewProgram`'s updated track list or a future
//! `Trunk`-level track-set-changed signal), so an addition has a real call
//! site to land on instead of only a log line.
//!
//! **The deliberate design decision this step was asked to make, not
//! default on:** what does an output do when it genuinely cannot accept the
//! new track set — a WHEP peer that already answered an SDP offer, a DASH
//! manifest that already published a `Period` with a fixed
//! `AdaptationSet` list? [`NegotiationOutcome::Refused`] is the answer: a
//! truthful "I understand the change and cannot apply it, and here is why"
//! (`reason: &'static str`), distinct from [`NegotiationOutcome::Accepted`]
//! *and* from [`NegotiationOutcome::Error`] (a malformed/unsatisfiable
//! offer, not a refusal of an otherwise-valid one). The alternative —
//! silently keeping the old [`TrackSelection`] and returning `Accepted`
//! anyway, or simply dropping the new track with no return value at all —
//! was rejected: both look identical to "nothing changed" from the caller's
//! side, which is exactly the silent-drop failure mode #781 already
//! demonstrates for the *ingest* side (a warning logged where nobody reads
//! logs as a control-flow signal). `Refused` is in-band, exactly like every
//! other loss/refusal signal in this crate
//! ([`crate::trunk::SampleCursorItem::Lagged`],
//! [`crate::trunk::SegmentCursorItem::Gap`],
//! [`crate::ingress::HealthState::HandshakeTimedOut`]) — the caller (Step 5's
//! driver reacting to #781) can log it, surface it as a metric, or decide to
//! cut a discontinuity and restart the output; what it cannot do is confuse
//! it with success. After a `Refused`, the output continues running on
//! whichever [`TrackSelection`] it last had `Accepted` (or, if `Refused` was
//! the answer to the very first `negotiate`, it never started at all — the
//! driver's job, not this trait's).
//!
//! # `SegmentEgress` reuses `SegmentCursorItem` verbatim — and does not add
//! `on_manifest`
//!
//! [`SegmentEgress::on_segment`] takes `&`[`crate::trunk::SegmentCursorItem`]
//! — the exact type [`crate::trunk::SegmentCursor::poll`] already produces,
//! including [`crate::trunk::SegmentCursorItem::Gap`]/`Terminated` — so a DVR
//! writer sees the same [`crate::trunk::ArchiveOverrun`] loss/stall/drop
//! signal every other segment reader does, and this step reuses rather than
//! reimplements 3b-ii's pinning: nothing here re-derives when a segment is
//! evicted or a pin releases; that logic stays exactly where it was tested.
//!
//! The architecture spec's §3 pseudocode also sketches
//! `async fn on_manifest(&mut self, m: &ManifestSnapshot)`. `ManifestSnapshot`
//! does not exist anywhere in this workspace, and the spec's own §3 admits
//! why a single type would be wrong here: DVR's "manifest" is a recording
//! index, MABR's is a service/session list, and "a FLUTE/ROUTE sender
//! additionally needs a carousel repeat schedule with per-object deadlines,
//! TOI allocation and FDT expiry — none of which is a `TrackBatch`". Typing
//! one `ManifestSnapshot` now, before any of those four real shapes has a
//! concrete producer, would be exactly the kind of field/variant this
//! crate's own precedent (`crate::byte_merge`'s `Hitless2022_7` note) argues
//! against: don't add a method whose payload nothing can correctly produce
//! yet. `on_segment` alone is what this step can honestly ship;
//! `on_manifest` (or four protocol-specific equivalents) is Step 5's problem,
//! once a real FLUTE/MABR/DVR-index writer exists to shape it.

use broadcast_common::Timestamp;
use bytes::Bytes;
use transmux::TrackSpec;

use crate::trunk::{SampleCursorItem, SegmentCursorItem};

/// `Cache-Control` policy an adapter should apply to a resolved
/// [`EgressResponse::Ready`] body — playlists/manifests are always
/// re-fetched for liveness, while a produced init/segment/part byte range
/// never changes once produced. Mirrors
/// `ll_hls_runtime::server::CachePolicy` in spirit (this crate cannot depend
/// on that crate — the dependency runs the other way, per the migration
/// order in `docs/superpowers/plans/2026-07-26-media-plane-implementation.md`
/// Step 4) so it is redefined here rather than borrowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CachePolicy {
    /// Safe to cache indefinitely — this URI's bytes never change once
    /// produced.
    Immutable,
    /// Must always be re-fetched (liveness-sensitive: a playlist, a
    /// manifest).
    NoCache,
}

impl CachePolicy {
    /// The spec/field-enum label (workspace #204 convention): a stable,
    /// lowercase token suitable for logs/metrics/`Cache-Control` diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            CachePolicy::Immutable => "immutable",
            CachePolicy::NoCache => "no-cache",
        }
    }
}

broadcast_common::impl_spec_display!(CachePolicy);

/// Bounds how long [`ServedEgress::resolve`] may keep answering
/// [`EgressResponse::Await`] for **one logical request** — the same
/// absolute-deadline shape as
/// [`crate::ingress::HandshakePolicy::establish_by`], for the same reason:
/// the failure mode is wall-clock ("the awaited part never arrives"), not an
/// attempt/iteration count, and a fixed [`Timestamp`] costs no new clock
/// concept since [`Timestamp`] is already threaded through this crate.
///
/// A caller constructs one `AwaitPolicy` when a request first arrives (e.g.
/// LL-HLS's RFC 8216bis §6.2.5.2 hold, or a caller-chosen HTTP timeout) and
/// passes the **same** value to every re-resolve of that request as `now`
/// advances — never a fresh, later deadline for the same logical request,
/// which would defeat the bound entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AwaitPolicy {
    /// Absolute [`Timestamp`] past which [`ServedEgress::resolve`] must not
    /// answer [`EgressResponse::Await`] again for this request.
    pub deadline: Timestamp,
}

impl AwaitPolicy {
    /// Bound a request to expire at `deadline`.
    pub fn new(deadline: Timestamp) -> Self {
        AwaitPolicy { deadline }
    }

    /// `true` once `now` has reached or passed [`Self::deadline`] — the
    /// point past which [`EgressResponse::pending`] stops answering `Await`.
    pub fn expired(&self, now: Timestamp) -> bool {
        now >= self.deadline
    }
}

/// What [`ServedEgress::resolve`] hands back: the resolved body, a bounded
/// "not yet", or a reason resolution stops immediately.
///
/// `#[non_exhaustive]`: the growth point for a later, protocol-specific
/// outcome (e.g. a redirect) without a breaking change to every match arm in
/// the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EgressResponse<B> {
    /// The request is satisfied now.
    Ready {
        /// The resolved body (a playlist/manifest string, segment/part
        /// bytes — `B` is per-implementor, deliberately not fixed to
        /// `Vec<u8>`/`String` here since a `ServedEgress` may want to hand
        /// back something richer, e.g. a body plus headers).
        body: B,
        /// `Cache-Control` policy the adapter should apply to `body`.
        cache: CachePolicy,
    },
    /// The awaited condition (a part that has not landed yet, a segment
    /// number just beyond the live edge) is not satisfied *yet* — see
    /// [The `Await` bounding section](self#egressresponseawait-is-bounded-by-construction-not-by-convention).
    /// The caller should wait no later than `retry_no_earlier_than` (a
    /// scheduling hint, not a hard requirement — waiting less is always
    /// fine) and re-resolve. **Must not** be returned once the caller's own
    /// [`AwaitPolicy`] has expired — see [`EgressResponse::pending`].
    Await {
        /// Hint: do not bother re-resolving before this point. An
        /// implementation with no better estimate may set this to the `now`
        /// it was given — "you may retry immediately" is still a truthful
        /// answer.
        retry_no_earlier_than: Timestamp,
    },
    /// The requested resource does not exist and, as far as this call can
    /// tell, never will (an unknown filename shape, a part whose segment
    /// already closed without producing it, or — see
    /// [`EgressResponse::pending`] — a wait whose patience ran out).
    NotFound,
    /// The request itself is malformed or abusive (LL-HLS's own
    /// `_HLS_msn`/`_HLS_part` abuse-prevention bound is exactly this
    /// class) — reject now, do not wait.
    BadRequest {
        /// A static, human-readable reason (matching this crate's
        /// `&'static str` convention for a non-fabricated, always-correct
        /// error string — see [`crate::byte_merge::MergeError`]'s
        /// `#[error(...)]` messages for the same style, one level up).
        reason: &'static str,
    },
}

impl<B> EgressResponse<B> {
    /// The one correct path through [`AwaitPolicy`]'s bound: `Await` while
    /// `policy` has not yet expired, [`EgressResponse::NotFound`] once it
    /// has — see
    /// [The `Await` bounding section](self#egressresponseawait-is-bounded-by-construction-not-by-convention)
    /// for why this exists as real, mutation-tested code rather than a
    /// per-implementation comparison every `ServedEgress` would otherwise
    /// have to hand-roll (and could get wrong) independently.
    pub fn pending(policy: AwaitPolicy, now: Timestamp, retry_no_earlier_than: Timestamp) -> Self {
        if policy.expired(now) {
            EgressResponse::NotFound
        } else {
            EgressResponse::Await {
                retry_no_earlier_than,
            }
        }
    }
}

/// Pull egress: answers a **request** against whatever window state an
/// implementation keeps in sync (LL-HLS, DASH, catch-up) — see
/// [the module docs](self#why-three-shapes-not-one) for why this is not a
/// stream/consume interface, and
/// [why `resolve` takes no `&Trunk`](self#servedegressresolve-does-not-take-trunk--a-correction-found-before-step-4-the-same-way-ingresss-dial-was)
/// for why the signature below differs from the architecture spec's rough
/// pseudocode.
///
/// No `axum`, no `tokio`, no HTTP type appears here on purpose — `multimux`
/// supplies the HTTP adapter (Step 5); this trait is what that adapter maps
/// a wire request onto and renders a wire response from.
pub trait ServedEgress: Send + Sync + 'static {
    /// The protocol-specific request shape (e.g. LL-HLS's Media Sequence
    /// Number + part index; a DASH segment/manifest request; a catch-up time
    /// range) — owned by the implementing crate, not this one.
    type Request;
    /// The protocol-specific resolved body (a playlist string, manifest XML,
    /// segment bytes, ...).
    type Body;

    /// Resolve `request` against this implementation's current state at
    /// time `now`, bounded by `await_policy` — see
    /// [`EgressResponse::Await`]'s contract.
    fn resolve(
        &self,
        request: Self::Request,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<Self::Body>;
}

/// Which of the currently-known tracks a [`PushEgress`] selected.
///
/// `#[non_exhaustive]`: constructed via [`TrackSelection::new`] — a later
/// step may need per-track selection metadata (e.g. a chosen bitrate ladder
/// rung) alongside the id list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrackSelection {
    /// The selected tracks' ids, matching [`transmux::TrackSpec::track_id`].
    pub track_ids: Vec<u32>,
}

impl TrackSelection {
    /// Select exactly `track_ids`.
    pub fn new(track_ids: Vec<u32>) -> Self {
        TrackSelection { track_ids }
    }
}

/// The result of [`PushEgress::negotiate`]/[`PushEgress::renegotiate`] — see
/// [the module docs](self#renegotiate-and-issue-781-a-track-addition-must-be-expressible-and-a-refusal-must-be-truthful)
/// for why `Refused` is a distinct, truthful outcome rather than a silent
/// fallback to the previous selection.
///
/// `#[non_exhaustive]`: the growth point for a later outcome (e.g. "accepted,
/// but only after the next discontinuity") without breaking every match arm.
#[derive(Debug)]
#[non_exhaustive]
pub enum NegotiationOutcome<E> {
    /// The proposed track set was accepted; this is the new selection.
    Accepted(TrackSelection),
    /// This output understood the proposed change and cannot apply it
    /// without breaking an already-established contract with its peer (an
    /// SDP already answered, a `Period` already published) — see the module
    /// docs. The output continues on whichever [`TrackSelection`] it last
    /// had `Accepted`.
    Refused {
        /// A static, human-readable reason — matching
        /// [`EgressResponse::BadRequest`]'s `&'static str` convention.
        reason: &'static str,
    },
    /// The proposed track set itself is unsatisfiable for this output (e.g.
    /// no track of a codec it can carry at all) — distinct from `Refused`:
    /// this is "the offer is bad", not "the offer is fine but I am already
    /// committed elsewhere".
    Error(E),
}

/// Sample push egress: streams samples that already exist at whoever is
/// listening (WHEP, RTMP-out, SRT-out, a loudness tap) — see
/// [the module docs](self#pushegress-one-cursor-send-takes-one-item-poll_transmit-reused-from-ingestsession).
///
/// # One cursor, never one per peer
///
/// A conforming implementation owns **exactly one**
/// [`crate::trunk::SampleCursor`] (obtained via [`crate::Trunk::subscribe`])
/// regardless of how many peers it serves — see that method's own docs for
/// the O(N)-in-cursor-count writer cost this exists to avoid. This trait
/// does not take a cursor as a parameter (a driver, not this trait, owns and
/// drains it — see [`crate::trunk::SampleCursor::poll`]'s own docs), so
/// nothing here can *enforce* the rule; it is exactly as documented-only as
/// `Trunk::subscribe`'s own guidance.
pub trait PushEgress: Send {
    /// Why negotiation, renegotiation, or a send failed outright — distinct
    /// from [`NegotiationOutcome::Refused`], which is not an error.
    type Error;

    /// Connect-time negotiation: given the tracks known so far, select which
    /// ones this output will carry.
    fn negotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error>;

    /// The track set changed mid-stream (issue #781: a PMT version bump, a
    /// `SessionEvent::NewProgram` update) — propose the **current** full
    /// track list again. See
    /// [the module docs](self#renegotiate-and-issue-781-a-track-addition-must-be-expressible-and-a-refusal-must-be-truthful)
    /// for why this is a distinct call from `negotiate`, not a default
    /// implemented in terms of it: an output legitimately behaves
    /// differently once already live (a WHEP peer with an answered SDP must
    /// refuse what an as-yet-unconnected `negotiate` would have happily
    /// accepted).
    fn renegotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error>;

    /// Send one polled item from this output's single
    /// [`crate::trunk::SampleCursor`] — a real sample, or a
    /// [`crate::trunk::SampleCursorItem::Lagged`]/`Degraded` loss report the
    /// output may react to (a discontinuity marker, a dropped-frame counter)
    /// but must not silently ignore, matching every other cannot-skip-past
    /// loss report in this crate.
    fn send(&mut self, item: &SampleCursorItem) -> Result<(), Self::Error>;

    /// Outbound bytes this output wants written to its transport (an RTCP
    /// receiver report, an SRT ACK) — mirrors
    /// [`crate::ingress::IngestSession::poll_transmit`] exactly, including
    /// its `None` default: most `PushEgress` implementations never transmit
    /// anything of their own accord.
    fn poll_transmit(&mut self) -> Option<Bytes> {
        None
    }
}

/// Segment/object push egress: consumes finished segments (DVR, DVB-MABR,
/// ATSC ROUTE, Smooth-push) via one [`crate::trunk::SegmentCursor`] — see
/// [the module docs](self#segmentegress-reuses-segmentcursoritem-verbatim--and-does-not-add-on_manifest).
///
/// Exactly like [`PushEgress`], a conforming implementation owns **one**
/// cursor (ordinary, from [`crate::Trunk::subscribe_segments`], or pinning,
/// from [`crate::Trunk::pin_segments`], for a consumer that must not miss a
/// segment) — the same single-digit-reader, one-cursor-per-consumer rule as
/// every other `Trunk` reader.
pub trait SegmentEgress: Send {
    /// Why handling a segment failed outright.
    type Error;

    /// Handle one polled item from this output's single
    /// [`crate::trunk::SegmentCursor`] — a finished segment, or a
    /// [`crate::trunk::SegmentCursorItem::Lagged`]/`Gap`/`Terminated` report
    /// this output must record (a DVR writer's hole in the recording), not
    /// silently drop. [`crate::trunk::ArchiveOverrun`] (3b-ii) governs which
    /// of these a pinning cursor can produce and when — this trait does not
    /// re-decide any of that, only reacts to what the cursor already
    /// reports.
    fn on_segment(&mut self, item: &SegmentCursorItem) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trunk::{ArchiveOverrun, SegmentEntry, Trunk, TrunkConfig};
    use std::num::NonZeroUsize;
    use std::sync::Mutex;
    use std::time::Duration;
    use transmux::{CodecConfig, Sample};

    /// `NonZeroUsize` from a literal capacity — see `trunk`'s identical test
    /// helper.
    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    fn sample(byte: u8) -> Sample {
        Sample::new(
            bytes::Bytes::from(vec![byte; 4]),
            Some(0),
            Some(0),
            None,
            true,
        )
    }

    fn video_spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Vp8 {
                width: 320,
                height: 240,
            },
        )
    }

    fn segment_entry(seq: u32) -> SegmentEntry {
        SegmentEntry::new(
            bytes::Bytes::from(vec![seq as u8; 8]),
            seq,
            Duration::from_secs(2),
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            transmux::SegmentMeta {
                discontinuous: false,
            },
        )
    }

    // --- 1. ServedEgress: resolve against a Trunk-fed cache; Await is
    //        bounded; resolves once the data lands ------------------------

    /// A minimal `ServedEgress`: `Request` is a segment sequence number,
    /// `Body` is that segment's bytes. Its cache is fed by draining a real
    /// `SegmentCursor` — exactly the shape
    /// [the module docs](self#servedegressresolve-does-not-take-trunk--a-correction-found-before-step-4-the-same-way-ingresss-dial-was)
    /// describe a real implementation using, rather than reading `&Trunk`
    /// per call.
    struct FakeServedEgress {
        segments: Mutex<std::collections::HashMap<u32, Vec<u8>>>,
    }

    impl FakeServedEgress {
        fn new() -> Self {
            FakeServedEgress {
                segments: Mutex::new(std::collections::HashMap::new()),
            }
        }

        /// The write side a real driver (Step 4/5) provides by draining a
        /// `SegmentCursor` — not part of the `ServedEgress` trait itself.
        fn absorb(&self, item: SegmentCursorItem) {
            if let SegmentCursorItem::Segment(entry) = item {
                self.segments
                    .lock()
                    .unwrap()
                    .insert(entry.sequence_number, entry.bytes.to_vec());
            }
        }
    }

    impl ServedEgress for FakeServedEgress {
        type Request = u32;
        type Body = Vec<u8>;

        fn resolve(
            &self,
            request: u32,
            now: Timestamp,
            await_policy: AwaitPolicy,
        ) -> EgressResponse<Vec<u8>> {
            if let Some(bytes) = self.segments.lock().unwrap().get(&request) {
                return EgressResponse::Ready {
                    body: bytes.clone(),
                    cache: CachePolicy::Immutable,
                };
            }
            EgressResponse::pending(await_policy, now, now)
        }
    }

    /// MUTATION VERIFIED: changing `EgressResponse::pending`'s condition
    /// from `policy.expired(now)` to `false` (i.e. never expiring) makes
    /// this test's final assertion fail — the resolve at
    /// `now == deadline` (the "gave up waiting" check) returns
    /// `Await { .. }` instead of `NotFound`, so
    /// `assert!(matches!(.., EgressResponse::NotFound))` fails with the
    /// actual value being `Await { retry_no_earlier_than: .. }`. Recompiled
    /// and re-run to confirm the failure, then reverted.
    #[test]
    fn served_egress_resolves_once_populated_and_awaits_bounded_before_that() {
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();
        let mut cursor = trunk.subscribe_segments();
        let egress = FakeServedEgress::new();

        let deadline = Timestamp::from_nanos(1_000_000_000);
        let policy = AwaitPolicy::new(deadline);

        // Nothing published yet: must Await, not NotFound/Ready.
        let before = egress.resolve(1, Timestamp::from_nanos(0), policy);
        assert!(
            matches!(before, EgressResponse::Await { .. }),
            "expected Await before the segment exists, got {before:?}"
        );

        // Publish segment 1 and drain it into the fake's cache (the write
        // side a real driver provides).
        writer.publish_segment(segment_entry(1));
        while let Some(item) = cursor.poll() {
            egress.absorb(item);
        }

        // Now it resolves.
        match egress.resolve(1, Timestamp::from_nanos(0), policy) {
            EgressResponse::Ready { body, cache } => {
                assert_eq!(body, vec![1u8; 8]);
                assert_eq!(cache, CachePolicy::Immutable);
            }
            other => panic!("expected Ready once populated, got {other:?}"),
        }

        // Bounded: a request for a segment that never arrives must not
        // Await forever — once `now` reaches/passes the deadline, resolve
        // must answer something other than Await.
        let never = egress.resolve(99, deadline, policy);
        assert!(
            matches!(never, EgressResponse::NotFound),
            "expected NotFound once the deadline has passed, got {never:?}"
        );
        // And strictly before the deadline, the same never-arriving request
        // still legitimately Awaits.
        let still_waiting = egress.resolve(99, Timestamp::from_nanos(999_999_999), policy);
        assert!(
            matches!(still_waiting, EgressResponse::Await { .. }),
            "expected Await strictly before the deadline, got {still_waiting:?}"
        );
    }

    // --- 2. PushEgress: every sample from one cursor, in order ------------

    struct RecordingPushEgress {
        received: Vec<u8>,
    }

    impl PushEgress for RecordingPushEgress {
        type Error = ();

        fn negotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
            NegotiationOutcome::Accepted(TrackSelection::new(
                tracks.iter().map(|t| t.track_id).collect(),
            ))
        }

        fn renegotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
            NegotiationOutcome::Accepted(TrackSelection::new(
                tracks.iter().map(|t| t.track_id).collect(),
            ))
        }

        fn send(&mut self, item: &SampleCursorItem) -> Result<(), Self::Error> {
            // MUTATION TARGET: dropping the `Timed` arm here (e.g. matching
            // only `SampleCursorItem::Sparse`) is exactly the bug
            // `push_egress_streams_every_sample_in_order` exists to catch —
            // see that test's doc comment.
            if let SampleCursorItem::Timed { sample, .. } = item {
                self.received.push(sample.data[0]);
            }
            Ok(())
        }
    }

    /// MUTATION VERIFIED: changing `RecordingPushEgress::send`'s match arm
    /// from `SampleCursorItem::Timed` to `SampleCursorItem::Sparse` (so it
    /// silently ignores every `Timed` sample instead of recording it) makes
    /// this test fail: `assert_eq!(egress.received, vec![0, 1, 2, 3, 4])`
    /// instead sees an empty `Vec` (no `Sparse` samples were ever
    /// published), a mismatch the assertion reports directly. Recompiled and
    /// re-run to confirm the failure, then reverted.
    #[test]
    fn push_egress_streams_every_sample_in_order() {
        let trunk = Trunk::new(TrunkConfig::new(nz(100), nz(10), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();
        let mut cursor = trunk.subscribe();
        let mut egress = RecordingPushEgress {
            received: Vec::new(),
        };

        assert!(matches!(
            egress.negotiate(&[video_spec(1)]),
            NegotiationOutcome::Accepted(_)
        ));

        for i in 0u8..5 {
            writer.publish(1, crate::trunk::RetentionClass::Timed, sample(i));
        }

        // No executor anywhere: draining a cursor and calling `send` is
        // ordinary synchronous iteration.
        for _ in 0..5 {
            let item = cursor.poll().expect("sample must be ready synchronously");
            egress.send(&item).unwrap();
        }
        assert!(cursor.poll().is_none(), "nothing left to drain");

        assert_eq!(egress.received, vec![0, 1, 2, 3, 4]);
    }

    // --- 3. PushEgress::renegotiate: an added track is expressible, and a
    //        refusing output reports why rather than dropping silently ----

    /// An output that has already "answered SDP" (`committed = true`) and
    /// must refuse any renegotiation that changes the track set — the WHEP/
    /// DASH-published-`Period` case from
    /// [the module docs](self#renegotiate-and-issue-781-a-track-addition-must-be-expressible-and-a-refusal-must-be-truthful).
    struct WhepLikePushEgress {
        selection: TrackSelection,
        committed: bool,
    }

    impl PushEgress for WhepLikePushEgress {
        type Error = ();

        fn negotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
            self.selection = TrackSelection::new(tracks.iter().map(|t| t.track_id).collect());
            self.committed = true;
            NegotiationOutcome::Accepted(self.selection.clone())
        }

        fn renegotiate(&mut self, tracks: &[TrackSpec]) -> NegotiationOutcome<Self::Error> {
            let proposed: Vec<u32> = tracks.iter().map(|t| t.track_id).collect();
            if self.committed && proposed != self.selection.track_ids {
                // MUTATION TARGET: returning `NegotiationOutcome::Accepted`
                // here instead (silently adopting `proposed` without
                // reporting anything) is exactly the silent-drop failure
                // mode issue #781 already demonstrates on the ingest side —
                // see `renegotiate_refusal_is_reported_not_silently_dropped`.
                return NegotiationOutcome::Refused {
                    reason: "SDP already answered; cannot add a track mid-session",
                };
            }
            self.selection = TrackSelection::new(proposed);
            NegotiationOutcome::Accepted(self.selection.clone())
        }

        fn send(&mut self, _item: &SampleCursorItem) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn renegotiate_expresses_a_mid_stream_track_addition() {
        // An output that has NOT yet committed (e.g. still mid-negotiation)
        // accepts the addition.
        let mut egress = WhepLikePushEgress {
            selection: TrackSelection::default(),
            committed: false,
        };
        match egress.negotiate(&[video_spec(1)]) {
            NegotiationOutcome::Accepted(sel) => assert_eq!(sel.track_ids, vec![1]),
            other => panic!("expected Accepted, got {other:?}"),
        }
        egress.committed = false; // still free to accept the next change
        match egress.renegotiate(&[video_spec(1), video_spec(2)]) {
            NegotiationOutcome::Accepted(sel) => {
                assert_eq!(
                    sel.track_ids,
                    vec![1, 2],
                    "the added track (2) must reach the selection"
                )
            }
            other => panic!("expected Accepted with the added track, got {other:?}"),
        }
    }

    /// MUTATION VERIFIED: changing `WhepLikePushEgress::renegotiate`'s
    /// refusal branch to return `NegotiationOutcome::Accepted(self.selection.clone())`
    /// with `proposed` silently discarded (simulating a silent drop) makes
    /// this test's `matches!(.., NegotiationOutcome::Refused { .. })`
    /// assertion fail — the outcome is `Accepted` instead, with no `reason`
    /// anywhere a caller could observe. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn renegotiate_refusal_is_reported_not_silently_dropped() {
        let mut egress = WhepLikePushEgress {
            selection: TrackSelection::default(),
            committed: false,
        };
        egress.negotiate(&[video_spec(1)]);
        assert!(egress.committed);

        match egress.renegotiate(&[video_spec(1), video_spec(2)]) {
            NegotiationOutcome::Refused { reason } => {
                assert!(!reason.is_empty(), "a refusal must carry a real reason");
            }
            other => panic!(
                "expected a truthful Refused once already committed, got {other:?} \
                 (a silent Accepted here would be exactly issue #781's failure mode)"
            ),
        }
        // The output's own selection is unchanged by the refused proposal.
        assert_eq!(egress.selection.track_ids, vec![1]);
    }

    // --- 4. SegmentEgress: every segment while pinning; ArchiveOverrun
    //        still governs (3b-ii reused, not reimplemented) --------------

    struct DvrWriter {
        written: Vec<u32>,
        holes: Vec<u64>,
    }

    impl SegmentEgress for DvrWriter {
        type Error = ();

        fn on_segment(&mut self, item: &SegmentCursorItem) -> Result<(), Self::Error> {
            match item {
                SegmentCursorItem::Segment(entry) => self.written.push(entry.sequence_number),
                // MUTATION TARGET: dropping this arm (not recording `Gap`)
                // is exactly the "DVR silently has a hole nobody is told
                // about" bug this test's mutation proves is caught.
                SegmentCursorItem::Gap { skipped } => self.holes.push(*skipped),
                SegmentCursorItem::Lagged { .. } | SegmentCursorItem::Terminated => {}
            }
            Ok(())
        }
    }

    /// MUTATION VERIFIED: removing the `SegmentCursorItem::Gap` arm from
    /// `DvrWriter::on_segment` (folding it into the no-op
    /// `Lagged | Terminated` arm, so a `Gap` report is silently swallowed)
    /// makes this test's `assert_eq!(writer.holes, vec![1])` fail — `holes`
    /// stays empty instead of recording the one segment `ArchiveOverrun::Gap`
    /// dropped. Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn segment_egress_receives_every_pinned_segment_and_archive_overrun_still_governs() {
        // Capacity 2: publishing 3 segments while pinned overflows by
        // exactly 1, forcing `ArchiveOverrun::Gap` (the default) to fire.
        let trunk = Trunk::new(TrunkConfig::new(nz(10), nz(10), nz(2), nz(8), nz(8)));
        let writer_handle = trunk.writer().unwrap();
        let mut cursor = trunk.pin_segments(ArchiveOverrun::Gap);

        writer_handle.publish_segment(segment_entry(1));
        writer_handle.publish_segment(segment_entry(2));
        writer_handle.publish_segment(segment_entry(3)); // evicts seq 1's pin

        let mut writer = DvrWriter {
            written: Vec::new(),
            holes: Vec::new(),
        };
        while let Some(item) = cursor.poll() {
            writer.on_segment(&item).unwrap();
        }

        assert_eq!(
            writer.holes,
            vec![1],
            "ArchiveOverrun::Gap must still report exactly the one evicted, \
             un-consumed segment as a hole"
        );
        assert_eq!(
            writer.written,
            vec![2, 3],
            "the two segments that survived the overrun must both reach the writer"
        );
    }

    // --- 5. No trait requires an executor: everything above already ran
    //        entirely synchronously (no `#[tokio::test]`, no `.await`) ----
    #[test]
    fn every_egress_trait_method_is_callable_without_an_executor() {
        // This test's only assertion is its own existence and the fact it
        // is a plain `#[test]`, not `#[tokio::test]`/`#[async_std::test]`:
        // every call above (`resolve`, `negotiate`, `renegotiate`, `send`,
        // `on_segment`) is an ordinary synchronous function call. If any of
        // `ServedEgress`/`PushEgress`/`SegmentEgress`'s methods required an
        // executor, this whole module would fail to compile as a plain
        // `#[test]` fn (no `async fn` bodies, no `.await` anywhere above).
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(4), nz(8)));
        assert!(trunk.writer().is_some());
    }
}
