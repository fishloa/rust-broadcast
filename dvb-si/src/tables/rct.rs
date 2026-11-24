//! Related Content Table — ETSI TS 102 323 v1.4.1 §10.4.
//!
//! Signals links to related material for a service. Carried in the ES whose
//! PID is named by a `related_content_descriptor` in that service's PMT
//! (stream_type 0x05, private sections). There is no fixed PID.
//!
//! The link_info loop is unfolded into [`LinkInfo`] entries (Table 110, §10.4.3)
//! with the conditional `dvb_binary_locator` sub-structure typed as
//! [`DvbBinaryLocator`] (Table 31, §7.3.2.3.3).

use crate::descriptors::DescriptorLoop;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::reserved::ReservedOnes;

/// Identifier type — ETSI TS 102 323 v1.4.1 §7.3.2.3.3 Table 32
/// (`private/specs-verified/dvb/dvb-si/`, Table 32 — Identifier type).
///
/// 2-bit field in the DVB binary locator. Selects what kind of event identifier
/// field (if any) is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IdentifierType {
    /// `0b00` — no event identifier field is present.
    None,
    /// `0b01` — event identifier is an `event_id`.
    EventId,
    /// `0b10` — event identifier is a `TVA_id` carried in EIT.
    TvaIdEit,
    /// `0b11` — event identifier is a `TVA_id` carried in PES.
    TvaIdPes,
}

impl IdentifierType {
    #[must_use]
    /// Creates a value from a 2-bit wire nibble (upper bits masked off).
    pub fn from_u8(v: u8) -> Self {
        match v & IDENTIFIER_TYPE_MASK {
            0x00 => Self::None,
            0x01 => Self::EventId,
            0x02 => Self::TvaIdEit,
            // The mask leaves exactly 0..=3, so the last arm is `0x03`.
            _ => Self::TvaIdPes,
        }
    }

    #[must_use]
    /// Returns the 2-bit wire nibble for this value.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::None => 0x00,
            Self::EventId => 0x01,
            Self::TvaIdEit => 0x02,
            Self::TvaIdPes => 0x03,
        }
    }

    #[must_use]
    /// Returns the spec token for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::EventId => "event_id",
            Self::TvaIdEit => "TVA_id (EIT)",
            Self::TvaIdPes => "TVA_id (PES)",
        }
    }
}
broadcast_common::impl_spec_display!(IdentifierType);
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// `table_id` for Related Content Table.
pub const TABLE_ID: u8 = 0x76;

// No fixed PID: this table's PID is signalled (PMT / descriptors). A `PID`
// constant of `0x0000` used to sit here, which is the real PAT PID, so a
// caller filtering on it would pick up PAT traffic (audit r02-W15/O12).

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const POST_EXT_FIXED_LEN: usize = 3;
const LINK_ENTRY_HEADER_LEN: usize = 2;
const DESC_LOOP_LEN_FIELD: usize = 2;
use super::CRC_LEN;

/// Width of the `reserved` header fields.
const RCT_RESERVED_BITS: u32 = 2;
/// Width of the `reserved_future_use` entry fields.
const RCT_RESERVED_FUTURE_USE_BITS: u32 = 4;
const MIN_SECTION_LEN: usize =
    MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN + DESC_LOOP_LEN_FIELD + CRC_LEN;

const LINK_TYPE_MASK: u8 = 0xF0;
const LINK_TYPE_SHIFT: u8 = 4;
const HOW_RELATED_HI_MASK: u8 = 0x03;
const TERM_ID_HI_MASK: u8 = 0x0F;
const TERM_ID_HI_SHIFT: u8 = 8;
const GROUP_ID_MASK: u8 = 0xF0;
const GROUP_ID_SHIFT: u8 = 4;
const PRECEDENCE_MASK: u8 = 0x0F;

const LOCATOR_ID_TYPE_MASK: u8 = 0xC0;
const LOCATOR_ID_TYPE_SHIFT: u8 = 6;
const LOCATOR_RELIABILITY_MASK: u8 = 0x20;
const LOCATOR_INLINE_MASK: u8 = 0x10;
const LOCATOR_START_DATE_HI_MASK: u8 = 0x07;
const LOCATOR_START_DATE_HI_SHIFT: usize = 6;
const LOCATOR_RESERVED_BITS: u8 = 0x08;
const LOCATOR_RESERVED_SHIFT: u32 = 3;

const ITEM_RFU_SHIFT: u32 = 6;
const ITEM_COUNT_MASK: u8 = 0x3F;

/// Bit widths passed to the bit-field helpers.
const ITEM_COUNT_BITS: u32 = 6;
const LOCATOR_TRIPLET_ID_BITS: u32 = 10;
const ICON_DESC_LEN_BITS: u32 = 12;

/// Wire masks for the code enums and packed fields.
const IDENTIFIER_TYPE_MASK: u8 = 0x03;
const LINK_TYPE_VALUE_MASK: u8 = 0x0F;
const HOW_RELATED_VALUE_MASK: u8 = 0x3F;
const HOW_RELATED_LO_MASK: u8 = 0x0F;
const GROUP_ID_VALUE_MASK: u8 = 0x0F;
const ICON_ID_VALUE_MASK: u8 = 0x07;
const LOCATOR_START_DATE_LO_MASK: u8 = 0x3F;
const LOCATOR_TRIPLET_ID_HI_MASK: u16 = 0x03;

/// Packed-field shifts.
const LOCATOR_START_DATE_LO_SHIFT: u8 = 2;
const LOCATOR_RELIABILITY_SHIFT: u8 = 5;
const LOCATOR_INLINE_SHIFT: u8 = 4;
const EARLY_START_WINDOW_SHIFT: u8 = 5;
const HOW_RELATED_SHIFT: u8 = 4;
const ICON_FLAG_SHIFT: u8 = 7;

/// Byte lengths of the fixed layouts.
const WORD_LEN: usize = 2;
const LOCATOR_HEADER_LEN: usize = 2;
const LOCATOR_TRIPLET_LEN: usize = 3;
const LOCATOR_FULL_TRIPLET_LEN: usize = 6;
const LOCATOR_TIME_FIELDS_LEN: usize = 4;
const LOCATOR_TVA_PES_LEN: usize = 3;
const LINK_INFO_HEADER_LEN: usize = 4;
const LINK_ITEM_HEADER_LEN: usize = 4;

/// Byte offsets inside the locator and link_info layouts.
const LOCATOR_TRIPLET_ID_LO_OFFSET: usize = 2;
const TRIPLET_ONID_OFFSET: usize = 2;
const TRIPLET_SID_OFFSET: usize = 4;
const TVA_PES_COMPONENT_OFFSET: usize = 2;
const LOCATOR_TSID_OFFSET: usize = 2;
const LOCATOR_ONID_OFFSET: usize = 4;
const LOCATOR_SID_OFFSET: usize = 6;
const LOCATOR_FULL_TRIPLET_END: usize = 8;
const LINK_ITEM_LANG_LAST_OFFSET: usize = 2;
const LINK_ITEM_TEXT_LEN_OFFSET: usize = 3;
const TERM_ID_LO_OFFSET: usize = 2;
const GROUP_ID_OFFSET: usize = 3;

/// Byte offsets in the RCT section header.
const YEAR_OFFSET_OFFSET: usize = 8;
const LINK_COUNT_OFFSET: usize = 10;

const LINK_INFO_HEADER_RFU: u8 = 0x0C;
const LINK_INFO_HEADER_RFU_SHIFT: u32 = 2;

const ICON_FLAG_MASK: u8 = 0x80;
const ICON_ID_MASK: u8 = 0x70;
const ICON_ID_SHIFT: u8 = 4;
const ICON_DESC_LEN_HI_MASK: u8 = 0x0F;
/// `early_start_window(3)` mask inside the locator window byte: bits `[7:5]`
/// (ETSI TS 102 323 §10.4.3).
const EARLY_START_WINDOW_MASK: u8 = 0x07;
/// `late_end_window(5)` mask inside the locator window byte: bits `[4:0]`.
const LATE_END_WINDOW_MASK: u8 = 0x1F;

/// Link type — ETSI TS 102 323 §10.4.3 Table 111.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LinkType {
    /// 0x0 — URI string only.
    UriString,
    /// 0x1 — Binary locator only.
    BinaryLocator,
    /// 0x2 — Both binary locator and URI.
    Both,
    /// 0x3 — Through means of a descriptor.
    Descriptor,
    /// 0x4..=0xF — DVB reserved.
    DvbReserved(u8),
}

impl LinkType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v & LINK_TYPE_VALUE_MASK {
            0x0 => Self::UriString,
            0x1 => Self::BinaryLocator,
            0x2 => Self::Both,
            0x3 => Self::Descriptor,
            v => Self::DvbReserved(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::UriString => 0x0,
            Self::BinaryLocator => 0x1,
            Self::Both => 0x2,
            Self::Descriptor => 0x3,
            Self::DvbReserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::UriString => "URI String",
            Self::BinaryLocator => "Binary Locator",
            Self::Both => "Both",
            Self::Descriptor => "Descriptor",
            Self::DvbReserved(_) => "DVB Reserved",
        }
    }
}
broadcast_common::impl_spec_display!(LinkType, DvbReserved);

