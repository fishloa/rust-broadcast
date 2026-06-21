//! Status Query objects + audience-metering item structures — ETSI TS 101 699
//! V1.1.1 §6.2, Tables 35-51 (PDF pp. 42-50). See `docs/ci_plus/status-query.md`.
//!
//! Resource ID `0x00211ii1` (`ii` = Module ID, `type = 1*`). The module
//! interrogates the host's status items (Table 36): Selection Information (1),
//! Port Profile (2), Viewed Service (3), Activation Status (4).
//!
//! APDU objects (Tables 37-41):
//! - `StatusQueryReq` (`9F 80 00`) — module → host: a 32-bit `StatusItem`.
//! - `TrapReq` (`9F 80 01`) — module → host: a 32-bit `StatusItem` to monitor.
//! - `GetNextItemReq` (`9F 80 02`) — module → host: a 32-bit `StartStatusItem`.
//! - `GetNextItemAck` (`9F 80 03`) — host → module: a 32-bit `NextStatusItem`.
//! - `StatusAck` (`9F 80 04`) — host → module: the `StatusItem` + its
//!   `StatusBytes` (format depends on the item — Tables 43/48/49/50).
//!
//! The `StatusBytes` of a [`StatusAck`] are carried as an opaque `&[u8]`; the
//! audience-metering content structures ([`SelectionInformation`],
//! [`PortProfile`], [`ViewedService`], [`ActivationStatus`]) implement
//! `Parse`/`Serialize` over **just those bytes** (no `apdu_tag`) so a caller can
//! decode them once the `StatusItem` identifies the format.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for Status Query (Tables 37-41).
pub mod tag {
    use crate::tag::ApduTag;
    /// `StatusQueryReqTag` = `9F 80 00`.
    pub const STATUS_QUERY_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `TrapReqTag` = `9F 80 01`.
    pub const TRAP_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `GetNextItemReqTag` = `9F 80 02`.
    pub const GET_NEXT_ITEM_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
    /// `GetNextItemAckTag` = `9F 80 03`.
    pub const GET_NEXT_ITEM_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x03);
    /// `StatusAckTag` = `9F 80 04`.
    pub const STATUS_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x04);
}

/// Width of the `StatusItem` / `StartStatusItem` / `NextStatusItem` field (32 bits).
const STATUS_ITEM_LEN: usize = 4;

fn read_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// `StatusItem` (Table 36) — the host status item a query refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StatusItem {
    /// `1` — Selection Information (Audience Metering; Table 43).
    SelectionInformation,
    /// `2` — Port Profile (Audience Metering; Table 48).
    PortProfile,
    /// `3` — Viewed Service (Table 49).
    ViewedService,
    /// `4` — Activation Status (Table 50).
    ActivationStatus,
    /// `0` and `> 4` — reserved.
    Reserved(u32),
}

impl StatusItem {
    /// Decode a `StatusItem` value.
    #[must_use]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::SelectionInformation,
            2 => Self::PortProfile,
            3 => Self::ViewedService,
            4 => Self::ActivationStatus,
            other => Self::Reserved(other),
        }
    }
    /// Wire value.
    #[must_use]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::SelectionInformation => 1,
            Self::PortProfile => 2,
            Self::ViewedService => 3,
            Self::ActivationStatus => 4,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SelectionInformation => "Selection Information",
            Self::PortProfile => "Port Profile",
            Self::ViewedService => "Viewed Service",
            Self::ActivationStatus => "Activation Status",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(StatusItem, Reserved);

// =================== APDU objects ===================

/// `StatusQueryReq()` (Table 37): module → host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StatusQueryReq {
    /// `StatusItem` queried.
    pub status_item: StatusItem,
}

/// `TrapReq()` (Table 38): module → host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrapReq {
    /// `StatusItem` to monitor for changes.
    pub status_item: StatusItem,
}

