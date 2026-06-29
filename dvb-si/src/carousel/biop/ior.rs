//! IOP::IOR and its component types for the DVB object-carousel profile.
//!
//! All wire layouts are from `docs/iso_13818_6_biop.md` (ETSI TR 101 202 §4.7.3).
//!
//! # Layout overview
//!
//! `IOP::IOR` (Table 4.3) → one or more `TaggedProfile` variants:
//! - [`TaggedProfile::Biop`] — BIOP Profile Body (Table 4.5): contains an
//!   [`ObjectLocation`] and a [`ConnBinder`] (each a [`LiteComponent`]).
//! - [`TaggedProfile::LiteOptions`] — Lite Options Profile Body (Table 4.7):
//!   contains a [`ServiceLocation`] (with an [`NsapAddress`]).
//! - [`TaggedProfile::Unknown`] — any other tag; data preserved raw.

use super::{
    BIOP_DELIVERY_PARA_USE, BYTE_ORDER_BIG_ENDIAN, TAG_BIOP, TAG_CONN_BINDER, TAG_LITE_OPTIONS,
    TAG_OBJECT_LOCATION, TAG_SERVICE_LOCATION,
};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

// ── wire-layout byte counts ────────────────────────────────────────────────────

/// IOR: type_id_length (4) + taggedProfiles_count (4).
const IOR_FIXED_LEN: usize = 8;
/// Per-profile: profileId_tag (4) + profile_data_length (4).
const PROFILE_HEADER_LEN: usize = 8;
/// BIOP Profile Body: byte_order (1) + liteComponents_count (1).
const BIOP_BODY_FIXED_LEN: usize = 2;
/// Per liteComponent: componentId_tag (4) + component_data_length (1).
const COMPONENT_HEADER_LEN: usize = 5;
/// ObjectLocation fixed: carousel_id(4)+module_id(2)+version_major(1)+version_minor(1)+key_len(1) = 9.
const OBJECT_LOCATION_FIXED_LEN: usize = 9;
/// ConnBinder fixed: taps_count(1).
const CONN_BINDER_FIXED_LEN: usize = 1;
/// Tap fixed: id(2)+use(2)+association_tag(2)+selector_length(1) = 7.
const TAP_FIXED_LEN: usize = 7;
/// LiteOptions Profile Body: byte_order(1) + component_count(1).
const LITE_OPTIONS_BODY_FIXED_LEN: usize = 2;
/// ServiceLocation component header: componentId_tag(4)+component_data_length(4).
const SERVICE_LOCATION_COMP_HEADER_LEN: usize = 8;
/// serviceDomain_length(1) = 1 prefix before the 20-byte NSAP address.
const SERVICE_DOMAIN_LEN_FIELD: usize = 1;
/// DVB Carousel NSAP address: always 20 bytes.  See Table 4.8.
const NSAP_ADDRESS_LEN: usize = 20;
/// NSAP: AFI(1)+Type(1)+carouselId(4)+specifierType(1)+specifierData(3)+tsid(2)+onid(2)+sid(2)+reserved(4) = 20.
const _NSAP_FIELDS_LEN: usize = NSAP_ADDRESS_LEN; // same constant, alias for clarity
/// CosNaming nameComponents_count in ServiceLocation: 32-bit.
const NAMING_COUNT_LEN: usize = 4;
/// id_length / kind_length in ServiceLocation CosNaming: 32-bit each.
const NAMING_FIELD_LEN: usize = 4;
/// initialContext_length in ServiceLocation: 32-bit.
const INITIAL_CONTEXT_LEN_FIELD: usize = 4;

// ── ObjectKind ────────────────────────────────────────────────────────────────

/// DVB alias `type_id` values (4 bytes) that appear in IOR `type_id` and
/// BIOP object `objectKind` fields.  TR 101 202 §4.7.3.1, Table 4.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ObjectKind {
    /// `"dir\0"` (0x64697200) — DSM::Directory.
    Directory,
    /// `"fil\0"` (0x66696C00) — DSM::File.
    File,
    /// `"str\0"` (0x73747200) — DSM::Stream.
    Stream,
    /// `"srg\0"` (0x73726700) — DSM::ServiceGateway.
    ServiceGateway,
    /// `"ste\0"` (0x73746500) — BIOP::StreamEvent.
    StreamEvent,
    /// Unknown 4-byte kind.
    Unknown([u8; 4]),
}

impl ObjectKind {
    /// Parse 4 bytes into an `ObjectKind`.
    pub fn from_bytes(b: [u8; 4]) -> Self {
        match &b {
            b"dir\0" => Self::Directory,
            b"fil\0" => Self::File,
            b"str\0" => Self::Stream,
            b"srg\0" => Self::ServiceGateway,
            b"ste\0" => Self::StreamEvent,
            _ => Self::Unknown(b),
        }
    }

    /// Serialize this kind to its 4-byte wire representation.
    pub fn to_bytes(&self) -> [u8; 4] {
        match self {
            Self::Directory => *b"dir\0",
            Self::File => *b"fil\0",
            Self::Stream => *b"str\0",
            Self::ServiceGateway => *b"srg\0",
            Self::StreamEvent => *b"ste\0",
            Self::Unknown(b) => *b,
        }
    }
}

// ── Tap ───────────────────────────────────────────────────────────────────────

/// BIOP::Tap — one delivery-parameter or object-use tap.
/// TR 101 202 §4.7.3.2, Table 4.5 (ConnBinder Tap list).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Tap<'a> {
    /// `id` field — user private; typically 0.
    pub id: u16,
    /// `use` field — e.g. [`BIOP_DELIVERY_PARA_USE`].
    pub use_: u16,
    /// `association_tag` — ES on which this tap is broadcast.
    pub association_tag: u16,
    /// Raw selector bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub selector: &'a [u8],
}

impl<'a> Tap<'a> {
    /// Returns the `transactionId` decoded from the selector when this is a
    /// `BIOP_DELIVERY_PARA_USE` tap with a 10-byte MESSAGE selector.
    ///
    /// Selector layout (when `use == 0x0016` and `selector.len() >= 10`):
    /// selector_type(2) | transactionId(4) | timeout(4).
    pub fn transaction_id(&self) -> Option<u32> {
        if self.use_ == BIOP_DELIVERY_PARA_USE && self.selector.len() >= 10 {
            let (chunk, _) = self.selector[2..].split_first_chunk::<4>()?;
            Some(u32::from_be_bytes(*chunk))
        } else {
            None
        }
    }

