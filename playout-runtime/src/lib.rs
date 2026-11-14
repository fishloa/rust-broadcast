#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod error;
pub mod schedule;
pub mod scte35;
pub mod transition;

pub use error::{Error, Result};
pub use schedule::{CodecConfigId, EntryKind, Playhead, Schedule, ScheduleEntry};
pub use scte35::{BreakEdge, ConditionedSplicePoint, build_splice_insert, to_section};
pub use transition::{PlannedTransition, TransitionPlan, next_transition};