/// `GetNextItemReq()` (Table 39): module → host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GetNextItemReq {
    /// `StartStatusItem` — search start point (raw 32-bit; not necessarily a
    /// supported item).
    pub start_status_item: u32,
}

/// `GetNextItemAck()` (Table 40): host → module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GetNextItemAck {
    /// `NextStatusItem` — first supported item greater than `StartStatusItem`
    /// (raw 32-bit; `0` = no higher item).
    pub next_status_item: u32,
}

/// `StatusAck()` (Table 41): host → module. The `StatusBytes` are an opaque
/// byte string whose format depends on `status_item` (Tables 43/48/49/50); an
/// empty `status_bytes` means the host does not support the item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StatusAck<'a> {
    /// `StatusItem` this reply answers.
    pub status_item: StatusItem,
    /// `StatusBytes` — item-dependent payload.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub status_bytes: &'a [u8],
}

macro_rules! status_item_object {
    ($ty:ident, $tag:expr, $what:literal, $field:ident, $decode:expr, $encode:expr) => {
        impl<'a> Parse<'a> for $ty {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                let body = objects::parse_apdu_header(bytes, $tag, $what)?;
                if body.len() < STATUS_ITEM_LEN {
                    return Err(Error::BufferTooShort {
                        need: STATUS_ITEM_LEN,
                        have: body.len(),
                        what: $what,
                    });
                }
                let raw = read_u32(body);
                Ok(Self {
                    $field: $decode(raw),
                })
            }
        }
        impl Serialize for $ty {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::apdu_len(STATUS_ITEM_LEN)
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let pos = objects::write_apdu_header($tag, STATUS_ITEM_LEN, buf)?;
                let raw: u32 = $encode(self.$field);
                buf[pos..pos + STATUS_ITEM_LEN].copy_from_slice(&raw.to_be_bytes());
                Ok(pos + STATUS_ITEM_LEN)
            }
        }
    };
}

status_item_object!(
    StatusQueryReq,
    tag::STATUS_QUERY_REQ,
    "StatusQueryReq",
    status_item,
    StatusItem::from_u32,
    StatusItem::to_u32
);
status_item_object!(
    TrapReq,
    tag::TRAP_REQ,
    "TrapReq",
    status_item,
    StatusItem::from_u32,
    StatusItem::to_u32
);
status_item_object!(
    GetNextItemReq,
    tag::GET_NEXT_ITEM_REQ,
    "GetNextItemReq",
    start_status_item,
    core::convert::identity,
    core::convert::identity
);
status_item_object!(
    GetNextItemAck,
    tag::GET_NEXT_ITEM_ACK,
    "GetNextItemAck",
    next_status_item,
    core::convert::identity,
    core::convert::identity
);

impl<'a> Parse<'a> for StatusAck<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::STATUS_ACK, "StatusAck")?;
        if body.len() < STATUS_ITEM_LEN {
            return Err(Error::BufferTooShort {
                need: STATUS_ITEM_LEN,
                have: body.len(),
                what: "StatusAck",
            });
        }
        Ok(Self {
            status_item: StatusItem::from_u32(read_u32(body)),
            status_bytes: &body[STATUS_ITEM_LEN..],
        })
    }
}
impl Serialize for StatusAck<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(STATUS_ITEM_LEN + self.status_bytes.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = STATUS_ITEM_LEN + self.status_bytes.len();
        let mut pos = objects::write_apdu_header(tag::STATUS_ACK, body_len, buf)?;
        buf[pos..pos + STATUS_ITEM_LEN].copy_from_slice(&self.status_item.to_u32().to_be_bytes());
        pos += STATUS_ITEM_LEN;
        buf[pos..pos + self.status_bytes.len()].copy_from_slice(self.status_bytes);
        Ok(pos + self.status_bytes.len())
    }
}

// =================== Audience-metering item content structures ===================
//
// These decode the `StatusBytes` of a StatusAck for items 1-4. They have no
// apdu_tag/length_field of their own (the StatusAck supplies length); their
// Parse consumes the whole input slice and Serialize emits exactly the wire
// bytes, so a round-trip is byte-exact.