    /// Returns the `timeout` (µs) decoded from the selector when this is a
    /// `BIOP_DELIVERY_PARA_USE` tap with a 10-byte MESSAGE selector.
    pub fn timeout(&self) -> Option<u32> {
        if self.use_ == BIOP_DELIVERY_PARA_USE && self.selector.len() >= 10 {
            let (chunk, _) = self.selector[6..].split_first_chunk::<4>()?;
            Some(u32::from_be_bytes(*chunk))
        } else {
            None
        }
    }

    pub(crate) fn serialized_len(&self) -> usize {
        TAP_FIXED_LEN + self.selector.len()
    }

    pub(crate) fn parse_from(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        let (tap_hdr, _) =
            bytes[pos..end]
                .split_first_chunk::<TAP_FIXED_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: pos + TAP_FIXED_LEN,
                    have: end,
                    what: "BIOP Tap fixed fields",
                })?;
        let id = u16::from_be_bytes([tap_hdr[0], tap_hdr[1]]);
        let use_ = u16::from_be_bytes([tap_hdr[2], tap_hdr[3]]);
        let association_tag = u16::from_be_bytes([tap_hdr[4], tap_hdr[5]]);
        let selector_length = tap_hdr[6] as usize;
        let data_start = pos + TAP_FIXED_LEN;
        if data_start + selector_length > end {
            return Err(Error::SectionLengthOverflow {
                declared: selector_length,
                available: end - data_start,
            });
        }
        let selector = &bytes[data_start..data_start + selector_length];
        Ok((
            Tap {
                id,
                use_,
                association_tag,
                selector,
            },
            data_start + selector_length,
        ))
    }

    pub(crate) fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.selector.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.selector.len(),
                available: u8::MAX as usize,
            });
        }
        buf[0..2].copy_from_slice(&self.id.to_be_bytes());
        buf[2..4].copy_from_slice(&self.use_.to_be_bytes());
        buf[4..6].copy_from_slice(&self.association_tag.to_be_bytes());
        buf[6] = self.selector.len() as u8;
        buf[7..7 + self.selector.len()].copy_from_slice(self.selector);
        Ok(len)
    }
}

// ── ObjectLocation ────────────────────────────────────────────────────────────

/// BIOP::ObjectLocation — first mandatory liteComponent in the BIOP Profile Body.
/// TR 101 202 §4.7.3.2, Table 4.5.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ObjectLocation<'a> {
    /// `carouselId` — identifies the carousel delivering this object.
    pub carousel_id: u32,
    /// `moduleId` — module within the carousel.
    pub module_id: u16,
    /// `version.major` — always 1 for DVB.
    pub version_major: u8,
    /// `version.minor` — always 0 for DVB.
    pub version_minor: u8,
    /// `objectKey_data` — key for this object within the module.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_key: &'a [u8],
}

impl<'a> ObjectLocation<'a> {
    fn serialized_len(&self) -> usize {
        OBJECT_LOCATION_FIXED_LEN + self.object_key.len()
    }

    fn parse_from(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        let (ol_hdr, _) = bytes[pos..end]
            .split_first_chunk::<OBJECT_LOCATION_FIXED_LEN>()
            .ok_or(Error::BufferTooShort {
                need: pos + OBJECT_LOCATION_FIXED_LEN,
                have: end,
                what: "BIOP ObjectLocation fixed fields",
            })?;
        let carousel_id = u32::from_be_bytes([ol_hdr[0], ol_hdr[1], ol_hdr[2], ol_hdr[3]]);
        let module_id = u16::from_be_bytes([ol_hdr[4], ol_hdr[5]]);
        let version_major = ol_hdr[6];
        let version_minor = ol_hdr[7];
        let object_key_length = ol_hdr[8] as usize;
        let data_start = pos + OBJECT_LOCATION_FIXED_LEN;
        if data_start + object_key_length > end {
            return Err(Error::SectionLengthOverflow {
                declared: object_key_length,
                available: end - data_start,
            });
        }
        Ok((
            ObjectLocation {
                carousel_id,
                module_id,
                version_major,
                version_minor,
                object_key: &bytes[data_start..data_start + object_key_length],
            },
            data_start + object_key_length,
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
        if self.object_key.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.object_key.len(),
                available: u8::MAX as usize,
            });
        }
        buf[0..4].copy_from_slice(&self.carousel_id.to_be_bytes());
        buf[4..6].copy_from_slice(&self.module_id.to_be_bytes());
        buf[6] = self.version_major;
        buf[7] = self.version_minor;
        buf[8] = self.object_key.len() as u8;
        buf[9..9 + self.object_key.len()].copy_from_slice(self.object_key);
        Ok(len)
    }
}

// ── ConnBinder ────────────────────────────────────────────────────────────────

/// DSM::ConnBinder — second mandatory liteComponent in the BIOP Profile Body.
/// TR 101 202 §4.7.3.2, Table 4.5.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConnBinder<'a> {
    /// Taps list.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub taps: Vec<Tap<'a>>,
}

impl<'a> ConnBinder<'a> {
    fn serialized_len(&self) -> usize {
        CONN_BINDER_FIXED_LEN + self.taps.iter().map(|t| t.serialized_len()).sum::<usize>()
    }

    fn parse_from(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        if pos + CONN_BINDER_FIXED_LEN > end {
            return Err(Error::BufferTooShort {
                need: pos + CONN_BINDER_FIXED_LEN,
                have: end,
                what: "BIOP ConnBinder taps_count",
            });
        }
        let taps_count = bytes[pos] as usize;
        let mut cur = pos + CONN_BINDER_FIXED_LEN;
        let mut taps = Vec::with_capacity(taps_count.min(16));
        for _ in 0..taps_count {
            let (tap, next) = Tap::parse_from(bytes, cur, end)?;
            taps.push(tap);
            cur = next;
        }
        Ok((ConnBinder { taps }, cur))
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.taps.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.taps.len(),
                available: u8::MAX as usize,
            });
        }
        buf[0] = self.taps.len() as u8;
        let mut pos = CONN_BINDER_FIXED_LEN;
        for tap in &self.taps {
            let written = tap.serialize_into_buf(&mut buf[pos..])?;
            pos += written;
        }
        Ok(len)
    }
}

