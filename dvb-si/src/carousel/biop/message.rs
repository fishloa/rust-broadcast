//! BIOP message types and `ModuleInfo` / `ServiceGatewayInfo` wire structures.
//!
//! All wire layouts from `docs/iso_13818_6_biop.md` (ETSI TR 101 202 §4.7.4–4.7.5).
//!
//! # Key entry points
//!
//! - [`BiopMessage::parse_at`] — parse one BIOP message from a slice, returning the
//!   message and the number of bytes consumed (use to walk a module buffer).
//! - [`ModuleInfo::parse`] — parse the DII `moduleInfoBytes` (Table 4.14).
//! - [`ServiceGatewayInfo::parse`] — parse the DSI `privateData` (Table 4.15).

use super::{
    ior::{Ior, NameComponent},
    BIOP_MAGIC, BIOP_VERSION_MAJOR, BIOP_VERSION_MINOR, BYTE_ORDER_BIG_ENDIAN,
    COMPRESSED_MODULE_DESCRIPTOR_TAG,
};
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

// ── Message header constants ──────────────────────────────────────────────────

/// BIOP message header: magic(4)+major(1)+minor(1)+byte_order(1)+message_type(1)+message_size(4) = 12.
const BIOP_HEADER_LEN: usize = 12;
/// `objectKey_length` (1 byte) field size.
const OBJECT_KEY_LEN_FIELD: usize = 1;
/// `objectKind_length` (4 bytes) field size.
const OBJECT_KIND_LEN_FIELD: usize = 4;
/// `objectKind_data` is always 4 bytes in DVB.
const OBJECT_KIND_DATA_LEN: usize = 4;
/// `objectInfo_length` (2 bytes) field size.
const OBJECT_INFO_LEN_FIELD: usize = 2;
/// `serviceContextList_count` (1 byte) field size.
const SERVICE_CONTEXT_COUNT_FIELD: usize = 1;
/// Per service context: context_id(4) + context_data_length(2).
const SERVICE_CONTEXT_FIXED: usize = 6;
/// `messageBody_length` (4 bytes) field size.
const MESSAGE_BODY_LEN_FIELD: usize = 4;
/// `bindings_count` (2 bytes) field size.
const BINDINGS_COUNT_FIELD: usize = 2;
/// `nameComponents_count` in a BIOP binding name: 1 byte.
const BINDING_NAME_COUNT_FIELD: usize = 1;
/// `bindingType` (1 byte) field.
const BINDING_TYPE_FIELD: usize = 1;
/// `objectInfo_length` in a binding (2 bytes).
const BINDING_OBJ_INFO_LEN_FIELD: usize = 2;
/// FileMessage: `content_length` (4 bytes).
const FILE_CONTENT_LEN_FIELD: usize = 4;
/// FileMessage: `ContentSize` (8 bytes, first 8 bytes of objectInfo).
const FILE_CONTENT_SIZE_LEN: usize = 8;
/// StreamMessage: `aDescription_length` field (1 byte).
const STREAM_ADESC_LEN_FIELD: usize = 1;
/// StreamMessage: `duration.aSeconds`(4) + `duration.aMicroSeconds`(2) + `audio`(1) + `video`(1) + `data`(1) = 9.
const STREAM_INFO_FIXED: usize = 9;
/// StreamMessage/StreamEventMessage: `taps_count` field (1 byte) in the message body.
const STREAM_TAPS_COUNT_FIELD: usize = 1;
/// StreamEventMessage: `eventNames_count` field (2 bytes).
const STREAM_EVENT_NAMES_COUNT_FIELD: usize = 2;
/// StreamEventMessage: per `eventName_length` field (1 byte).
const STREAM_EVENT_NAME_LEN_FIELD: usize = 1;
/// StreamEventMessage: `eventIds_count` field (1 byte).
const STREAM_EVENT_IDS_COUNT_FIELD: usize = 1;
/// StreamEventMessage: each `eventId` (2 bytes).
const STREAM_EVENT_ID_LEN: usize = 2;
/// ModuleInfo: ModuleTimeOut(4)+BlockTimeOut(4)+MinBlockTime(4) = 12.
const MODULE_INFO_FIXED: usize = 12;
/// ModuleInfo: taps_count (1 byte).
const MODULE_TAPS_COUNT_FIELD: usize = 1;
/// ModuleInfo: UserInfoLength (1 byte) — note: 8-bit, not 16-bit.
const MODULE_USER_INFO_LEN_FIELD: usize = 1;
/// SGI: downloadTaps_count (1 byte).
const SGI_DOWNLOAD_TAPS_COUNT_FIELD: usize = 1;
/// SGI: userInfoLength (2 bytes).
const SGI_USER_INFO_LEN_FIELD: usize = 2;

// ── Binding ───────────────────────────────────────────────────────────────────

/// One binding in a `DirectoryMessage` or `ServiceGatewayMessage`.
/// TR 101 202 §4.7.4.1, Table 4.9.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Binding<'a> {
    /// Name components — DVB: exactly one component.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub name: Vec<NameComponent<'a>>,
    /// `bindingType` — `0x01` (`nobject`) or `0x02` (`ncontext`); see the module-level constants.
    pub binding_type: u8,
    /// IOR of the bound object.
    pub ior: Ior<'a>,
    /// Per-binding `objectInfo` data.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_info: &'a [u8],
}

impl<'a> Binding<'a> {
    fn parse_from(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        // nameComponents_count (1 byte)
        if pos + BINDING_NAME_COUNT_FIELD > end {
            return Err(Error::BufferTooShort {
                need: pos + BINDING_NAME_COUNT_FIELD,
                have: end,
                what: "Binding nameComponents_count",
            });
        }
        let name_count = bytes[pos] as usize;
        let mut cur = pos + BINDING_NAME_COUNT_FIELD;
        let mut name = Vec::with_capacity(name_count.min(4));
        for _ in 0..name_count {
            let (nc, next) = NameComponent::parse_8bit(bytes, cur, end)?;
            name.push(nc);
            cur = next;
        }

        // bindingType (1 byte)
        if cur + BINDING_TYPE_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + BINDING_TYPE_FIELD,
                have: end,
                what: "Binding bindingType",
            });
        }
        let binding_type = bytes[cur];
        cur += BINDING_TYPE_FIELD;

        // IOR — parse the remainder using Ior::parse which reads from position 0
        // of a slice; we need to slice from cur to end.
        let ior_slice = &bytes[cur..end];
        let ior = Ior::parse(ior_slice)?;
        let ior_len = ior.serialized_len();
        cur += ior_len;

        // objectInfo_length (2 bytes)
        if cur + BINDING_OBJ_INFO_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + BINDING_OBJ_INFO_LEN_FIELD,
                have: end,
                what: "Binding objectInfo_length",
            });
        }
        let obj_info_len = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]) as usize;
        cur += BINDING_OBJ_INFO_LEN_FIELD;
        if cur + obj_info_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: obj_info_len,
                available: end - cur,
            });
        }
        let object_info = &bytes[cur..cur + obj_info_len];
        cur += obj_info_len;

        Ok((
            Binding {
                name,
                binding_type,
                ior,
                object_info,
            },
            cur,
        ))
    }

    fn serialized_len(&self) -> usize {
        let name_len: usize = self.name.iter().map(|n| n.serialized_len_8bit()).sum();
        BINDING_NAME_COUNT_FIELD
            + name_len
            + BINDING_TYPE_FIELD
            + self.ior.serialized_len()
            + BINDING_OBJ_INFO_LEN_FIELD
            + self.object_info.len()
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.name.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.name.len(),
                available: u8::MAX as usize,
            });
        }
        buf[0] = self.name.len() as u8;
        let mut pos = BINDING_NAME_COUNT_FIELD;
        for nc in &self.name {
            let written = nc.serialize_8bit(&mut buf[pos..])?;
            pos += written;
        }
        buf[pos] = self.binding_type;
        pos += BINDING_TYPE_FIELD;
        let written = self.ior.serialize_into(&mut buf[pos..])?;
        pos += written;
        if self.object_info.len() > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.object_info.len(),
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 2].copy_from_slice(&(self.object_info.len() as u16).to_be_bytes());
        pos += BINDING_OBJ_INFO_LEN_FIELD;
        buf[pos..pos + self.object_info.len()].copy_from_slice(self.object_info);
        pos += self.object_info.len();
        Ok(pos)
    }
}

// ── ServiceContext ────────────────────────────────────────────────────────────