/// Width of the `time` field of Selection Information (40 bits = 5 bytes, UTC
/// time coded as the DVB SI Time and Date Table).
pub const SELECTION_TIME_LEN: usize = 5;

/// An output entry within a Selection Information [`InPortDescription`] (Table 43).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OutputSignal<'a> {
    /// `out_port_id` (Table 46).
    pub out_port_id: u8,
    /// `out_signal_desc` — block describing the output signal (opaque; format
    /// depends on the output port — Table 47).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub out_signal_desc: &'a [u8],
}

/// One `in_port_id` block of Selection Information (Table 43): an input signal
/// source and the list of outputs it is routed to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InPortDescription<'a> {
    /// `in_port_id` (Table 44).
    pub in_port_id: u8,
    /// `in_signal_desc` — block describing the input signal (opaque; format
    /// depends on the input port — Table 45).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub in_signal_desc: &'a [u8],
    /// The output signals this input is routed to (`length_outputs` bytes total).
    pub outputs: Vec<OutputSignal<'a>>,
}

/// `Selection information status data` (Table 43) — status item 1: a timestamp
/// and a list of input-port → output-port routing descriptions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SelectionInformation<'a> {
    /// `time` — 5-byte UTC time (DVB SI TDT coding).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub time: &'a [u8],
    /// One entry per input port present in the object.
    pub ports: Vec<InPortDescription<'a>>,
}

