#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub(crate) mod auth;
mod buffer;
mod cookies;
pub mod error;
pub mod events;
mod headers;
mod join;
pub mod playout;
mod queue;
pub(crate) mod redirect;
mod rendered;
pub mod session;
pub mod source;
pub mod timeline;

pub use broadcast_auth::Credentials;
#[cfg(feature = "__test-seams")]
#[doc(hidden)]
pub use buffer::BufferInput;
pub use buffering_runtime::Hint;
pub use buffering_runtime::Hint as BufferHint;
pub use dash_runtime::client::config::{DashClockPolicy, DashConfig};
pub use dash_runtime::timing::clock::ClockSource;
pub use error::{Error, TransportError};
pub use events::{
    AUDIO_TRACK_ID_OFFSET, Action, AvSync, BufferEvent, BufferStats, Config, ErrorKind, Event,
    MAX_SERVER_REFUSED_REASON_BYTES, PauseReason, ResumeMode, SourceKind, Status, StreamKind,
};
pub use playout::PlayoutConfig;
pub use session::Session;
pub use source::ts_http::TsHttpSource;
pub use timeline::TimelineJumpConfig;
pub use transmux::{DisplayInfo, Sample, TrackId, TrackSpec};
pub use tshttp_runtime::StarveKind;

/// Re-exported connection identifier.
pub type ConnId = u32;
