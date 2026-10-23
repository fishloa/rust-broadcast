use alloc::format;
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

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Demux(a), Self::Demux(b)) => format!("{a:?}") == format!("{b:?}"),
            (Self::StallNotice { secs: a }, Self::StallNotice { secs: b }) => a == b,
            (Self::Starved(a), Self::Starved(b)) => a == b,
            (Self::NotFound { status: a }, Self::NotFound { status: b }) => a == b,
            (Self::Connected, Self::Connected) => true,
            (Self::Finite { expected_bytes: a }, Self::Finite { expected_bytes: b }) => a == b,
            (Self::Ended, Self::Ended) => true,
            (Self::Reconnecting { attempt: a }, Self::Reconnecting { attempt: b }) => a == b,
            _ => false,
        }
    }
}

impl Eq for Event {}

#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
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

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let e = EventSerde::deserialize(deserializer)?;
        Ok(match e {
            EventSerde::StallNotice { secs } => Self::StallNotice { secs },
            EventSerde::Starved(k) => Self::Starved(k),
            EventSerde::NotFound { status } => Self::NotFound { status },
            EventSerde::Connected => Self::Connected,
            EventSerde::Finite { expected_bytes } => Self::Finite { expected_bytes },
            EventSerde::Ended => Self::Ended,
            EventSerde::Reconnecting { attempt } => Self::Reconnecting { attempt },
        })
    }
}
