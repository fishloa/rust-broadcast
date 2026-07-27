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
//! **This release (0.1.0, plan step 3a-i) delivers only the byte-stage
//! piece** — [`ByteStage`] and nothing else. `ByteTap`/`ByteMerge`, `Trunk` +
//! cursors, `Dialer`/`Listener`/`IngestSession`, and the three egress traits
//! are later steps of the same plan (`docs/superpowers/plans/2026-07-26-media-plane-implementation.md`
//! Step 3b onward) and are deliberately absent here.
//!
//! # The byte layer ([`byte_stage`])
//!
//! A byte stage is pre-demux, byte-to-byte, deadline-driven work: CAM
//! descramble, TS continuity/PCR repair, T2-MI/BBFrame inner-TS recovery,
//! program-PID filtering. See the [`byte_stage`] module docs for why it is
//! defined as a `Stage` specialisation rather than a second trait, and for the
//! exact form that was validated to compile.

#![cfg_attr(not(feature = "std"), no_std)]
#![doc(html_root_url = "https://docs.rs/media-plane")]

extern crate alloc;

pub mod byte_stage;

pub use byte_stage::ByteStage;
