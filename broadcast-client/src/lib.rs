#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub(crate) mod auth;
mod buffer;
mod cookies;
pub mod error;

/// Sans-IO fetch cores (retry/backoff policy and helpers).
pub mod fetch {
    pub use crate::source::fetch::retry::retry_after_us;
}
pub mod events;
mod headers;
pub mod identity;
mod join;
pub mod playout;
mod queue;
pub(crate) mod redirect;
mod rendered;
pub mod session;
pub mod source;
pub mod srt;
pub mod timeline;
pub mod tshttp;
pub mod udp;

pub use crate::tshttp::StarveKind;
pub use broadcast_auth::Credentials;
pub use buffering_runtime::Hint;
pub use buffering_runtime::Hint as BufferHint;
pub use buffering_runtime::input_log::{Input as BufferInput, InputLog};
pub use dash_runtime::client::config::{DashClockPolicy, DashConfig};
pub use dash_runtime::timing::clock::ClockSource;
pub use error::{Error, TransportError};
pub use events::{
    AUDIO_TRACK_ID_OFFSET, Action, AvSync, BufferEvent, BufferStats, Config, ErrorKind, Event,
    MAX_SERVER_REFUSED_REASON_BYTES, PauseReason, ResumeMode, SourceKind, Status, StreamKind,
};
pub use identity::{ClientIdentity, ClientIdentityScope};
pub use playout::PlayoutConfig;
pub use session::Session;
pub use source::ts_http::TsHttpSource;
pub use srt::{SrtConfig, SrtEncryption, SrtEntropySeed, SrtKeyLength};
pub use timeline::TimelineJumpConfig;
pub use transmux::{DisplayInfo, Sample, TrackId, TrackSpec};
pub use udp::{DatagramStats, MulticastJoin, UdpConfig, UdpMode};

/// Re-exported connection identifier.
pub type ConnId = u32;
