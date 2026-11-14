#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

extern crate alloc;

mod buffer;
pub mod error;

/// Sans-IO fetch cores (retry/backoff policy and helpers).
pub mod fetch {
    pub use crate::source::fetch::retry::{HEALTHY_CONNECTION_US, ReconnectTimer, retry_after_us};
    pub use crate::source::fetch::wall::WallClock;
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
pub(crate) mod tshttp;
pub mod udp;

pub use broadcast_auth::Credentials;
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
pub use srt::{SrtCallerConfig, SrtEncryption, SrtEntropySeed, SrtKeyLength};
pub use transmux::TimelineJumpConfig;
pub use transmux::{DisplayInfo, Sample, TrackId, TrackSpec};
pub use tshttp::StarveKind;
pub use tshttp::TsHttpConfig;
pub use udp::{DatagramStats, UdpConfig, UdpMode};

/// Connection identifier, naming one host-side socket the session's actions
/// and callbacks refer to.
///
/// A newtype over `u32` rather than an alias, so a connection id cannot be
/// passed where a track id, a sequence number or any other `u32` belongs (and
/// vice versa). [`Session`] hands them out via its actions; the host feeds one
/// back to [`Session::on_connected`]/[`Session::on_data`]/
/// [`Session::on_closed`]/[`Session::on_response`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ConnId(pub u32);

impl ConnId {
    /// The plain `u32` this identifier wraps, for a host that stores it in a
    /// numeric registry of its own.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Build an identifier from a plain `u32` — for a host replaying values it
    /// persisted, or a test constructing a specific id.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

impl core::fmt::Display for ConnId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::ConnId;

    #[test]
    fn a_connection_id_is_a_distinct_type_over_a_plain_u32() {
        // a connection id is not interchangeable with a track id or any other u32
        assert_eq!(ConnId::new(7).get(), 7);
        assert_eq!(ConnId::new(7).to_string(), "7");
        assert_eq!(ConnId::new(3), ConnId::new(3));
        assert_ne!(ConnId::new(3), ConnId::new(4));
        // copy, hash and ord work, so a host can key a registry by connection id
        let a = ConnId::new(9);
        let b = a;
        assert_eq!(a, b);
        assert!(ConnId::new(1) < ConnId::new(2));
        assert_eq!(core::mem::size_of::<ConnId>(), core::mem::size_of::<u32>());
    }
}