/// One `serviceContext` entry in a BIOP message's `serviceContextList`.
/// ISO/IEC 13818-6 / TR 101 202 §4.7.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceContext<'a> {
    /// CDR `context_id` (32-bit).
    pub context_id: u32,
    /// `context_data` bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data: &'a [u8],
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse the common BIOP message header (magic, version, byte_order, message_type,
/// message_size, objectKey, objectKind).
/// Returns (object_key, object_kind_bytes, message_size, end_of_header_pos).
fn parse_biop_header(bytes: &[u8]) -> Result<(&[u8], [u8; 4], usize, usize)> {
    let total = bytes.len();
    if total < BIOP_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: BIOP_HEADER_LEN,
            have: total,
            what: "BIOP message header",
        });
    }
    let magic = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if magic != BIOP_MAGIC {
        return Err(Error::ReservedBitsViolation {
            field: "BIOP magic",
            reason: "must be 0x42494F50 (\"BIOP\")",
        });
    }
    if bytes[4] != BIOP_VERSION_MAJOR || bytes[5] != BIOP_VERSION_MINOR {
        return Err(Error::ReservedBitsViolation {
            field: "biop_version",
            reason: "must be 1.0",
        });
    }
    if bytes[6] != BYTE_ORDER_BIG_ENDIAN {
        return Err(Error::ReservedBitsViolation {
            field: "byte_order",
            reason: "must be 0x00 (big-endian) per DVB mandatory constraint",
        });
    }
    // bytes[7] = message_type (must be 0x00 per DVB)
    let message_size = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let end = BIOP_HEADER_LEN + message_size;
    if total < end {
        return Err(Error::SectionLengthOverflow {
            declared: message_size,
            available: total - BIOP_HEADER_LEN,
        });
    }
    let mut pos = BIOP_HEADER_LEN;

    // objectKey_length (1 byte) + objectKey_data
    if pos + OBJECT_KEY_LEN_FIELD > end {
        return Err(Error::BufferTooShort {
            need: pos + OBJECT_KEY_LEN_FIELD,
            have: end,
            what: "BIOP objectKey_length",
        });
    }
    let obj_key_len = bytes[pos] as usize;
    pos += OBJECT_KEY_LEN_FIELD;
    if pos + obj_key_len > end {
        return Err(Error::SectionLengthOverflow {
            declared: obj_key_len,
            available: end - pos,
        });
    }
    let object_key = &bytes[pos..pos + obj_key_len];
    pos += obj_key_len;

    // objectKind_length (4 bytes) + objectKind_data (4 bytes)
    if pos + OBJECT_KIND_LEN_FIELD > end {
        return Err(Error::BufferTooShort {
            need: pos + OBJECT_KIND_LEN_FIELD,
            have: end,
            what: "BIOP objectKind_length",
        });
    }
    let kind_len =
        u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
    pos += OBJECT_KIND_LEN_FIELD;
    if kind_len != OBJECT_KIND_DATA_LEN {
        return Err(Error::ValueOutOfRange {
            field: "objectKind_length",
            reason: "DVB BIOP objectKind must be exactly 4 bytes",
        });
    }
    if pos + OBJECT_KIND_DATA_LEN > end {
        return Err(Error::SectionLengthOverflow {
            declared: OBJECT_KIND_DATA_LEN,
            available: end - pos,
        });
    }
    let mut kind_bytes = [0u8; 4];
    kind_bytes.copy_from_slice(&bytes[pos..pos + 4]);
    pos += OBJECT_KIND_DATA_LEN;

    Ok((object_key, kind_bytes, message_size, pos))
}

/// Parse the `serviceContextList` and return the typed entries plus the
/// position after the list.
fn parse_service_context_list<'a>(
    bytes: &'a [u8],
    pos: usize,
    end: usize,
) -> Result<(Vec<ServiceContext<'a>>, usize)> {
    if pos + SERVICE_CONTEXT_COUNT_FIELD > end {
        return Err(Error::BufferTooShort {
            need: pos + SERVICE_CONTEXT_COUNT_FIELD,
            have: end,
            what: "serviceContextList_count",
        });
    }
    let count = bytes[pos] as usize;
    let mut cur = pos + SERVICE_CONTEXT_COUNT_FIELD;
    let mut list = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        if cur + SERVICE_CONTEXT_FIXED > end {
            return Err(Error::BufferTooShort {
                need: cur + SERVICE_CONTEXT_FIXED,
                have: end,
                what: "serviceContext entry",
            });
        }
        let context_id =
            u32::from_be_bytes([bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]]);
        let ctx_data_len = u16::from_be_bytes([bytes[cur + 4], bytes[cur + 5]]) as usize;
        cur += SERVICE_CONTEXT_FIXED;
        if cur + ctx_data_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: ctx_data_len,
                available: end - cur,
            });
        }
        let data = &bytes[cur..cur + ctx_data_len];
        cur += ctx_data_len;
        list.push(ServiceContext { context_id, data });
    }
    Ok((list, cur))
}

/// Serialized byte length of a `serviceContextList` (count byte + all entries).
fn service_context_list_len(list: &[ServiceContext]) -> usize {
    SERVICE_CONTEXT_COUNT_FIELD
        + list
            .iter()
            .map(|e| SERVICE_CONTEXT_FIXED + e.data.len())
            .sum::<usize>()
}

/// Write a `serviceContextList` into `buf` starting at offset 0. Returns bytes written.
fn write_service_context_list(buf: &mut [u8], list: &[ServiceContext]) -> Result<usize> {
    if list.len() > u8::MAX as usize {
        return Err(Error::SectionLengthOverflow {
            declared: list.len(),
            available: u8::MAX as usize,
        });
    }
    buf[0] = list.len() as u8;
    let mut pos = SERVICE_CONTEXT_COUNT_FIELD;
    for entry in list {
        if entry.data.len() > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: entry.data.len(),
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 4].copy_from_slice(&entry.context_id.to_be_bytes());
        buf[pos + 4..pos + 6].copy_from_slice(&(entry.data.len() as u16).to_be_bytes());
        pos += SERVICE_CONTEXT_FIXED;
        buf[pos..pos + entry.data.len()].copy_from_slice(entry.data);
        pos += entry.data.len();
    }
    Ok(pos)
}

/// Write the 12-byte BIOP message header to `buf` at position 0.
fn write_biop_header(buf: &mut [u8], message_size: u32) {
    buf[0..4].copy_from_slice(&BIOP_MAGIC.to_be_bytes());
    buf[4] = BIOP_VERSION_MAJOR;
    buf[5] = BIOP_VERSION_MINOR;
    buf[6] = BYTE_ORDER_BIG_ENDIAN;
    buf[7] = 0x00; // message_type
    buf[8..12].copy_from_slice(&message_size.to_be_bytes());
}

// ── DirectoryMessage ──────────────────────────────────────────────────────────

/// BIOP::DirectoryMessage — or ServiceGatewayMessage (same wire format, kind differs).
/// TR 101 202 §4.7.4.1/§4.7.4.4, Table 4.9.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DirectoryMessage<'a> {
    /// Object kind (`"dir\0"` or `"srg\0"`).
    pub object_kind: [u8; 4],
    /// `objectKey_data`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_key: &'a [u8],
    /// `objectInfo_data` (after key and kind but before serviceContextList).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_info: &'a [u8],
    /// Parsed `serviceContextList` entries.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_context: Vec<ServiceContext<'a>>,
    /// Binding entries.
    pub bindings: Vec<Binding<'a>>,
}

impl<'a> DirectoryMessage<'a> {
    /// True if this is a ServiceGateway object (`object_kind == "srg\0"`).
    pub fn is_service_gateway(&self) -> bool {
        &self.object_kind == b"srg\0"
    }

    fn parse_from(
        bytes: &'a [u8],
        object_key: &'a [u8],
        object_kind: [u8; 4],
        pos: usize,
        end: usize,
    ) -> Result<Self> {
        let mut cur = pos;

        // objectInfo_length (2 bytes) + objectInfo_data
        if cur + OBJECT_INFO_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + OBJECT_INFO_LEN_FIELD,
                have: end,
                what: "DirectoryMessage objectInfo_length",
            });
        }
        let obj_info_len = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]) as usize;
        cur += OBJECT_INFO_LEN_FIELD;
        if cur + obj_info_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: obj_info_len,
                available: end - cur,
            });
        }
        let object_info = &bytes[cur..cur + obj_info_len];
        cur += obj_info_len;

        // serviceContextList (raw)
        let (service_context, next) = parse_service_context_list(bytes, cur, end)?;
        cur = next;

        // messageBody_length (4 bytes)
        if cur + MESSAGE_BODY_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + MESSAGE_BODY_LEN_FIELD,
                have: end,
                what: "DirectoryMessage messageBody_length",
            });
        }
        let body_len =
            u32::from_be_bytes([bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]])
                as usize;
        cur += MESSAGE_BODY_LEN_FIELD;
        let body_end = cur + body_len;
        if body_end > end {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: end - cur,
            });
        }

        // bindings_count (2 bytes)
        if cur + BINDINGS_COUNT_FIELD > body_end {
            return Err(Error::BufferTooShort {
                need: cur + BINDINGS_COUNT_FIELD,
                have: body_end,
                what: "DirectoryMessage bindings_count",
            });
        }
        let bindings_count = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]) as usize;
        cur += BINDINGS_COUNT_FIELD;

        let mut bindings = Vec::with_capacity(bindings_count.min(256));
        for _ in 0..bindings_count {
            let (binding, next) = Binding::parse_from(bytes, cur, body_end)?;
            bindings.push(binding);
            cur = next;
        }

        Ok(DirectoryMessage {
            object_kind,
            object_key,
            object_info,
            service_context,
            bindings,
        })
    }

    fn body_len(&self) -> usize {
        let bindings_len: usize = self.bindings.iter().map(|b| b.serialized_len()).sum();
        BINDINGS_COUNT_FIELD + bindings_len
    }

    fn serialized_len_inner(&self) -> usize {
        // after the header: objectKey + objectKind + objectInfo + serviceContext + messageBody
        let key_part = OBJECT_KEY_LEN_FIELD
            + self.object_key.len()
            + OBJECT_KIND_LEN_FIELD
            + OBJECT_KIND_DATA_LEN;
        let info_part = OBJECT_INFO_LEN_FIELD + self.object_info.len();
        let svc_ctx_part = service_context_list_len(&self.service_context);
        let body_part = MESSAGE_BODY_LEN_FIELD + self.body_len();
        key_part + info_part + svc_ctx_part + body_part
    }

    /// Total serialized length including the 12-byte BIOP header.
    pub fn serialized_len_total(&self) -> usize {
        BIOP_HEADER_LEN + self.serialized_len_inner()
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let inner_len = self.serialized_len_inner();
        let total = BIOP_HEADER_LEN + inner_len;
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        if inner_len > u32::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: inner_len,
                available: u32::MAX as usize,
            });
        }
        write_biop_header(buf, inner_len as u32);
        let mut pos = BIOP_HEADER_LEN;

        // objectKey
        if self.object_key.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.object_key.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.object_key.len() as u8;
        pos += OBJECT_KEY_LEN_FIELD;
        buf[pos..pos + self.object_key.len()].copy_from_slice(self.object_key);
        pos += self.object_key.len();

        // objectKind
        buf[pos..pos + 4].copy_from_slice(&(OBJECT_KIND_DATA_LEN as u32).to_be_bytes());
        pos += OBJECT_KIND_LEN_FIELD;
        buf[pos..pos + 4].copy_from_slice(&self.object_kind);
        pos += OBJECT_KIND_DATA_LEN;

        // objectInfo
        if self.object_info.len() > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.object_info.len(),
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 2].copy_from_slice(&(self.object_info.len() as u16).to_be_bytes());
        pos += OBJECT_INFO_LEN_FIELD;
        buf[pos..pos + self.object_info.len()].copy_from_slice(self.object_info);
        pos += self.object_info.len();

        // serviceContextList
        pos += write_service_context_list(&mut buf[pos..], &self.service_context)?;

        // messageBody
        let body_len = self.body_len();
        if body_len > u32::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: u32::MAX as usize,
            });
        }
        buf[pos..pos + 4].copy_from_slice(&(body_len as u32).to_be_bytes());
        pos += MESSAGE_BODY_LEN_FIELD;

        // bindings_count
        if self.bindings.len() > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.bindings.len(),
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 2].copy_from_slice(&(self.bindings.len() as u16).to_be_bytes());
        pos += BINDINGS_COUNT_FIELD;

        for binding in &self.bindings {
            let written = binding.serialize_into_buf(&mut buf[pos..])?;
            pos += written;
        }

        Ok(total)
    }
}