// ── LiteComponent ─────────────────────────────────────────────────────────────

/// An unknown extra liteComponent in a profile body — tag + raw data.
/// TR 101 202 §4.7.3.2, Table 4.5 (extra components beyond ObjectLocation + ConnBinder).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LiteComponent<'a> {
    /// `componentId_tag` (32-bit).
    pub tag: u32,
    /// Raw component data bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data: &'a [u8],
}

impl<'a> LiteComponent<'a> {
    fn serialized_len(&self) -> usize {
        COMPONENT_HEADER_LEN + self.data.len()
    }
}

// ── BiopProfileBody ───────────────────────────────────────────────────────────

/// BIOP Profile Body — decoded contents of a `TAG_BIOP` profile.
/// TR 101 202 §4.7.3.2, Table 4.5.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BiopProfileBody<'a> {
    /// First mandatory component: BIOP::ObjectLocation.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub object_location: ObjectLocation<'a>,
    /// Second mandatory component: DSM::ConnBinder.
    pub conn_binder: ConnBinder<'a>,
    /// Extra liteComponents beyond the mandatory two (N−2).
    pub extra: Vec<LiteComponent<'a>>,
}

impl<'a> BiopProfileBody<'a> {
    /// Parse from profile_data bytes (after the 4+4 profileId_tag+length fields).
    fn parse_from(bytes: &'a [u8]) -> Result<Self> {
        let end = bytes.len();
        if end < BIOP_BODY_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: BIOP_BODY_FIXED_LEN,
                have: end,
                what: "BIOP Profile Body fixed fields",
            });
        }
        let byte_order = bytes[0];
        if byte_order != BYTE_ORDER_BIG_ENDIAN {
            return Err(Error::ReservedBitsViolation {
                field: "profile_data_byte_order",
                reason:
                    "must be 0x00 (big-endian) per DVB mandatory constraint (TR 101 202 §4.7.3.2)",
            });
        }
        let lite_components_count = bytes[1] as usize;
        if lite_components_count < 2 {
            return Err(Error::ValueOutOfRange {
                field: "liteComponents_count",
                reason: "BIOP Profile Body must have at least 2 components (ObjectLocation + ConnBinder)",
            });
        }
        let mut pos = BIOP_BODY_FIXED_LEN;

        // First component: ObjectLocation
        let (ch0, _) = bytes[pos..end]
            .split_first_chunk::<COMPONENT_HEADER_LEN>()
            .ok_or(Error::BufferTooShort {
                need: pos + COMPONENT_HEADER_LEN,
                have: end,
                what: "BIOP ObjectLocation component header",
            })?;
        let comp0_tag = u32::from_be_bytes([ch0[0], ch0[1], ch0[2], ch0[3]]);
        if comp0_tag != TAG_OBJECT_LOCATION {
            return Err(Error::ReservedBitsViolation {
                field: "componentId_tag[0]",
                reason: "first liteComponent must be TAG_ObjectLocation (0x49534F50)",
            });
        }
        let comp0_len = ch0[4] as usize;
        pos += COMPONENT_HEADER_LEN;
        let comp0_end = pos + comp0_len;
        if comp0_end > end {
            return Err(Error::SectionLengthOverflow {
                declared: comp0_len,
                available: end - pos,
            });
        }
        let (object_location, _) = ObjectLocation::parse_from(bytes, pos, comp0_end)?;
        pos = comp0_end;

        // Second component: ConnBinder
        let (ch1, _) = bytes[pos..end]
            .split_first_chunk::<COMPONENT_HEADER_LEN>()
            .ok_or(Error::BufferTooShort {
                need: pos + COMPONENT_HEADER_LEN,
                have: end,
                what: "BIOP ConnBinder component header",
            })?;
        let comp1_tag = u32::from_be_bytes([ch1[0], ch1[1], ch1[2], ch1[3]]);
        if comp1_tag != TAG_CONN_BINDER {
            return Err(Error::ReservedBitsViolation {
                field: "componentId_tag[1]",
                reason: "second liteComponent must be TAG_ConnBinder (0x49534F40)",
            });
        }
        let comp1_len = ch1[4] as usize;
        pos += COMPONENT_HEADER_LEN;
        let comp1_end = pos + comp1_len;
        if comp1_end > end {
            return Err(Error::SectionLengthOverflow {
                declared: comp1_len,
                available: end - pos,
            });
        }
        let (conn_binder, _) = ConnBinder::parse_from(bytes, pos, comp1_end)?;
        pos = comp1_end;

        // Remaining extra components
        let extra_count = lite_components_count - 2;
        let mut extra = Vec::with_capacity(extra_count.min(8));
        for _ in 0..extra_count {
            let (ch_ex, _) = bytes[pos..end]
                .split_first_chunk::<COMPONENT_HEADER_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: pos + COMPONENT_HEADER_LEN,
                    have: end,
                    what: "BIOP extra liteComponent header",
                })?;
            let tag = u32::from_be_bytes([ch_ex[0], ch_ex[1], ch_ex[2], ch_ex[3]]);
            let data_len = ch_ex[4] as usize;
            pos += COMPONENT_HEADER_LEN;
            if pos + data_len > end {
                return Err(Error::SectionLengthOverflow {
                    declared: data_len,
                    available: end - pos,
                });
            }
            extra.push(LiteComponent {
                tag,
                data: &bytes[pos..pos + data_len],
            });
            pos += data_len;
        }

        Ok(BiopProfileBody {
            object_location,
            conn_binder,
            extra,
        })
    }

    fn serialized_len(&self) -> usize {
        let ol_len = self.object_location.serialized_len();
        let cb_len = self.conn_binder.serialized_len();
        let extra_len: usize = self.extra.iter().map(|c| c.serialized_len()).sum();
        BIOP_BODY_FIXED_LEN
            + COMPONENT_HEADER_LEN
            + ol_len
            + COMPONENT_HEADER_LEN
            + cb_len
            + extra_len
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let total_components = 2 + self.extra.len();
        if total_components > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: total_components,
                available: u8::MAX as usize,
            });
        }
        buf[0] = BYTE_ORDER_BIG_ENDIAN;
        buf[1] = total_components as u8;
        let mut pos = BIOP_BODY_FIXED_LEN;

        // ObjectLocation component
        let ol_data_len = self.object_location.serialized_len();
        if ol_data_len > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: ol_data_len,
                available: u8::MAX as usize,
            });
        }
        buf[pos..pos + 4].copy_from_slice(&TAG_OBJECT_LOCATION.to_be_bytes());
        buf[pos + 4] = ol_data_len as u8;
        pos += COMPONENT_HEADER_LEN;
        let written = self.object_location.serialize_into_buf(&mut buf[pos..])?;
        pos += written;

        // ConnBinder component
        let cb_data_len = self.conn_binder.serialized_len();
        if cb_data_len > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: cb_data_len,
                available: u8::MAX as usize,
            });
        }
        buf[pos..pos + 4].copy_from_slice(&TAG_CONN_BINDER.to_be_bytes());
        buf[pos + 4] = cb_data_len as u8;
        pos += COMPONENT_HEADER_LEN;
        let written = self.conn_binder.serialize_into_buf(&mut buf[pos..])?;
        pos += written;

        // Extra components
        for comp in &self.extra {
            let data_len = comp.data.len();
            if data_len > u8::MAX as usize {
                return Err(Error::SectionLengthOverflow {
                    declared: data_len,
                    available: u8::MAX as usize,
                });
            }
            buf[pos..pos + 4].copy_from_slice(&comp.tag.to_be_bytes());
            buf[pos + 4] = data_len as u8;
            pos += COMPONENT_HEADER_LEN;
            buf[pos..pos + data_len].copy_from_slice(comp.data);
            pos += data_len;
        }

        Ok(len)
    }
}

