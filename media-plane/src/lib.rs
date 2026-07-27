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
//! **This release (plan steps 3a-ii and 3b-i) completes the byte layer and
//! the `Trunk`'s sample path**: [`ByteStage`], [`ByteTap`] (a non-blocking
//! observer), [`ByteMerge`] (the one bounded multi-input primitive), and now
//! [`Trunk`]/[`TrunkWriter`]/[`SampleCursor`] (the bounded, dual-retention
//! sample ring). The segment log, the 90 kHz event log,
//! `Dialer`/`Listener`/`IngestSession`, and the three egress traits are later
//! steps of the same plan (`docs/superpowers/plans/2026-07-26-media-plane-implementation.md`
//! Step 3b-ii onward) and are deliberately absent here.
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
//! # `Trunk`, the writer, and the sample cursor ([`trunk`], `std`-only)
//!
//! Above the byte layer and demux sits [`Trunk`]: the bounded sample ring one
//! [`TrunkWriter`] publishes into and any number of [`SampleCursor`]s read
//! from. It requires the `std` feature (`Arc`/`Mutex` for cross-thread
//! sharing) — see the [`trunk`] module docs for why that is the right line to
//! draw rather than reaching for a `no_std` spinlock crate, the benchmark
//! (`spikes/trunk-bench`) that shaped the design, and — critically, before
//! calling [`Trunk::subscribe`] once per connection — why supported reader
//! count is single-digit by design.

#![cfg_attr(not(feature = "std"), no_std)]
#![doc(html_root_url = "https://docs.rs/media-plane")]

extern crate alloc;

pub mod byte_merge;
pub mod byte_stage;
pub mod byte_tap;
#[cfg(feature = "std")]
pub mod trunk;

pub use byte_merge::{ByteMerge, MergeError, MergePolicy, SourceId};
pub use byte_stage::ByteStage;
pub use byte_tap::{ByteTap, TapItem, TapPoint};
#[cfg(feature = "std")]
pub use trunk::{RetentionClass, SampleCursor, SampleCursorItem, Trunk, TrunkConfig, TrunkWriter};