impl<'a> Parse<'a> for SelectionInformation<'a> {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < SELECTION_TIME_LEN {
            return Err(Error::BufferTooShort {
                need: SELECTION_TIME_LEN,
                have: body.len(),
                what: "SelectionInformation.time",
            });
        }
        let time = &body[..SELECTION_TIME_LEN];
        let mut rest = &body[SELECTION_TIME_LEN..];
        let mut ports = Vec::new();
        while !rest.is_empty() {
            // in_port_id(1) + length_in_signal_desc(1).
            if rest.len() < 2 {
                return Err(Error::BufferTooShort {
                    need: 2,
                    have: rest.len(),
                    what: "SelectionInformation.in_port",
                });
            }
            let in_port_id = rest[0];
            let in_len = rest[1] as usize;
            let after_in = 2 + in_len;
            if rest.len() < after_in + 2 {
                return Err(Error::BufferTooShort {
                    need: after_in + 2,
                    have: rest.len(),
                    what: "SelectionInformation.in_signal_desc",
                });
            }
            let in_signal_desc = &rest[2..after_in];
            // reserved(4) + length_outputs(12) packed into 2 bytes.
            let length_outputs =
                (((rest[after_in] & 0x0F) as usize) << 8) | rest[after_in + 1] as usize;
            let outputs_start = after_in + 2;
            let outputs_end = outputs_start + length_outputs;
            if rest.len() < outputs_end {
                return Err(Error::BufferTooShort {
                    need: outputs_end,
                    have: rest.len(),
                    what: "SelectionInformation.outputs",
                });
            }
            let mut out_rest = &rest[outputs_start..outputs_end];
            let mut outputs = Vec::new();
            while !out_rest.is_empty() {
                // out_port_id(1) + length_out_signal_desc(1).
                if out_rest.len() < 2 {
                    return Err(Error::BufferTooShort {
                        need: 2,
                        have: out_rest.len(),
                        what: "SelectionInformation.out_port",
                    });
                }
                let out_port_id = out_rest[0];
                let out_len = out_rest[1] as usize;
                let out_end = 2 + out_len;
                if out_rest.len() < out_end {
                    return Err(Error::BufferTooShort {
                        need: out_end,
                        have: out_rest.len(),
                        what: "SelectionInformation.out_signal_desc",
                    });
                }
                outputs.push(OutputSignal {
                    out_port_id,
                    out_signal_desc: &out_rest[2..out_end],
                });
                out_rest = &out_rest[out_end..];
            }
            ports.push(InPortDescription {
                in_port_id,
                in_signal_desc,
                outputs,
            });
            rest = &rest[outputs_end..];
        }
        Ok(Self { time, ports })
    }
}
impl Serialize for SelectionInformation<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = SELECTION_TIME_LEN;
        for p in &self.ports {
            n += 2 + p.in_signal_desc.len() + 2;
            for o in &p.outputs {
                n += 2 + o.out_signal_desc.len();
            }
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        if self.time.len() != SELECTION_TIME_LEN {
            return Err(Error::InvalidObject {
                what: "SelectionInformation",
                reason: "time must be exactly 5 bytes",
            });
        }
        let mut pos = 0;
        buf[pos..pos + SELECTION_TIME_LEN].copy_from_slice(self.time);
        pos += SELECTION_TIME_LEN;
        for p in &self.ports {
            if p.in_signal_desc.len() > u8::MAX as usize {
                return Err(Error::InvalidObject {
                    what: "SelectionInformation",
                    reason: "in_signal_desc longer than 255 bytes",
                });
            }
            buf[pos] = p.in_port_id;
            buf[pos + 1] = p.in_signal_desc.len() as u8;
            pos += 2;
            buf[pos..pos + p.in_signal_desc.len()].copy_from_slice(p.in_signal_desc);
            pos += p.in_signal_desc.len();
            // length_outputs = total bytes of the output entries.
            let mut length_outputs = 0usize;
            for o in &p.outputs {
                length_outputs += 2 + o.out_signal_desc.len();
            }
            if length_outputs > 0x0FFF {
                return Err(Error::InvalidObject {
                    what: "SelectionInformation",
                    reason: "length_outputs exceeds 12-bit maximum",
                });
            }
            // reserved(4) = 0, length_outputs(12).
            buf[pos] = ((length_outputs >> 8) & 0x0F) as u8;
            buf[pos + 1] = (length_outputs & 0xFF) as u8;
            pos += 2;
            for o in &p.outputs {
                if o.out_signal_desc.len() > u8::MAX as usize {
                    return Err(Error::InvalidObject {
                        what: "SelectionInformation",
                        reason: "out_signal_desc longer than 255 bytes",
                    });
                }
                buf[pos] = o.out_port_id;
                buf[pos + 1] = o.out_signal_desc.len() as u8;
                pos += 2;
                buf[pos..pos + o.out_signal_desc.len()].copy_from_slice(o.out_signal_desc);
                pos += o.out_signal_desc.len();
            }
        }
        Ok(pos)
    }
}

/// One input or output port description within a [`PortProfile`] (Table 48).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PortDescription<'a> {
    /// `in_port_id` (Table 44).
    pub in_port_id: u8,
    /// `in_port_desc` — string describing the input port (DVB SI Annex A coding).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub in_port_desc: &'a [u8],
    /// `out_port_id` (Table 46).
    pub out_port_id: u8,
    /// `out_signal_desc` — string describing the output port (DVB SI Annex A).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub out_port_desc: &'a [u8],
}

/// `Port profile status data` (Table 48) — status item 2: a textual receiver
/// identity and per-port descriptions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PortProfile<'a> {
    /// `receiver_identification` — manufacturer/model/version string (DVB SI
    /// Annex A coding).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub receiver_identification: &'a [u8],
    /// One entry per `{in,out}` port pair (`N` entries, consuming the remainder).
    pub ports: Vec<PortDescription<'a>>,
}

