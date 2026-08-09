//! Sans-IO linear channel playout — issue
//! [#748](https://github.com/fishloa/rust-broadcast/issues/748).
//!
//! A schedule model for assembling a linear channel from an ordered list of
//! sources ([`schedule`]), the transition planning that turns "what's next"
//! into a concrete timeline-continuity plan ([`transition`]), and the
//! SCTE-35 `splice_insert()` emission points a transition implies
//! ([`scte35`]).
//!
//! ## Design decisions (issue #748's design-decision comment settles these)
//!
//! - **The schedule format is ours to define.** There is no applicable open
//!   standard for "ordered channel-assembly playlist" to adopt: SCTE 224
//!   (ESNI) covers policy/blackout signalling, not channel assembly, and is
//!   paywalled besides. [`schedule::Schedule`] is therefore a plain in-memory
//!   model with no `Parse`/`Serialize` pair — there is no wire format to be
//!   symmetric about.
//! - **This crate builds on `ssai-runtime` rather than duplicating it.**
//!   `ssai_runtime::splice::condition_splice_point` already owns
//!   nearest-boundary splice-point conditioning (with an explicit tolerance,
//!   refusing rather than silently snapping when nothing is close enough);
//!   [`scte35::build_splice_insert`] calls it rather than re-implementing
//!   the same nearest-boundary math a second time. This crate's own job is
//!   deciding *when* a transition happens (the schedule); `ssai-runtime`'s
//!   is deciding *how the splice lands* against real boundaries.
//! - **The hard part is the transition, not the schedule.** Joining two
//!   sources means timeline continuity: a PTS-rebase offset
//!   ([`transition::TransitionPlan::rebase`]) so the incoming source's own
//!   clock lands continuously on the shared channel timeline, and a
//!   discontinuity flag ([`transition::TransitionPlan::discontinuity`]) when
//!   the codec configuration changes across the join. "A schedule that plays
//!   the right thing at the wrong timestamp is worse than no scheduler" —
//!   this crate's tests hold timeline correctness as the primary property,
//!   not scheduling logic.
//!
//! ## What this crate is **not**
//!
//! - **No transcoding or conforming.** A differing codec config across a
//!   transition is a discontinuity to *signal*
//!   ([`transition::TransitionPlan::discontinuity`]), never something this
//!   crate re-encodes. This workspace parses containers and never touches
//!   the codec bitstream, and that holds here too.
//! - **No actual sample-timestamp rewriting.** `transmux`'s IR transform is
//!   what would apply [`transition::TransitionPlan::pts_rebase_offset`] to
//!   real sample PTS/DTS values; this crate computes the number, not the
//!   rewrite. Likewise `broadcast_hls::mark_init_discontinuities` is what
//!   would act on [`transition::TransitionPlan::discontinuity`] at the
//!   playlist/init-segment level — neither dependency is pulled in here.
//! - **No HTTP, no tokio.** A `multimux` adapter driving a real channel
//!   clock against this crate's planning is deliberately future work, not
//!   this crate — mirroring the `rtsp-runtime`/`hls-runtime` and
//!   `ssai-runtime` split the rest of the workspace uses.
//!
//! `no_std` + `alloc`.

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