/// How-related classification scheme ID — ETSI TS 102 323 §10.4.3 Table 112.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HowRelated {
    /// 0x00 — HowRelatedCS:2004.
    Cs2004,
    /// 0x01 — HowRelatedCS:2005.
    Cs2005,
    /// 0x02 — HowRelatedCS:2007.
    Cs2007,
    /// 0x03..=0x2F — DVB reserved.
    DvbReserved(u8),
    /// 0x30..=0x3F — User private.
    UserPrivate(u8),
}

impl HowRelated {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Cs2004,
            0x01 => Self::Cs2005,
            0x02 => Self::Cs2007,
            v @ 0x03..0x30 => Self::DvbReserved(v),
            _ => Self::UserPrivate(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Cs2004 => 0x00,
            Self::Cs2005 => 0x01,
            Self::Cs2007 => 0x02,
            Self::DvbReserved(v) | Self::UserPrivate(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cs2004 => "HowRelatedCS:2004",
            Self::Cs2005 => "HowRelatedCS:2005",
            Self::Cs2007 => "HowRelatedCS:2007",
            Self::DvbReserved(_) => "DVB Reserved",
            Self::UserPrivate(_) => "User Private",
        }
    }
}
broadcast_common::impl_spec_display!(HowRelated, DvbReserved, UserPrivate);

/// A promotional text item within a link_info entry (Table 110, §10.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LinkItem<'a> {
    /// ISO 639-2 language code.
    pub language_code: LangCode,
    /// Promotional text (EN 300 468 Annex A).
    pub promotional_text: DvbText<'a>,
}

fn link_item_serialized_len(item: &LinkItem) -> usize {
    LINK_ITEM_HEADER_LEN + item.promotional_text.len()
}

/// Service identification within a DVB binary locator (Table 31, §7.3.2.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DvbLocatorService {
    /// `inline_service == 0`: 10-bit `DVB_service_triplet_ID`.
    Triplet {
        /// 10-bit DVB service triplet ID.
        dvb_service_triplet_id: u16,
    },
    /// `inline_service == 1`: full triplet.
    Full {
        /// `transport_stream_id`.
        transport_stream_id: u16,
        /// `original_network_id`.
        original_network_id: u16,
        /// `service_id`.
        service_id: u16,
    },
}

/// Event identifier within a DVB binary locator (Table 31).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DvbLocatorIdentifier {
    /// `identifier_type == 0x00`: no identifier field.
    None,
    /// `identifier_type == 0x01`: `event_id`.
    EventId {
        /// 16-bit event_id.
        event_id: u16,
    },
    /// `identifier_type == 0x02`: `TVA_id` carried in EIT.
    TvaIdEit {
        /// 16-bit TVA_id.
        tva_id: u16,
    },
    /// `identifier_type == 0x03`: `TVA_id` carried in PES + `component`.
    TvaIdPes {
        /// 16-bit TVA_id.
        tva_id: u16,
        /// 8-bit component.
        component: u8,
    },
}

/// Time windows within a DVB binary locator (Table 31).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DvbLocatorWindows {
    /// `early_start_window` (3 bits).
    pub early_start_window: u8,
    /// `late_end_window` (5 bits).
    pub late_end_window: u8,
}

/// DVB binary locator (Table 31, §7.3.2.3.3).
///
/// `identifier_type` and `inline_service` are wire-level discriminants for
/// `identifier`/`service` — rather than carry them as separate fields (a
/// second source of truth that a caller could set inconsistently with the
/// enum variant actually being serialized, r02-W17), they are derived from
/// `identifier`/`service` at serialize time via
/// [`identifier_type`](Self::identifier_type) /
/// [`inline_service`](Self::inline_service).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DvbBinaryLocator {
    /// `scheduled_time_reliability`.
    pub scheduled_time_reliability: bool,
    /// `start_date` (9 bits).
    pub start_date: u16,
    /// Service identification; also determines the wire `inline_service` bit.
    pub service: DvbLocatorService,
    /// `start_time` (16 bits).
    pub start_time: u16,
    /// `duration` (16 bits).
    pub duration: u16,
    /// Event identifier; also determines the wire `identifier_type` field.
    pub identifier: DvbLocatorIdentifier,
    /// Time windows. Must be `Some` iff `identifier` is
    /// [`DvbLocatorIdentifier::None`] and `scheduled_time_reliability` is
    /// `true` — [`serialize_into`](broadcast_common::Serialize::serialize_into)
    /// rejects any other combination rather than silently miswriting it.
    pub windows: Option<DvbLocatorWindows>,
    /// `reserved(1)` in the first locator byte (`1`); preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<1>,
    /// `reserved(2)` that replaces the 10-bit triplet id when
    /// `inline_service == 1` (`11`); preserved from the wire, written only for
    /// [`DvbLocatorService::Full`].
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_inline_service: ReservedOnes<RCT_RESERVED_BITS>,
}

impl DvbBinaryLocator {
    /// Construct a locator with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        scheduled_time_reliability: bool,
        start_date: u16,
        service: DvbLocatorService,
        start_time: u16,
        duration: u16,
        identifier: DvbLocatorIdentifier,
        windows: Option<DvbLocatorWindows>,
    ) -> Self {
        Self {
            scheduled_time_reliability,
            start_date,
            service,
            start_time,
            duration,
            identifier,
            windows,
            reserved: ReservedOnes::FILL,
            reserved_inline_service: ReservedOnes::FILL,
        }
    }

    /// The wire `identifier_type` (Table 32), derived from `identifier`.
    #[must_use]
    pub const fn identifier_type(&self) -> IdentifierType {
        match self.identifier {
            DvbLocatorIdentifier::None => IdentifierType::None,
            DvbLocatorIdentifier::EventId { .. } => IdentifierType::EventId,
            DvbLocatorIdentifier::TvaIdEit { .. } => IdentifierType::TvaIdEit,
            DvbLocatorIdentifier::TvaIdPes { .. } => IdentifierType::TvaIdPes,
        }
    }

    /// The wire `inline_service` bit, derived from `service`.
    #[must_use]
    pub const fn inline_service(&self) -> bool {
        matches!(self.service, DvbLocatorService::Full { .. })
    }
}

fn locator_serialized_len(loc: &DvbBinaryLocator) -> usize {
    let mut len = LOCATOR_HEADER_LEN;
    len += match &loc.service {
        DvbLocatorService::Triplet { .. } => 1,
        DvbLocatorService::Full { .. } => LOCATOR_FULL_TRIPLET_LEN,
    };
    len += LOCATOR_TIME_FIELDS_LEN;
    len += match &loc.identifier {
        DvbLocatorIdentifier::None => 0,
        DvbLocatorIdentifier::EventId { .. } => 2,
        DvbLocatorIdentifier::TvaIdEit { .. } => 2,
        DvbLocatorIdentifier::TvaIdPes { .. } => 3,
    };
    if loc.windows.is_some() {
        len += 1;
    }
    len
}

/// A single link_info entry (Table 110, §10.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LinkInfo<'a> {
    /// `link_type` (4 bits, Table 111).
    pub link_type: LinkType,
    /// `how_related_classification_scheme_id` (6 bits).
    pub how_related: HowRelated,
    /// `term_id` (12 bits).
    pub term_id: u16,
    /// `group_id` (4 bits).
    pub group_id: u8,
    /// `precedence` (4 bits).
    pub precedence: u8,
    /// Media URI bytes (present iff `link_type == 0x00 || link_type == 0x02`).
    pub media_uri: Option<&'a [u8]>,
    /// DVB binary locator (present iff `link_type == 0x01 || link_type == 0x02`).
    pub dvb_binary_locator: Option<DvbBinaryLocator>,
    /// Promotional text items.
    pub items: Vec<LinkItem<'a>>,
    /// `default_icon_flag`.
    pub default_icon_flag: bool,
    /// `icon_id` (3 bits).
    pub icon_id: u8,
    /// Per-link descriptor loop.
    pub descriptors: DescriptorLoop<'a>,
    /// `reserved_future_use(4)` before `link_info_length` (`1111`); preserved.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_link_info_length: ReservedOnes<RCT_RESERVED_FUTURE_USE_BITS>,
    /// `reserved_future_use(2)` between `link_type` and
    /// `how_related_classification_scheme_id` (`11`); preserved.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_link_type: ReservedOnes<RCT_RESERVED_BITS>,
    /// `reserved_future_use(2)` before `number_items` (`11`); preserved.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_number_items: ReservedOnes<RCT_RESERVED_BITS>,
}