impl<'a> Parse<'a> for PortProfile<'a> {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        if body.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "PortProfile.receiver_identification_length",
            });
        }
        let id_len = body[0] as usize;
        if body.len() < 1 + id_len {
            return Err(Error::BufferTooShort {
                need: 1 + id_len,
                have: body.len(),
                what: "PortProfile.receiver_identification",
            });
        }
        let receiver_identification = &body[1..1 + id_len];
        let mut rest = &body[1 + id_len..];
        let mut ports = Vec::new();
        while !rest.is_empty() {
            // in_port_id(1) + length_in_port_desc(1).
            if rest.len() < 2 {
                return Err(Error::BufferTooShort {
                    need: 2,
                    have: rest.len(),
                    what: "PortProfile.in_port",
                });
            }
            let in_port_id = rest[0];
            let in_len = rest[1] as usize;
            let after_in = 2 + in_len;
            // + out_port_id(1) + length_out_port_desc(1).
            if rest.len() < after_in + 2 {
                return Err(Error::BufferTooShort {
                    need: after_in + 2,
                    have: rest.len(),
                    what: "PortProfile.out_port",
                });
            }
            let in_port_desc = &rest[2..after_in];
            let out_port_id = rest[after_in];
            let out_len = rest[after_in + 1] as usize;
            let after_out = after_in + 2 + out_len;
            if rest.len() < after_out {
                return Err(Error::BufferTooShort {
                    need: after_out,
                    have: rest.len(),
                    what: "PortProfile.out_port_desc",
                });
            }
            ports.push(PortDescription {
                in_port_id,
                in_port_desc,
                out_port_id,
                out_port_desc: &rest[after_in + 2..after_out],
            });
            rest = &rest[after_out..];
        }
        Ok(Self {
            receiver_identification,
            ports,
        })
    }
}
impl Serialize for PortProfile<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = 1 + self.receiver_identification.len();
        for p in &self.ports {
            n += 2 + p.in_port_desc.len() + 2 + p.out_port_desc.len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        if self.receiver_identification.len() > u8::MAX as usize {
            return Err(Error::InvalidObject {
                what: "PortProfile",
                reason: "receiver_identification longer than 255 bytes",
            });
        }
        let mut pos = 0;
        buf[pos] = self.receiver_identification.len() as u8;
        pos += 1;
        buf[pos..pos + self.receiver_identification.len()]
            .copy_from_slice(self.receiver_identification);
        pos += self.receiver_identification.len();
        for p in &self.ports {
            if p.in_port_desc.len() > u8::MAX as usize || p.out_port_desc.len() > u8::MAX as usize {
                return Err(Error::InvalidObject {
                    what: "PortProfile",
                    reason: "port description longer than 255 bytes",
                });
            }
            buf[pos] = p.in_port_id;
            buf[pos + 1] = p.in_port_desc.len() as u8;
            pos += 2;
            buf[pos..pos + p.in_port_desc.len()].copy_from_slice(p.in_port_desc);
            pos += p.in_port_desc.len();
            buf[pos] = p.out_port_id;
            buf[pos + 1] = p.out_port_desc.len() as u8;
            pos += 2;
            buf[pos..pos + p.out_port_desc.len()].copy_from_slice(p.out_port_desc);
            pos += p.out_port_desc.len();
        }
        Ok(pos)
    }
}

/// `Viewed service status data` (Table 49) — status item 3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ViewedService {
    /// `service_id` / program_number currently selected (`0x0000` = not in the TS).
    pub service_id: u16,
    /// `component_tag`s of the components currently selected for decoding.
    pub component_tags: Vec<u8>,
}

impl<'a> Parse<'a> for ViewedService {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        // service_id(2) + number_components(1).
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "ViewedService",
            });
        }
        let service_id = u16::from_be_bytes([body[0], body[1]]);
        let n = body[2] as usize;
        if body.len() < 3 + n {
            return Err(Error::BufferTooShort {
                need: 3 + n,
                have: body.len(),
                what: "ViewedService.component_tags",
            });
        }
        Ok(Self {
            service_id,
            component_tags: body[3..3 + n].to_vec(),
        })
    }
}
impl Serialize for ViewedService {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        3 + self.component_tags.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        if self.component_tags.len() > u8::MAX as usize {
            return Err(Error::InvalidObject {
                what: "ViewedService",
                reason: "more than 255 component tags",
            });
        }
        buf[0..2].copy_from_slice(&self.service_id.to_be_bytes());
        buf[2] = self.component_tags.len() as u8;
        buf[3..3 + self.component_tags.len()].copy_from_slice(&self.component_tags);
        Ok(total)
    }
}

