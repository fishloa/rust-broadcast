//! Sans-IO linear channel playout — issue
//! [#748](https://github.com/fishloa/rust-broadcast/issues/748).
//!
//! A schedule model for assembling a linear channel from an ordered list of
//! sources ([`schedule`]), the transition planning that turns "what's next"
//! into a concrete timeline-continuity plan ([`transition`]), and the
//! SCTE-35 `splice_insert()` emission points a transition implies
//! ([`scte35`]).
//!
//! # What it does / what it does not
//!
//! **Does:** finds the next join a schedule implies and computes it (channel
//! instant, PTS-rebase offset, discontinuity flag), and builds the
//! `splice_insert()` for entering or returning from a break, conditioned against
//! real candidate boundaries.
//!
//! **Does not:**
//!
//! - **No transcoding or conforming.** A differing codec config is a
//!   discontinuity to *signal* ([`transition::TransitionPlan::discontinuity`]).
//! - **No sample-timestamp rewriting.** `transmux`'s IR transform would apply
//!   [`transition::TransitionPlan::pts_rebase_offset`]; this crate computes the
//!   number. `broadcast_hls::mark_init_discontinuities` would act on the
//!   discontinuity flag; neither is a dependency here.
//! - **No boundary conditioning of its own:** [`scte35::build_splice_insert`]
//!   calls `ssai_runtime::splice::condition_splice_point_wrapping`.
//! - **No schedule wire format, no SCTE 224** (it covers policy and blackout
//!   signalling, not channel assembly): [`schedule::Schedule`] is an in-memory
//!   model with no `Parse`/`Serialize` pair.
//! - **No HTTP, no tokio**, no real-time clock; a `multimux` adapter is future
//!   work.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | ANSI/SCTE 35 | 2023r1 | `splice_insert()` (section 9.7.3, Table 10) via `scte35-splice`; clear `splice_info_section` wrapper | Partial: `splice_insert` only (no `time_signal`, segmentation descriptors, encryption, components); 90 kHz ticks |
//! | schedule model | n/a | programme / ad / slate | Not a standard (SCTE 224 not adopted) |
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | yes | Links `std`; off gives `#![no_std]` + `alloc` |
//! | `serde` | no | `serde` derives on the public types |
//!
//! # Quick start
//!
//! ```rust
//! use playout_runtime::{
//!     BreakEdge, CodecConfigId, EntryKind, Schedule, ScheduleEntry, build_splice_insert,
//!     next_transition,
//! };
//!
//! let mut schedule = Schedule::new();
//! schedule
//!     .push(ScheduleEntry {
//!         id: "news".into(),
//!         kind: EntryKind::Programme,
//!         planned_start: 0,
//!         source_start_pts: 0,
//!         codec_config: CodecConfigId(1),
//!     })
//!     .unwrap();
//! // A 30 s ad at 10 minutes (90 kHz ticks), cut from an asset whose own clock
//! // starts at 5_000, with a different codec configuration.
//! schedule
//!     .push(ScheduleEntry {
//!         id: "ad".into(),
//!         kind: EntryKind::Ad,
//!         planned_start: 54_000_000,
//!         source_start_pts: 5_000,
//!         codec_config: CodecConfigId(2),
//!     })
//!     .unwrap();
//!
//! // The join: when, how to rebase the incoming clock, and whether to signal a
//! // discontinuity.
//! let t = next_transition(&schedule, 0).unwrap();
//! assert_eq!(t.plan.at_pts, 54_000_000);
//! assert!(t.plan.discontinuity);
//! assert_eq!(t.plan.rebase(5_000), Some(54_000_000));
//!
//! // The splice_insert() that entering the break implies, snapped to the nearest
//! // real boundary (2_000 ticks before the nominal instant beats 3_000 after).
//! let (point, insert) = build_splice_insert(
//!     BreakEdge::Enter,
//!     1,
//!     t.plan.at_pts,
//!     &[53_998_000, 54_003_000],
//!     5_000,
//!     Some(2_700_000),
//! )
//! .unwrap();
//! assert_eq!(point.snapped_pts, 53_998_000);
//! assert!(insert.out_of_network_indicator);
//! ```
//!
//! # What is implemented
//!
//! - [`schedule`]: [`Schedule`] of [`ScheduleEntry`] ([`EntryKind`],
//!   [`CodecConfigId`]).
//! - [`transition`]: [`next_transition`] returns a [`PlannedTransition`];
//!   [`TransitionPlan::plan`] computes `at_pts`, `pts_rebase_offset` (`i128`,
//!   never clamped) and `discontinuity`; [`TransitionPlan::rebase`] returns
//!   `None` rather than wrapping, and [`TransitionPlan::rebase_wrapped`] unrolls a
//!   raw 33-bit PTS first.
//! - [`scte35`]: [`build_splice_insert`] (circular distances modulo 2^33) and
//!   [`to_section`], with [`BreakEdge`] naming the break edge.
//!
//! # Design notes
//!
//! Sans-IO, `no_std` + `alloc`, `#![forbid(unsafe_code)]`, structured [`Error`]s.
//! "A schedule that plays the right thing at the wrong timestamp is worse than no
//! scheduler" — timeline correctness is the primary tested property (including a
//! real, non-IDR-aligned cue from `fixtures/scte35-ssai/`). Units are 90 kHz ticks.
//!
//! # Related crates
//!
//! Depends on `broadcast-common`, `ssai-runtime` and `scte35-splice`. MSRV 1.95.0;
//! MIT OR Apache-2.0.
//!
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod error;
pub mod schedule;
pub mod scte35;
pub mod transition;

pub use error::{Error, Result};
pub use schedule::{CodecConfigId, EntryKind, Schedule, ScheduleEntry};
pub use scte35::{BreakEdge, ConditionedSplicePoint, build_splice_insert, to_section};
pub use transition::{PlannedTransition, TransitionPlan, next_transition};
