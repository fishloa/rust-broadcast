#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod anchor;
pub mod convert;
pub mod daterange;
pub mod error;
pub mod event;
pub mod timeline;
pub mod webvtt;

pub use anchor::TimeAnchor;
pub use broadcast_common::clock33::PTS_HZ;
pub use daterange::DateRange;
pub use error::{Error, Result};
pub use event::{EventKind, MediaDuration, MediaTime, SourcePayload, TimedEvent};
pub use timeline::Timeline;