// ── FileMessage ───────────────────────────────────────────────────────────────

/// BIOP::FileMessage — TR 101 202 §4.7.4.2, Table 4.10.
///
/// `objectInfo_length ≥ 8`; the first 8 bytes of objectInfo are the
/// `DSM::File::ContentSize` (64-bit big-endian).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileMessage<'a> {
    /// `objectKey_data`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_key: &'a [u8],
    /// `DSM::File::ContentSize` from the first 8 bytes of objectInfo.
    pub content_size: u64,
    /// Remaining objectInfo bytes after the 8-byte ContentSize.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_info_extra: &'a [u8],
    /// Parsed `serviceContextList` entries.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_context: Vec<ServiceContext<'a>>,
    /// File content bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub content: &'a [u8],
}

impl<'a> FileMessage<'a> {
    fn parse_from(bytes: &'a [u8], object_key: &'a [u8], pos: usize, end: usize) -> Result<Self> {
        let mut cur = pos;

        // objectInfo_length (2 bytes)
        if cur + OBJECT_INFO_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + OBJECT_INFO_LEN_FIELD,
                have: end,
                what: "FileMessage objectInfo_length",
            });
        }
        let obj_info_len = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]) as usize;
        cur += OBJECT_INFO_LEN_FIELD;
        if obj_info_len < FILE_CONTENT_SIZE_LEN {
            return Err(Error::ValueOutOfRange {
                field: "FileMessage.objectInfo_length",
                reason: "FileMessage objectInfo must be at least 8 bytes (ContentSize)",
            });
        }
        if cur + obj_info_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: obj_info_len,
                available: end - cur,
            });
        }
        let content_size = u64::from_be_bytes([
            bytes[cur],
            bytes[cur + 1],
            bytes[cur + 2],
            bytes[cur + 3],
            bytes[cur + 4],
            bytes[cur + 5],
            bytes[cur + 6],
            bytes[cur + 7],
        ]);
        let object_info_extra = &bytes[cur + FILE_CONTENT_SIZE_LEN..cur + obj_info_len];
        cur += obj_info_len;

        // serviceContextList
        let (service_context, next) = parse_service_context_list(bytes, cur, end)?;
        cur = next;

        // messageBody_length (4 bytes)
        if cur + MESSAGE_BODY_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + MESSAGE_BODY_LEN_FIELD,
                have: end,
                what: "FileMessage messageBody_length",
            });
        }
        let body_len =
            u32::from_be_bytes([bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]])
                as usize;
        cur += MESSAGE_BODY_LEN_FIELD;
        let body_end = cur + body_len;
        if body_end > end {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: end - cur,
            });
        }

        // content_length (4 bytes) + content_data
        if cur + FILE_CONTENT_LEN_FIELD > body_end {
            return Err(Error::BufferTooShort {
                need: cur + FILE_CONTENT_LEN_FIELD,
                have: body_end,
                what: "FileMessage content_length",
            });
        }
        let content_len =
            u32::from_be_bytes([bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]])
                as usize;
        cur += FILE_CONTENT_LEN_FIELD;
        if cur + content_len > body_end {
            return Err(Error::SectionLengthOverflow {
                declared: content_len,
                available: body_end - cur,
            });
        }
        let content = &bytes[cur..cur + content_len];

        Ok(FileMessage {
            object_key,
            content_size,
            object_info_extra,
            service_context,
            content,
        })
    }

    fn serialized_len_inner(&self) -> usize {
        let obj_info_total = FILE_CONTENT_SIZE_LEN + self.object_info_extra.len();
        OBJECT_KEY_LEN_FIELD
            + self.object_key.len()
            + OBJECT_KIND_LEN_FIELD
            + OBJECT_KIND_DATA_LEN
            + OBJECT_INFO_LEN_FIELD
            + obj_info_total
            + service_context_list_len(&self.service_context)
            + MESSAGE_BODY_LEN_FIELD
            + FILE_CONTENT_LEN_FIELD
            + self.content.len()
    }

    /// Total serialized length including the 12-byte BIOP header.
    pub fn serialized_len_total(&self) -> usize {
        BIOP_HEADER_LEN + self.serialized_len_inner()
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let inner_len = self.serialized_len_inner();
        let total = BIOP_HEADER_LEN + inner_len;
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        write_biop_header(buf, inner_len as u32);
        let mut pos = BIOP_HEADER_LEN;

        if self.object_key.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.object_key.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.object_key.len() as u8;
        pos += OBJECT_KEY_LEN_FIELD;
        buf[pos..pos + self.object_key.len()].copy_from_slice(self.object_key);
        pos += self.object_key.len();

        // objectKind = "fil\0"
        buf[pos..pos + 4].copy_from_slice(&(OBJECT_KIND_DATA_LEN as u32).to_be_bytes());
        pos += OBJECT_KIND_LEN_FIELD;
        buf[pos..pos + 4].copy_from_slice(b"fil\0");
        pos += OBJECT_KIND_DATA_LEN;

        // objectInfo: ContentSize(8) + extra
        let obj_info_total = FILE_CONTENT_SIZE_LEN + self.object_info_extra.len();
        if obj_info_total > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: obj_info_total,
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 2].copy_from_slice(&(obj_info_total as u16).to_be_bytes());
        pos += OBJECT_INFO_LEN_FIELD;
        buf[pos..pos + 8].copy_from_slice(&self.content_size.to_be_bytes());
        pos += FILE_CONTENT_SIZE_LEN;
        buf[pos..pos + self.object_info_extra.len()].copy_from_slice(self.object_info_extra);
        pos += self.object_info_extra.len();

        // serviceContextList
        pos += write_service_context_list(&mut buf[pos..], &self.service_context)?;

        // messageBody
        let body_len = FILE_CONTENT_LEN_FIELD + self.content.len();
        buf[pos..pos + 4].copy_from_slice(&(body_len as u32).to_be_bytes());
        pos += MESSAGE_BODY_LEN_FIELD;
        buf[pos..pos + 4].copy_from_slice(&(self.content.len() as u32).to_be_bytes());
        pos += FILE_CONTENT_LEN_FIELD;
        buf[pos..pos + self.content.len()].copy_from_slice(self.content);

        Ok(total)
    }
}

// ── DsmStreamInfo ─────────────────────────────────────────────────────────────

/// `DSM::Stream::Info_T` — the mandatory objectInfo head shared by
/// `StreamMessage` and `StreamEventMessage`.
/// TR 101 202 §4.7.4.3, Table 4.11.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DsmStreamInfo<'a> {
    /// `aDescription_bytes` — freeform description of the stream.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub description: &'a [u8],
    /// `duration.aSeconds` — AppNPT seconds (signed, `simsbf`).
    pub duration_seconds: i32,
    /// `duration.aMicroSeconds`.
    pub duration_microseconds: u16,
    /// `audio` flag byte.
    pub audio: u8,
    /// `video` flag byte.
    pub video: u8,
    /// `data` flag byte.
    pub data: u8,
}

impl<'a> DsmStreamInfo<'a> {
    /// Serialized byte length of this Info_T block (N2 + 10 per the spec).
    fn serialized_len(&self) -> usize {
        STREAM_ADESC_LEN_FIELD + self.description.len() + STREAM_INFO_FIXED
    }

    /// Parse an Info_T block from `bytes[pos..end]`, return `(Self, next_pos)`.
    fn parse_from(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        // aDescription_length (1 byte)
        if pos + STREAM_ADESC_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: pos + STREAM_ADESC_LEN_FIELD,
                have: end,
                what: "DsmStreamInfo aDescription_length",
            });
        }
        let desc_len = bytes[pos] as usize;
        let mut cur = pos + STREAM_ADESC_LEN_FIELD;

        // aDescription_bytes
        if cur + desc_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: desc_len,
                available: end - cur,
            });
        }
        let description = &bytes[cur..cur + desc_len];
        cur += desc_len;

        // duration.aSeconds(4 signed) + aMicroSeconds(2) + audio(1) + video(1) + data(1)
        if cur + STREAM_INFO_FIXED > end {
            return Err(Error::BufferTooShort {
                need: cur + STREAM_INFO_FIXED,
                have: end,
                what: "DsmStreamInfo fixed fields",
            });
        }
        let duration_seconds =
            i32::from_be_bytes([bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]]);
        cur += 4;
        let duration_microseconds = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]);
        cur += 2;
        let audio = bytes[cur];
        cur += 1;
        let video = bytes[cur];
        cur += 1;
        let data = bytes[cur];
        cur += 1;

        Ok((
            DsmStreamInfo {
                description,
                duration_seconds,
                duration_microseconds,
                audio,
                video,
                data,
            },
            cur,
        ))
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.description.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.description.len(),
                available: u8::MAX as usize,
            });
        }
        buf[0] = self.description.len() as u8;
        let mut pos = STREAM_ADESC_LEN_FIELD;
        buf[pos..pos + self.description.len()].copy_from_slice(self.description);
        pos += self.description.len();
        buf[pos..pos + 4].copy_from_slice(&self.duration_seconds.to_be_bytes());
        pos += 4;
        buf[pos..pos + 2].copy_from_slice(&self.duration_microseconds.to_be_bytes());
        pos += 2;
        buf[pos] = self.audio;
        pos += 1;
        buf[pos] = self.video;
        pos += 1;
        buf[pos] = self.data;
        pos += 1;
        Ok(pos)
    }
}

