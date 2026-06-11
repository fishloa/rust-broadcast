//! Linkage Descriptor — ETSI EN 300 468 §6.2.19 (tag 0x4A).
//!
//! Carried inside NIT (network linkage), BAT (bouquet linkage) and
//! SDT (service replacement / premiere hand-over).
//!
//! Conditional inner structures (§6.2.19.2–4):
//! - `linkage_type == 0x08` → [`MobileHandOverInfo`] (Table 61)
//! - `linkage_type == 0x0D` → [`EventLinkageInfo`]    (Table 64)
//! - `linkage_type 0x0E..=0x1F` → [`ExtendedEventLinkageInfo`] (Table 65)
//!
//! Other linkage types (including user-defined `0x80..=0xFE` and those
//! defined in companion specs such as TS 102 006 / EN 301 192 / EN 303 560)
//! have no EN 300 468 conditional block; their bytes after the fixed fields
//! are the `private_data_byte` tail.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for linkage_descriptor.
pub const TAG: u8 = 0x4A;
const HEADER_LEN: usize = 2;
const FIXED_FIELDS_LEN: usize = 7;

const HANDOVER_TYPE_MASK: u8 = 0xF0;
const ORIGIN_TYPE_MASK: u8 = 0x01;
const RESERVED_HANDOVER_MASK: u8 = 0x0E;

const TARGET_LISTED_MASK: u8 = 0x80;
const EVENT_SIMULCAST_MASK: u8 = 0x40;
const RESERVED_EVENT_MASK: u8 = 0x3F;

const EXT_TARGET_LISTED_MASK: u8 = 0x80;
const EXT_EVENT_SIMULCAST_MASK: u8 = 0x40;
const EXT_LINK_TYPE_MASK: u8 = 0x30;
const EXT_TARGET_ID_TYPE_MASK: u8 = 0x0C;
const EXT_ONID_FLAG_MASK: u8 = 0x02;
const EXT_SID_FLAG_MASK: u8 = 0x01;

/// Mobile hand-over info — EN 300 468 Table 61 (`linkage_type == 0x08`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MobileHandOverInfo {
    /// hand_over_type — 4 bits `[7:4]`.  See Table 62.
    pub hand_over_type: u8,
    /// origin_type — 1 bit `[0]`.  `false` = NIT, `true` = SDT (Table 63).
    pub origin_type: bool,
    /// network_id — 16 bits, present only when hand_over_type in {1, 2, 3}.
    pub network_id: Option<u16>,
    /// initial_service_id — present only when `origin_type == false` (NIT).
    pub initial_service_id: Option<u16>,
}

impl MobileHandOverInfo {
    fn serialized_len(&self) -> usize {
        1 + self.network_id.map_or(0, |_| 2) + self.initial_service_id.map_or(0, |_| 2)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let flags_byte =
            (self.hand_over_type << 4) | RESERVED_HANDOVER_MASK | u8::from(self.origin_type);
        buf[0] = flags_byte;
        let mut pos = 1;
        if let Some(nid) = self.network_id {
            buf[pos..pos + 2].copy_from_slice(&nid.to_be_bytes());
            pos += 2;
        }
        if let Some(sid) = self.initial_service_id {
            buf[pos..pos + 2].copy_from_slice(&sid.to_be_bytes());
        }
        Ok(len)
    }
}

/// Event linkage info — EN 300 468 Table 64 (`linkage_type == 0x0D`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EventLinkageInfo {
    /// target_event_id — 16 bits.
    pub target_event_id: u16,
    /// target_listed — 1 bit `[7]`.
    pub target_listed: bool,
    /// event_simulcast — 1 bit `[6]`.
    pub event_simulcast: bool,
}

impl EventLinkageInfo {
    const SERIALIZED_LEN: usize = 3;