impl<'a> LinkInfo<'a> {
    /// Construct a link_info entry with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        link_type: LinkType,
        how_related: HowRelated,
        term_id: u16,
        group_id: u8,
        precedence: u8,
        media_uri: Option<&'a [u8]>,
        dvb_binary_locator: Option<DvbBinaryLocator>,
        items: Vec<LinkItem<'a>>,
        default_icon_flag: bool,
        icon_id: u8,
        descriptors: DescriptorLoop<'a>,
    ) -> Self {
        Self {
            link_type,
            how_related,
            term_id,
            group_id,
            precedence,
            media_uri,
            dvb_binary_locator,
            items,
            default_icon_flag,
            icon_id,
            descriptors,
            reserved_link_info_length: ReservedOnes::FILL,
            reserved_link_type: ReservedOnes::FILL,
            reserved_number_items: ReservedOnes::FILL,
        }
    }
}

fn link_info_serialized_len(li: &LinkInfo) -> usize {
    let mut len = LINK_INFO_HEADER_LEN;
    len += li.media_uri.map_or(0, |u| 1 + u.len());
    len += li
        .dvb_binary_locator
        .as_ref()
        .map_or(0, locator_serialized_len);
    len += 1;
    len += li.items.iter().map(link_item_serialized_len).sum::<usize>();
    len += WORD_LEN;
    len += li.descriptors.len();
    len
}

/// Related Content Table (ETSI TS 102 323 v1.4.1 §10.4.2, Table 109).
///
/// The link_info loop is unfolded into typed [`LinkInfo`] entries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct RctSection<'a> {
    /// `section_syntax_indicator` (byte 1 bit 7): `1` for a constructed value,
    /// whatever the wire carried for a parsed one (parse tolerant, serialize
    /// preserves).
    pub section_syntax_indicator: bool,
    /// `table_id_extension_flag` (bit 6 of byte 1).
    pub table_id_extension_flag: bool,
    /// `service_id` — table_id_extension field.
    pub service_id: u16,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// `current_next_indicator`.
    pub current_next_indicator: bool,
    /// `section_number`.
    pub section_number: u8,
    /// `last_section_number`.
    pub last_section_number: u8,
    /// `year_offset` — reference year.
    pub year_offset: u16,
    /// Link info entries — unfolded per Table 110.
    pub links: Vec<LinkInfo<'a>>,
    /// Trailing descriptor loop.
    pub descriptors: DescriptorLoop<'a>,
    /// `reserved(2)` in byte 1 bits `[5:4]` (`11`); preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_b1: ReservedOnes<RCT_RESERVED_BITS>,
    /// `reserved(2)` in front of `version_number` (`11`); preserved.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_b5: ReservedOnes<RCT_RESERVED_BITS>,
    /// `reserved_future_use(4)` before `descriptor_loop_length` (`1111`).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_descriptor_loop_length: ReservedOnes<RCT_RESERVED_FUTURE_USE_BITS>,
}

impl<'a> RctSection<'a> {
    /// Construct an RCT section with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        table_id_extension_flag: bool,
        service_id: u16,
        section: crate::tables::SectionVersion,
        year_offset: u16,
        links: Vec<LinkInfo<'a>>,
        descriptors: DescriptorLoop<'a>,
    ) -> Self {
        Self {
            section_syntax_indicator: true,
            table_id_extension_flag,
            service_id,
            version_number: section.version_number,
            current_next_indicator: section.current_next_indicator,
            section_number: section.section_number,
            last_section_number: section.last_section_number,
            year_offset,
            links,
            descriptors,
            reserved_b1: ReservedOnes::FILL,
            reserved_b5: ReservedOnes::FILL,
            reserved_descriptor_loop_length: ReservedOnes::FILL,
        }
    }
}

fn parse_locator(data: &[u8]) -> Result<(DvbBinaryLocator, usize)> {
    need_in(
        data,
        LOCATOR_HEADER_LEN,
        "RctSection dvb_binary_locator header",
    )?;
    let b0 = data[0];
    let b1 = data[1];
    let identifier_type =
        IdentifierType::from_u8((b0 & LOCATOR_ID_TYPE_MASK) >> LOCATOR_ID_TYPE_SHIFT);
    let scheduled_time_reliability = (b0 & LOCATOR_RELIABILITY_MASK) != 0;
    let inline_service = (b0 & LOCATOR_INLINE_MASK) != 0;
    let start_date = ((b0 & LOCATOR_START_DATE_HI_MASK) as u16) << LOCATOR_START_DATE_HI_SHIFT
        | ((b1 >> LOCATOR_START_DATE_LO_SHIFT) as u16 & u16::from(LOCATOR_START_DATE_LO_MASK));

    let mut pos = LOCATOR_HEADER_LEN;
    let service = if inline_service {
        need_in(
            data,
            pos + LOCATOR_FULL_TRIPLET_LEN,
            "RctSection dvb_binary_locator full triplet",
        )?;
        let tsid = crate::wire::be_u16(data, pos, "rct")?;
        let onid = crate::wire::be_u16(data, pos + TRIPLET_ONID_OFFSET, "rct")?;
        let sid = crate::wire::be_u16(data, pos + TRIPLET_SID_OFFSET, "rct")?;
        pos += LOCATOR_FULL_TRIPLET_LEN;
        DvbLocatorService::Full {
            transport_stream_id: tsid,
            original_network_id: onid,
            service_id: sid,
        }
    } else {
        need_in(
            data,
            LOCATOR_TRIPLET_LEN,
            "RctSection dvb_binary_locator triplet",
        )?;
        let triplet_id = ((b1 as u16 & LOCATOR_TRIPLET_ID_HI_MASK) << u8::BITS)
            | data[LOCATOR_TRIPLET_ID_LO_OFFSET] as u16;
        pos = LOCATOR_TRIPLET_LEN;
        DvbLocatorService::Triplet {
            dvb_service_triplet_id: triplet_id,
        }
    };

    need_in(
        data,
        pos + LOCATOR_TIME_FIELDS_LEN,
        "RctSection dvb_binary_locator start_time/duration",
    )?;
    let (b2, rest) = data
        .get(pos..)
        .and_then(|s| s.split_first_chunk::<WORD_LEN>())
        .ok_or(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: pos + LOCATOR_TIME_FIELDS_LEN,
                have: data.len(),
                what: "RctSection dvb_binary_locator start_time/duration",
            },
        ))?;
    let start_time = u16::from_be_bytes(*b2);
    let (b2, _) = rest
        .split_first_chunk::<WORD_LEN>()
        .ok_or(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: pos + LOCATOR_TIME_FIELDS_LEN,
                have: data.len(),
                what: "RctSection dvb_binary_locator start_time/duration",
            },
        ))?;
    let duration = u16::from_be_bytes(*b2);
    pos += LOCATOR_TIME_FIELDS_LEN;

    let identifier = match identifier_type {
        IdentifierType::None => DvbLocatorIdentifier::None,
        IdentifierType::EventId => {
            need_in(
                data,
                pos + WORD_LEN,
                "RctSection dvb_binary_locator event_id",
            )?;
            let (b2, _) = data
                .get(pos..)
                .and_then(|s| s.split_first_chunk::<WORD_LEN>())
                .ok_or(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: pos + WORD_LEN,
                        have: data.len(),
                        what: "RctSection dvb_binary_locator event_id",
                    },
                ))?;
            let event_id = u16::from_be_bytes(*b2);
            pos += WORD_LEN;
            DvbLocatorIdentifier::EventId { event_id }
        }
        IdentifierType::TvaIdEit => {
            need_in(
                data,
                pos + WORD_LEN,
                "RctSection dvb_binary_locator TVA_id (EIT)",
            )?;
            let (b2, _) = data
                .get(pos..)
                .and_then(|s| s.split_first_chunk::<WORD_LEN>())
                .ok_or(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: pos + WORD_LEN,
                        have: data.len(),
                        what: "RctSection dvb_binary_locator TVA_id (EIT)",
                    },
                ))?;
            let tva_id = u16::from_be_bytes(*b2);
            pos += WORD_LEN;
            DvbLocatorIdentifier::TvaIdEit { tva_id }
        }
        IdentifierType::TvaIdPes => {
            need_in(
                data,
                pos + LOCATOR_TVA_PES_LEN,
                "RctSection dvb_binary_locator TVA_id (PES)",
            )?;
            let (b2, _) = data
                .get(pos..)
                .and_then(|s| s.split_first_chunk::<WORD_LEN>())
                .ok_or(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: pos + LOCATOR_TVA_PES_LEN,
                        have: data.len(),
                        what: "RctSection dvb_binary_locator TVA_id (PES)",
                    },
                ))?;
            let tva_id = u16::from_be_bytes(*b2);
            let component = data[pos + TVA_PES_COMPONENT_OFFSET];
            pos += LOCATOR_TVA_PES_LEN;
            DvbLocatorIdentifier::TvaIdPes { tva_id, component }
        }
    };

    let windows = if identifier_type == IdentifierType::None && scheduled_time_reliability {
        if pos >= data.len() {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + 1,
                    have: data.len(),
                    what: "RctSection dvb_binary_locator windows",
                },
            ));
        }
        let wb = data[pos];
        pos += 1;
        Some(DvbLocatorWindows {
            early_start_window: (wb >> EARLY_START_WINDOW_SHIFT) & EARLY_START_WINDOW_MASK,
            late_end_window: wb & LATE_END_WINDOW_MASK,
        })
    } else {
        None
    };

    Ok((
        {
            let mut loc = DvbBinaryLocator::new(
                scheduled_time_reliability,
                start_date,
                service,
                start_time,
                duration,
                identifier,
                windows,
            );
            loc.reserved = ReservedOnes::from_bits(u64::from(
                (b0 & LOCATOR_RESERVED_BITS) >> LOCATOR_RESERVED_SHIFT,
            ));
            loc.reserved_inline_service = if inline_service {
                ReservedOnes::from_bits(u64::from(b1 & super::RESERVED_2))
            } else {
                ReservedOnes::FILL
            };
            loc
        },
        pos,
    ))
}

