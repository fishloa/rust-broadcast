#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub mod auth;
pub mod error;
pub mod events;
pub mod redirect;
pub mod session;
pub mod source;

pub use auth::SessionOrigin;
pub use broadcast_auth::Credentials;
pub use error::{Error, TransportError};
pub use events::{Action, Config, Event, SourceKind, Status};
pub use session::Session;
pub use source::ts_http::TsHttpSource;
pub use transmux::{DisplayInfo, Sample, TrackId, TrackSpec};
pub use tshttp_runtime::StarveKind;

/// Re-exported connection identifier.
pub type ConnId = u32;