// ── StreamMessage ─────────────────────────────────────────────────────────────

/// BIOP::StreamMessage — TR 101 202 §4.7.4.3, Table 4.11.
/// `objectKind = "str\0"`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StreamMessage<'a> {
    /// `objectKey_data`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_key: &'a [u8],
    /// `DSM::Stream::Info_T` parsed from the head of objectInfo.
    pub stream_info: DsmStreamInfo<'a>,
    /// Trailing objectInfo bytes after Info_T (may be empty).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_info_extra: &'a [u8],
    /// Parsed `serviceContextList` entries.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_context: Vec<ServiceContext<'a>>,
    /// Tap entries from the message body.
    pub taps: Vec<super::ior::Tap<'a>>,
}

impl<'a> StreamMessage<'a> {
    fn parse_from(bytes: &'a [u8], object_key: &'a [u8], pos: usize, end: usize) -> Result<Self> {
        let mut cur = pos;

        // objectInfo_length (2 bytes)
        if cur + OBJECT_INFO_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + OBJECT_INFO_LEN_FIELD,
                have: end,
                what: "StreamMessage objectInfo_length",
            });
        }
        let obj_info_len = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]) as usize;
        cur += OBJECT_INFO_LEN_FIELD;
        if cur + obj_info_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: obj_info_len,
                available: end - cur,
            });
        }
        let obj_info_start = cur;
        let obj_info_end = cur + obj_info_len;

        // DSM::Stream::Info_T
        let (stream_info, _) = DsmStreamInfo::parse_from(bytes, cur, obj_info_end)?;
        let info_len = stream_info.serialized_len();
        if obj_info_len < info_len {
            return Err(Error::ValueOutOfRange {
                field: "StreamMessage.objectInfo_length",
                reason: "objectInfo too short for DSM::Stream::Info_T",
            });
        }
        let object_info_extra = &bytes[obj_info_start + info_len..obj_info_end];
        cur = obj_info_end;

        // serviceContextList
        let (service_context, next) = parse_service_context_list(bytes, cur, end)?;
        cur = next;

        // messageBody_length (4 bytes)
        if cur + MESSAGE_BODY_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + MESSAGE_BODY_LEN_FIELD,
                have: end,
                what: "StreamMessage messageBody_length",
            });
        }
        let body_len =
            u32::from_be_bytes([bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]])
                as usize;
        cur += MESSAGE_BODY_LEN_FIELD;
        let body_end = cur + body_len;
        if body_end > end {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: end - cur,
            });
        }

        // taps_count (1 byte)
        if cur + STREAM_TAPS_COUNT_FIELD > body_end {
            return Err(Error::BufferTooShort {
                need: cur + STREAM_TAPS_COUNT_FIELD,
                have: body_end,
                what: "StreamMessage taps_count",
            });
        }
        let taps_count = bytes[cur] as usize;
        cur += STREAM_TAPS_COUNT_FIELD;

        let mut taps = Vec::with_capacity(taps_count.min(16));
        for _ in 0..taps_count {
            let (tap, next) = super::ior::Tap::parse_from(bytes, cur, body_end)?;
            taps.push(tap);
            cur = next;
        }

        Ok(StreamMessage {
            object_key,
            stream_info,
            object_info_extra,
            service_context,
            taps,
        })
    }

    fn body_len(&self) -> usize {
        let taps_len: usize = self.taps.iter().map(|t| t.serialized_len()).sum();
        STREAM_TAPS_COUNT_FIELD + taps_len
    }

    fn obj_info_len(&self) -> usize {
        self.stream_info.serialized_len() + self.object_info_extra.len()
    }

    fn serialized_len_inner(&self) -> usize {
        OBJECT_KEY_LEN_FIELD
            + self.object_key.len()
            + OBJECT_KIND_LEN_FIELD
            + OBJECT_KIND_DATA_LEN
            + OBJECT_INFO_LEN_FIELD
            + self.obj_info_len()
            + service_context_list_len(&self.service_context)
            + MESSAGE_BODY_LEN_FIELD
            + self.body_len()
    }

    /// Total serialized length including the 12-byte BIOP header.
    pub fn serialized_len_total(&self) -> usize {
        BIOP_HEADER_LEN + self.serialized_len_inner()
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let inner_len = self.serialized_len_inner();
        let total = BIOP_HEADER_LEN + inner_len;
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        write_biop_header(buf, inner_len as u32);
        let mut pos = BIOP_HEADER_LEN;

        // objectKey
        if self.object_key.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.object_key.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.object_key.len() as u8;
        pos += OBJECT_KEY_LEN_FIELD;
        buf[pos..pos + self.object_key.len()].copy_from_slice(self.object_key);
        pos += self.object_key.len();

        // objectKind = "str\0"
        buf[pos..pos + 4].copy_from_slice(&(OBJECT_KIND_DATA_LEN as u32).to_be_bytes());
        pos += OBJECT_KIND_LEN_FIELD;
        buf[pos..pos + 4].copy_from_slice(b"str\0");
        pos += OBJECT_KIND_DATA_LEN;

        // objectInfo_length
        let oi_len = self.obj_info_len();
        if oi_len > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: oi_len,
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 2].copy_from_slice(&(oi_len as u16).to_be_bytes());
        pos += OBJECT_INFO_LEN_FIELD;

        // Info_T
        let written = self.stream_info.serialize_into_buf(&mut buf[pos..])?;
        pos += written;

        // object_info_extra
        buf[pos..pos + self.object_info_extra.len()].copy_from_slice(self.object_info_extra);
        pos += self.object_info_extra.len();

        // serviceContextList
        pos += write_service_context_list(&mut buf[pos..], &self.service_context)?;

        // messageBody_length
        let bl = self.body_len();
        buf[pos..pos + 4].copy_from_slice(&(bl as u32).to_be_bytes());
        pos += MESSAGE_BODY_LEN_FIELD;

        // taps_count
        if self.taps.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.taps.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.taps.len() as u8;
        pos += STREAM_TAPS_COUNT_FIELD;
        for tap in &self.taps {
            let written = tap.serialize_into_buf(&mut buf[pos..])?;
            pos += written;
        }

        Ok(total)
    }
}

// ── StreamEventMessage ────────────────────────────────────────────────────────

/// BIOP::StreamEventMessage — TR 101 202 §4.7.4.5, Table 4.13.
/// `objectKind = "ste\0"`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StreamEventMessage<'a> {
    /// `objectKey_data`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_key: &'a [u8],
    /// `DSM::Stream::Info_T` parsed from the head of objectInfo.
    pub stream_info: DsmStreamInfo<'a>,
    /// Event names from `DSM::Event::EventList_T` (each = raw `eventName_data` bytes,
    /// without the length prefix).
    pub event_names: Vec<&'a [u8]>,
    /// Trailing objectInfo bytes after Info_T and EventList_T (may be empty).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_info_extra: &'a [u8],
    /// Parsed `serviceContextList` entries.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_context: Vec<ServiceContext<'a>>,
    /// Tap entries from the message body.
    pub taps: Vec<super::ior::Tap<'a>>,
    /// `eventId` values — one per `event_names` entry.
    pub event_ids: Vec<u16>,
}