    fn serialized_len(&self) -> usize {
        Self::SERIALIZED_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < Self::SERIALIZED_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: Self::SERIALIZED_LEN,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.target_event_id.to_be_bytes());
        let mut byte2: u8 = RESERVED_EVENT_MASK;
        if self.target_listed {
            byte2 |= TARGET_LISTED_MASK;
        }
        if self.event_simulcast {
            byte2 |= EVENT_SIMULCAST_MASK;
        }
        buf[2] = byte2;
        Ok(Self::SERIALIZED_LEN)
    }
}

/// Target identification — EN 300 468 Table 65 inner conditional fields.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum TargetId {
    /// `target_id_type == 3` — user_defined_id (16 bits).
    UserDefined {
        /// User-defined target service identifier.
        user_defined_id: u16,
    },
    /// `target_id_type != 3` — DVB addressing with optional sub-fields.
    Dvb {
        /// `target_id_type` value (0, 1, or 2).  See Table 67.
        target_id_type: u8,
        /// target_transport_stream_id — present when `target_id_type == 1`.
        target_transport_stream_id: Option<u16>,
        /// target_original_network_id — present when `original_network_id_flag == 1`.
        target_original_network_id: Option<u16>,
        /// target_service_id — present when `service_id_flag == 1`.
        target_service_id: Option<u16>,
    },
}

impl TargetId {
    fn serialized_len(&self) -> usize {
        match self {
            TargetId::UserDefined { .. } => 2,
            TargetId::Dvb {
                target_transport_stream_id,
                target_original_network_id,
                target_service_id,
                ..
            } => {
                usize::from(target_transport_stream_id.is_some()) * 2
                    + usize::from(target_original_network_id.is_some()) * 2
                    + usize::from(target_service_id.is_some()) * 2
            }
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        match self {
            TargetId::UserDefined { user_defined_id } => {
                buf[..2].copy_from_slice(&user_defined_id.to_be_bytes());
            }
            TargetId::Dvb {
                target_transport_stream_id,
                target_original_network_id,
                target_service_id,
                ..
            } => {
                let ts_len = target_transport_stream_id.map_or(0, |_| 2);
                let onid_len = target_original_network_id.map_or(0, |_| 2);
                if let Some(ts_id) = target_transport_stream_id {
                    buf[..2].copy_from_slice(&ts_id.to_be_bytes());
                }
                if let Some(onid) = target_original_network_id {
                    buf[ts_len..ts_len + 2].copy_from_slice(&onid.to_be_bytes());
                }
                if let Some(sid) = target_service_id {
                    let off = ts_len + onid_len;
                    buf[off..off + 2].copy_from_slice(&sid.to_be_bytes());
                }
            }
        }
        Ok(len)
    }
}

/// One entry in the extended event linkage loop — EN 300 468 Table 65.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtendedEventLinkageEntry {
    /// target_event_id — 16 bits.
    pub target_event_id: u16,
    /// target_listed — 1 bit `[7]`.
    pub target_listed: bool,
    /// event_simulcast — 1 bit `[6]`.
    pub event_simulcast: bool,
    /// link_type — 2 bits `[5:4]`.  See Table 66.
    pub link_type: u8,
    /// target_id_type — 2 bits `[3:2]`.  See Table 67.
    pub target_id_type: u8,
    /// original_network_id_flag — 1 bit `[1]`.
    pub original_network_id_flag: bool,
    /// service_id_flag — 1 bit `[0]`.
    pub service_id_flag: bool,
    /// Conditional target identification.
    pub target_id: TargetId,
}

impl ExtendedEventLinkageEntry {
    fn serialized_len(&self) -> usize {
        2 + 1 + self.target_id.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let flags_byte_len = 3;
        if buf.len() < flags_byte_len {
            return Err(Error::OutputBufferTooSmall {
                need: flags_byte_len,
                have: buf.len(),
            });
        }
        buf[0..2].copy_from_slice(&self.target_event_id.to_be_bytes());
        let mut byte2: u8 = 0;
        if self.target_listed {
            byte2 |= EXT_TARGET_LISTED_MASK;
        }
        if self.event_simulcast {
            byte2 |= EXT_EVENT_SIMULCAST_MASK;
        }
        byte2 |= (self.link_type & 0x03) << 4;
        byte2 |= (self.target_id_type & 0x03) << 2;
        if self.original_network_id_flag {
            byte2 |= EXT_ONID_FLAG_MASK;
        }
        if self.service_id_flag {
            byte2 |= EXT_SID_FLAG_MASK;
        }
        buf[2] = byte2;
        let tid_written = self.target_id.serialize_into(&mut buf[3..])?;
        Ok(3 + tid_written)
    }
}