fn serialize_locator(loc: &DvbBinaryLocator, buf: &mut [u8]) -> Result<usize> {
    // `windows` has no independent wire flag: its presence is entirely
    // implied by `identifier`/`scheduled_time_reliability`. Reject a
    // constructed value that disagrees, rather than silently writing
    // windows bytes that don't match what a reparse would derive (or
    // omitting `windows` a reparse would expect) (r02-W17).
    let windows_expected =
        matches!(loc.identifier, DvbLocatorIdentifier::None) && loc.scheduled_time_reliability;
    if loc.windows.is_some() != windows_expected {
        return Err(Error::InvalidDescriptor {
            tag: 0,
            reason: "DvbBinaryLocator.windows must be Some iff identifier is None \
                     and scheduled_time_reliability is true",
        });
    }

    broadcast_common::len::fit_mask(
        loc.start_date,
        (u16::from(LOCATOR_START_DATE_HI_MASK) << LOCATOR_START_DATE_HI_SHIFT)
            | u16::from(LOCATOR_START_DATE_LO_MASK),
        "start_date",
    )?;
    let b0 = (loc.identifier_type().to_u8() << LOCATOR_ID_TYPE_SHIFT)
        | (u8::from(loc.scheduled_time_reliability) << LOCATOR_RELIABILITY_SHIFT)
        | (u8::from(loc.inline_service()) << LOCATOR_INLINE_SHIFT)
        | ((loc.reserved.get() as u8) << LOCATOR_RESERVED_SHIFT)
        | ((loc.start_date >> LOCATOR_START_DATE_HI_SHIFT) as u8 & LOCATOR_START_DATE_HI_MASK);
    buf[0] = b0;

    let mut pos: usize;
    match &loc.service {
        DvbLocatorService::Triplet {
            dvb_service_triplet_id,
        } => {
            broadcast_common::len::fit_bits(
                u64::from(*dvb_service_triplet_id),
                LOCATOR_TRIPLET_ID_BITS,
                "dvb_service_triplet_id",
            )?;
            let sd_lo = (loc.start_date & u16::from(LOCATOR_START_DATE_LO_MASK)) as u8;
            buf[1] = (sd_lo << LOCATOR_START_DATE_LO_SHIFT)
                | ((dvb_service_triplet_id >> u8::BITS) as u8 & LOCATOR_TRIPLET_ID_HI_MASK as u8);
            buf[WORD_LEN] = (dvb_service_triplet_id & u16::from(u8::MAX)) as u8;
            pos = LOCATOR_TRIPLET_LEN;
        }
        DvbLocatorService::Full {
            transport_stream_id,
            original_network_id,
            service_id,
        } => {
            let sd_lo = (loc.start_date & u16::from(LOCATOR_START_DATE_LO_MASK)) as u8;
            buf[1] =
                (sd_lo << LOCATOR_START_DATE_LO_SHIFT) | (loc.reserved_inline_service.get() as u8);
            buf[LOCATOR_TSID_OFFSET..LOCATOR_ONID_OFFSET]
                .copy_from_slice(&transport_stream_id.to_be_bytes());
            buf[LOCATOR_ONID_OFFSET..LOCATOR_SID_OFFSET]
                .copy_from_slice(&original_network_id.to_be_bytes());
            buf[LOCATOR_SID_OFFSET..LOCATOR_FULL_TRIPLET_END]
                .copy_from_slice(&service_id.to_be_bytes());
            pos = LOCATOR_FULL_TRIPLET_END;
        }
    }
    buf[pos..pos + WORD_LEN].copy_from_slice(&loc.start_time.to_be_bytes());
    buf[pos + WORD_LEN..pos + LOCATOR_TIME_FIELDS_LEN].copy_from_slice(&loc.duration.to_be_bytes());
    pos += LOCATOR_TIME_FIELDS_LEN;

    match &loc.identifier {
        DvbLocatorIdentifier::None => {}
        DvbLocatorIdentifier::EventId { event_id } => {
            buf[pos..pos + WORD_LEN].copy_from_slice(&event_id.to_be_bytes());
            pos += WORD_LEN;
        }
        DvbLocatorIdentifier::TvaIdEit { tva_id } => {
            buf[pos..pos + WORD_LEN].copy_from_slice(&tva_id.to_be_bytes());
            pos += WORD_LEN;
        }
        DvbLocatorIdentifier::TvaIdPes { tva_id, component } => {
            buf[pos..pos + WORD_LEN].copy_from_slice(&tva_id.to_be_bytes());
            buf[pos + TVA_PES_COMPONENT_OFFSET] = *component;
            pos += LOCATOR_TVA_PES_LEN;
        }
    }
    if let Some(w) = &loc.windows {
        buf[pos] = (broadcast_common::len::fit_mask(
            w.early_start_window,
            EARLY_START_WINDOW_MASK,
            "early_start_window",
        )? << EARLY_START_WINDOW_SHIFT)
            | broadcast_common::len::fit_mask(
                w.late_end_window,
                LATE_END_WINDOW_MASK,
                "late_end_window",
            )?;
        pos += 1;
    }
    Ok(pos)
}

fn parse_link_info(data: &[u8], link_info_length: usize) -> Result<LinkInfo<'_>> {
    need_in(data, LINK_INFO_HEADER_LEN, "RctSection link_info header")?;
    let link_type = LinkType::from_u8((data[0] & LINK_TYPE_MASK) >> LINK_TYPE_SHIFT);
    let how_related_raw = (data[0] & HOW_RELATED_HI_MASK) << HOW_RELATED_SHIFT
        | (data[1] >> HOW_RELATED_SHIFT) & HOW_RELATED_LO_MASK;
    let how_related = HowRelated::from_u8(how_related_raw);
    let term_id = ((data[1] as u16 & TERM_ID_HI_MASK as u16) << TERM_ID_HI_SHIFT)
        | data[TERM_ID_LO_OFFSET] as u16;
    let group_id = (data[GROUP_ID_OFFSET] & GROUP_ID_MASK) >> GROUP_ID_SHIFT;
    let precedence = data[GROUP_ID_OFFSET] & PRECEDENCE_MASK;

    let mut pos = LINK_INFO_HEADER_LEN;
    let end = link_info_length;

    let media_uri = if link_type == LinkType::UriString || link_type == LinkType::Both {
        if pos >= end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + 1,
                    have: end,
                    what: "RctSection media_uri_length",
                },
            ));
        }
        let uri_len = data[pos] as usize;
        pos += 1;
        if pos + uri_len > end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + uri_len,
                    have: end,
                    what: "RctSection media_uri",
                },
            ));
        }
        let uri = &data[pos..pos + uri_len];
        pos += uri_len;
        Some(uri)
    } else {
        None
    };

    let dvb_binary_locator = if link_type == LinkType::BinaryLocator || link_type == LinkType::Both
    {
        if pos >= end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + 1,
                    have: end,
                    what: "RctSection dvb_binary_locator",
                },
            ));
        }
        let (loc, consumed) = parse_locator(&data[pos..end])?;
        pos += consumed;
        Some(loc)
    } else {
        None
    };

    if pos >= end {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: pos + 1,
                have: end,
                what: "RctSection number_items",
            },
        ));
    }
    let ni_byte = data[pos];
    let number_items = (ni_byte & ITEM_COUNT_MASK) as usize;
    pos += 1;

    let mut items = Vec::with_capacity(number_items);
    for _ in 0..number_items {
        if pos + LINK_ITEM_HEADER_LEN > end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + LINK_ITEM_HEADER_LEN,
                    have: end,
                    what: "RctSection link_item",
                },
            ));
        }
        let language_code = LangCode([
            data[pos],
            data[pos + 1],
            data[pos + LINK_ITEM_LANG_LAST_OFFSET],
        ]);
        let text_len = data[pos + LINK_ITEM_TEXT_LEN_OFFSET] as usize;
        pos += LOCATOR_TIME_FIELDS_LEN;
        if pos + text_len > end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + text_len,
                    have: end,
                    what: "RctSection promotional_text",
                },
            ));
        }
        let promotional_text = DvbText::new(&data[pos..pos + text_len]);
        pos += text_len;
        items.push(LinkItem {
            language_code,
            promotional_text,
        });
    }

    if pos + WORD_LEN > end {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: pos + WORD_LEN,
                have: end,
                what: "RctSection icon/desc",
            },
        ));
    }
    let default_icon_flag = (data[pos] & ICON_FLAG_MASK) != 0;
    let icon_id = (data[pos] & ICON_ID_MASK) >> ICON_ID_SHIFT;
    let desc_len =
        (((data[pos] & ICON_DESC_LEN_HI_MASK) as usize) << u8::BITS) | data[pos + 1] as usize;
    pos += WORD_LEN;
    let desc_start = pos;
    let desc_end = desc_start + desc_len;
    if desc_end > end {
        return Err(Error::SectionLengthOverflow {
            declared: desc_len,
            available: end.saturating_sub(desc_start),
        });
    }
    // Reject bytes between the descriptor loop and `link_info_length`'s own
    // end — silently dropping them would break the parse -> serialize
    // byte-identity invariant (r02-W19), as AIT/EIT/SDT already reject the
    // equivalent trailing slack.
    if desc_end != end {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: end - desc_end,
                have: 0,
                what: "RctSection link_info trailing bytes",
            },
        ));
    }
    let descriptors = DescriptorLoop::new(&data[desc_start..desc_end]);

    let mut li = LinkInfo::new(
        link_type,
        how_related,
        term_id,
        group_id,
        precedence,
        media_uri,
        dvb_binary_locator,
        items,
        default_icon_flag,
        icon_id,
        descriptors,
    );
    li.reserved_link_type = ReservedOnes::from_bits(u64::from(
        (data[0] & LINK_INFO_HEADER_RFU) >> LINK_INFO_HEADER_RFU_SHIFT,
    ));
    li.reserved_number_items = ReservedOnes::from_bits(u64::from(ni_byte >> ITEM_RFU_SHIFT));
    Ok(li)
}