// ── NsapAddress ───────────────────────────────────────────────────────────────

/// DVB Carousel NSAP Address — 20 bytes.  TR 101 202 §4.7.3.4, Table 4.8.
///
/// Fixed layout: AFI(1)=0x00, Type(1)=0x00, carouselId(4), specifierType(1)=0x01,
/// specifierData/IEEE OUI(3), tsid(2), onid(2), sid(2), reserved(4)=0xFFFFFFFF.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NsapAddress {
    /// `carouselId` field.
    pub carousel_id: u32,
    /// `specifierData` — 3-byte IEEE OUI (e.g. DVB OUI).
    pub specifier_data: [u8; 3],
    /// `transport_stream_id`.
    pub transport_stream_id: u16,
    /// `original_network_id`.
    pub original_network_id: u16,
    /// `service_id` — equals MPEG-2 `program_number`.
    pub service_id: u16,
}

impl NsapAddress {
    /// Expected AFI value: 0x00 (NSAP for private use).
    const AFI: u8 = 0x00;
    /// Expected Type value: 0x00 (Object carousel NSAP address).
    const NSAP_TYPE: u8 = 0x00;
    /// Expected specifierType value: 0x01 (IEEE OUI).
    const SPECIFIER_TYPE: u8 = 0x01;
    /// Reserved field value: 0xFFFFFFFF.
    const RESERVED: u32 = 0xFFFF_FFFF;

    fn parse_from(bytes: &[u8], pos: usize) -> Result<Self> {
        let end = pos + NSAP_ADDRESS_LEN;
        let (nsap, _) = bytes
            .get(pos..)
            .and_then(|s| s.split_first_chunk::<NSAP_ADDRESS_LEN>())
            .ok_or(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "NSAP address (20 bytes)",
            })?;
        // AFI and Type are fixed per DVB profile; do not reject if they differ
        // (be tolerant, but documented).
        let carousel_id = u32::from_be_bytes([nsap[2], nsap[3], nsap[4], nsap[5]]);
        // specifierType at nsap[6]
        let specifier_data = [nsap[7], nsap[8], nsap[9]];
        let transport_stream_id = u16::from_be_bytes([nsap[10], nsap[11]]);
        let original_network_id = u16::from_be_bytes([nsap[12], nsap[13]]);
        let service_id = u16::from_be_bytes([nsap[14], nsap[15]]);
        // reserved nsap[16..20]
        Ok(NsapAddress {
            carousel_id,
            specifier_data,
            transport_stream_id,
            original_network_id,
            service_id,
        })
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < NSAP_ADDRESS_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: NSAP_ADDRESS_LEN,
                have: buf.len(),
            });
        }
        buf[0] = Self::AFI;
        buf[1] = Self::NSAP_TYPE;
        buf[2..6].copy_from_slice(&self.carousel_id.to_be_bytes());
        buf[6] = Self::SPECIFIER_TYPE;
        buf[7..10].copy_from_slice(&self.specifier_data);
        buf[10..12].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[12..14].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[14..16].copy_from_slice(&self.service_id.to_be_bytes());
        buf[16..20].copy_from_slice(&Self::RESERVED.to_be_bytes());
        Ok(NSAP_ADDRESS_LEN)
    }
}

// ── NameComponent ─────────────────────────────────────────────────────────────

/// CosNaming name component — used in `ServiceLocation` path.
/// TR 101 202 §4.7.3.3, Table 4.7.  id and kind lengths are 32-bit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NameComponent<'a> {
    /// Component id bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub id: &'a [u8],
    /// Component kind bytes — typically a 4-byte alias type_id.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub kind: &'a [u8],
}

impl<'a> NameComponent<'a> {
    /// Wire size for a CosNaming component (32-bit lengths).
    pub(crate) fn serialized_len_32bit(&self) -> usize {
        NAMING_FIELD_LEN + self.id.len() + NAMING_FIELD_LEN + self.kind.len()
    }

