//! `media-plane` — the media-plane integration layer.
//!
//! The workspace's ingress/egress architecture
//! (`docs/superpowers/specs/2026-07-26-media-plane-architecture.md` in the
//! `rust-broadcast` repository) is four layers, not one pipeline glued
//! together per protocol:
//!
//! ```text
//! Dialer|Listener ──► [ByteStage]* ──► IngestSession ──► [IrTransform]* ──► TrunkWriter
//!    (N sources)       byte→byte          demux              IR→IR                │
//!                                                                                  ▼
//!                                                                   ┌──────── Trunk ────────┐
//!                                                                   │ sample ring           │
//!                                                                   │ segment log           │
//!                                                                   │ EVENT log (90 kHz)    │
//!                                                                   └───────────────────────┘
//!                          subscribe() ─► SampleCursor  ─► PushEgress    (WHEP, RTMP-out, SRT-out)
//!                          subscribe() ─► SegmentCursor ─► SegmentEgress (DVR, MABR, ROUTE, Smooth)
//!                          resolve()   ─────────────────► ServedEgress   (LL-HLS, DASH, catch-up)
//! ```
//!
//! `media-plane` is where that shape lives in code: it is the crate that ties
//! ingress, the byte layer, container demux (`transmux`), IR transforms, the
//! `Trunk`, and the three egress shapes together into one runnable pipeline.
//! It depends on `broadcast-common` for the shared drive contract
//! ([`broadcast_common::Stage`]) and clock/backpressure types
//! ([`broadcast_common::Timestamp`], [`broadcast_common::Demand`]).
//!
//! **This release (plan steps 3a-ii through 3b-iii) completes the byte
//! layer and the whole `Trunk`**: [`ByteStage`], [`ByteTap`] (a
//! non-blocking observer), [`ByteMerge`] (the one bounded multi-input
//! primitive), [`Trunk`]/[`TrunkWriter`]/[`SampleCursor`] (the bounded,
//! dual-retention sample ring), the segment log with [`SegmentCursor`] and
//! [`ArchiveOverrun`] (lossless-by-retention pinning for a DVR/archive
//! consumer, without the writer ever blocking by default), and now the
//! 90 kHz event log with [`EventCursor`] and [`EventAnchor`] — the piece
//! that resolves architecture-audit finding B1 (rev 1's false claim of one
//! time model for every event; see the [`trunk`] module docs' event-log
//! section). `Dialer`/`Listener`/`IngestSession` and the three egress
//! traits are later steps of the same plan
//! (`docs/superpowers/plans/2026-07-26-media-plane-implementation.md`
//! Step 3c onward) and are deliberately absent here.
//!
//! # The byte layer ([`byte_stage`], [`byte_tap`], [`byte_merge`])
//!
//! A byte stage is pre-demux, byte-to-byte, deadline-driven work: CAM
//! descramble, TS continuity/PCR repair, T2-MI/BBFrame inner-TS recovery,
//! program-PID filtering. See the [`byte_stage`] module docs for why it is
//! defined as a `Stage` specialisation rather than a second trait, and for the
//! exact form that was validated to compile.
//!
//! [`ByteTap`] sits alongside the byte stages, not in their chain: a
//! non-blocking positional observer that lets analysis (`dvb-conformance`,
//! `media-doctor watch`, #737's T-STD) see bytes a demuxer would reject. See
//! the [`byte_tap`] module docs for the non-blocking/`Lagged` trade and why it
//! is not a `Stage`.
//!
//! [`ByteMerge`] is the one place `N` byte sources reduce to one stream —
//! everything above the byte layer stays strictly single-input. See the
//! [`byte_merge`] module docs for why it operates on discrete messages, its
//! two policies, and why ST 2022-7 hitless switching is deliberately absent
//! rather than stubbed.
//!
//! # `Trunk`, the writer, and the cursors ([`trunk`], `std`-only)
//!
//! Above the byte layer and demux sits [`Trunk`]: the bounded sample ring and
//! segment log one [`TrunkWriter`] publishes into and any number of
//! [`SampleCursor`]/[`SegmentCursor`]s read from. It requires the `std`
//! feature (`Arc`/`Mutex`/`Condvar` for cross-thread sharing) — see the
//! [`trunk`] module docs for why that is the right line to draw rather than
//! reaching for a `no_std` spinlock crate, the benchmark
//! (`spikes/trunk-bench`) that shaped the design, and — critically, before
//! calling [`Trunk::subscribe`]/[`Trunk::subscribe_segments`] once per
//! connection — why supported reader count is single-digit by design.
//!
//! The segment log resolves a real contradiction: a DVR/archive consumer
//! must never miss a segment, but the writer must never block. See the
//! [`trunk`] module docs' "DVR contradiction" section for why the answer is
//! retention (a pinning cursor from [`Trunk::pin_segments`]), not
//! back-pressure, and for the three-way [`ArchiveOverrun`] trade a pinning
//! cursor's caller makes explicit when the retention bound is finally hit.
//!
//! The event log carries [`timed_metadata::TimedEvent`] on the trunk's own
//! 90 kHz absolute clock, addressable both by media time
//! ([`Trunk::events_between`]) and by segment
//! ([`Trunk::events_in_segment`]) — and, critically, never fabricates a
//! media time for an event that is only segment-relative (`emsg` v0) or
//! wall-clock-only (SCTE-35 `splice_schedule`) until the boundary or
//! [`timed_metadata::TimeAnchor`] it actually needs arrives. See the
//! [`trunk`] module docs' event-log section for the full B1 story.

#![cfg_attr(not(feature = "std"), no_std)]
#![doc(html_root_url = "https://docs.rs/media-plane")]

extern crate alloc;

pub mod byte_merge;
pub mod byte_stage;
pub mod byte_tap;
#[cfg(feature = "std")]
pub mod egress;
#[cfg(feature = "std")]
pub mod ingress;
#[cfg(feature = "std")]
pub mod trunk;

pub use byte_merge::{ByteMerge, MergeError, MergePolicy, SourceId};
pub use byte_stage::ByteStage;
pub use byte_tap::{ByteTap, TapItem, TapPoint};
#[cfg(feature = "std")]
pub use egress::{
    AwaitPolicy, CachePolicy, EgressResponse, NegotiationOutcome, PushEgress, SegmentEgress,
    ServedEgress, TrackSelection,
};
#[cfg(feature = "std")]
pub use ingress::{
    AcceptOutcome, DialAttempt, DialSupervisor, Dialer, HandshakePolicy, HealthState, IngestDriver,
    IngestSession, ListenDriver, Listener, ProgramId, ReconnectPolicy, SessionEvent, SessionId,
    run_dial, run_listen,
};
#[cfg(feature = "std")]
pub use trunk::{
    ArchiveOverrun, EventAnchor, EventCursor, EventCursorItem, EventEntry, RetentionClass,
    SampleCursor, SampleCursorItem, SegmentCursor, SegmentCursorItem, SegmentEntry, Trunk,
    TrunkConfig, TrunkWriter,
};
