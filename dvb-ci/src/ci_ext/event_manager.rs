//! Event Manager objects — ETSI TS 101 699 V1.1.1 §6.4, Tables 56-61
//! (PDF pp. 55-56). See `docs/ci_plus/event-manager.md`.
//!
//! Resource ID `0x00231ii1` (`ii` = Module ID). Lets a module book timer events
//! that wake the host.
//!
//! - `event_request` (`9F 80 00`, Table 56) — module → host: book/cancel an event.
//! - `event_request_ack` (`9F 80 01`, Table 59) — host → module: reply.
//! - `event_notification` (`9F 80 02`, Table 61) — host → module: event occurred.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use broadcast_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Event Manager (Tables 56, 59, 61).
pub mod tag {
    use crate::tag::ApduTag;
    /// `event_request_tag` = `9F 80 00`.
    pub const EVENT_REQUEST: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `event_request_ack_tag` = `9F 80 01`.
    pub const EVENT_REQUEST_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `event_notification_tag` = `9F 80 02`.
    pub const EVENT_NOTIFICATION: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
}

/// `event_type` — the kind of event (Table 57).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EventType {
    /// `0` — Timer.
    Timer,
    /// `1`-`255` — reserved for future use.
    Reserved(u8),
}

impl EventType {
    /// Decode the `event_type` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Timer,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Timer => 0,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Timer => "Timer",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(EventType, Reserved);

/// `reply` — the event request reply code (Table 60).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EventReply {
    /// `0` — Event booked OK.
    BookedOk,
    /// `1` — Event type not supported.
    TypeNotSupported,
    /// `2` — Event resources consumed.
    ResourcesConsumed,
    /// `3`-`255` — reserved for future use.
    Reserved(u8),
}

impl EventReply {
    /// Decode the `reply` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::BookedOk,
            1 => Self::TypeNotSupported,
            2 => Self::ResourcesConsumed,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::BookedOk => 0,
            Self::TypeNotSupported => 1,
            Self::ResourcesConsumed => 2,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::BookedOk => "Event booked OK",
            Self::TypeNotSupported => "Event type not supported",
            Self::ResourcesConsumed => "Event resources consumed",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(EventReply, Reserved);

/// `event_request()` (Table 56): module → host. The `event_desc` bytes define
/// the event; their format depends on `event_type` (Table 58 — for a Timer,
/// 40-bit start_time + 24-bit duration). An empty `event_desc` cancels any
/// previously-booked event of this type. Carried verbatim for fidelity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EventRequest<'a> {
    /// `event_type`.
    pub event_type: EventType,
    /// The `event_desc` block (format depends on `event_type`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub event_desc: &'a [u8],
}

/// `event_request_ack()` (Table 59): host → module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EventRequestAck {
    /// `event_type`.
    pub event_type: EventType,
    /// `reply`.
    pub reply: EventReply,
}

/// `event_notification()` (Table 61): host → module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EventNotification {
    /// `event_type`.
    pub event_type: EventType,
}

// --- event_request ---

// event_type(1) + event_desc(N).
const EVENT_REQUEST_PREFIX: usize = 1;

impl<'a> Parse<'a> for EventRequest<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::EVENT_REQUEST, "event_request")?;
        if body.len() < EVENT_REQUEST_PREFIX {
            return Err(Error::BufferTooShort {
                need: EVENT_REQUEST_PREFIX,
                have: body.len(),
                what: "event_request",
            });
        }
        Ok(Self {
            event_type: EventType::from_u8(body[0]),
            event_desc: &body[EVENT_REQUEST_PREFIX..],
        })
    }
}
impl Serialize for EventRequest<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(EVENT_REQUEST_PREFIX + self.event_desc.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = EVENT_REQUEST_PREFIX + self.event_desc.len();
        let mut pos = objects::write_apdu_header(tag::EVENT_REQUEST, body_len, buf)?;
        buf[pos] = self.event_type.to_u8();
        pos += EVENT_REQUEST_PREFIX;
        buf[pos..pos + self.event_desc.len()].copy_from_slice(self.event_desc);
        Ok(pos + self.event_desc.len())
    }
}

// --- event_request_ack ---

// event_type(1) + reply(1).
const EVENT_REQUEST_ACK_BODY: usize = 2;

impl<'a> Parse<'a> for EventRequestAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::EVENT_REQUEST_ACK, "event_request_ack")?;
        if body.len() < EVENT_REQUEST_ACK_BODY {
            return Err(Error::BufferTooShort {
                need: EVENT_REQUEST_ACK_BODY,
                have: body.len(),
                what: "event_request_ack",
            });
        }
        Ok(Self {
            event_type: EventType::from_u8(body[0]),
            reply: EventReply::from_u8(body[1]),
        })
    }
}
impl Serialize for EventRequestAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(EVENT_REQUEST_ACK_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::EVENT_REQUEST_ACK, EVENT_REQUEST_ACK_BODY, buf)?;
        buf[pos] = self.event_type.to_u8();
        buf[pos + 1] = self.reply.to_u8();
        Ok(pos + EVENT_REQUEST_ACK_BODY)
    }
}