/// `activation_state` (Table 51) — host power mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ActivationState {
    /// `1` — Standby-active.
    StandbyActive,
    /// `2` — On.
    On,
    /// `0` and `3`-`7` — reserved.
    Reserved(u8),
}

impl ActivationState {
    /// Decode the 3-bit `activation_state` value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::StandbyActive,
            2 => Self::On,
            other => Self::Reserved(other),
        }
    }
    /// 3-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::StandbyActive => 1,
            Self::On => 2,
            Self::Reserved(v) => v & 0x07,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::StandbyActive => "Standby-active",
            Self::On => "On",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(ActivationState, Reserved);

/// `Activation status data` (Table 50) — status item 4: a single packed byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ActivationStatus {
    /// `event_activated` — host activated by an event (vs user action).
    pub event_activated: bool,
    /// `activation_state` (3 bits).
    pub activation_state: Option<ActivationState>,
}

impl ActivationStatus {
    const EVENT_ACTIVATED_BIT: u8 = 0x08;
    const ACTIVATION_STATE_MASK: u8 = 0x07;
}

impl<'a> Parse<'a> for ActivationStatus {
    type Error = Error;
    fn parse(body: &'a [u8]) -> Result<Self> {
        if body.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "ActivationStatus",
            });
        }
        Ok(Self {
            event_activated: body[0] & Self::EVENT_ACTIVATED_BIT != 0,
            activation_state: Some(ActivationState::from_u8(
                body[0] & Self::ACTIVATION_STATE_MASK,
            )),
        })
    }
}
impl Serialize for ActivationStatus {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Err(Error::OutputBufferTooSmall { need: 1, have: 0 });
        }
        let mut b = 0u8;
        if self.event_activated {
            b |= Self::EVENT_ACTIVATED_BIT;
        }
        if let Some(state) = self.activation_state {
            b |= state.to_u8() & Self::ACTIVATION_STATE_MASK;
        }
        buf[0] = b;
        Ok(1)
    }
}

/// Resource-scoped dispatch over the Status Query objects (Tables 37-41).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StatusQueryApdu<'a> {
    /// `StatusQueryReq` (`9F 80 00`).
    StatusQueryReq(StatusQueryReq),
    /// `TrapReq` (`9F 80 01`).
    TrapReq(TrapReq),
    /// `GetNextItemReq` (`9F 80 02`).
    GetNextItemReq(GetNextItemReq),
    /// `GetNextItemAck` (`9F 80 03`).
    GetNextItemAck(GetNextItemAck),
    /// `StatusAck` (`9F 80 04`).
    StatusAck(StatusAck<'a>),
}

impl<'a> StatusQueryApdu<'a> {
    /// Parse a Status Query APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "status_query apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::STATUS_QUERY_REQ => Ok(Self::StatusQueryReq(StatusQueryReq::parse(body)?)),
            tag::TRAP_REQ => Ok(Self::TrapReq(TrapReq::parse(body)?)),
            tag::GET_NEXT_ITEM_REQ => Ok(Self::GetNextItemReq(GetNextItemReq::parse(body)?)),
            tag::GET_NEXT_ITEM_ACK => Ok(Self::GetNextItemAck(GetNextItemAck::parse(body)?)),
            tag::STATUS_ACK => Ok(Self::StatusAck(StatusAck::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::STATUS_QUERY_REQ.as_u24(),
                what: "status_query",
            }),
        }
    }
}