/// Extended event linkage info — EN 300 468 Table 65
/// (`linkage_type 0x0E..=0x1F`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtendedEventLinkageInfo {
    /// Entries in the extended event linkage loop.
    pub entries: Vec<ExtendedEventLinkageEntry>,
}

impl ExtendedEventLinkageInfo {
    fn serialized_len(&self) -> usize {
        1 + self
            .entries
            .iter()
            .map(|e| e.serialized_len())
            .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let loop_len: usize = self.entries.iter().map(|e| e.serialized_len()).sum();
        let total = 1 + loop_len;
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        buf[0] = loop_len as u8;
        let mut pos = 1;
        for entry in &self.entries {
            let written = entry.serialize_into(&mut buf[pos..])?;
            pos += written;
        }
        Ok(total)
    }
}

/// Linkage-type-conditional inner data — EN 300 468 §6.2.19.2–4.
///
/// Typed variants correspond to the conditional blocks defined in EN 300 468;
/// `None` covers all other linkage types (no conditional block per the main
/// spec, remaining bytes go to `private_data`); `Other` captures the raw tail
/// for linkage types whose conditional structure is defined in companion specs
/// (e.g. TS 102 006 linkage_type 0x09/0x0A, EN 301 192 0x0B/0x0C,
/// EN 303 560 0x20) or user-defined types (`0x80..=0xFE`) where we cannot
/// distinguish the conditional block from the `private_data_byte` loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum LinkageData<'a> {
    /// `linkage_type == 0x08` — mobile hand-over info (Table 61).
    MobileHandOver(MobileHandOverInfo),
    /// `linkage_type == 0x0D` — event linkage info (Table 64).
    EventLinkage(EventLinkageInfo),
    /// `linkage_type 0x0E..=0x1F` — extended event linkage info (Table 65).
    ExtendedEventLinkage(ExtendedEventLinkageInfo),
    /// No EN 300 468 conditional block — `private_data` starts immediately
    /// after the fixed fields.
    None,
    /// Raw tail for linkage types with externally-defined or user-defined
    /// conditional structure.  Because we cannot determine the boundary
    /// between the conditional block and the `private_data_byte` loop, all
    /// remaining bytes are captured here and `private_data` is empty.
    #[cfg_attr(feature = "serde", serde(borrow))]
    Other(&'a [u8]),
}

impl LinkageData<'_> {
    fn serialized_len(&self) -> usize {
        match self {
            LinkageData::MobileHandOver(m) => m.serialized_len(),
            LinkageData::EventLinkage(e) => e.serialized_len(),
            LinkageData::ExtendedEventLinkage(x) => x.serialized_len(),
            LinkageData::None => 0,
            LinkageData::Other(b) => b.len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            LinkageData::MobileHandOver(m) => m.serialize_into(buf),
            LinkageData::EventLinkage(e) => e.serialize_into(buf),
            LinkageData::ExtendedEventLinkage(x) => x.serialize_into(buf),
            LinkageData::None => Ok(0),
            LinkageData::Other(b) => {
                if buf.len() < b.len() {
                    return Err(Error::OutputBufferTooSmall {
                        need: b.len(),
                        have: buf.len(),
                    });
                }
                buf[..b.len()].copy_from_slice(b);
                Ok(b.len())
            }
        }
    }
}