impl<'a> StreamEventMessage<'a> {
    fn parse_from(bytes: &'a [u8], object_key: &'a [u8], pos: usize, end: usize) -> Result<Self> {
        let mut cur = pos;

        // objectInfo_length (2 bytes)
        if cur + OBJECT_INFO_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + OBJECT_INFO_LEN_FIELD,
                have: end,
                what: "StreamEventMessage objectInfo_length",
            });
        }
        let obj_info_len = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]) as usize;
        cur += OBJECT_INFO_LEN_FIELD;
        if cur + obj_info_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: obj_info_len,
                available: end - cur,
            });
        }
        let obj_info_end = cur + obj_info_len;

        // DSM::Stream::Info_T
        let (stream_info, next_cur) = DsmStreamInfo::parse_from(bytes, cur, obj_info_end)?;
        cur = next_cur;

        // DSM::Event::EventList_T: eventNames_count (2 bytes)
        if cur + STREAM_EVENT_NAMES_COUNT_FIELD > obj_info_end {
            return Err(Error::BufferTooShort {
                need: cur + STREAM_EVENT_NAMES_COUNT_FIELD,
                have: obj_info_end,
                what: "StreamEventMessage eventNames_count",
            });
        }
        let event_names_count = u16::from_be_bytes([bytes[cur], bytes[cur + 1]]) as usize;
        cur += STREAM_EVENT_NAMES_COUNT_FIELD;

        let mut event_names = Vec::with_capacity(event_names_count.min(64));
        for _ in 0..event_names_count {
            if cur + STREAM_EVENT_NAME_LEN_FIELD > obj_info_end {
                return Err(Error::BufferTooShort {
                    need: cur + STREAM_EVENT_NAME_LEN_FIELD,
                    have: obj_info_end,
                    what: "StreamEventMessage eventName_length",
                });
            }
            let name_len = bytes[cur] as usize;
            cur += STREAM_EVENT_NAME_LEN_FIELD;
            if cur + name_len > obj_info_end {
                return Err(Error::SectionLengthOverflow {
                    declared: name_len,
                    available: obj_info_end - cur,
                });
            }
            event_names.push(&bytes[cur..cur + name_len]);
            cur += name_len;
        }

        // trailing objectInfo extra
        let object_info_extra = &bytes[cur..obj_info_end];
        cur = obj_info_end;

        // serviceContextList
        let (service_context, next) = parse_service_context_list(bytes, cur, end)?;
        cur = next;

        // messageBody_length (4 bytes)
        if cur + MESSAGE_BODY_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: cur + MESSAGE_BODY_LEN_FIELD,
                have: end,
                what: "StreamEventMessage messageBody_length",
            });
        }
        let body_len =
            u32::from_be_bytes([bytes[cur], bytes[cur + 1], bytes[cur + 2], bytes[cur + 3]])
                as usize;
        cur += MESSAGE_BODY_LEN_FIELD;
        let body_end = cur + body_len;
        if body_end > end {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: end - cur,
            });
        }

        // taps_count (1 byte)
        if cur + STREAM_TAPS_COUNT_FIELD > body_end {
            return Err(Error::BufferTooShort {
                need: cur + STREAM_TAPS_COUNT_FIELD,
                have: body_end,
                what: "StreamEventMessage taps_count",
            });
        }
        let taps_count = bytes[cur] as usize;
        cur += STREAM_TAPS_COUNT_FIELD;

        let mut taps = Vec::with_capacity(taps_count.min(16));
        for _ in 0..taps_count {
            let (tap, next) = super::ior::Tap::parse_from(bytes, cur, body_end)?;
            taps.push(tap);
            cur = next;
        }

        // eventIds_count (1 byte) — must equal eventNames_count
        if cur + STREAM_EVENT_IDS_COUNT_FIELD > body_end {
            return Err(Error::BufferTooShort {
                need: cur + STREAM_EVENT_IDS_COUNT_FIELD,
                have: body_end,
                what: "StreamEventMessage eventIds_count",
            });
        }
        let event_ids_count = bytes[cur] as usize;
        cur += STREAM_EVENT_IDS_COUNT_FIELD;
        if event_ids_count != event_names_count {
            return Err(Error::ValueOutOfRange {
                field: "StreamEventMessage.eventIds_count",
                reason: "eventIds_count must equal eventNames_count",
            });
        }

        let mut event_ids = Vec::with_capacity(event_ids_count.min(64));
        for _ in 0..event_ids_count {
            if cur + STREAM_EVENT_ID_LEN > body_end {
                return Err(Error::BufferTooShort {
                    need: cur + STREAM_EVENT_ID_LEN,
                    have: body_end,
                    what: "StreamEventMessage eventId",
                });
            }
            event_ids.push(u16::from_be_bytes([bytes[cur], bytes[cur + 1]]));
            cur += STREAM_EVENT_ID_LEN;
        }

        let _ = cur; // consumed
        Ok(StreamEventMessage {
            object_key,
            stream_info,
            event_names,
            object_info_extra,
            service_context,
            taps,
            event_ids,
        })
    }

    /// Byte count of the EventList_T block as written on the wire.
    fn event_list_wire_len(&self) -> usize {
        let names_len: usize = self
            .event_names
            .iter()
            .map(|n| STREAM_EVENT_NAME_LEN_FIELD + n.len())
            .sum();
        STREAM_EVENT_NAMES_COUNT_FIELD + names_len
    }

    fn body_len(&self) -> usize {
        let taps_len: usize = self.taps.iter().map(|t| t.serialized_len()).sum();
        STREAM_TAPS_COUNT_FIELD
            + taps_len
            + STREAM_EVENT_IDS_COUNT_FIELD
            + self.event_ids.len() * STREAM_EVENT_ID_LEN
    }

    fn obj_info_len(&self) -> usize {
        self.stream_info.serialized_len()
            + self.event_list_wire_len()
            + self.object_info_extra.len()
    }

    fn serialized_len_inner(&self) -> usize {
        OBJECT_KEY_LEN_FIELD
            + self.object_key.len()
            + OBJECT_KIND_LEN_FIELD
            + OBJECT_KIND_DATA_LEN
            + OBJECT_INFO_LEN_FIELD
            + self.obj_info_len()
            + service_context_list_len(&self.service_context)
            + MESSAGE_BODY_LEN_FIELD
            + self.body_len()
    }

    /// Total serialized length including the 12-byte BIOP header.
    pub fn serialized_len_total(&self) -> usize {
        BIOP_HEADER_LEN + self.serialized_len_inner()
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let inner_len = self.serialized_len_inner();
        let total = BIOP_HEADER_LEN + inner_len;
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        write_biop_header(buf, inner_len as u32);
        let mut pos = BIOP_HEADER_LEN;

        // objectKey
        if self.object_key.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.object_key.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.object_key.len() as u8;
        pos += OBJECT_KEY_LEN_FIELD;
        buf[pos..pos + self.object_key.len()].copy_from_slice(self.object_key);
        pos += self.object_key.len();

        // objectKind = "ste\0"
        buf[pos..pos + 4].copy_from_slice(&(OBJECT_KIND_DATA_LEN as u32).to_be_bytes());
        pos += OBJECT_KIND_LEN_FIELD;
        buf[pos..pos + 4].copy_from_slice(b"ste\0");
        pos += OBJECT_KIND_DATA_LEN;

        // objectInfo_length
        let oi_len = self.obj_info_len();
        if oi_len > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: oi_len,
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 2].copy_from_slice(&(oi_len as u16).to_be_bytes());
        pos += OBJECT_INFO_LEN_FIELD;

        // Info_T
        let written = self.stream_info.serialize_into_buf(&mut buf[pos..])?;
        pos += written;

        // EventList_T: eventNames_count (2 bytes)
        if self.event_names.len() > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.event_names.len(),
                available: u16::MAX as usize,
            });
        }
        buf[pos..pos + 2].copy_from_slice(&(self.event_names.len() as u16).to_be_bytes());
        pos += STREAM_EVENT_NAMES_COUNT_FIELD;
        for name in &self.event_names {
            if name.len() > u8::MAX as usize {
                return Err(Error::SectionLengthOverflow {
                    declared: name.len(),
                    available: u8::MAX as usize,
                });
            }
            buf[pos] = name.len() as u8;
            pos += STREAM_EVENT_NAME_LEN_FIELD;
            buf[pos..pos + name.len()].copy_from_slice(name);
            pos += name.len();
        }

        // object_info_extra
        buf[pos..pos + self.object_info_extra.len()].copy_from_slice(self.object_info_extra);
        pos += self.object_info_extra.len();

        // serviceContextList
        pos += write_service_context_list(&mut buf[pos..], &self.service_context)?;

        // messageBody_length
        let bl = self.body_len();
        buf[pos..pos + 4].copy_from_slice(&(bl as u32).to_be_bytes());
        pos += MESSAGE_BODY_LEN_FIELD;

        // taps_count
        if self.taps.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.taps.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.taps.len() as u8;
        pos += STREAM_TAPS_COUNT_FIELD;
        for tap in &self.taps {
            let written = tap.serialize_into_buf(&mut buf[pos..])?;
            pos += written;
        }

        // eventIds_count (1 byte) + eventIds
        if self.event_ids.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.event_ids.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.event_ids.len() as u8;
        pos += STREAM_EVENT_IDS_COUNT_FIELD;
        for &id in &self.event_ids {
            buf[pos..pos + 2].copy_from_slice(&id.to_be_bytes());
            pos += STREAM_EVENT_ID_LEN;
        }

        Ok(total)
    }
}

// ── BiopMessage ───────────────────────────────────────────────────────────────

/// A parsed BIOP message — discriminated by `objectKind`.
/// TR 101 202 §4.7.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum BiopMessage<'a> {
    /// `"dir\0"` — DSM::DirectoryMessage.
    Directory(DirectoryMessage<'a>),
    /// `"fil\0"` — DSM::FileMessage.
    File(FileMessage<'a>),
    /// `"srg\0"` — DSM::ServiceGatewayMessage (same wire format as Directory).
    ServiceGateway(DirectoryMessage<'a>),
    /// `"str\0"` — DSM::StreamMessage.
    Stream(StreamMessage<'a>),
    /// `"ste\0"` — BIOP::StreamEventMessage.
    StreamEvent(StreamEventMessage<'a>),
}

impl<'a> BiopMessage<'a> {
    /// Parse one BIOP message from `bytes` starting at offset 0.
    ///
    /// Returns `(message, consumed)` where `consumed` is exactly
    /// `12 + message_size` (the number of bytes consumed from `bytes`).
    pub fn parse_at(bytes: &'a [u8]) -> Result<(Self, usize)> {
        let (object_key, kind_bytes, message_size, pos) = parse_biop_header(bytes)?;
        let consumed = BIOP_HEADER_LEN + message_size;
        let end = consumed;

        let msg = match &kind_bytes {
            b"dir\0" => {
                let dm = DirectoryMessage::parse_from(bytes, object_key, kind_bytes, pos, end)?;
                BiopMessage::Directory(dm)
            }
            b"srg\0" => {
                let dm = DirectoryMessage::parse_from(bytes, object_key, kind_bytes, pos, end)?;
                BiopMessage::ServiceGateway(dm)
            }
            b"fil\0" => {
                let fm = FileMessage::parse_from(bytes, object_key, pos, end)?;
                BiopMessage::File(fm)
            }
            b"str\0" => {
                let sm = StreamMessage::parse_from(bytes, object_key, pos, end)?;
                BiopMessage::Stream(sm)
            }
            b"ste\0" => {
                let se = StreamEventMessage::parse_from(bytes, object_key, pos, end)?;
                BiopMessage::StreamEvent(se)
            }
            _ => {
                return Err(Error::ValueOutOfRange {
                    field: "BiopMessage.objectKind",
                    reason: "unknown BIOP objectKind",
                });
            }
        };

        Ok((msg, consumed))
    }

    fn serialized_len_total(&self) -> usize {
        match self {
            Self::Directory(d) | Self::ServiceGateway(d) => d.serialized_len_total(),
            Self::File(f) => f.serialized_len_total(),
            Self::Stream(s) => s.serialized_len_total(),
            Self::StreamEvent(se) => se.serialized_len_total(),
        }
    }
}

impl Serialize for BiopMessage<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        self.serialized_len_total()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len_total();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        match self {
            Self::Directory(d) | Self::ServiceGateway(d) => {
                d.serialize_into_buf(buf)?;
            }
            Self::File(f) => {
                f.serialize_into_buf(buf)?;
            }
            Self::Stream(s) => {
                s.serialize_into_buf(buf)?;
            }
            Self::StreamEvent(se) => {
                se.serialize_into_buf(buf)?;
            }
        }
        Ok(len)
    }
}

// ── ModuleInfo ────────────────────────────────────────────────────────────────

/// BIOP::ModuleInfo — carried in the DII `moduleInfoBytes`.
/// TR 101 202 §4.7.5.1, Table 4.14.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ModuleInfo<'a> {
    /// `ModuleTimeOut` — µs to time out acquisition of all blocks.
    pub module_timeout: u32,
    /// `BlockTimeOut` — µs to time out the next block.
    pub block_timeout: u32,
    /// `MinBlockTime` — min µs between two blocks.
    pub min_block_time: u32,
    /// BIOP::Tap entries (≥1 BIOP_OBJECT_USE tap).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub taps: Vec<super::ior::Tap<'a>>,
    /// `userInfo` descriptor loop bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub user_info: &'a [u8],
}

impl<'a> ModuleInfo<'a> {
    /// Iterate over descriptors in the `userInfo` loop.
    ///
    /// Each item is `(tag: u8, data: &[u8])`.
    pub fn descriptors(&self) -> impl Iterator<Item = (u8, &[u8])> {
        DescriptorIter {
            data: self.user_info,
            pos: 0,
        }
    }

    /// Return the `compressed_module_descriptor` (tag 0x09) from the userInfo
    /// loop, if present.
    pub fn compressed_module_descriptor(&self) -> Option<CompressedModuleDescriptor<'_>> {
        for (tag, data) in self.descriptors() {
            if tag == COMPRESSED_MODULE_DESCRIPTOR_TAG {
                return Some(CompressedModuleDescriptor { body: data });
            }
        }
        None
    }
}