    /// Parse one CosNaming name component with 32-bit length prefixes.
    pub(crate) fn parse_32bit(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        let (bid, _) = bytes[pos..end]
            .split_first_chunk::<4>()
            .ok_or(Error::BufferTooShort {
                need: pos + NAMING_FIELD_LEN,
                have: end,
                what: "CosNaming id_length",
            })?;
        let id_len = u32::from_be_bytes(*bid) as usize;
        let id_start = pos + NAMING_FIELD_LEN;
        if id_start + id_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: id_len,
                available: end - id_start,
            });
        }
        let id = &bytes[id_start..id_start + id_len];
        let kind_pos = id_start + id_len;
        let (bkind, _) =
            bytes[kind_pos..end]
                .split_first_chunk::<4>()
                .ok_or(Error::BufferTooShort {
                    need: kind_pos + NAMING_FIELD_LEN,
                    have: end,
                    what: "CosNaming kind_length",
                })?;
        let kind_len = u32::from_be_bytes(*bkind) as usize;
        let kind_start = kind_pos + NAMING_FIELD_LEN;
        if kind_start + kind_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: kind_len,
                available: end - kind_start,
            });
        }
        let kind = &bytes[kind_start..kind_start + kind_len];
        Ok((NameComponent { id, kind }, kind_start + kind_len))
    }

    /// Serialize one CosNaming name component with 32-bit length prefixes.
    pub(crate) fn serialize_32bit(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len_32bit();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&(self.id.len() as u32).to_be_bytes());
        buf[4..4 + self.id.len()].copy_from_slice(self.id);
        let kind_pos = 4 + self.id.len();
        buf[kind_pos..kind_pos + 4].copy_from_slice(&(self.kind.len() as u32).to_be_bytes());
        buf[kind_pos + 4..kind_pos + 4 + self.kind.len()].copy_from_slice(self.kind);
        Ok(len)
    }

    /// Wire size for a DVB Directory BIOP name component (8-bit lengths).
    pub(crate) fn serialized_len_8bit(&self) -> usize {
        1 + self.id.len() + 1 + self.kind.len()
    }

    /// Parse one BIOP Directory name component with 8-bit length prefixes.
    pub(crate) fn parse_8bit(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        if pos + 1 > end {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: end,
                what: "BIOP NameComponent id_length (8-bit)",
            });
        }
        let id_len = bytes[pos] as usize;
        let id_start = pos + 1;
        if id_start + id_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: id_len,
                available: end - id_start,
            });
        }
        let id = &bytes[id_start..id_start + id_len];
        let kind_pos = id_start + id_len;
        if kind_pos + 1 > end {
            return Err(Error::BufferTooShort {
                need: kind_pos + 1,
                have: end,
                what: "BIOP NameComponent kind_length (8-bit)",
            });
        }
        let kind_len = bytes[kind_pos] as usize;
        let kind_start = kind_pos + 1;
        if kind_start + kind_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: kind_len,
                available: end - kind_start,
            });
        }
        let kind = &bytes[kind_start..kind_start + kind_len];
        Ok((NameComponent { id, kind }, kind_start + kind_len))
    }

    /// Serialize one BIOP Directory name component with 8-bit length prefixes.
    pub(crate) fn serialize_8bit(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len_8bit();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.id.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.id.len(),
                available: u8::MAX as usize,
            });
        }
        if self.kind.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.kind.len(),
                available: u8::MAX as usize,
            });
        }
        buf[0] = self.id.len() as u8;
        buf[1..1 + self.id.len()].copy_from_slice(self.id);
        let kind_pos = 1 + self.id.len();
        buf[kind_pos] = self.kind.len() as u8;
        buf[kind_pos + 1..kind_pos + 1 + self.kind.len()].copy_from_slice(self.kind);
        Ok(len)
    }
}

// ── ServiceLocation ───────────────────────────────────────────────────────────

/// DSM::ServiceLocation — first mandatory component of Lite Options Profile Body.
/// TR 101 202 §4.7.3.3, Table 4.7.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ServiceLocation<'a> {
    /// DVB carousel NSAP address (always 20 bytes, serviceDomain_length=0x14).
    pub service_domain: NsapAddress,
    /// CosNaming path components (nameComponents_count, 32-bit-length fields).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub path: Vec<NameComponent<'a>>,
    /// `InitialContext_data`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub initial_context: &'a [u8],
}

impl<'a> ServiceLocation<'a> {
    fn serialized_len(&self) -> usize {
        SERVICE_DOMAIN_LEN_FIELD
            + NSAP_ADDRESS_LEN
            + NAMING_COUNT_LEN
            + self
                .path
                .iter()
                .map(|c| c.serialized_len_32bit())
                .sum::<usize>()
            + INITIAL_CONTEXT_LEN_FIELD
            + self.initial_context.len()
    }