fn parse_mobile_handover(bytes: &[u8], end: usize) -> Result<MobileHandOverInfo> {
    if end < 1 {
        return Err(Error::InvalidDescriptor {
            tag: TAG,
            reason: "mobile hand-over info needs at least flags byte",
        });
    }
    let flags_byte = bytes[0];
    let hand_over_type = (flags_byte & HANDOVER_TYPE_MASK) >> 4;
    let origin_type = (flags_byte & ORIGIN_TYPE_MASK) != 0;
    let mut pos = 1;
    let network_id = if matches!(hand_over_type, 0x01..=0x03) {
        if pos + 2 > end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "mobile hand-over info with gated network_id needs at least 3 bytes",
            });
        }
        let nid = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
        pos += 2;
        Some(nid)
    } else {
        None
    };
    let initial_service_id = if !origin_type {
        if pos + 2 > end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "mobile hand-over info with origin_type=NIT needs initial_service_id",
            });
        }
        Some(u16::from_be_bytes([bytes[pos], bytes[pos + 1]]))
    } else {
        None
    };
    Ok(MobileHandOverInfo {
        hand_over_type,
        origin_type,
        network_id,
        initial_service_id,
    })
}

fn parse_event_linkage(bytes: &[u8]) -> Result<EventLinkageInfo> {
    if bytes.len() < 3 {
        return Err(Error::InvalidDescriptor {
            tag: TAG,
            reason: "event linkage info needs 3 bytes",
        });
    }
    let target_event_id = u16::from_be_bytes([bytes[0], bytes[1]]);
    let target_listed = (bytes[2] & TARGET_LISTED_MASK) != 0;
    let event_simulcast = (bytes[2] & EVENT_SIMULCAST_MASK) != 0;
    Ok(EventLinkageInfo {
        target_event_id,
        target_listed,
        event_simulcast,
    })
}

fn parse_extended_event_linkage(bytes: &[u8]) -> Result<ExtendedEventLinkageInfo> {
    if bytes.is_empty() {
        return Err(Error::InvalidDescriptor {
            tag: TAG,
            reason: "extended event linkage info needs at least loop_length byte",
        });
    }
    let loop_length = bytes[0] as usize;
    let loop_end = 1 + loop_length;
    if bytes.len() < loop_end {
        return Err(Error::BufferTooShort {
            need: loop_end,
            have: bytes.len(),
            what: "extended event linkage info loop",
        });
    }
    let mut entries = Vec::new();
    let mut pos = 1;
    let read_u16 = |p: &mut usize| -> Result<u16> {
        if *p + 2 > loop_end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "extended event linkage entry truncated (need u16)",
            });
        }
        let v = u16::from_be_bytes([bytes[*p], bytes[*p + 1]]);
        *p += 2;
        Ok(v)
    };
    while pos < loop_end {
        let target_event_id = read_u16(&mut pos)?;
        if pos >= loop_end {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "extended event linkage entry truncated (need flags byte)",
            });
        }
        let fb = bytes[pos];
        pos += 1;
        let target_listed = (fb & EXT_TARGET_LISTED_MASK) != 0;
        let event_simulcast = (fb & EXT_EVENT_SIMULCAST_MASK) != 0;
        let link_type = (fb & EXT_LINK_TYPE_MASK) >> 4;
        let target_id_type = (fb & EXT_TARGET_ID_TYPE_MASK) >> 2;
        let original_network_id_flag = (fb & EXT_ONID_FLAG_MASK) != 0;
        let service_id_flag = (fb & EXT_SID_FLAG_MASK) != 0;

        let target_id = if target_id_type == 3 {
            let user_defined_id = read_u16(&mut pos)?;
            TargetId::UserDefined { user_defined_id }
        } else {
            let target_transport_stream_id = if target_id_type == 1 {
                Some(read_u16(&mut pos)?)
            } else {
                None
            };
            let target_original_network_id = if original_network_id_flag {
                Some(read_u16(&mut pos)?)
            } else {
                None
            };
            let target_service_id = if service_id_flag {
                Some(read_u16(&mut pos)?)
            } else {
                None
            };
            TargetId::Dvb {
                target_id_type,
                target_transport_stream_id,
                target_original_network_id,
                target_service_id,
            }
        };
        entries.push(ExtendedEventLinkageEntry {
            target_event_id,
            target_listed,
            event_simulcast,
            link_type,
            target_id_type,
            original_network_id_flag,
            service_id_flag,
            target_id,
        });
    }
    Ok(ExtendedEventLinkageInfo { entries })
}