struct DescriptorIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for DescriptorIter<'a> {
    type Item = (u8, &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        let end = self.data.len();
        if self.pos + 2 > end {
            return None;
        }
        let tag = self.data[self.pos];
        let len = self.data[self.pos + 1] as usize;
        self.pos += 2;
        if self.pos + len > end {
            return None;
        }
        let d = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Some((tag, d))
    }
}

impl<'a> Parse<'a> for ModuleInfo<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let end = bytes.len();
        if end < MODULE_INFO_FIXED + MODULE_TAPS_COUNT_FIELD {
            return Err(Error::BufferTooShort {
                need: MODULE_INFO_FIXED + MODULE_TAPS_COUNT_FIELD,
                have: end,
                what: "ModuleInfo fixed fields",
            });
        }
        let module_timeout = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let block_timeout = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let min_block_time = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let taps_count = bytes[12] as usize;
        let mut pos = MODULE_INFO_FIXED + MODULE_TAPS_COUNT_FIELD;

        let mut taps = Vec::with_capacity(taps_count.min(8));
        for _ in 0..taps_count {
            let (tap, next) = super::ior::Tap::parse_from(bytes, pos, end)?;
            taps.push(tap);
            pos = next;
        }

        if pos + MODULE_USER_INFO_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: pos + MODULE_USER_INFO_LEN_FIELD,
                have: end,
                what: "ModuleInfo UserInfoLength",
            });
        }
        let user_info_len = bytes[pos] as usize;
        pos += MODULE_USER_INFO_LEN_FIELD;
        if pos + user_info_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: user_info_len,
                available: end - pos,
            });
        }
        let user_info = &bytes[pos..pos + user_info_len];

        Ok(ModuleInfo {
            module_timeout,
            block_timeout,
            min_block_time,
            taps,
            user_info,
        })
    }
}

impl Serialize for ModuleInfo<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let taps_len: usize = self.taps.iter().map(|t| t.serialized_len()).sum();
        MODULE_INFO_FIXED
            + MODULE_TAPS_COUNT_FIELD
            + taps_len
            + MODULE_USER_INFO_LEN_FIELD
            + self.user_info.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.module_timeout.to_be_bytes());
        buf[4..8].copy_from_slice(&self.block_timeout.to_be_bytes());
        buf[8..12].copy_from_slice(&self.min_block_time.to_be_bytes());
        if self.taps.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.taps.len(),
                available: u8::MAX as usize,
            });
        }
        buf[12] = self.taps.len() as u8;
        let mut pos = MODULE_INFO_FIXED + MODULE_TAPS_COUNT_FIELD;
        for tap in &self.taps {
            let written = tap.serialize_into_buf(&mut buf[pos..])?;
            pos += written;
        }
        if self.user_info.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.user_info.len(),
                available: u8::MAX as usize,
            });
        }
        buf[pos] = self.user_info.len() as u8;
        pos += MODULE_USER_INFO_LEN_FIELD;
        buf[pos..pos + self.user_info.len()].copy_from_slice(self.user_info);
        pos += self.user_info.len();
        Ok(pos)
    }
}

// ── CompressedModuleDescriptor ────────────────────────────────────────────────

/// A `compressed_module_descriptor` (tag 0x09) found in a `ModuleInfo` userInfo loop.
/// TR 101 202 §4.6.6.10.
///
/// The body bytes are the zlib-encoded module payload (RFC 1950 CMF+FLG header,
/// DEFLATE stream, Adler-32 checksum).  Decompression requires the `flate2` feature.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompressedModuleDescriptor<'a> {
    /// Raw descriptor body (the zlib stream).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub body: &'a [u8],
}

/// Decompress a zlib-encoded module payload.
///
/// Uses [`flate2`](https://crates.io/crates/flate2) (optional feature `flate2`).
/// Returns the decompressed bytes, or an error if the zlib stream is invalid.
#[cfg(feature = "flate2")]
pub fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| Error::ReservedBitsViolation {
            field: "compressed_module_descriptor body",
            reason: if e.kind() == std::io::ErrorKind::InvalidData {
                "zlib decompression failed: invalid data"
            } else {
                "zlib decompression failed"
            },
        })?;
    Ok(out)
}

// ── ServiceGatewayInfo ────────────────────────────────────────────────────────

/// BIOP::ServiceGatewayInfo — the DSI `privateData` for an object carousel.
/// TR 101 202 §4.7.5.2, Table 4.15.
///
/// Parse with [`ServiceGatewayInfo::parse`]; serialize with [`ServiceGatewayInfo::to_bytes`].
/// The round-trip `to_bytes() == dsi.private_data` is a hard project invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceGatewayInfo<'a> {
    /// IOR of the ServiceGateway object.
    pub ior: Ior<'a>,
    /// Raw `Tap() × downloadTaps_count` bytes (count byte + tap data).
    /// In practice `downloadTaps_count` is typically 0, making this `&[0x00]`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub download_taps: &'a [u8],
    /// Parsed `serviceContextList` entries.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_context: Vec<ServiceContext<'a>>,
    /// `userInfo` descriptor loop bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub user_info: &'a [u8],
}

impl<'a> ServiceGatewayInfo<'a> {
    /// Parse the DSI `privateData` bytes as a ServiceGatewayInfo.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let end = bytes.len();
        let ior = Ior::parse(bytes)?;
        let mut pos = ior.serialized_len();

        // downloadTaps: count(1) + taps (raw, count × variable)
        // We preserve the entire block raw: start at pos (count byte), walk past taps.
        if pos + SGI_DOWNLOAD_TAPS_COUNT_FIELD > end {
            return Err(Error::BufferTooShort {
                need: pos + SGI_DOWNLOAD_TAPS_COUNT_FIELD,
                have: end,
                what: "ServiceGatewayInfo downloadTaps_count",
            });
        }
        let tap_count = bytes[pos] as usize;
        let dl_taps_start = pos;
        pos += SGI_DOWNLOAD_TAPS_COUNT_FIELD;
        for _ in 0..tap_count {
            let (_, next) = super::ior::Tap::parse_from(bytes, pos, end)?;
            pos = next;
        }
        let download_taps = &bytes[dl_taps_start..pos];

        // serviceContextList (raw)
        let (service_context, next) = parse_service_context_list(bytes, pos, end)?;
        pos = next;