    fn parse_from(bytes: &'a [u8], pos: usize, end: usize) -> Result<(Self, usize)> {
        if pos + SERVICE_DOMAIN_LEN_FIELD > end {
            return Err(Error::BufferTooShort {
                need: pos + SERVICE_DOMAIN_LEN_FIELD,
                have: end,
                what: "ServiceLocation serviceDomain_length",
            });
        }
        let sd_len = bytes[pos] as usize;
        if sd_len != NSAP_ADDRESS_LEN {
            return Err(Error::ValueOutOfRange {
                field: "serviceDomain_length",
                reason: "DVB Carousel NSAP address must be exactly 20 bytes (0x14)",
            });
        }
        let sd_start = pos + SERVICE_DOMAIN_LEN_FIELD;
        if sd_start + NSAP_ADDRESS_LEN > end {
            return Err(Error::BufferTooShort {
                need: sd_start + NSAP_ADDRESS_LEN,
                have: end,
                what: "ServiceLocation serviceDomain_data",
            });
        }
        let service_domain = NsapAddress::parse_from(bytes, sd_start)?;
        let mut cur = sd_start + NSAP_ADDRESS_LEN;

        let (bnc, _) = bytes[cur..end]
            .split_first_chunk::<4>()
            .ok_or(Error::BufferTooShort {
                need: cur + NAMING_COUNT_LEN,
                have: end,
                what: "ServiceLocation nameComponents_count",
            })?;
        let name_count = u32::from_be_bytes(*bnc) as usize;
        cur += NAMING_COUNT_LEN;
        let mut path = Vec::with_capacity(name_count.min(16));
        for _ in 0..name_count {
            let (nc, next) = NameComponent::parse_32bit(bytes, cur, end)?;
            path.push(nc);
            cur = next;
        }

        let (bic, _) = bytes[cur..end]
            .split_first_chunk::<4>()
            .ok_or(Error::BufferTooShort {
                need: cur + INITIAL_CONTEXT_LEN_FIELD,
                have: end,
                what: "ServiceLocation initialContext_length",
            })?;
        let ic_len = u32::from_be_bytes(*bic) as usize;
        cur += INITIAL_CONTEXT_LEN_FIELD;
        if cur + ic_len > end {
            return Err(Error::SectionLengthOverflow {
                declared: ic_len,
                available: end - cur,
            });
        }
        let initial_context = &bytes[cur..cur + ic_len];
        cur += ic_len;
        Ok((
            ServiceLocation {
                service_domain,
                path,
                initial_context,
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
        buf[0] = NSAP_ADDRESS_LEN as u8; // serviceDomain_length = 0x14
        self.service_domain
            .serialize_into_buf(&mut buf[1..1 + NSAP_ADDRESS_LEN])?;
        let mut pos = SERVICE_DOMAIN_LEN_FIELD + NSAP_ADDRESS_LEN;
        buf[pos..pos + 4].copy_from_slice(&(self.path.len() as u32).to_be_bytes());
        pos += NAMING_COUNT_LEN;
        for nc in &self.path {
            let written = nc.serialize_32bit(&mut buf[pos..])?;
            pos += written;
        }
        let ic_len = self.initial_context.len();
        buf[pos..pos + 4].copy_from_slice(&(ic_len as u32).to_be_bytes());
        pos += INITIAL_CONTEXT_LEN_FIELD;
        buf[pos..pos + ic_len].copy_from_slice(self.initial_context);
        pos += ic_len;
        Ok(pos)
    }
}

// ── LiteOptionsProfileBody ────────────────────────────────────────────────────

/// Lite Options Profile Body — decoded contents of a `TAG_LITE_OPTIONS` profile.
/// TR 101 202 §4.7.3.3, Table 4.7.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LiteOptionsProfileBody<'a> {
    /// First mandatory component: DSM::ServiceLocation.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub service_location: ServiceLocation<'a>,
    /// Extra components beyond the mandatory one (tag + raw data, 8-bit length prefix).
    pub extra: Vec<LiteComponent<'a>>,
}

impl<'a> LiteOptionsProfileBody<'a> {
    fn parse_from(bytes: &'a [u8]) -> Result<Self> {
        let end = bytes.len();
        if end < LITE_OPTIONS_BODY_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: LITE_OPTIONS_BODY_FIXED_LEN,
                have: end,
                what: "LiteOptions Profile Body fixed fields",
            });
        }
        let byte_order = bytes[0];
        if byte_order != BYTE_ORDER_BIG_ENDIAN {
            return Err(Error::ReservedBitsViolation {
                field: "profile_data_byte_order (LiteOptions)",
                reason: "must be 0x00 (big-endian) per DVB mandatory constraint",
            });
        }
        let component_count = bytes[1] as usize;
        if component_count < 1 {
            return Err(Error::ValueOutOfRange {
                field: "component_count (LiteOptions)",
                reason: "must have at least 1 component (ServiceLocation)",
            });
        }
        let mut pos = LITE_OPTIONS_BODY_FIXED_LEN;

        // First component: ServiceLocation (32-bit component_data_length)
        let (slch, _) = bytes[pos..end]
            .split_first_chunk::<SERVICE_LOCATION_COMP_HEADER_LEN>()
            .ok_or(Error::BufferTooShort {
                need: pos + SERVICE_LOCATION_COMP_HEADER_LEN,
                have: end,
                what: "LiteOptions ServiceLocation component header",
            })?;
        let comp0_tag = u32::from_be_bytes([slch[0], slch[1], slch[2], slch[3]]);
        if comp0_tag != TAG_SERVICE_LOCATION {
            return Err(Error::ReservedBitsViolation {
                field: "componentId_tag[0] (LiteOptions)",
                reason: "first component must be TAG_ServiceLocation (0x49534F46)",
            });
        }
        let comp0_len = u32::from_be_bytes([slch[4], slch[5], slch[6], slch[7]]) as usize;
        pos += SERVICE_LOCATION_COMP_HEADER_LEN;
        let comp0_end = pos + comp0_len;
        if comp0_end > end {
            return Err(Error::SectionLengthOverflow {
                declared: comp0_len,
                available: end - pos,
            });
        }
        let (service_location, _) = ServiceLocation::parse_from(bytes, pos, comp0_end)?;
        pos = comp0_end;

        // Extra components (8-bit length)
        let extra_count = component_count - 1;
        let mut extra = Vec::with_capacity(extra_count.min(8));
        for _ in 0..extra_count {
            let (ech, _) = bytes[pos..end]
                .split_first_chunk::<COMPONENT_HEADER_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: pos + COMPONENT_HEADER_LEN,
                    have: end,
                    what: "LiteOptions extra component header",
                })?;
            let tag = u32::from_be_bytes([ech[0], ech[1], ech[2], ech[3]]);
            let data_len = ech[4] as usize;
            pos += COMPONENT_HEADER_LEN;
            if pos + data_len > end {
                return Err(Error::SectionLengthOverflow {
                    declared: data_len,
                    available: end - pos,
                });
            }
            extra.push(LiteComponent {
                tag,
                data: &bytes[pos..pos + data_len],
            });
            pos += data_len;
        }

        Ok(LiteOptionsProfileBody {
            service_location,
            extra,
        })
    }

    fn serialized_len(&self) -> usize {
        let sl_data_len = self.service_location.serialized_len();
        let extra_len: usize = self.extra.iter().map(|c| c.serialized_len()).sum();
        LITE_OPTIONS_BODY_FIXED_LEN + SERVICE_LOCATION_COMP_HEADER_LEN + sl_data_len + extra_len
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let total_components = 1 + self.extra.len();
        if total_components > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: total_components,
                available: u8::MAX as usize,
            });
        }
        buf[0] = BYTE_ORDER_BIG_ENDIAN;
        buf[1] = total_components as u8;
        let mut pos = LITE_OPTIONS_BODY_FIXED_LEN;

        // ServiceLocation component (32-bit data length)
        let sl_data_len = self.service_location.serialized_len();
        buf[pos..pos + 4].copy_from_slice(&TAG_SERVICE_LOCATION.to_be_bytes());
        buf[pos + 4..pos + 8].copy_from_slice(&(sl_data_len as u32).to_be_bytes());
        pos += SERVICE_LOCATION_COMP_HEADER_LEN;
        let written = self.service_location.serialize_into_buf(&mut buf[pos..])?;
        pos += written;

        // Extra components (8-bit data length)
        for comp in &self.extra {
            let data_len = comp.data.len();
            if data_len > u8::MAX as usize {
                return Err(Error::SectionLengthOverflow {
                    declared: data_len,
                    available: u8::MAX as usize,
                });
            }
            buf[pos..pos + 4].copy_from_slice(&comp.tag.to_be_bytes());
            buf[pos + 4] = data_len as u8;
            pos += COMPONENT_HEADER_LEN;
            buf[pos..pos + data_len].copy_from_slice(comp.data);
            pos += data_len;
        }

        Ok(len)
    }
}