/// Linkage Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct LinkageDescriptor<'a> {
    /// transport_stream_id of the linked-to TS.
    pub transport_stream_id: u16,
    /// original_network_id of the linked-to TS.
    pub original_network_id: u16,
    /// service_id of the linked-to service (0 if linkage is at the network or
    /// bouquet level).
    pub service_id: u16,
    /// linkage_type byte (Table 60): 0x01 information, 0x02 EPG,
    /// 0x03 CA_replacement, 0x04 TS_containing_complete_SI, 0x05
    /// service_replacement, 0x06 data_broadcast, 0x07 RCS_map,
    /// 0x08 mobile_hand-over, 0x09 SSU, 0x0A SSU BAT/NIT,
    /// 0x0B IP/MAC notification, 0x0C INT BAT/NIT, 0x0D event_linkage,
    /// 0x0E..=0x1F extended_event_linkage, 0x20 downloadable font,
    /// 0x21 Native IP bootstrap, 0x80..=0xFE user defined.
    pub linkage_type: u8,
    /// Linkage-type-conditional inner structure.
    pub linkage_data: LinkageData<'a>,
    /// Trailing `private_data_byte` run — bytes after the conditional block
    /// (if any).  Empty when `linkage_data` is `Other`.
    pub private_data: &'a [u8],
}

const LINKAGE_TYPES_WITH_OTHER: &[u8] = &[0x09, 0x0A, 0x0B, 0x0C, 0x20, 0x21];

impl<'a> Parse<'a> for LinkageDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "LinkageDescriptor",
            "unexpected tag for linkage_descriptor",
        )?;
        if body.len() < FIXED_FIELDS_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "linkage_descriptor body shorter than minimum 7 bytes",
            });
        }
        let transport_stream_id = u16::from_be_bytes([body[0], body[1]]);
        let original_network_id = u16::from_be_bytes([body[2], body[3]]);
        let service_id = u16::from_be_bytes([body[4], body[5]]);
        let linkage_type = body[6];
        let tail = &body[FIXED_FIELDS_LEN..];
        let tail_len = tail.len();

        let (linkage_data, private_data) = match linkage_type {
            0x08 => {
                let info = parse_mobile_handover(tail, tail_len)?;
                let consumed = info.serialized_len();
                (LinkageData::MobileHandOver(info), &tail[consumed..])
            }
            0x0D => {
                let info = parse_event_linkage(tail)?;
                let consumed = EventLinkageInfo::SERIALIZED_LEN;
                (LinkageData::EventLinkage(info), &tail[consumed..])
            }
            0x0E..=0x1F => {
                let info = parse_extended_event_linkage(tail)?;
                let consumed = info.serialized_len();
                (LinkageData::ExtendedEventLinkage(info), &tail[consumed..])
            }
            lt if LINKAGE_TYPES_WITH_OTHER.contains(&lt) || (0x80..=0xFE).contains(&lt) => {
                (LinkageData::Other(tail), &[] as &[u8])
            }
            _ => (LinkageData::None, tail),
        };
        Ok(Self {
            transport_stream_id,
            original_network_id,
            service_id,
            linkage_type,
            linkage_data,
            private_data,
        })
    }
}

