#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub mod auth;
mod buffer;
pub mod error;
pub mod events;
pub mod redirect;
pub mod session;
pub mod source;

pub use broadcast_auth::Credentials;
pub use buffering_runtime::Hint;
pub use buffering_runtime::Hint as BufferHint;
pub use error::{Error, TransportError};
pub use events::{Action, BufferEvent, Config, Event, SourceKind, Status};
pub use session::Session;
pub use source::ts_http::TsHttpSource;
pub use transmux::{DisplayInfo, Sample, TrackId, TrackSpec};
pub use tshttp_runtime::StarveKind;

/// Re-exported connection identifier.
pub type ConnId = u32;