        // userInfoLength (2 bytes, 16-bit) + userInfo_data
        if pos + SGI_USER_INFO_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: pos + SGI_USER_INFO_LEN_FIELD,
                have: end,
                what: "ServiceGatewayInfo userInfoLength",
            });
        }
        let ui_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += SGI_USER_INFO_LEN_FIELD;
        if pos + ui_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: ui_len,
                available: end - pos,
            });
        }
        let user_info = &bytes[pos..pos + ui_len];

        Ok(ServiceGatewayInfo {
            ior,
            download_taps,
            service_context,
            user_info,
        })
    }

    /// Serialize to an owned byte vector.  The result MUST equal the original
    /// `dsi.private_data` bytes byte-for-byte.
    pub fn to_bytes(&self) -> Vec<u8> {
        let len = self.ior.serialized_len()
            + self.download_taps.len()
            + service_context_list_len(&self.service_context)
            + SGI_USER_INFO_LEN_FIELD
            + self.user_info.len();
        let mut buf = vec![0u8; len];
        let mut pos = 0;
        let written = self
            .ior
            .serialize_into(&mut buf[pos..])
            .expect("IOR serialize");
        pos += written;
        buf[pos..pos + self.download_taps.len()].copy_from_slice(self.download_taps);
        pos += self.download_taps.len();
        pos += write_service_context_list(&mut buf[pos..], &self.service_context)
            .expect("serviceContext fits");
        buf[pos..pos + 2].copy_from_slice(&(self.user_info.len() as u16).to_be_bytes());
        pos += SGI_USER_INFO_LEN_FIELD;
        buf[pos..pos + self.user_info.len()].copy_from_slice(self.user_info);
        buf
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::carousel::biop::BINDING_NOBJECT;
    use dvb_common::Parse;

    /// Build a simple FileMessage around a buffer of bytes.
    fn sample_file_message(key: &'static [u8], content: &'static [u8]) -> BiopMessage<'static> {
        BiopMessage::File(FileMessage {
            object_key: key,
            content_size: content.len() as u64,
            object_info_extra: &[],
            service_context: vec![],
            content,
        })
    }

    /// Build a minimal DirectoryMessage.
    fn sample_dir_message() -> BiopMessage<'static> {
        use crate::carousel::biop::ior::{
            BiopProfileBody, ConnBinder, ObjectLocation, TaggedProfile,
        };
        let ior = crate::carousel::biop::ior::Ior {
            type_id: b"fil\0",
            profiles: vec![TaggedProfile::Biop(BiopProfileBody {
                object_location: ObjectLocation {
                    carousel_id: 0xAB,
                    module_id: 2,
                    version_major: 1,
                    version_minor: 0,
                    object_key: &[0x02],
                },
                conn_binder: ConnBinder { taps: vec![] },
                extra: vec![],
            })],
        };
        BiopMessage::Directory(DirectoryMessage {
            object_kind: *b"dir\0",
            object_key: &[0x01],
            object_info: &[],
            service_context: vec![],
            bindings: vec![Binding {
                name: vec![NameComponent {
                    id: b"index.html",
                    kind: b"fil\0",
                }],
                binding_type: BINDING_NOBJECT,
                ior,
                object_info: &[],
            }],
        })
    }

    #[test]
    fn file_message_round_trip() {
        let content: &[u8] = b"Hello, BIOP!";
        let msg = sample_file_message(&[0x01], content);
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        let (parsed, consumed) = BiopMessage::parse_at(&buf).unwrap();
        assert_eq!(consumed, buf.len());
        assert_eq!(parsed, msg);
        // byte-exact re-serialize
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2);
    }

    #[test]
    fn directory_message_round_trip() {
        let msg = sample_dir_message();
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        let (parsed, consumed) = BiopMessage::parse_at(&buf).unwrap();
        assert_eq!(consumed, buf.len());
        assert_eq!(parsed, msg);
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "Directory message byte-exact re-serialize");
    }

    #[test]
    fn module_info_round_trip() {
        use crate::carousel::biop::ior::Tap;
        let info = ModuleInfo {
            module_timeout: 0x00FFFFFF,
            block_timeout: 0x00FFFFFF,
            min_block_time: 0x00000064,
            taps: vec![Tap {
                id: 0,
                use_: 0x0017,
                association_tag: 0x0042,
                selector: &[],
            }],
            user_info: &[],
        };
        let mut buf = vec![0u8; info.serialized_len()];
        info.serialize_into(&mut buf).unwrap();
        let parsed = ModuleInfo::parse(&buf).unwrap();
        assert_eq!(parsed, info);
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "ModuleInfo byte-exact re-serialize");
    }

    #[test]
    fn module_info_byte_anchor() {
        use crate::carousel::biop::ior::Tap;
        // Hand-built ModuleInfo:
        //   moduleTimeout=0x000F4240, blockTimeout=0x000F4240, minBlockTime=0x00000064
        //   taps_count=1: id=0, use=0x0017, assoc=0x47, selector_length=0
        //   UserInfoLength=0
        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x00, 0x0F, 0x42, 0x40, // moduleTimeout
            0x00, 0x0F, 0x42, 0x40, // blockTimeout
            0x00, 0x00, 0x00, 0x64, // minBlockTime
            0x01,                   // taps_count=1
            0x00, 0x00,             // id=0
            0x00, 0x17,             // use=0x0017
            0x00, 0x47,             // assoc=0x47
            0x00,                   // selector_length=0
            0x00,                   // UserInfoLength=0
        ];
        let info = ModuleInfo {
            module_timeout: 0x000F4240,
            block_timeout: 0x000F4240,
            min_block_time: 0x00000064,
            taps: vec![Tap {
                id: 0,
                use_: 0x0017,
                association_tag: 0x0047,
                selector: &[],
            }],
            user_info: &[],
        };
        let mut buf = vec![0u8; info.serialized_len()];
        info.serialize_into(&mut buf).unwrap();
        assert_eq!(buf.as_slice(), expected);
        let parsed = ModuleInfo::parse(expected).unwrap();
        assert_eq!(parsed, info);
    }

    #[test]
    fn sgi_byte_anchor_m6() {
        // The 64-byte SGI private_data from the m6 broadcast capture.
        // Independently parsed in the py script above.
        #[rustfmt::skip]
        let raw: &[u8] = &[
            0x00, 0x00, 0x00, 0x04,  // type_id_length=4
            0x73, 0x72, 0x67, 0x00,  // type_id="srg\0"
            0x00, 0x00, 0x00, 0x01,  // taggedProfiles_count=1
            0x49, 0x53, 0x4F, 0x06,  // TAG_BIOP
            0x00, 0x00, 0x00, 0x28,  // profile_data_length=40
            0x00, 0x02,              // byte_order=0, liteComponents_count=2
            0x49, 0x53, 0x4F, 0x50, 0x0A, // TAG_ObjectLocation, len=10
            0x00, 0x00, 0x00, 0xAB,  // carouselId=0xAB
            0x00, 0x01,              // moduleId=1
            0x01, 0x00,              // version 1.0
            0x01, 0x01,              // objectKey_length=1, objectKey=0x01
            0x49, 0x53, 0x4F, 0x40, 0x12, // TAG_ConnBinder, len=18
            0x01,                    // taps_count=1
            0x00, 0x00,              // tap id=0
            0x00, 0x16,              // use=0x0016
            0x00, 0x47,              // association_tag=0x47
            0x0A,                    // selector_length=10
            0x00, 0x01, 0x80, 0x00, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF,
            0x00,                    // downloadTaps_count=0
            0x00,                    // serviceContextList_count=0
            0x00, 0x00,              // userInfoLength=0
        ];
        assert_eq!(raw.len(), 64);

        let sgi = ServiceGatewayInfo::parse(raw).unwrap();

        // IOR assertions
        assert_eq!(sgi.ior.type_id, b"srg\0");
        assert_eq!(sgi.ior.profiles.len(), 1);
        let bp = sgi.ior.biop_profile().unwrap();
        assert_eq!(bp.object_location.carousel_id, 0xAB);
        assert_eq!(bp.object_location.module_id, 1);
        assert_eq!(bp.object_location.version_major, 1);
        assert_eq!(bp.object_location.version_minor, 0);
        assert_eq!(bp.object_location.object_key, &[0x01]);
        assert_eq!(bp.conn_binder.taps.len(), 1);
        let tap = &bp.conn_binder.taps[0];
        assert_eq!(tap.use_, 0x0016);
        assert_eq!(tap.association_tag, 0x47);
        assert_eq!(tap.transaction_id(), Some(0x80000002));
        assert_eq!(tap.timeout(), Some(0xFFFFFFFF));

        // Byte-exact round-trip
        let out = sgi.to_bytes();
        assert_eq!(out.len(), 64, "SGI serialized length");
        assert_eq!(out.as_slice(), raw, "SGI byte-exact round-trip");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn biop_serde_round_trip() {
        let content: &[u8] = b"test content";
        let msg = sample_file_message(&[0x01], content);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("content_size"));
    }

    #[cfg(feature = "flate2")]
    #[test]
    fn zlib_round_trip() {
        use flate2::{write::ZlibEncoder, Compression};
        use std::io::Write;

        let original = b"Hello, compressed BIOP world! ".repeat(10);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_zlib(&compressed).unwrap();
        assert_eq!(decompressed.as_slice(), original.as_slice());
    }

    // ── StreamMessage tests ───────────────────────────────────────────────────

    #[test]
    fn stream_message_round_trip() {
        use crate::carousel::biop::ior::Tap;
        let msg = BiopMessage::Stream(StreamMessage {
            object_key: &[0x01, 0x02],
            stream_info: DsmStreamInfo {
                description: b"audio stream",
                duration_seconds: -5,
                duration_microseconds: 500,
                audio: 1,
                video: 0,
                data: 0,
            },
            object_info_extra: b"\xDE\xAD",
            service_context: vec![],
            taps: vec![
                Tap {
                    id: 0,
                    use_: 0x0018,
                    association_tag: 0x0010,
                    selector: &[],
                },
                Tap {
                    id: 0,
                    use_: 0x0019,
                    association_tag: 0x0011,
                    selector: &[],
                },
            ],
        });
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        let (parsed, consumed) = BiopMessage::parse_at(&buf).unwrap();
        assert_eq!(consumed, buf.len(), "consumed must equal total buf len");
        assert_eq!(parsed, msg);
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "StreamMessage byte-exact re-serialize");
    }

    #[test]
    fn stream_event_message_round_trip() {
        use crate::carousel::biop::ior::Tap;
        let msg = BiopMessage::StreamEvent(StreamEventMessage {
            object_key: &[0x03],
            stream_info: DsmStreamInfo {
                description: b"event stream",
                duration_seconds: 3600,
                duration_microseconds: 0,
                audio: 0,
                video: 1,
                data: 0,
            },
            event_names: vec![b"play".as_ref(), b"pause".as_ref(), b"stop".as_ref()],
            object_info_extra: &[],
            service_context: vec![],
            taps: vec![Tap {
                id: 0,
                use_: 0x000C,
                association_tag: 0x0020,
                selector: &[],
            }],
            event_ids: vec![0x0001, 0x0002, 0x0003],
        });
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();
        let (parsed, consumed) = BiopMessage::parse_at(&buf).unwrap();
        assert_eq!(consumed, buf.len(), "consumed must equal total buf len");
        assert_eq!(parsed, msg);
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "StreamEventMessage byte-exact re-serialize");
    }

    #[test]
    fn stream_message_byte_anchor() {
        // Hand-built minimal StreamMessage from Table 4.11:
        //
        // Offset table (from byte 0):
        //  [0..4]   magic = 0x42494F50
        //  [4]      biop_version.major = 0x01
        //  [5]      biop_version.minor = 0x00
        //  [6]      byte_order = 0x00
        //  [7]      message_type = 0x00
        //  [8..12]  message_size = 38
        //  [12]     objectKey_length = 1
        //  [13]     objectKey_data = 0xAB
        //  [14..18] objectKind_length = 4
        //  [18..22] objectKind_data = "str\0"
        //  [22..24] objectInfo_length = 13 (= N6; Info_T = aDesc_len(1)+desc(3)+fixed(9) = 13)
        //    Info_T:
        //    [24]     aDescription_length = 3 (= N2)
        //    [25..28] aDescription_bytes = "vid"
        //    [28..32] duration.aSeconds = 0 (i32 big-endian)
        //    [32..34] duration.aMicroSeconds = 0
        //    [34]     audio = 1
        //    [35]     video = 1
        //    [36]     data = 0
        //    (no trailing objectInfo; N6 - (N2+10) = 13 - 13 = 0)
        //  [37]     serviceContextList_count = 0
        //  [38..42] messageBody_length = 8
        //    [42]     taps_count = 1
        //    [43..45] tap.id = 0
        //    [45..47] tap.use = 0x0018 (BIOP_ES_USE)
        //    [47..49] tap.association_tag = 0x0047
        //    [49]     tap.selector_length = 0
        // Total = 50 bytes
        // message_size = 50 - 12 = 38; objectInfo_length = 1+3+9 = 13
        use crate::carousel::biop::ior::Tap;
        #[rustfmt::skip]
        let expected: &[u8] = &[
            // BIOP header (12 bytes)
            0x42, 0x49, 0x4F, 0x50, // magic "BIOP"
            0x01,                   // major
            0x00,                   // minor
            0x00,                   // byte_order
            0x00,                   // message_type
            0x00, 0x00, 0x00, 0x26, // message_size = 38
            // objectKey (2 bytes)
            0x01,                   // objectKey_length = 1
            0xAB,                   // objectKey_data
            // objectKind (8 bytes)
            0x00, 0x00, 0x00, 0x04, // objectKind_length = 4
            0x73, 0x74, 0x72, 0x00, // "str\0"
            // objectInfo_length (2 bytes) + Info_T (13 bytes)
            0x00, 0x0D,             // objectInfo_length = 13
            0x03,                   // aDescription_length = 3 (= N2)
            0x76, 0x69, 0x64,       // "vid"
            0x00, 0x00, 0x00, 0x00, // duration.aSeconds = 0
            0x00, 0x00,             // duration.aMicroSeconds = 0
            0x01,                   // audio = 1
            0x01,                   // video = 1
            0x00,                   // data = 0
            // serviceContextList_count (1 byte)
            0x00,
            // messageBody_length (4 bytes)
            0x00, 0x00, 0x00, 0x08, // body_len = 8
            // body: taps_count(1) + 1 tap(7)
            0x01,                   // taps_count = 1
            0x00, 0x00,             // tap.id = 0
            0x00, 0x18,             // tap.use = 0x0018
            0x00, 0x47,             // tap.association_tag = 0x47
            0x00,                   // tap.selector_length = 0
        ];
        assert_eq!(expected.len(), 50);
        let expected_msg = BiopMessage::Stream(StreamMessage {
            object_key: &[0xAB],
            stream_info: DsmStreamInfo {
                description: b"vid",
                duration_seconds: 0,
                duration_microseconds: 0,
                audio: 1,
                video: 1,
                data: 0,
            },
            object_info_extra: &[],
            service_context: vec![],
            taps: vec![Tap {
                id: 0,
                use_: 0x0018,
                association_tag: 0x0047,
                selector: &[],
            }],
        });

        // serialize → expected bytes
        let mut buf = vec![0u8; expected_msg.serialized_len()];
        expected_msg.serialize_into(&mut buf).unwrap();
        assert_eq!(
            buf.as_slice(),
            expected,
            "StreamMessage serialize must match byte anchor"
        );

        // parse expected bytes → expected struct
        let (parsed, consumed) = BiopMessage::parse_at(expected).unwrap();
        assert_eq!(consumed, expected.len());
        assert_eq!(
            parsed, expected_msg,
            "StreamMessage parse must match byte anchor struct"
        );
    }

    #[test]
    fn stream_event_message_byte_anchor() {
        // Hand-built minimal StreamEventMessage from Table 4.13:
        //
        // Offset table (from byte 0):
        //  [0..4]   magic = 0x42494F50
        //  [4]      major = 0x01
        //  [5]      minor = 0x00
        //  [6]      byte_order = 0x00
        //  [7]      message_type = 0x00
        //  [8..12]  message_size = 43
        //  [12]     objectKey_length = 1
        //  [13]     objectKey_data = 0xCD
        //  [14..18] objectKind_length = 4
        //  [18..22] objectKind_data = "ste\0"
        //  [22..24] objectInfo_length = 20 (= N6)
        //    Info_T (10 bytes, N2=0):
        //    [24]     aDescription_length = 0
        //    (no description bytes)
        //    [25..29] duration.aSeconds = 0
        //    [29..31] duration.aMicroSeconds = 0
        //    [31]     audio = 0
        //    [32]     video = 0
        //    [33]     data = 0
        //    EventList_T (10 bytes):
        //    [34..36] eventNames_count = 2
        //    [36]     name0_length = 3
        //    [37..40] "foo"
        //    [40]     name1_length = 3
        //    [41..44] "bar"
        //    (no trailing objectInfo extra; 20 - 10 - 10 = 0)
        //  [44]     serviceContextList_count = 0
        //  [45..49] messageBody_length = 6
        //    [49]     taps_count = 0
        //    [50]     eventIds_count = 2
        //    [51..53] eventId[0] = 0x0001
        //    [53..55] eventId[1] = 0x0002
        // Total = 55 bytes
        #[rustfmt::skip]
        let expected: &[u8] = &[
            // BIOP header (12 bytes)
            0x42, 0x49, 0x4F, 0x50, // magic "BIOP"
            0x01,                   // major
            0x00,                   // minor
            0x00,                   // byte_order
            0x00,                   // message_type
            0x00, 0x00, 0x00, 0x2B, // message_size = 43
            // objectKey (2 bytes)
            0x01,                   // objectKey_length = 1
            0xCD,                   // objectKey_data
            // objectKind (8 bytes)
            0x00, 0x00, 0x00, 0x04, // objectKind_length = 4
            0x73, 0x74, 0x65, 0x00, // "ste\0"
            // objectInfo_length (2 bytes) + objectInfo (20 bytes)
            0x00, 0x14,             // objectInfo_length = 20
            // Info_T (10 bytes, N2=0)
            0x00,                   // aDescription_length = 0
            0x00, 0x00, 0x00, 0x00, // duration.aSeconds = 0
            0x00, 0x00,             // duration.aMicroSeconds = 0
            0x00,                   // audio = 0
            0x00,                   // video = 0
            0x00,                   // data = 0
            // EventList_T (10 bytes): eventNames_count(2) + 2×(len(1)+name(3))
            0x00, 0x02,             // eventNames_count = 2
            0x03,                   // name0_length = 3
            0x66, 0x6F, 0x6F,       // "foo"
            0x03,                   // name1_length = 3
            0x62, 0x61, 0x72,       // "bar"
            // serviceContextList_count (1 byte)
            0x00,
            // messageBody_length (4 bytes)
            0x00, 0x00, 0x00, 0x06, // body_len = 6
            // body: taps_count(1)=0 + eventIds_count(1)=2 + eventId×2 (4)
            0x00,                   // taps_count = 0
            0x02,                   // eventIds_count = 2
            0x00, 0x01,             // eventId[0] = 1
            0x00, 0x02,             // eventId[1] = 2
        ];
        assert_eq!(expected.len(), 55);

        let expected_msg = BiopMessage::StreamEvent(StreamEventMessage {
            object_key: &[0xCD],
            stream_info: DsmStreamInfo {
                description: &[],
                duration_seconds: 0,
                duration_microseconds: 0,
                audio: 0,
                video: 0,
                data: 0,
            },
            event_names: vec![b"foo".as_ref(), b"bar".as_ref()],
            object_info_extra: &[],
            service_context: vec![],
            taps: vec![],
            event_ids: vec![1, 2],
        });

        // serialize → expected bytes
        let mut buf = vec![0u8; expected_msg.serialized_len()];
        expected_msg.serialize_into(&mut buf).unwrap();
        assert_eq!(
            buf.as_slice(),
            expected,
            "StreamEventMessage serialize must match byte anchor"
        );

        // parse expected bytes → expected struct
        let (parsed, consumed) = BiopMessage::parse_at(expected).unwrap();
        assert_eq!(consumed, expected.len());
        assert_eq!(
            parsed, expected_msg,
            "StreamEventMessage parse must match byte anchor struct"
        );
    }

    #[test]
    fn service_context_typed_round_trip() {
        // FileMessage with two non-trivial serviceContext entries.
        let msg = BiopMessage::File(FileMessage {
            object_key: &[0x01],
            content_size: 3,
            object_info_extra: &[],
            service_context: vec![
                ServiceContext {
                    context_id: 0xDEADBEEF,
                    data: &[1, 2, 3],
                },
                ServiceContext {
                    context_id: 0x11223344,
                    data: &[],
                },
            ],
            content: b"abc",
        });

        // serialize
        let mut buf = vec![0u8; msg.serialized_len()];
        msg.serialize_into(&mut buf).unwrap();

        // parse back
        let (parsed, consumed) = BiopMessage::parse_at(&buf).unwrap();
        assert_eq!(consumed, buf.len(), "consumed must equal total buf len");
        assert_eq!(parsed, msg, "parsed must equal original");

        // byte-exact re-serialize
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(
            buf, buf2,
            "serviceContext typed round-trip must be byte-exact"
        );

        // spot-check the wire: count byte should be 2
        // serviceContextList starts after the BIOP header + key + kind + objectInfo
        // (12 + 2 + 8 + 2 + 8 = 32 bytes in)
        assert_eq!(buf[32], 2, "serviceContextList_count must be 2");
        // first entry: context_id = 0xDEADBEEF
        assert_eq!(&buf[33..37], &[0xDE, 0xAD, 0xBE, 0xEF]);
        // first entry: context_data_length = 3
        assert_eq!(&buf[37..39], &[0x00, 0x03]);
        // first entry: context_data = [1, 2, 3]
        assert_eq!(&buf[39..42], &[0x01, 0x02, 0x03]);
        // second entry: context_id = 0x11223344
        assert_eq!(&buf[42..46], &[0x11, 0x22, 0x33, 0x44]);
        // second entry: context_data_length = 0
        assert_eq!(&buf[46..48], &[0x00, 0x00]);
    }
}