impl Serialize for LinkageDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_FIELDS_LEN + self.linkage_data.serialized_len() + self.private_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        let bs = HEADER_LEN;
        buf[bs..bs + 2].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[bs + 2..bs + 4].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[bs + 4..bs + 6].copy_from_slice(&self.service_id.to_be_bytes());
        buf[bs + 6] = self.linkage_type;
        let ld_start = bs + FIXED_FIELDS_LEN;
        let ld_written = self.linkage_data.serialize_into(&mut buf[ld_start..])?;
        let pd_start = ld_start + ld_written;
        if !self.private_data.is_empty() {
            buf[pd_start..pd_start + self.private_data.len()].copy_from_slice(self.private_data);
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for LinkageDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "LINKAGE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_tsid_onid_sid() {
        let bytes = [
            TAG, 0x09, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05, 0xAA, 0xBB,
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.transport_stream_id, 0x0001);
        assert_eq!(d.original_network_id, 0x0002);
        assert_eq!(d.service_id, 0x0003);
    }

    #[test]
    fn parse_extracts_linkage_type() {
        let bytes = [TAG, 0x07, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x06];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.linkage_type, 0x06);
    }

    #[test]
    fn parse_none_type_preserves_private_data() {
        let bytes = [
            TAG, 0x0A, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05, 0xAA, 0xBB, 0xCC,
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert!(matches!(d.linkage_data, LinkageData::None));
        assert_eq!(d.private_data, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn parse_accepts_empty_private_data() {
        let bytes = [TAG, 0x07, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert!(d.private_data.is_empty());
    }

    #[test]
    fn parse_mobile_handover_with_initial_sid() {
        let bytes = [
            TAG, 0x0E, // length 14 = 7 fixed + 5 handover + 2 priv
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x08, // linkage_type = mobile hand-over
            0x12, // hand_over_type=1, rfu=110, origin_type=0 (NIT)
            0x00, 0x10, // network_id
            0x00, 0x20, // initial_service_id
            0xDE, 0xAD, // private_data
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.linkage_type, 0x08);
        match &d.linkage_data {
            LinkageData::MobileHandOver(m) => {
                assert_eq!(m.hand_over_type, 1);
                assert!(!m.origin_type);
                assert_eq!(m.network_id, Some(0x0010));
                assert_eq!(m.initial_service_id, Some(0x0020));
            }
            other => panic!("expected MobileHandOver, got {other:?}"),
        }
        assert_eq!(d.private_data, &[0xDE, 0xAD]);
    }

    #[test]
    fn parse_mobile_handover_sdt_no_initial_sid() {
        let bytes = [
            TAG, 0x0C, // length 12 = 7 fixed + 3 handover + 2 priv
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x08, // linkage_type = mobile hand-over
            0x2F, // hand_over_type=2, rfu=111, origin_type=1 (SDT)
            0x00, 0x10, // network_id
            0xCA, 0xFE, // private_data
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        match &d.linkage_data {
            LinkageData::MobileHandOver(m) => {
                assert_eq!(m.hand_over_type, 2);
                assert!(m.origin_type);
                assert_eq!(m.network_id, Some(0x0010));
                assert_eq!(m.initial_service_id, None);
            }
            other => panic!("expected MobileHandOver, got {other:?}"),
        }
        assert_eq!(d.private_data, &[0xCA, 0xFE]);
    }

    #[test]
    fn parse_event_linkage() {
        let bytes = [
            TAG, 0x0C, // length 12 = 7 fixed + 3 event + 2 priv
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x0D, // linkage_type = event linkage
            0xAB, 0xCD, // target_event_id
            0xC0, // target_listed=1, event_simulcast=1, rfu=000000
            0xBE, 0xEF, // private_data
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        match &d.linkage_data {
            LinkageData::EventLinkage(e) => {
                assert_eq!(e.target_event_id, 0xABCD);
                assert!(e.target_listed);
                assert!(e.event_simulcast);
            }
            other => panic!("expected EventLinkage, got {other:?}"),
        }
        assert_eq!(d.private_data, &[0xBE, 0xEF]);
    }

    #[test]
    fn parse_extended_event_linkage_user_defined() {
        let bytes = [
            TAG, 0x0E, // length 14 = 7 fixed + 6 ext + 1 priv
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x0E, // linkage_type = extended event linkage
            0x05, // loop_length = 5
            0x12, 0x34, // target_event_id
            0xCC, // target_listed=1, event_simulcast=1, link_type=0, target_id_type=3, flags=0
            0x56, 0x78, // user_defined_id
            0xCC, // private_data
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        match &d.linkage_data {
            LinkageData::ExtendedEventLinkage(x) => {
                assert_eq!(x.entries.len(), 1);
                let e = &x.entries[0];
                assert_eq!(e.target_event_id, 0x1234);
                assert!(e.target_listed);
                assert!(e.event_simulcast);
                assert_eq!(e.link_type, 0);
                assert_eq!(e.target_id_type, 3);
                assert_eq!(
                    e.target_id,
                    TargetId::UserDefined {
                        user_defined_id: 0x5678
                    }
                );
            }
            other => panic!("expected ExtendedEventLinkage, got {other:?}"),
        }
        assert_eq!(d.private_data, &[0xCC]);
    }

    #[test]
    fn parse_extended_event_linkage_dvb_target() {
        let bytes = [
            TAG, 0x0F, // length 15 = 7 fixed + 8 ext
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x0F, // linkage_type
            0x07, // loop_length = 7
            0xAA, 0xBB, // target_event_id
            0x26, // target_listed=0, event_simulcast=0, link_type=2, target_id_type=1, onid_flag=1, sid_flag=0
            0x00, 0x11, // target_transport_stream_id (target_id_type=1)
            0x00, 0x22, // target_original_network_id (onid_flag=1)
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        match &d.linkage_data {
            LinkageData::ExtendedEventLinkage(x) => {
                assert_eq!(x.entries.len(), 1);
                let e = &x.entries[0];
                assert_eq!(e.target_id_type, 1);
                assert!(e.original_network_id_flag);
                assert!(!e.service_id_flag);
                assert_eq!(
                    e.target_id,
                    TargetId::Dvb {
                        target_id_type: 1,
                        target_transport_stream_id: Some(0x0011),
                        target_original_network_id: Some(0x0022),
                        target_service_id: None,
                    }
                );
            }
            other => panic!("expected ExtendedEventLinkage, got {other:?}"),
        }
        assert_eq!(d.private_data, &[] as &[u8]);
    }

    #[test]
    fn parse_other_type_captures_raw_tail() {
        let bytes = [
            TAG, 0x0A, // length 10
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x0B, // linkage_type = IP/MAC notification (EN 301 192)
            0xAA, 0xBB, 0xCC, // raw tail
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        match &d.linkage_data {
            LinkageData::Other(b) => assert_eq!(*b, &[0xAA, 0xBB, 0xCC]),
            other => panic!("expected Other, got {other:?}"),
        }
        assert!(d.private_data.is_empty());
    }

    #[test]
    fn parse_user_defined_type_is_other() {
        let bytes = [
            TAG, 0x09, // length 9
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x90, // user-defined linkage_type
            0xFF, 0xFE, // raw tail
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        assert!(matches!(d.linkage_data, LinkageData::Other(_)));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = LinkageDescriptor::parse(&[0x4B, 0x07, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x05])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x4B, .. }));
    }

    #[test]
    fn parse_rejects_body_shorter_than_seven() {
        let bytes = [TAG, 0x05, 0x00, 0x01, 0x00, 0x02, 0x00];
        let err = LinkageDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_truncated_buffer() {
        let err = LinkageDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_truncated_mobile_handover() {
        let bytes = [
            TAG, 0x08, // length 8
            0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x08, // linkage_type = mobile hand-over
            0x10, // flags byte (origin_type=0, needs initial_service_id)
            0x00, 0x10, // network_id
        ];
        let err = LinkageDescriptor::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::InvalidDescriptor { .. }),
            "expected InvalidDescriptor for truncated mobile hand-over, got {err:?}"
        );
    }

    #[test]
    fn serialize_round_trip_no_linkage_data() {
        let d = LinkageDescriptor {
            transport_stream_id: 0x1234,
            original_network_id: 0x5678,
            service_id: 0xABCD,
            linkage_type: 0x02,
            linkage_data: LinkageData::None,
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_with_private_data() {
        let d = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x05,
            linkage_data: LinkageData::None,
            private_data: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_mobile_handover() {
        let d = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x08,
            linkage_data: LinkageData::MobileHandOver(MobileHandOverInfo {
                hand_over_type: 3,
                origin_type: false,
                network_id: Some(0x0044),
                initial_service_id: Some(0x0055),
            }),
            private_data: &[0xFF],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_event_linkage() {
        let d = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x0D,
            linkage_data: LinkageData::EventLinkage(EventLinkageInfo {
                target_event_id: 0x1234,
                target_listed: true,
                event_simulcast: false,
            }),
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_extended_event_linkage() {
        let d = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x0E,
            linkage_data: LinkageData::ExtendedEventLinkage(ExtendedEventLinkageInfo {
                entries: vec![ExtendedEventLinkageEntry {
                    target_event_id: 0xAAAA,
                    target_listed: true,
                    event_simulcast: true,
                    link_type: 1,
                    target_id_type: 1,
                    original_network_id_flag: true,
                    service_id_flag: true,
                    target_id: TargetId::Dvb {
                        target_id_type: 1,
                        target_transport_stream_id: Some(0x1111),
                        target_original_network_id: Some(0x2222),
                        target_service_id: Some(0x3333),
                    },
                }],
            }),
            private_data: &[0xCC],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_round_trip_other() {
        let raw = [0xAA, 0xBB, 0xCC];
        let d = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x0B,
            linkage_data: LinkageData::Other(&raw),
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = LinkageDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_reserved_bits_are_set() {
        let d = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x0D,
            linkage_data: LinkageData::EventLinkage(EventLinkageInfo {
                target_event_id: 0x0000,
                target_listed: false,
                event_simulcast: false,
            }),
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[11] & RESERVED_EVENT_MASK, RESERVED_EVENT_MASK);

        let d2 = LinkageDescriptor {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
            linkage_type: 0x08,
            linkage_data: LinkageData::MobileHandOver(MobileHandOverInfo {
                hand_over_type: 0,
                origin_type: true,
                network_id: None,
                initial_service_id: None,
            }),
            private_data: &[],
        };
        let mut buf2 = vec![0u8; d2.serialized_len()];
        d2.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf2[9] & RESERVED_HANDOVER_MASK, RESERVED_HANDOVER_MASK);
    }

    #[test]
    fn parse_mobile_handover_type4_no_network_id() {
        let bytes = [
            TAG, 0x0A, // length 10 = 7 fixed + flags(1) + initial_sid(2)
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x08, // linkage_type = mobile hand-over
            0x4E, // hand_over_type=4, rfu=111, origin_type=0 (NIT)
            0x00, 0x20, // initial_service_id
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        match &d.linkage_data {
            LinkageData::MobileHandOver(m) => {
                assert_eq!(m.hand_over_type, 4);
                assert!(!m.origin_type);
                assert_eq!(m.network_id, None);
                assert_eq!(m.initial_service_id, Some(0x0020));
            }
            other => panic!("expected MobileHandOver, got {other:?}"),
        }
    }

    #[test]
    fn parse_mobile_handover_type1_network_id_present() {
        let bytes = [
            TAG, 0x0C, // length 12 = 7 fixed + 3 handover + 2 priv
            0x00, 0x01, // ts_id
            0x00, 0x02, // onid
            0x00, 0x03, // sid
            0x08, // linkage_type = mobile hand-over
            0x1F, // hand_over_type=1, rfu=111, origin_type=1 (SDT)
            0x00, 0x10, // network_id
            0xCA, 0xFE, // private_data
        ];
        let d = LinkageDescriptor::parse(&bytes).unwrap();
        match &d.linkage_data {
            LinkageData::MobileHandOver(m) => {
                assert_eq!(m.hand_over_type, 1);
                assert!(m.origin_type);
                assert_eq!(m.network_id, Some(0x0010));
                assert_eq!(m.initial_service_id, None);
            }
            other => panic!("expected MobileHandOver, got {other:?}"),
        }
        assert_eq!(d.private_data, &[0xCA, 0xFE]);
    }
}