fn serialize_link_info(li: &LinkInfo, buf: &mut [u8]) -> Result<usize> {
    let lt = li.link_type.to_u8();
    let hr = broadcast_common::len::fit_mask(
        li.how_related.to_u8(),
        HOW_RELATED_VALUE_MASK,
        "how_related",
    )?;
    broadcast_common::len::fit_mask(
        li.term_id,
        (u16::from(TERM_ID_HI_MASK) << TERM_ID_HI_SHIFT) | u16::from(u8::MAX),
        "term_id",
    )?;
    buf[0] = (broadcast_common::len::fit_mask(lt, LINK_TYPE_VALUE_MASK, "lt")? << LINK_TYPE_SHIFT)
        | ((li.reserved_link_type.get() as u8) << LINK_INFO_HEADER_RFU_SHIFT)
        | ((hr >> HOW_RELATED_SHIFT) & HOW_RELATED_HI_MASK);
    buf[1] = ((hr & HOW_RELATED_LO_MASK) << HOW_RELATED_SHIFT)
        | ((li.term_id >> TERM_ID_HI_SHIFT) as u8 & TERM_ID_HI_MASK);
    buf[TERM_ID_LO_OFFSET] = li.term_id as u8;
    buf[GROUP_ID_OFFSET] =
        (broadcast_common::len::fit_mask(li.group_id, GROUP_ID_VALUE_MASK, "group_id")?
            << GROUP_ID_SHIFT)
            | broadcast_common::len::fit_mask(li.precedence, PRECEDENCE_MASK, "precedence")?;

    let mut pos = LINK_INFO_HEADER_LEN;
    if let Some(uri) = li.media_uri {
        let uri_len = broadcast_common::len::fit_u8(uri.len(), "media_uri_length")?;
        buf[pos] = uri_len;
        pos += 1;
        buf[pos..pos + uri.len()].copy_from_slice(uri);
        pos += uri.len();
    }
    if let Some(ref loc) = li.dvb_binary_locator {
        let n = serialize_locator(loc, &mut buf[pos..])?;
        pos += n;
    }
    let item_count =
        broadcast_common::len::fit_bits(li.items.len() as u64, ITEM_COUNT_BITS, "number_items")?;
    buf[pos] = ((li.reserved_number_items.get() as u8) << ITEM_RFU_SHIFT) | item_count as u8;
    pos += 1;
    for item in &li.items {
        let text_len =
            broadcast_common::len::fit_u8(item.promotional_text.len(), "promotional_text_length")?;
        buf[pos..pos + LINK_ITEM_TEXT_LEN_OFFSET].copy_from_slice(&item.language_code.0);
        buf[pos + LINK_ITEM_TEXT_LEN_OFFSET] = text_len;
        pos += LOCATOR_TIME_FIELDS_LEN;
        buf[pos..pos + item.promotional_text.len()].copy_from_slice(item.promotional_text.raw());
        pos += item.promotional_text.len();
    }
    let dll = broadcast_common::len::fit_bits(
        li.descriptors.len() as u64,
        ICON_DESC_LEN_BITS,
        "icon_descriptors_length",
    )?;
    buf[pos] = u8::from(li.default_icon_flag) << ICON_FLAG_SHIFT
        | (broadcast_common::len::fit_mask(li.icon_id, ICON_ID_VALUE_MASK, "icon_id")?
            << ICON_ID_SHIFT)
        | ((dll >> u8::BITS) as u8);
    buf[pos + 1] = dll as u8;
    pos += WORD_LEN;
    buf[pos..pos + li.descriptors.len()].copy_from_slice(li.descriptors.raw());
    pos += li.descriptors.len();
    Ok(pos)
}

impl<'a> Parse<'a> for RctSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, MIN_SECTION_LEN, "RctSection")?;
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "RctSection",
                expected: &[TABLE_ID],
            });
        }

        let table_id_extension_flag = (bytes[1] & super::SECTION_B1_PRIVATE_INDICATOR) != 0;
        let section_length = super::section_length_of(bytes);
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            section_length,
            MIN_SECTION_LEN,
        )?;

        let service_id = crate::wire::be_u16(bytes, super::TABLE_ID_EXTENSION_OFFSET, "rct")?;
        let version_number = super::version_number_of(bytes[super::VERSION_FLAGS_OFFSET]);
        let current_next_indicator = super::current_next_of(bytes[super::VERSION_FLAGS_OFFSET]);
        let section_number = bytes[super::SECTION_NUMBER_OFFSET];
        let last_section_number = bytes[super::LAST_SECTION_NUMBER_OFFSET];
        let year_offset = crate::wire::be_u16(bytes, YEAR_OFFSET_OFFSET, "rct")?;
        let link_count = bytes[LINK_COUNT_OFFSET];

        let payload_end = total - CRC_LEN;
        let mut pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN;

        let mut links = Vec::with_capacity(link_count as usize);
        for _ in 0..link_count {
            if pos + LINK_ENTRY_HEADER_LEN > payload_end {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: pos + LINK_ENTRY_HEADER_LEN,
                        have: payload_end,
                        what: "RctSection link_entry header",
                    },
                ));
            }
            let link_info_length = super::desc_loop_len_of(bytes[pos], bytes[pos + 1]);
            let link_data_start = pos + LINK_ENTRY_HEADER_LEN;
            let link_data_end = link_data_start + link_info_length;
            if link_data_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: link_info_length,
                    available: payload_end.saturating_sub(link_data_start),
                });
            }
            let mut link_info =
                parse_link_info(&bytes[link_data_start..link_data_end], link_info_length)?;
            link_info.reserved_link_info_length =
                ReservedOnes::from_bits(u64::from(super::len_reserved_of(bytes[pos])));
            links.push(link_info);
            pos = link_data_end;
        }

        if pos + DESC_LOOP_LEN_FIELD > payload_end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: pos + DESC_LOOP_LEN_FIELD,
                    have: payload_end,
                    what: "RctSection descriptor_loop_length field",
                },
            ));
        }
        let descriptor_loop_length = super::desc_loop_len_of(bytes[pos], bytes[pos + 1]);
        let desc_start = pos + DESC_LOOP_LEN_FIELD;
        let desc_end = desc_start + descriptor_loop_length;
        if desc_end > payload_end {
            return Err(Error::SectionLengthOverflow {
                declared: descriptor_loop_length,
                available: payload_end.saturating_sub(desc_start),
            });
        }
        // Reject bytes between the descriptor loop and the CRC — silently
        // dropping them would break the parse -> serialize byte-identity
        // invariant (r02-W19), as AIT/EIT/SDT already reject the equivalent
        // trailing slack.
        if desc_end != payload_end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: payload_end - desc_end,
                    have: 0,
                    what: "RctSection trailing bytes after descriptor loop",
                },
            ));
        }
        let descriptors = DescriptorLoop::new(&bytes[desc_start..desc_end]);

        Ok(RctSection {
            section_syntax_indicator: bytes[1] & super::SECTION_B1_SSI != 0,
            table_id_extension_flag,
            service_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            year_offset,
            links,
            descriptors,
            reserved_b1: ReservedOnes::from_bits(u64::from(super::b1_reserved_of(bytes[1]))),
            reserved_b5: ReservedOnes::from_bits(u64::from(super::b5_reserved_of(
                bytes[super::VERSION_FLAGS_OFFSET],
            ))),
            reserved_descriptor_loop_length: ReservedOnes::from_bits(u64::from(
                super::len_reserved_of(bytes[pos]),
            )),
        })
    }
}

