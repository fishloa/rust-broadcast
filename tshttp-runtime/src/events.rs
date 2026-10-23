use alloc::format;
use alloc::string::String;
use transmux::DemuxEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Action {
    Open { url: String },
    Reconnect { after_ms: u64 },
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum StarveKind {
    Data,
    NoData,
}

impl StarveKind {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::NoData => "no-data",
        }
    }
}

broadcast_common::impl_spec_display!(StarveKind);

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum Event {
    Demux(DemuxEvent),
    StallNotice { secs: u64 },
    Starved(StarveKind),
    NotFound { status: u16 },
    Connected,
    Reconnecting { attempt: u64 },
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Demux(a), Self::Demux(b)) => format!("{a:?}") == format!("{b:?}"),
            (Self::StallNotice { secs: a }, Self::StallNotice { secs: b }) => a == b,
            (Self::Starved(a), Self::Starved(b)) => a == b,
            (Self::NotFound { status: a }, Self::NotFound { status: b }) => a == b,
            (Self::Connected, Self::Connected) => true,
            (Self::Reconnecting { attempt: a }, Self::Reconnecting { attempt: b }) => a == b,
            _ => false,
        }
    }
}

impl Eq for Event {}

// Action and Event are data-carrying dispatch ADTs: skipped in label_coverage.

#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
enum EventSerde {
    StallNotice { secs: u64 },
    Starved(StarveKind),
    NotFound { status: u16 },
    Connected,
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
            EventSerde::Reconnecting { attempt } => Self::Reconnecting { attempt },
        })
    }
}