impl Serialize for StatusQueryApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::StatusQueryReq(o) => o.serialized_len(),
            Self::TrapReq(o) => o.serialized_len(),
            Self::GetNextItemReq(o) => o.serialized_len(),
            Self::GetNextItemAck(o) => o.serialized_len(),
            Self::StatusAck(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::StatusQueryReq(o) => o.serialize_into(buf),
            Self::TrapReq(o) => o.serialize_into(buf),
            Self::GetNextItemReq(o) => o.serialize_into(buf),
            Self::GetNextItemAck(o) => o.serialize_into(buf),
            Self::StatusAck(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_query_req_round_trips_and_bites() {
        let req = StatusQueryReq {
            status_item: StatusItem::PortProfile,
        };
        let bytes = req.to_bytes();
        // tag(3) + len(1=4) + StatusItem(4 = 0x00000002).
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02]);
        assert_eq!(StatusQueryReq::parse(&bytes).unwrap(), req);
        let other = StatusQueryReq {
            status_item: StatusItem::ViewedService,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn trap_and_getnext_round_trip() {
        let trap = TrapReq {
            status_item: StatusItem::ActivationStatus,
        };
        assert_eq!(trap.to_bytes(), [0x9F, 0x80, 0x01, 0x04, 0, 0, 0, 4]);
        assert_eq!(TrapReq::parse(&trap.to_bytes()).unwrap(), trap);

        let gnr = GetNextItemReq {
            start_status_item: 0x0000_0007,
        };
        assert_eq!(gnr.to_bytes(), [0x9F, 0x80, 0x02, 0x04, 0, 0, 0, 7]);
        assert_eq!(GetNextItemReq::parse(&gnr.to_bytes()).unwrap(), gnr);

        let gna = GetNextItemAck {
            next_status_item: 0x0000_0002,
        };
        assert_eq!(gna.to_bytes(), [0x9F, 0x80, 0x03, 0x04, 0, 0, 0, 2]);
        assert_eq!(GetNextItemAck::parse(&gna.to_bytes()).unwrap(), gna);
    }

    #[test]
    fn status_item_reserved_round_trips() {
        let req = StatusQueryReq {
            status_item: StatusItem::from_u32(0),
        };
        assert_eq!(req.status_item, StatusItem::Reserved(0));
        assert_eq!(req.status_item.name(), "reserved");
        assert_eq!(StatusQueryReq::parse(&req.to_bytes()).unwrap(), req);
    }

    #[test]
    fn status_ack_with_status_bytes_round_trips_and_bites() {
        let ack = StatusAck {
            status_item: StatusItem::ViewedService,
            status_bytes: &[0x01, 0x23, 0x00],
        };
        let bytes = ack.to_bytes();
        // tag(3) + len(1=7) + item(4) + 3 status bytes.
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x04, 0x07, 0, 0, 0, 3, 0x01, 0x23, 0x00]
        );
        assert_eq!(StatusAck::parse(&bytes).unwrap(), ack);
        let mut other = ack.clone();
        other.status_bytes = &[0x01, 0x23, 0x01];
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn status_ack_empty_status_bytes_means_unsupported() {
        let ack = StatusAck {
            status_item: StatusItem::PortProfile,
            status_bytes: &[],
        };
        let bytes = ack.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x04, 0x04, 0, 0, 0, 2]);
        assert_eq!(StatusAck::parse(&bytes).unwrap(), ack);
    }

    #[test]
    fn selection_information_multi_port_round_trips_and_bites() {
        // Two input ports, the first routed to two outputs (boundary >= 2 loop).
        let si = SelectionInformation {
            time: &[0x20, 0x06, 0x18, 0x12, 0x00],
            ports: alloc::vec![
                InPortDescription {
                    in_port_id: 0x01,
                    in_signal_desc: &[0xAA, 0xBB],
                    outputs: alloc::vec![
                        OutputSignal {
                            out_port_id: 0x00,
                            out_signal_desc: &[0x02],
                        },
                        OutputSignal {
                            out_port_id: 0x01,
                            out_signal_desc: &[0x01],
                        },
                    ],
                },
                InPortDescription {
                    in_port_id: 0x18,
                    in_signal_desc: &[],
                    outputs: alloc::vec![],
                },
            ],
        };
        let bytes = si.to_bytes();
        assert_eq!(SelectionInformation::parse(&bytes).unwrap(), si);
        // Mutate one out_port_id -> different bytes.
        let mut other = si.clone();
        other.ports[0].outputs[1].out_port_id = 0x02;
        assert_ne!(bytes, other.to_bytes());
        // Verify a precise byte layout for the first port's output block.
        // time(5) | in_port(0x01) in_len(0x02) [AA BB] | reserved/len_out(0x00 0x06)
        //   out0: 00 01 02 | out1: 01 01 01 | in_port(0x18) in_len(0x00) len_out(0x00 0x00)
        assert_eq!(
            bytes,
            [
                0x20, 0x06, 0x18, 0x12, 0x00, // time
                0x01, 0x02, 0xAA, 0xBB, // in port 1
                0x00, 0x06, // length_outputs = 6
                0x00, 0x01, 0x02, // out 0
                0x01, 0x01, 0x01, // out 1
                0x18, 0x00, // in port 2
                0x00, 0x00, // length_outputs = 0
            ]
        );
    }

    #[test]
    fn port_profile_multi_port_round_trips_and_bites() {
        let pp = PortProfile {
            receiver_identification: b"ACME-TV1",
            ports: alloc::vec![
                PortDescription {
                    in_port_id: 0x00,
                    in_port_desc: b"RF0",
                    out_port_id: 0x00,
                    out_port_desc: b"D0",
                },
                PortDescription {
                    in_port_id: 0x18,
                    in_port_desc: b"CI",
                    out_port_id: 0x10,
                    out_port_desc: b"SCART",
                },
            ],
        };
        let bytes = pp.to_bytes();
        assert_eq!(PortProfile::parse(&bytes).unwrap(), pp);
        let mut other = pp.clone();
        other.ports[1].out_port_id = 0x11;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn viewed_service_multi_component_round_trips_and_bites() {
        let vs = ViewedService {
            service_id: 0x1234,
            component_tags: alloc::vec![0x10, 0x20, 0x30],
        };
        let bytes = vs.to_bytes();
        assert_eq!(bytes, [0x12, 0x34, 0x03, 0x10, 0x20, 0x30]);
        assert_eq!(ViewedService::parse(&bytes).unwrap(), vs);
        let mut other = vs.clone();
        other.component_tags[2] = 0x31;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn activation_status_packs_bits() {
        let a = ActivationStatus {
            event_activated: true,
            activation_state: Some(ActivationState::On),
        };
        let bytes = a.to_bytes();
        // event_activated(1<<3) | activation_state(2) = 0x0A.
        assert_eq!(bytes, [0x0A]);
        assert_eq!(ActivationStatus::parse(&bytes).unwrap(), a);
        // Standby-active, no event.
        let b = ActivationStatus {
            event_activated: false,
            activation_state: Some(ActivationState::StandbyActive),
        };
        assert_eq!(b.to_bytes(), [0x01]);
        assert_eq!(ActivationStatus::parse(&[0x01]).unwrap(), b);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let q = StatusQueryReq {
            status_item: StatusItem::SelectionInformation,
        }
        .to_bytes();
        assert!(matches!(
            StatusQueryApdu::parse(&q).unwrap(),
            StatusQueryApdu::StatusQueryReq(_)
        ));
        let ack = StatusAck {
            status_item: StatusItem::ActivationStatus,
            status_bytes: &[0x0A],
        }
        .to_bytes();
        let parsed = StatusQueryApdu::parse(&ack).unwrap();
        assert!(matches!(parsed, StatusQueryApdu::StatusAck(_)));
        assert_eq!(parsed.to_bytes(), ack);
    }
}