// ── TaggedProfile ─────────────────────────────────────────────────────────────

/// A tagged profile entry in an `IOP::IOR`.
/// TR 101 202 §4.7.3, Table 4.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TaggedProfile<'a> {
    /// `TAG_BIOP` (0x49534F06) — BIOP Profile Body.
    Biop(BiopProfileBody<'a>),
    /// `TAG_LITE_OPTIONS` (0x49534F05) — Lite Options Profile Body.
    LiteOptions(LiteOptionsProfileBody<'a>),
    /// Any other `profileId_tag`.
    Unknown {
        /// The raw `profileId_tag` value.
        tag: u32,
        /// Raw `profile_data` bytes.
        #[cfg_attr(feature = "serde", serde(borrow))]
        data: &'a [u8],
    },
}

impl<'a> TaggedProfile<'a> {
    fn serialized_len(&self) -> usize {
        let data_len = match self {
            Self::Biop(b) => b.serialized_len(),
            Self::LiteOptions(l) => l.serialized_len(),
            Self::Unknown { data, .. } => data.len(),
        };
        PROFILE_HEADER_LEN + data_len
    }

    fn serialize_into_buf(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let (tag, data_len) = match self {
            Self::Biop(b) => (TAG_BIOP, b.serialized_len()),
            Self::LiteOptions(l) => (TAG_LITE_OPTIONS, l.serialized_len()),
            Self::Unknown { tag, data } => (*tag, data.len()),
        };
        buf[0..4].copy_from_slice(&tag.to_be_bytes());
        buf[4..8].copy_from_slice(&(data_len as u32).to_be_bytes());
        let pos = PROFILE_HEADER_LEN;
        match self {
            Self::Biop(b) => {
                b.serialize_into_buf(&mut buf[pos..])?;
            }
            Self::LiteOptions(l) => {
                l.serialize_into_buf(&mut buf[pos..])?;
            }
            Self::Unknown { data, .. } => {
                buf[pos..pos + data.len()].copy_from_slice(data);
            }
        }
        Ok(len)
    }
}

// ── Ior ───────────────────────────────────────────────────────────────────────

/// `IOP::IOR` — Interoperable Object Reference.
/// TR 101 202 §4.7.3.1, Table 4.3.
///
/// Carries the `type_id` (object kind) and one or more tagged profiles that
/// describe where to find the object in the carousel.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ior<'a> {
    /// `type_id` bytes — DVB: always a 4-byte alias (see [`ObjectKind`]).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub type_id: &'a [u8],
    /// One or more tagged profiles.  The first must be BIOP or LiteOptions.
    pub profiles: Vec<TaggedProfile<'a>>,
}

impl<'a> Ior<'a> {
    /// Decode the `type_id` as an [`ObjectKind`].
    pub fn object_kind(&self) -> ObjectKind {
        if self.type_id.len() == 4 {
            let mut arr = [0u8; 4];
            arr.copy_from_slice(self.type_id);
            ObjectKind::from_bytes(arr)
        } else {
            ObjectKind::Unknown([0; 4])
        }
    }

    /// Return the first BIOP Profile Body from this IOR, if present.
    pub fn biop_profile(&self) -> Option<&BiopProfileBody<'a>> {
        for p in &self.profiles {
            if let TaggedProfile::Biop(b) = p {
                return Some(b);
            }
        }
        None
    }
}

impl<'a> Parse<'a> for Ior<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let end = bytes.len();
        let (ior_hdr, _) =
            bytes
                .split_first_chunk::<IOR_FIXED_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: IOR_FIXED_LEN,
                    have: end,
                    what: "IOP::IOR fixed fields",
                })?;
        let type_id_length =
            u32::from_be_bytes([ior_hdr[0], ior_hdr[1], ior_hdr[2], ior_hdr[3]]) as usize;
        // DVB: only alias type_ids (N%4==0); reject non-conformant.
        if type_id_length % 4 != 0 {
            return Err(Error::ValueOutOfRange {
                field: "IOR.type_id_length",
                reason: "type_id_length must be a multiple of 4 (DVB alias type_ids only — \
                         non-aligned type_ids are not supported per TR 101 202 §4.7.3.1)",
            });
        }
        let mut pos = 4;
        if pos + type_id_length > end {
            return Err(Error::SectionLengthOverflow {
                declared: type_id_length,
                available: end - pos,
            });
        }
        let type_id = &bytes[pos..pos + type_id_length];
        pos += type_id_length;

        let (bpc, _) = bytes[pos..end]
            .split_first_chunk::<4>()
            .ok_or(Error::BufferTooShort {
                need: pos + 4,
                have: end,
                what: "IOR taggedProfiles_count",
            })?;
        let profiles_count = u32::from_be_bytes(*bpc) as usize;
        pos += 4;

        let mut profiles = Vec::with_capacity(profiles_count.min(8));
        for _ in 0..profiles_count {
            let (phdr, _) = bytes[pos..end]
                .split_first_chunk::<PROFILE_HEADER_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: pos + PROFILE_HEADER_LEN,
                    have: end,
                    what: "TaggedProfile header",
                })?;
            let tag = u32::from_be_bytes([phdr[0], phdr[1], phdr[2], phdr[3]]);
            let data_len = u32::from_be_bytes([phdr[4], phdr[5], phdr[6], phdr[7]]) as usize;
            pos += PROFILE_HEADER_LEN;
            if pos + data_len > end {
                return Err(Error::SectionLengthOverflow {
                    declared: data_len,
                    available: end - pos,
                });
            }
            let profile_data = &bytes[pos..pos + data_len];
            let profile = match tag {
                TAG_BIOP => TaggedProfile::Biop(BiopProfileBody::parse_from(profile_data)?),
                TAG_LITE_OPTIONS => {
                    TaggedProfile::LiteOptions(LiteOptionsProfileBody::parse_from(profile_data)?)
                }
                _ => TaggedProfile::Unknown {
                    tag,
                    data: profile_data,
                },
            };
            profiles.push(profile);
            pos += data_len;
        }

        Ok(Ior { type_id, profiles })
    }
}