// --- event_notification ---

// event_type(1).
const EVENT_NOTIFICATION_BODY: usize = 1;

impl<'a> Parse<'a> for EventNotification {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body =
            objects::parse_apdu_header(bytes, tag::EVENT_NOTIFICATION, "event_notification")?;
        if body.len() < EVENT_NOTIFICATION_BODY {
            return Err(Error::BufferTooShort {
                need: EVENT_NOTIFICATION_BODY,
                have: body.len(),
                what: "event_notification",
            });
        }
        Ok(Self {
            event_type: EventType::from_u8(body[0]),
        })
    }
}
impl Serialize for EventNotification {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(EVENT_NOTIFICATION_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos =
            objects::write_apdu_header(tag::EVENT_NOTIFICATION, EVENT_NOTIFICATION_BODY, buf)?;
        buf[pos] = self.event_type.to_u8();
        Ok(pos + EVENT_NOTIFICATION_BODY)
    }
}

/// Resource-scoped dispatch over the Event Manager objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EventManagerApdu<'a> {
    /// `event_request` (`9F 80 00`).
    EventRequest(EventRequest<'a>),
    /// `event_request_ack` (`9F 80 01`).
    EventRequestAck(EventRequestAck),
    /// `event_notification` (`9F 80 02`).
    EventNotification(EventNotification),
}

impl<'a> EventManagerApdu<'a> {
    /// Parse an Event Manager APDU, dispatching on the `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "event_manager apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::EVENT_REQUEST => Ok(Self::EventRequest(EventRequest::parse(body)?)),
            tag::EVENT_REQUEST_ACK => Ok(Self::EventRequestAck(EventRequestAck::parse(body)?)),
            tag::EVENT_NOTIFICATION => Ok(Self::EventNotification(EventNotification::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::EVENT_REQUEST.as_u24(),
                what: "event_manager",
            }),
        }
    }
}

impl Serialize for EventManagerApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::EventRequest(o) => o.serialized_len(),
            Self::EventRequestAck(o) => o.serialized_len(),
            Self::EventNotification(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::EventRequest(o) => o.serialize_into(buf),
            Self::EventRequestAck(o) => o.serialize_into(buf),
            Self::EventNotification(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_request_timer_round_trips_and_bites() {
        // Timer: 40-bit start_time + 24-bit duration = 8 desc bytes.
        let desc = [0x11, 0x22, 0x33, 0x44, 0x55, 0x06, 0x07, 0x08];
        let r = EventRequest {
            event_type: EventType::Timer,
            event_desc: &desc,
        };
        let bytes = r.to_bytes();
        // tag(3) + len(1) + type(1) + 8 = 13; body len = 9 = 0x09.
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x00, 0x09, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x06, 0x07, 0x08
            ]
        );
        assert_eq!(EventRequest::parse(&bytes).unwrap(), r);
        // mutate a desc byte.
        let mut desc2 = desc;
        desc2[0] = 0xFF;
        let other = EventRequest {
            event_type: EventType::Timer,
            event_desc: &desc2,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn event_request_cancel_empty_desc() {
        let r = EventRequest {
            event_type: EventType::Timer,
            event_desc: &[],
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x01, 0x00]);
        assert_eq!(EventRequest::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn event_request_ack_round_trips_and_bites() {
        let a = EventRequestAck {
            event_type: EventType::Timer,
            reply: EventReply::ResourcesConsumed,
        };
        let bytes = a.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x02, 0x00, 0x02]);
        assert_eq!(EventRequestAck::parse(&bytes).unwrap(), a);
        assert_eq!(a.reply.name(), "Event resources consumed");
        let mut other = a;
        other.reply = EventReply::BookedOk;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn event_notification_round_trips() {
        let n = EventNotification {
            event_type: EventType::Timer,
        };
        let bytes = n.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x02, 0x01, 0x00]);
        assert_eq!(EventNotification::parse(&bytes).unwrap(), n);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let req = EventRequest {
            event_type: EventType::Timer,
            event_desc: &[],
        }
        .to_bytes();
        assert!(matches!(
            EventManagerApdu::parse(&req).unwrap(),
            EventManagerApdu::EventRequest(_)
        ));
        let notif = EventNotification {
            event_type: EventType::Timer,
        }
        .to_bytes();
        let parsed = EventManagerApdu::parse(&notif).unwrap();
        assert!(matches!(parsed, EventManagerApdu::EventNotification(_)));
        assert_eq!(parsed.to_bytes(), notif);
    }
}
