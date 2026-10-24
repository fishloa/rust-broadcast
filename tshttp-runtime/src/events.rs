use alloc::string::String;

use transmux::DemuxEvent;

/// Host action produced by the session (T1, T6, T8, T9, T12, T13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Action {
    /// Open (or reopen) the configured URL.
    Open {
        /// URL to open.
        url: String,
    },
    /// Wait then reopen after `after_ms`.
    Reconnect {
        /// Delay before reopen.
        after_ms: u64,
    },
    /// Drop the current connection.
    Abort,
}

/// Starvation classification (T14, T15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum StarveKind {
    /// Bytes flowing but no sample.
    Data,
    /// No bytes.
    NoData,
}

impl StarveKind {
    /// Spec label for this kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::NoData => "no-data",
        }
    }
}

broadcast_common::impl_spec_display!(StarveKind);

/// Session event (T2-T15).
///
/// Neither `PartialEq` nor `Deserialize` is implemented: [`DemuxEvent`]
/// (transmux) has no structural equality and no serde support, so an `Event`
/// carrying one can be neither compared nor reconstructed. With the `serde`
/// feature an `Event` serializes only; [`Event::Demux`] serializes as the
/// bare variant tag, without its payload.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum Event {
    /// Demuxer event (T5, T14).
    Demux(DemuxEvent),
    /// Stall notice (T11).
    StallNotice {
        /// Stalled seconds.
        secs: u64,
    },
    /// Starvation (T14, T15).
    Starved(StarveKind),
    /// Not found response (T3).
    NotFound {
        /// HTTP status.
        status: u16,
    },
    /// Connected (T2).
    Connected,
    /// Finite resource: expected body byte count (VOD).
    Finite {
        /// Expected body bytes.
        expected_bytes: u64,
    },
    /// Stream ended cleanly (VOD).
    Ended,
    /// Reconnecting (T9).
    Reconnecting {
        /// Attempt number.
        attempt: u64,
    },
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize)]
enum EventSerde {
    StallNotice { secs: u64 },
    Starved(StarveKind),
    NotFound { status: u16 },
    Connected,
    Finite { expected_bytes: u64 },
    Ended,
    Reconnecting { attempt: u64 },
}

#[cfg(feature = "serde")]
impl serde::Serialize for Event {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Demux(_) => serializer.serialize_unit_variant("Event", 0, "Demux"),
            Self::StallNotice { secs } => {
                EventSerde::StallNotice { secs: *secs }.serialize(serializer)
            }
            Self::Starved(k) => EventSerde::Starved(*k).serialize(serializer),
            Self::NotFound { status } => {
                EventSerde::NotFound { status: *status }.serialize(serializer)
            }
            Self::Connected => EventSerde::Connected.serialize(serializer),
            Self::Finite { expected_bytes } => EventSerde::Finite {
                expected_bytes: *expected_bytes,
            }
            .serialize(serializer),
            Self::Ended => EventSerde::Ended.serialize(serializer),
            Self::Reconnecting { attempt } => {
                EventSerde::Reconnecting { attempt: *attempt }.serialize(serializer)
            }
        }
    }
}