impl Serialize for Ior<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let type_id_len = self.type_id.len();
        let profiles_len: usize = self.profiles.iter().map(|p| p.serialized_len()).sum();
        4 // type_id_length field
            + type_id_len
            + 4 // taggedProfiles_count field
            + profiles_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.type_id.len() % 4 != 0 {
            return Err(Error::ValueOutOfRange {
                field: "IOR.type_id_length",
                reason: "type_id_length must be a multiple of 4 (DVB alias type_ids only)",
            });
        }
        buf[0..4].copy_from_slice(&(self.type_id.len() as u32).to_be_bytes());
        buf[4..4 + self.type_id.len()].copy_from_slice(self.type_id);
        let mut pos = 4 + self.type_id.len();
        buf[pos..pos + 4].copy_from_slice(&(self.profiles.len() as u32).to_be_bytes());
        pos += 4;
        for profile in &self.profiles {
            let written = profile.serialize_into_buf(&mut buf[pos..])?;
            pos += written;
        }
        Ok(len)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Parse;

    fn sample_ior() -> Vec<u8> {
        // IOR for a ServiceGateway object in the m6 fixture format:
        // type_id = "srg\0", 1 profile TAG_BIOP
        // ObjectLocation: carousel_id=0xAB, module_id=1, v1.0, key=[0x01]
        // ConnBinder: 1 tap, use=0x0016, assoc=0x47, selector=[0x00,0x01,0x80,0x00,0x00,0x02,0xFF,0xFF,0xFF,0xFF]
        #[rustfmt::skip]
        let bytes: &[u8] = &[
            // type_id_length=4
            0x00, 0x00, 0x00, 0x04,
            // type_id = "srg\0"
            0x73, 0x72, 0x67, 0x00,
            // taggedProfiles_count=1
            0x00, 0x00, 0x00, 0x01,
            // profileId_tag = TAG_BIOP
            0x49, 0x53, 0x4F, 0x06,
            // profile_data_length = 40
            0x00, 0x00, 0x00, 0x28,
            // byte_order=0, liteComponents_count=2
            0x00, 0x02,
            // ObjectLocation: tag(4)+len(1)
            0x49, 0x53, 0x4F, 0x50,  0x0A,
            // carouselId=0xAB, moduleId=1, v1.0, key_len=1, key=0x01
            0x00, 0x00, 0x00, 0xAB,  0x00, 0x01,  0x01, 0x00,  0x01,  0x01,
            // ConnBinder: tag(4)+len(1)
            0x49, 0x53, 0x4F, 0x40,  0x12,
            // taps_count=1, tap: id=0, use=0x0016, assoc=0x47, sel_len=10, selector
            0x01,  0x00, 0x00,  0x00, 0x16,  0x00, 0x47,  0x0A,
            0x00, 0x01, 0x80, 0x00, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        bytes.to_vec()
    }

    #[test]
    fn ior_round_trip() {
        let raw = sample_ior();
        let ior = Ior::parse(&raw).unwrap();
        let mut out = vec![0u8; ior.serialized_len()];
        ior.serialize_into(&mut out).unwrap();
        assert_eq!(out, raw, "IOR round-trip byte-exact");
    }

    #[test]
    fn ior_byte_anchor_m6_sgw() {
        let raw = sample_ior();
        let ior = Ior::parse(&raw).unwrap();

        assert_eq!(ior.type_id, b"srg\0");
        assert_eq!(ior.object_kind(), ObjectKind::ServiceGateway);
        assert_eq!(ior.profiles.len(), 1);

        let bp = ior.biop_profile().unwrap();
        assert_eq!(bp.object_location.carousel_id, 0xAB);
        assert_eq!(bp.object_location.module_id, 1);
        assert_eq!(bp.object_location.version_major, 1);
        assert_eq!(bp.object_location.version_minor, 0);
        assert_eq!(bp.object_location.object_key, &[0x01]);

        assert_eq!(bp.conn_binder.taps.len(), 1);
        let tap = &bp.conn_binder.taps[0];
        assert_eq!(tap.id, 0);
        assert_eq!(tap.use_, 0x0016);
        assert_eq!(tap.association_tag, 0x47);
        assert_eq!(
            tap.selector,
            &[0x00, 0x01, 0x80, 0x00, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(tap.transaction_id(), Some(0x80000002));
        assert_eq!(tap.timeout(), Some(0xFFFFFFFF));
    }

    #[test]
    fn ior_rejects_non_aligned_type_id() {
        // type_id_length = 3 (not a multiple of 4)
        let bytes: &[u8] = &[
            0x00, 0x00, 0x00, 0x03, 0x64, 0x69, 0x72, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(matches!(
            Ior::parse(bytes).unwrap_err(),
            crate::error::Error::ValueOutOfRange {
                field: "IOR.type_id_length",
                ..
            }
        ));
    }

    #[test]
    fn object_kind_roundtrip() {
        let kinds = [
            ObjectKind::Directory,
            ObjectKind::File,
            ObjectKind::Stream,
            ObjectKind::ServiceGateway,
            ObjectKind::StreamEvent,
            ObjectKind::Unknown([0x01, 0x02, 0x03, 0x04]),
        ];
        for k in &kinds {
            let b = k.to_bytes();
            assert_eq!(ObjectKind::from_bytes(b), *k);
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn ior_serde_round_trip() {
        let raw = sample_ior();
        let ior = Ior::parse(&raw).unwrap();
        let json = serde_json::to_string(&ior).unwrap();
        // type_id is serialized as byte array; carousel_id is a field in ObjectLocation
        assert!(
            json.contains("carousel_id"),
            "JSON must contain carousel_id field"
        );
        assert!(
            json.contains("\"Biop\""),
            "JSON must contain Biop profile variant"
        );
    }
}