impl Serialize for RctSection<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + POST_EXT_FIXED_LEN
            + self
                .links
                .iter()
                .map(|li| LINK_ENTRY_HEADER_LEN + link_info_serialized_len(li))
                .sum::<usize>()
            + DESC_LOOP_LEN_FIELD
            + self.descriptors.len()
            + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;

        buf[0] = TABLE_ID;
        buf[1] = super::b1_of(
            self.section_syntax_indicator,
            self.table_id_extension_flag,
            self.reserved_b1.get() as u8,
        )?;
        super::write_section_length(buf, len - MIN_HEADER_LEN)?;

        buf[super::TABLE_ID_EXTENSION_OFFSET..super::VERSION_FLAGS_OFFSET]
            .copy_from_slice(&self.service_id.to_be_bytes());
        buf[super::VERSION_FLAGS_OFFSET] = super::b5_of(
            self.reserved_b5.get() as u8,
            self.version_number,
            self.current_next_indicator,
        )?;
        buf[super::SECTION_NUMBER_OFFSET] = self.section_number;
        buf[super::LAST_SECTION_NUMBER_OFFSET] = self.last_section_number;
        buf[YEAR_OFFSET_OFFSET..LINK_COUNT_OFFSET].copy_from_slice(&self.year_offset.to_be_bytes());
        let link_count = broadcast_common::len::fit_u8(self.links.len(), "number_of_links")?;
        buf[LINK_COUNT_OFFSET] = link_count;

        let mut pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN;
        for li in &self.links {
            let li_body_len = broadcast_common::len::fit_bits(
                link_info_serialized_len(li) as u64,
                super::DESC_LOOP_LEN_BITS,
                "link_info_length",
            )?;
            super::write_desc_loop_len_r(
                &mut buf[pos..pos + WORD_LEN],
                li_body_len as usize,
                li.reserved_link_info_length.get() as u8,
            )?;
            pos += LINK_ENTRY_HEADER_LEN;
            pos += serialize_link_info(li, &mut buf[pos..])?;
        }

        let dll = broadcast_common::len::fit_bits(
            self.descriptors.len() as u64,
            super::DESC_LOOP_LEN_BITS,
            "descriptors_length",
        )?;
        super::write_desc_loop_len_r(
            &mut buf[pos..pos + WORD_LEN],
            dll as usize,
            self.reserved_descriptor_loop_length.get() as u8,
        )?;
        pos += DESC_LOOP_LEN_FIELD;
        buf[pos..pos + self.descriptors.len()].copy_from_slice(self.descriptors.raw());
        pos += self.descriptors.len();

        super::write_crc(buf, pos);
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for RctSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "RELATED_CONTENT";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_no_links_no_descriptors() {
        let rct = RctSection::new(
            false,
            0x0064,
            crate::tables::SectionVersion::new(3, true, 0, 0),
            0x07D3,
            Vec::new(),
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let p = RctSection::parse(&buf).unwrap();
        assert!(!p.table_id_extension_flag);
        assert_eq!(p.service_id, 0x0064);
        assert_eq!(p.year_offset, 0x07D3);
        assert!(p.links.is_empty());
    }

    /// Regression for r02-W19: a byte between the (declared-length)
    /// descriptor loop and the CRC must be rejected, not silently dropped.
    #[test]
    fn parse_rejects_trailing_bytes_after_descriptor_loop() {
        let rct = RctSection::new(
            false,
            0x0064,
            crate::tables::SectionVersion::new(3, true, 0, 0),
            0x07D3,
            Vec::new(),
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();

        // Grow section_length by 1 and splice one extra, UNDECLARED byte in
        // before the CRC — descriptor_loop_length stays 0, so this byte is
        // not covered by the (empty) descriptor loop at all.
        let crc_pos = buf.len() - CRC_LEN;
        let mut section_length = (((buf[1] & 0x0F) as u16) << 8) | buf[2] as u16;
        section_length += 1;
        buf[1] = (buf[1] & 0xF0) | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        let mut spliced = buf[..crc_pos].to_vec();
        spliced.push(0xEE); // the undeclared trailing byte
        spliced.extend_from_slice(&[0, 0, 0, 0]);
        let crc_start = spliced.len() - CRC_LEN;
        let crc = broadcast_common::crc32_mpeg2::compute(&spliced[..crc_start]);
        spliced[crc_start..].copy_from_slice(&crc.to_be_bytes());

        let err = RctSection::parse(&spliced).unwrap_err();
        assert!(
            matches!(&err, Error::BufferTooShort(broadcast_common::error::BufferTooShort { what, .. }) if what.contains("trailing bytes after descriptor loop")),
            "expected a trailing-bytes error, got {err:?}"
        );
    }

    #[test]
    fn parse_one_link_uri_only() {
        let li = LinkInfo::new(
            LinkType::UriString,
            HowRelated::Cs2005,
            0x123,
            0x5,
            0x9,
            Some(b"http://example.com"),
            None,
            Vec::new(),
            false,
            0,
            DescriptorLoop::new(&[]),
        );
        let rct = RctSection::new(
            false,
            0x1234,
            crate::tables::SectionVersion::new(7, true, 1, 3),
            2003,
            vec![li],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let p = RctSection::parse(&buf).unwrap();
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].link_type, LinkType::UriString);
        assert_eq!(p.links[0].how_related, HowRelated::Cs2005);
        assert_eq!(p.links[0].term_id, 0x123);
        assert_eq!(p.links[0].group_id, 0x5);
        assert_eq!(p.links[0].precedence, 0x9);
        assert_eq!(p.links[0].media_uri.unwrap(), b"http://example.com");
    }

    /// Regression for r02-W19: a byte between a link's own descriptor loop
    /// and its declared `link_info_length` must be rejected, not silently
    /// dropped.
    #[test]
    fn parse_rejects_trailing_bytes_inside_link_info() {
        let li = LinkInfo::new(
            LinkType::UriString,
            HowRelated::Cs2005,
            0x123,
            0x5,
            0x9,
            Some(b"http://example.com"),
            None,
            Vec::new(),
            false,
            0,
            DescriptorLoop::new(&[]),
        );
        let rct = RctSection::new(
            false,
            0x1234,
            crate::tables::SectionVersion::new(7, true, 1, 3),
            2003,
            vec![li],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();

        // Link entry header starts right after the fixed post-extension
        // header (MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN).
        let link_hdr_pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN;
        let old_link_info_length =
            crate::tables::desc_loop_len_of(buf[link_hdr_pos], buf[link_hdr_pos + 1]);
        let link_data_start = link_hdr_pos + LINK_ENTRY_HEADER_LEN;
        let old_link_data_end = link_data_start + old_link_info_length;

        // Grow link_info_length by 1 (so the link's own `end` boundary
        // moves past its actual content) and section_length by 1, then
        // splice one extra, undeclared byte in at the old boundary.
        let new_link_info_length = old_link_info_length + 1;
        buf[link_hdr_pos] = (buf[link_hdr_pos] & 0xF0) | ((new_link_info_length >> 8) as u8 & 0x0F);
        buf[link_hdr_pos + 1] = (new_link_info_length & 0xFF) as u8;
        let mut section_length = (((buf[1] & 0x0F) as u16) << 8) | buf[2] as u16;
        section_length += 1;
        buf[1] = (buf[1] & 0xF0) | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        let mut spliced = buf[..old_link_data_end].to_vec();
        spliced.push(0xEE); // the undeclared trailing byte
        spliced.extend_from_slice(&buf[old_link_data_end..buf.len() - CRC_LEN]);
        spliced.extend_from_slice(&[0, 0, 0, 0]);
        let crc_start = spliced.len() - CRC_LEN;
        let crc = broadcast_common::crc32_mpeg2::compute(&spliced[..crc_start]);
        spliced[crc_start..].copy_from_slice(&crc.to_be_bytes());

        let err = RctSection::parse(&spliced).unwrap_err();
        assert!(
            matches!(&err, Error::BufferTooShort(broadcast_common::error::BufferTooShort { what, .. }) if what.contains("link_info trailing bytes")),
            "expected a link_info trailing-bytes error, got {err:?}"
        );
    }

    #[test]
    fn parse_one_link_with_locator_and_items() {
        let loc = DvbBinaryLocator::new(
            false,
            0x0FF,
            DvbLocatorService::Full {
                transport_stream_id: 0x1000,
                original_network_id: 0x2000,
                service_id: 0x3000,
            },
            0x8000,
            0x4000,
            DvbLocatorIdentifier::EventId { event_id: 0xBEEF },
            None,
        );
        let li = LinkInfo::new(
            LinkType::BinaryLocator,
            HowRelated::Cs2007,
            0x456,
            0x1,
            0x2,
            None,
            Some(loc),
            vec![LinkItem {
                language_code: LangCode(*b"eng"),
                promotional_text: DvbText::new(b"Promo"),
            }],
            true,
            3,
            DescriptorLoop::new(&[0x80, 0x00]),
        );
        let rct = RctSection::new(
            true,
            0xABCD,
            crate::tables::SectionVersion::new(15, false, 2, 5),
            2024,
            vec![li],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let p = RctSection::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].link_type, LinkType::BinaryLocator);
        let l = p.links[0].dvb_binary_locator.as_ref().unwrap();
        assert_eq!(l.identifier_type(), IdentifierType::EventId);
        assert!(l.inline_service());
        assert_eq!(l.start_date, 0x0FF);
        assert_eq!(p.links[0].items.len(), 1);
        assert_eq!(p.links[0].items[0].language_code, LangCode(*b"eng"));
        assert!(p.links[0].default_icon_flag);
        assert_eq!(p.links[0].icon_id, 3);
    }

    /// Regression for r02-W17: `windows` has no independent wire flag — its
    /// presence is entirely implied by `identifier == None &&
    /// scheduled_time_reliability`. A constructed locator that disagrees
    /// (here: `windows: Some(..)` with a non-`None` identifier) must be
    /// rejected, not silently miswritten (previously there was no check at
    /// all, since `identifier_type` was a separate, independently-settable
    /// field).
    #[test]
    fn locator_serialize_rejects_windows_identifier_mismatch() {
        let loc = DvbBinaryLocator::new(
            true,
            0,
            DvbLocatorService::Triplet {
                dvb_service_triplet_id: 0,
            },
            0,
            0,
            DvbLocatorIdentifier::EventId { event_id: 1 },
            Some(DvbLocatorWindows {
                early_start_window: 0,
                late_end_window: 0,
            }),
        );
        let li = LinkInfo::new(
            LinkType::BinaryLocator,
            HowRelated::Cs2007,
            0,
            0,
            0,
            None,
            Some(loc),
            Vec::new(),
            false,
            0,
            DescriptorLoop::new(&[]),
        );
        let rct = RctSection::new(
            false,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            0,
            vec![li],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn byte_exact_round_trip_simple() {
        let li = LinkInfo::new(
            LinkType::UriString,
            HowRelated::Cs2004,
            0,
            0,
            0,
            Some(b"uri"),
            None,
            Vec::new(),
            false,
            0,
            DescriptorLoop::new(&[]),
        );
        let rct = RctSection::new(
            false,
            0x0001,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            0x07D3,
            vec![li],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let parsed = RctSection::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let rct = RctSection::new(
            false,
            0x0001,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            2024,
            Vec::new(),
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        buf[0] = 0x4A;
        assert!(matches!(
            RctSection::parse(&buf).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x4A, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_too_short() {
        assert!(matches!(
            RctSection::parse(&[0x76, 0x80, 0x00]).unwrap_err(),
            Error::BufferTooShort(..)
        ));
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let rct = RctSection::new(
            false,
            0x0001,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            0,
            Vec::new(),
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall(..)
        ));
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            RctSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    /// section_length is 12 bits (max 4095 = EXTENSION_HEADER_LEN(5) +
    /// POST_EXT_FIXED_LEN(3, incl. the link-count byte) +
    /// DESC_LOOP_LEN_FIELD(2) + descriptors + CRC_LEN(4), so 4081 is the
    /// largest trailing descriptor loop that fits with no links).
    #[test]
    fn serialize_rejects_trailing_descriptors_exceeding_section_length() {
        let desc = vec![0xAAu8; 4082];
        let rct = RctSection::new(
            false,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            2000,
            vec![],
            DescriptorLoop::new(&desc),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn serialize_accepts_trailing_descriptors_at_section_length_boundary() {
        let desc = vec![0xAAu8; 4081];
        let rct = RctSection::new(
            false,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            2000,
            vec![],
            DescriptorLoop::new(&desc),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).expect("boundary fits");
        let reparsed = RctSection::parse(&buf).expect("reparse");
        assert_eq!(rct, reparsed);
    }

    /// promotional_text_length is an 8-bit field; 256 must be rejected, not
    /// wrapped to 0.
    #[test]
    fn serialize_rejects_promotional_text_exceeding_8_bit_length() {
        let text = vec![b'a'; 256];
        let rct = RctSection::new(
            false,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            2000,
            vec![LinkInfo::new(
                LinkType::from_u8(0x03),
                HowRelated::from_u8(0),
                0,
                0,
                0,
                None,
                None,
                vec![LinkItem {
                    language_code: LangCode(*b"eng"),
                    promotional_text: DvbText::new(&text),
                }],
                false,
                0,
                DescriptorLoop::new(&[]),
            )],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    /// number_of_links is an 8-bit field; 256 links must be rejected, not
    /// wrapped to 0.
    #[test]
    fn serialize_rejects_link_count_exceeding_8_bit_length() {
        let link = LinkInfo::new(
            LinkType::from_u8(0x03),
            HowRelated::from_u8(0),
            0,
            0,
            0,
            None,
            None,
            vec![],
            false,
            0,
            DescriptorLoop::new(&[]),
        );
        let rct = RctSection::new(
            false,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            2000,
            core::iter::repeat_with(|| link.clone()).take(256).collect(),
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    /// number_items is a 6-bit count (#1004): 64 promotional-text items
    /// must be rejected, not written as `number_items = 0` followed by all
    /// 64 items.
    #[test]
    fn serialize_rejects_64_promotional_text_items_6_bit_count() {
        let items = || {
            (0..64)
                .map(|_| LinkItem {
                    language_code: LangCode(*b"eng"),
                    promotional_text: DvbText::new(b"x"),
                })
                .collect::<Vec<_>>()
        };
        let mk = |n: usize| {
            RctSection::new(
                false,
                1,
                crate::tables::SectionVersion::new(0, true, 0, 0),
                2000,
                vec![LinkInfo::new(
                    LinkType::from_u8(0x03),
                    HowRelated::from_u8(0),
                    0,
                    0,
                    0,
                    None,
                    None,
                    items().into_iter().take(n).collect(),
                    false,
                    0,
                    DescriptorLoop::new(&[]),
                )],
                DescriptorLoop::new(&[]),
            )
        };
        let rct = mk(64);
        let mut buf = vec![0u8; rct.serialized_len()];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
        // 63 is the widest legal count and must still round-trip.
        let ok = mk(63);
        let mut buf = vec![0u8; ok.serialized_len()];
        ok.serialize_into(&mut buf).unwrap();
        assert_eq!(RctSection::parse(&buf).unwrap().links[0].items.len(), 63);
    }

    /// media_URI_length is an 8-bit field (#1004); a 256-byte URI must be
    /// rejected, not wrapped to 0.
    #[test]
    fn serialize_rejects_256_byte_media_uri() {
        let uri = vec![b'u'; 256];
        let rct = RctSection::new(
            false,
            1,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            2000,
            vec![LinkInfo::new(
                LinkType::from_u8(0x00),
                HowRelated::from_u8(0),
                0,
                0,
                0,
                Some(&uri),
                None,
                vec![],
                false,
                0,
                DescriptorLoop::new(&[]),
            )],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    #[test]
    fn parse_handwritten_rct_no_links() {
        let mut bytes: Vec<u8> = vec![
            0x76, 0x80, 0x0E, 0x00, 0x64, 0xC7, 0x00, 0x00, 0x07, 0xD3, 0x00, 0xF0, 0x00,
        ];
        let crc = broadcast_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let rct = RctSection::parse(&bytes).unwrap();
        assert_eq!(rct.service_id, 0x0064);
        assert_eq!(rct.year_offset, 0x07D3);
        assert!(rct.links.is_empty());
    }

    #[test]
    fn link_type_full_range_round_trip() {
        for v in 0u8..=0x0F {
            let lt = LinkType::from_u8(v);
            assert_eq!(lt.to_u8(), v, "LinkType round-trip failed for {v:#04x}");
        }
    }

    #[test]
    fn how_related_full_range_round_trip() {
        for v in 0u8..=0x3F {
            let hr = HowRelated::from_u8(v);
            assert_eq!(hr.to_u8(), v, "HowRelated round-trip failed for {v:#04x}");
        }
    }

    #[test]
    fn how_related_arm_boundaries_are_classified() {
        assert_eq!(HowRelated::from_u8(0x00), HowRelated::Cs2004);
        assert_eq!(HowRelated::from_u8(0x01), HowRelated::Cs2005);
        assert_eq!(HowRelated::from_u8(0x02), HowRelated::Cs2007);
        assert_eq!(HowRelated::from_u8(0x03), HowRelated::DvbReserved(0x03));
        assert_eq!(HowRelated::from_u8(0x2F), HowRelated::DvbReserved(0x2F));
        assert_eq!(HowRelated::from_u8(0x30), HowRelated::UserPrivate(0x30));
    }

    #[test]
    fn identifier_type_full_range_round_trip() {
        for v in 0u8..=0x03 {
            let it = IdentifierType::from_u8(v);
            assert_eq!(
                it.to_u8(),
                v,
                "IdentifierType round-trip failed for 0x{v:02X}"
            );
        }
    }

    #[test]
    fn identifier_type_known_values() {
        assert_eq!(IdentifierType::from_u8(0), IdentifierType::None);
        assert_eq!(IdentifierType::from_u8(1), IdentifierType::EventId);
        assert_eq!(IdentifierType::from_u8(2), IdentifierType::TvaIdEit);
        assert_eq!(IdentifierType::from_u8(3), IdentifierType::TvaIdPes);
        assert_eq!(IdentifierType::None.name(), "none");
        assert_eq!(IdentifierType::EventId.name(), "event_id");
        assert_eq!(IdentifierType::TvaIdEit.name(), "TVA_id (EIT)");
    }

    /// Each reserved field is width-limited by its type, so a wider value is
    /// unrepresentable and cannot spill into the adjacent syntax bits.
    #[test]
    fn reserved_fields_cannot_hold_more_than_their_width() {
        let loc = DvbBinaryLocator {
            reserved: ReservedOnes::from_bits(0xFF),
            reserved_inline_service: ReservedOnes::from_bits(0xFF),
            ..DvbBinaryLocator::new(
                false,
                0,
                DvbLocatorService::Triplet {
                    dvb_service_triplet_id: 0,
                },
                0,
                0,
                DvbLocatorIdentifier::None,
                None,
            )
        };
        assert_eq!(loc.reserved.get(), 0x01);
        assert_eq!(loc.reserved_inline_service.get(), 0x03);
    }

    fn section_with_links(links: &[&[u8]]) -> Vec<u8> {
        let body_len: usize = links.iter().map(|l| 2 + l.len()).sum::<usize>() + 2;
        let total = 11 + body_len + 4;
        let section_length = (total - 3) as u16;
        let mut bytes = vec![
            0x76,
            0xB0 | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            0x00,
            0x64,
            0xC1,
            0x00,
            0x00,
            0x07,
            0xD3,
            links.len() as u8,
        ];
        for link in links {
            bytes.push(0xF0 | ((link.len() >> 8) as u8 & 0x0F));
            bytes.push(link.len() as u8);
            bytes.extend_from_slice(link);
        }
        bytes.extend_from_slice(&[0xF0, 0x00]);
        let crc = broadcast_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        bytes
    }

    fn binary_locator_link(locator: &[u8]) -> Vec<u8> {
        let mut link = vec![0x1F, 0x01, 0x23, 0xA5];
        link.extend_from_slice(locator);
        link.extend_from_slice(&[0xC0, 0x00, 0x00]);
        link
    }

    /// ETSI TS 102 323 Table 31/Table 110: every locator branch, written as
    /// wire bytes and checked field by field plus a byte-identical re-serialize
    /// (never serialise-then-parse of the same code).
    #[test]
    fn locator_shapes_parse_from_wire_bytes() {
        let triplet = DvbLocatorService::Triplet {
            dvb_service_triplet_id: 0x2AB,
        };
        let full = DvbLocatorService::Full {
            transport_stream_id: 0x0001,
            original_network_id: 0x0002,
            service_id: 0x0003,
        };
        type LocatorCase<'a> = (
            &'a [u8],
            bool,
            &'a DvbLocatorService,
            &'a DvbLocatorIdentifier,
            Option<(u8, u8)>,
        );
        let cases: &[LocatorCase<'_>] = &[
            (
                &[0x0D, 0x56, 0xAB, 0x12, 0x34, 0x56, 0x78],
                false,
                &triplet,
                &DvbLocatorIdentifier::None,
                None,
            ),
            (
                &[0x2D, 0x56, 0xAB, 0x12, 0x34, 0x56, 0x78, 0xB1],
                true,
                &triplet,
                &DvbLocatorIdentifier::None,
                Some((0x05, 0x11)),
            ),
            (
                &[
                    0x5D, 0x57, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x12, 0x34, 0x56, 0x78, 0x00,
                    0x42,
                ],
                false,
                &full,
                &DvbLocatorIdentifier::EventId { event_id: 0x0042 },
                None,
            ),
            (
                &[
                    0x9D, 0x57, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x12, 0x34, 0x56, 0x78, 0x00,
                    0xAA,
                ],
                false,
                &full,
                &DvbLocatorIdentifier::TvaIdEit { tva_id: 0x00AA },
                None,
            ),
            (
                &[
                    0xDD, 0x57, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x12, 0x34, 0x56, 0x78, 0x00,
                    0xAA, 0x07,
                ],
                false,
                &full,
                &DvbLocatorIdentifier::TvaIdPes {
                    tva_id: 0x00AA,
                    component: 0x07,
                },
                None,
            ),
        ];

        for (locator, reliability, service, identifier, windows) in cases {
            let link = binary_locator_link(locator);
            let bytes = section_with_links(&[&link]);
            let section = RctSection::parse(&bytes).expect("parse");
            assert_eq!(section.links.len(), 1);
            let parsed = &section.links[0];
            assert_eq!(parsed.link_type, LinkType::BinaryLocator);
            assert_eq!(parsed.how_related, HowRelated::UserPrivate(0x30));
            assert_eq!(parsed.term_id, 0x123);
            assert_eq!(parsed.group_id, 0x0A);
            assert_eq!(parsed.precedence, 0x05);
            assert!(parsed.media_uri.is_none());
            assert!(parsed.items.is_empty());

            let loc = parsed.dvb_binary_locator.as_ref().expect("locator");
            assert_eq!(loc.scheduled_time_reliability, *reliability);
            assert_eq!(loc.start_date, 0x155);
            assert_eq!(loc.start_time, 0x1234);
            assert_eq!(loc.duration, 0x5678);
            assert_eq!(&loc.service, *service);
            assert_eq!(&loc.identifier, *identifier);
            assert_eq!(
                loc.windows,
                windows.map(|(early_start_window, late_end_window)| DvbLocatorWindows {
                    early_start_window,
                    late_end_window,
                })
            );

            let mut out = vec![0u8; section.serialized_len()];
            section.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes, "locator {identifier:?} re-serializes");
        }
    }

    #[test]
    fn two_links_with_two_items_parse_from_wire_bytes() {
        let first = [
            0x0C, 0x00, 0x10, 0x00, 0x04, b'u', b'r', b'i', b'!', 0xC2, b'e', b'n', b'g', 0x03,
            b'a', b'b', b'c', b'd', b'e', b'u', 0x00, 0x00, 0x00,
        ];
        let second = [
            0x1F, 0x01, 0x23, 0xA5, 0x0D, 0x56, 0xAB, 0x12, 0x34, 0x56, 0x78, 0xC2, b'e', b'n',
            b'g', 0x03, b'a', b'b', b'c', b'd', b'e', b'u', 0x00, 0x00, 0x00,
        ];

        let bytes = section_with_links(&[&first, &second]);
        let section = RctSection::parse(&bytes).expect("parse");
        assert_eq!(section.links.len(), 2);

        let one = &section.links[0];
        assert_eq!(one.link_type, LinkType::UriString);
        assert_eq!(one.media_uri, Some(&b"uri!"[..]));
        assert_eq!(one.items.len(), 2);
        assert_eq!(one.items[0].language_code.0, *b"eng");
        assert_eq!(one.items[0].promotional_text.raw(), b"abc");
        assert_eq!(one.items[1].language_code.0, *b"deu");
        assert_eq!(one.items[1].promotional_text.raw(), b"");

        let two = &section.links[1];
        assert_eq!(two.link_type, LinkType::BinaryLocator);
        assert_eq!(two.items.len(), 2);
        assert_eq!(two.items[0].promotional_text.raw(), b"abc");
        assert_eq!(two.items[1].language_code.0, *b"deu");

        let mut out = vec![0u8; section.serialized_len()];
        section.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn icon_descriptors_length_high_nibble_is_written() {
        let desc = vec![0xAAu8; 300];
        let link = LinkInfo::new(
            LinkType::Descriptor,
            HowRelated::Cs2004,
            0,
            0,
            0,
            None,
            None,
            Vec::new(),
            true,
            0,
            DescriptorLoop::new(&desc),
        );
        let rct = RctSection::new(
            false,
            0x0064,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            0x07D3,
            vec![link],
            DescriptorLoop::new(&[]),
        );
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();

        assert_eq!(buf[18], 0x81);
        assert_eq!(buf[19], 0x2C);

        let reparsed = RctSection::parse(&buf).expect("reparse");
        assert_eq!(reparsed.links[0].descriptors.raw().len(), 300);
        assert_eq!(rct, reparsed);
    }
}
