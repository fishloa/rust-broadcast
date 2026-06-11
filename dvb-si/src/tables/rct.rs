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
use crate::error::{Error, Result};
use crate::text::{DvbText, LangCode};
use dvb_common::{Parse, Serialize};

/// `table_id` for Related Content Table.
pub const TABLE_ID: u8 = 0x76;

/// Well-known PID on which RCT is carried: none (signalled via PMT).
pub const PID: u16 = 0x0000;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const POST_EXT_FIXED_LEN: usize = 3;
const LINK_ENTRY_HEADER_LEN: usize = 2;
const DESC_LOOP_LEN_FIELD: usize = 2;
const CRC_LEN: usize = 4;
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

const ITEM_RFU_MASK: u8 = 0xC0;
const ITEM_COUNT_MASK: u8 = 0x3F;

const LINK_INFO_HEADER_RFU: u8 = 0x0C;

const ICON_FLAG_MASK: u8 = 0x80;
const ICON_ID_MASK: u8 = 0x70;
const ICON_ID_SHIFT: u8 = 4;
const ICON_DESC_LEN_HI_MASK: u8 = 0x0F;

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
    3 + 1 + item.promotional_text.len()
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DvbBinaryLocator {
    /// `identifier_type` (2 bits, Table 32).
    pub identifier_type: u8,
    /// `scheduled_time_reliability`.
    pub scheduled_time_reliability: bool,
    /// `inline_service`.
    pub inline_service: bool,
    /// `start_date` (9 bits).
    pub start_date: u16,
    /// Service identification (conditional on `inline_service`).
    pub service: DvbLocatorService,
    /// `start_time` (16 bits).
    pub start_time: u16,
    /// `duration` (16 bits).
    pub duration: u16,
    /// Event identifier (conditional on `identifier_type`).
    pub identifier: DvbLocatorIdentifier,
    /// Time windows (present iff `identifier_type == 0 && scheduled_time_reliability == 1`).
    pub windows: Option<DvbLocatorWindows>,
}

fn locator_serialized_len(loc: &DvbBinaryLocator) -> usize {
    let mut len = 2;
    len += match &loc.service {
        DvbLocatorService::Triplet { .. } => 1,
        DvbLocatorService::Full { .. } => 6,
    };
    len += 4;
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
pub struct LinkInfo<'a> {
    /// `link_type` (4 bits, Table 111).
    pub link_type: u8,
    /// `how_related_classification_scheme_id` (6 bits).
    pub how_related: u8,
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
}

fn link_info_serialized_len(li: &LinkInfo) -> usize {
    let mut len = 4;
    len += li.media_uri.map_or(0, |u| 1 + u.len());
    len += li
        .dvb_binary_locator
        .as_ref()
        .map_or(0, locator_serialized_len);
    len += 1;
    len += li.items.iter().map(link_item_serialized_len).sum::<usize>();
    len += 2;
    len += li.descriptors.len();
    len
}

/// Related Content Table (ETSI TS 102 323 v1.4.1 §10.4.2, Table 109).
///
/// The link_info loop is unfolded into typed [`LinkInfo`] entries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct RctSection<'a> {
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
}

fn parse_locator(data: &[u8]) -> Result<(DvbBinaryLocator, usize)> {
    if data.len() < 2 {
        return Err(Error::BufferTooShort {
            need: 2,
            have: data.len(),
            what: "RctSection dvb_binary_locator header",
        });
    }
    let b0 = data[0];
    let b1 = data[1];
    let identifier_type = (b0 & LOCATOR_ID_TYPE_MASK) >> LOCATOR_ID_TYPE_SHIFT;
    let scheduled_time_reliability = (b0 & LOCATOR_RELIABILITY_MASK) != 0;
    let inline_service = (b0 & LOCATOR_INLINE_MASK) != 0;
    let start_date = ((b0 & LOCATOR_START_DATE_HI_MASK) as u16) << LOCATOR_START_DATE_HI_SHIFT
        | ((b1 >> 2) as u16 & 0x3F);

    let mut pos = 2;
    let service = if inline_service {
        if pos + 6 > data.len() {
            return Err(Error::BufferTooShort {
                need: pos + 6,
                have: data.len(),
                what: "RctSection dvb_binary_locator full triplet",
            });
        }
        let tsid = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let onid = u16::from_be_bytes([data[pos + 2], data[pos + 3]]);
        let sid = u16::from_be_bytes([data[pos + 4], data[pos + 5]]);
        pos += 6;
        DvbLocatorService::Full {
            transport_stream_id: tsid,
            original_network_id: onid,
            service_id: sid,
        }
    } else {
        if data.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: data.len(),
                what: "RctSection dvb_binary_locator triplet",
            });
        }
        let triplet_id = ((b1 as u16 & 0x03) << 8) | data[2] as u16;
        pos = 3;
        DvbLocatorService::Triplet {
            dvb_service_triplet_id: triplet_id,
        }
    };

    if pos + 4 > data.len() {
        return Err(Error::BufferTooShort {
            need: pos + 4,
            have: data.len(),
            what: "RctSection dvb_binary_locator start_time/duration",
        });
    }
    let start_time = u16::from_be_bytes([data[pos], data[pos + 1]]);
    let duration = u16::from_be_bytes([data[pos + 2], data[pos + 3]]);
    pos += 4;

    let identifier = match identifier_type {
        0x00 => DvbLocatorIdentifier::None,
        0x01 => {
            if pos + 2 > data.len() {
                return Err(Error::BufferTooShort {
                    need: pos + 2,
                    have: data.len(),
                    what: "RctSection dvb_binary_locator event_id",
                });
            }
            let event_id = u16::from_be_bytes([data[pos], data[pos + 1]]);
            pos += 2;
            DvbLocatorIdentifier::EventId { event_id }
        }
        0x02 => {
            if pos + 2 > data.len() {
                return Err(Error::BufferTooShort {
                    need: pos + 2,
                    have: data.len(),
                    what: "RctSection dvb_binary_locator TVA_id (EIT)",
                });
            }
            let tva_id = u16::from_be_bytes([data[pos], data[pos + 1]]);
            pos += 2;
            DvbLocatorIdentifier::TvaIdEit { tva_id }
        }
        0x03 => {
            if pos + 3 > data.len() {
                return Err(Error::BufferTooShort {
                    need: pos + 3,
                    have: data.len(),
                    what: "RctSection dvb_binary_locator TVA_id (PES)",
                });
            }
            let tva_id = u16::from_be_bytes([data[pos], data[pos + 1]]);
            let component = data[pos + 2];
            pos += 3;
            DvbLocatorIdentifier::TvaIdPes { tva_id, component }
        }
        _ => DvbLocatorIdentifier::None,
    };

    let windows = if identifier_type == 0x00 && scheduled_time_reliability {
        if pos >= data.len() {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: data.len(),
                what: "RctSection dvb_binary_locator windows",
            });
        }
        let wb = data[pos];
        pos += 1;
        Some(DvbLocatorWindows {
            early_start_window: (wb >> 5) & 0x07,
            late_end_window: wb & 0x1F,
        })
    } else {
        None
    };

    Ok((
        DvbBinaryLocator {
            identifier_type,
            scheduled_time_reliability,
            inline_service,
            start_date,
            service,
            start_time,
            duration,
            identifier,
            windows,
        },
        pos,
    ))
}

fn serialize_locator(loc: &DvbBinaryLocator, buf: &mut [u8]) -> usize {
    let b0 = ((loc.identifier_type & 0x03) << LOCATOR_ID_TYPE_SHIFT)
        | (u8::from(loc.scheduled_time_reliability) << 5)
        | (u8::from(loc.inline_service) << 4)
        | LOCATOR_RESERVED_BITS
        | ((loc.start_date >> LOCATOR_START_DATE_HI_SHIFT) as u8 & LOCATOR_START_DATE_HI_MASK);
    buf[0] = b0;

    let mut pos: usize;
    match &loc.service {
        DvbLocatorService::Triplet {
            dvb_service_triplet_id,
        } => {
            let sd_lo = (loc.start_date & 0x3F) as u8;
            buf[1] = (sd_lo << 2) | ((dvb_service_triplet_id >> 8) as u8 & 0x03);
            buf[2] = (dvb_service_triplet_id & 0xFF) as u8;
            pos = 3;
        }
        DvbLocatorService::Full {
            transport_stream_id,
            original_network_id,
            service_id,
        } => {
            let sd_lo = (loc.start_date & 0x3F) as u8;
            buf[1] = (sd_lo << 2) | 0x03;
            buf[2..4].copy_from_slice(&transport_stream_id.to_be_bytes());
            buf[4..6].copy_from_slice(&original_network_id.to_be_bytes());
            buf[6..8].copy_from_slice(&service_id.to_be_bytes());
            pos = 8;
        }
    }
    buf[pos..pos + 2].copy_from_slice(&loc.start_time.to_be_bytes());
    buf[pos + 2..pos + 4].copy_from_slice(&loc.duration.to_be_bytes());
    pos += 4;

    match &loc.identifier {
        DvbLocatorIdentifier::None => {}
        DvbLocatorIdentifier::EventId { event_id } => {
            buf[pos..pos + 2].copy_from_slice(&event_id.to_be_bytes());
            pos += 2;
        }
        DvbLocatorIdentifier::TvaIdEit { tva_id } => {
            buf[pos..pos + 2].copy_from_slice(&tva_id.to_be_bytes());
            pos += 2;
        }
        DvbLocatorIdentifier::TvaIdPes { tva_id, component } => {
            buf[pos..pos + 2].copy_from_slice(&tva_id.to_be_bytes());
            buf[pos + 2] = *component;
            pos += 3;
        }
    }
    if let Some(w) = &loc.windows {
        buf[pos] = ((w.early_start_window & 0x07) << 5) | (w.late_end_window & 0x1F);
        pos += 1;
    }
    pos
}

fn parse_link_info(data: &[u8], link_info_length: usize) -> Result<LinkInfo<'_>> {
    if data.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: data.len(),
            what: "RctSection link_info header",
        });
    }
    let link_type = (data[0] & LINK_TYPE_MASK) >> LINK_TYPE_SHIFT;
    let how_related = (data[0] & HOW_RELATED_HI_MASK) << 4 | (data[1] >> 4) & 0x0F;
    let term_id = ((data[1] as u16 & TERM_ID_HI_MASK as u16) << TERM_ID_HI_SHIFT) | data[2] as u16;
    let group_id = (data[3] & GROUP_ID_MASK) >> GROUP_ID_SHIFT;
    let precedence = data[3] & PRECEDENCE_MASK;

    let mut pos = 4;
    let end = link_info_length;

    let media_uri = if link_type == 0x00 || link_type == 0x02 {
        if pos >= end {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: end,
                what: "RctSection media_uri_length",
            });
        }
        let uri_len = data[pos] as usize;
        pos += 1;
        if pos + uri_len > end {
            return Err(Error::BufferTooShort {
                need: pos + uri_len,
                have: end,
                what: "RctSection media_uri",
            });
        }
        let uri = &data[pos..pos + uri_len];
        pos += uri_len;
        Some(uri)
    } else {
        None
    };

    let dvb_binary_locator = if link_type == 0x01 || link_type == 0x02 {
        if pos >= end {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: end,
                what: "RctSection dvb_binary_locator",
            });
        }
        let (loc, consumed) = parse_locator(&data[pos..end])?;
        pos += consumed;
        Some(loc)
    } else {
        None
    };

    if pos >= end {
        return Err(Error::BufferTooShort {
            need: pos + 1,
            have: end,
            what: "RctSection number_items",
        });
    }
    let ni_byte = data[pos];
    let number_items = (ni_byte & ITEM_COUNT_MASK) as usize;
    pos += 1;

    let mut items = Vec::with_capacity(number_items);
    for _ in 0..number_items {
        if pos + 4 > end {
            return Err(Error::BufferTooShort {
                need: pos + 4,
                have: end,
                what: "RctSection link_item",
            });
        }
        let language_code = LangCode([data[pos], data[pos + 1], data[pos + 2]]);
        let text_len = data[pos + 3] as usize;
        pos += 4;
        if pos + text_len > end {
            return Err(Error::BufferTooShort {
                need: pos + text_len,
                have: end,
                what: "RctSection promotional_text",
            });
        }
        let promotional_text = DvbText::new(&data[pos..pos + text_len]);
        pos += text_len;
        items.push(LinkItem {
            language_code,
            promotional_text,
        });
    }

    if pos + 2 > end {
        return Err(Error::BufferTooShort {
            need: pos + 2,
            have: end,
            what: "RctSection icon/desc",
        });
    }
    let default_icon_flag = (data[pos] & ICON_FLAG_MASK) != 0;
    let icon_id = (data[pos] & ICON_ID_MASK) >> ICON_ID_SHIFT;
    let desc_len = (((data[pos] & ICON_DESC_LEN_HI_MASK) as usize) << 8) | data[pos + 1] as usize;
    pos += 2;
    let desc_start = pos;
    let desc_end = desc_start + desc_len;
    if desc_end > end {
        return Err(Error::SectionLengthOverflow {
            declared: desc_len,
            available: end.saturating_sub(desc_start),
        });
    }
    let descriptors = DescriptorLoop::new(&data[desc_start..desc_end]);

    Ok(LinkInfo {
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
    })
}

fn serialize_link_info(li: &LinkInfo, buf: &mut [u8]) -> usize {
    buf[0] = ((li.link_type & 0x0F) << LINK_TYPE_SHIFT)
        | LINK_INFO_HEADER_RFU
        | ((li.how_related >> 4) & HOW_RELATED_HI_MASK);
    buf[1] =
        ((li.how_related & 0x0F) << 4) | ((li.term_id >> TERM_ID_HI_SHIFT) as u8 & TERM_ID_HI_MASK);
    buf[2] = li.term_id as u8;
    buf[3] = ((li.group_id & 0x0F) << GROUP_ID_SHIFT) | (li.precedence & PRECEDENCE_MASK);

    let mut pos = 4;
    if let Some(uri) = li.media_uri {
        buf[pos] = uri.len() as u8;
        pos += 1;
        buf[pos..pos + uri.len()].copy_from_slice(uri);
        pos += uri.len();
    }
    if let Some(ref loc) = li.dvb_binary_locator {
        let n = serialize_locator(loc, &mut buf[pos..]);
        pos += n;
    }
    buf[pos] = ITEM_RFU_MASK | (li.items.len() as u8 & ITEM_COUNT_MASK);
    pos += 1;
    for item in &li.items {
        buf[pos..pos + 3].copy_from_slice(&item.language_code.0);
        buf[pos + 3] = item.promotional_text.len() as u8;
        pos += 4;
        buf[pos..pos + item.promotional_text.len()].copy_from_slice(item.promotional_text.raw());
        pos += item.promotional_text.len();
    }
    let dll = li.descriptors.len() as u16;
    buf[pos] = u8::from(li.default_icon_flag) << 7
        | ((li.icon_id & 0x07) << ICON_ID_SHIFT)
        | ((dll >> 8) as u8 & ICON_DESC_LEN_HI_MASK);
    buf[pos + 1] = (dll & 0xFF) as u8;
    pos += 2;
    buf[pos..pos + li.descriptors.len()].copy_from_slice(li.descriptors.raw());
    pos += li.descriptors.len();
    pos
}

impl<'a> Parse<'a> for RctSection<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "RctSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "RctSection",
                expected: &[TABLE_ID],
            });
        }

        let table_id_extension_flag = (bytes[1] & 0x40) != 0;
        let section_length = (((bytes[1] & 0x0F) as u16) << 8) | bytes[2] as u16;
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            section_length as usize,
            MIN_SECTION_LEN,
        )?;

        let service_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let year_offset = u16::from_be_bytes([bytes[8], bytes[9]]);
        let link_count = bytes[10];

        let payload_end = total - CRC_LEN;
        let mut pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN;

        let mut links = Vec::with_capacity(link_count as usize);
        for _ in 0..link_count {
            if pos + LINK_ENTRY_HEADER_LEN > payload_end {
                return Err(Error::BufferTooShort {
                    need: pos + LINK_ENTRY_HEADER_LEN,
                    have: payload_end,
                    what: "RctSection link_entry header",
                });
            }
            let link_info_length = (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
            let link_data_start = pos + LINK_ENTRY_HEADER_LEN;
            let link_data_end = link_data_start + link_info_length;
            if link_data_end > payload_end {
                return Err(Error::SectionLengthOverflow {
                    declared: link_info_length,
                    available: payload_end.saturating_sub(link_data_start),
                });
            }
            let link_info =
                parse_link_info(&bytes[link_data_start..link_data_end], link_info_length)?;
            links.push(link_info);
            pos = link_data_end;
        }

        if pos + DESC_LOOP_LEN_FIELD > payload_end {
            return Err(Error::BufferTooShort {
                need: pos + DESC_LOOP_LEN_FIELD,
                have: payload_end,
                what: "RctSection descriptor_loop_length field",
            });
        }
        let descriptor_loop_length =
            (((bytes[pos] & 0x0F) as usize) << 8) | bytes[pos + 1] as usize;
        let desc_start = pos + DESC_LOOP_LEN_FIELD;
        let desc_end = desc_start + descriptor_loop_length;
        if desc_end > payload_end {
            return Err(Error::SectionLengthOverflow {
                declared: descriptor_loop_length,
                available: payload_end.saturating_sub(desc_start),
            });
        }
        let descriptors = DescriptorLoop::new(&bytes[desc_start..desc_end]);

        Ok(RctSection {
            table_id_extension_flag,
            service_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            year_offset,
            links,
            descriptors,
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
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let section_length = (len - MIN_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        let tief_bit: u8 = if self.table_id_extension_flag {
            0x40
        } else {
            0x00
        };
        buf[1] = super::SECTION_B1_SSI
            | tief_bit
            | super::SECTION_B1_RESERVED_HI
            | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        buf[3..5].copy_from_slice(&self.service_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8..10].copy_from_slice(&self.year_offset.to_be_bytes());
        if self.links.len() > u8::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.links.len(),
                available: u8::MAX as usize,
            });
        }
        buf[10] = self.links.len() as u8;

        let mut pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXT_FIXED_LEN;
        for li in &self.links {
            let li_body_len = link_info_serialized_len(li) as u16;
            buf[pos] = 0xF0 | ((li_body_len >> 8) as u8 & 0x0F);
            buf[pos + 1] = (li_body_len & 0xFF) as u8;
            pos += LINK_ENTRY_HEADER_LEN;
            pos += serialize_link_info(li, &mut buf[pos..]);
        }

        let dll = self.descriptors.len() as u16;
        buf[pos] = 0xF0 | ((dll >> 8) as u8 & 0x0F);
        buf[pos + 1] = (dll & 0xFF) as u8;
        pos += DESC_LOOP_LEN_FIELD;
        buf[pos..pos + self.descriptors.len()].copy_from_slice(self.descriptors.raw());
        pos += self.descriptors.len();

        let crc = dvb_common::crc32_mpeg2::compute(&buf[..pos]);
        buf[pos..pos + CRC_LEN].copy_from_slice(&crc.to_be_bytes());
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

    #[test]
    fn parse_no_links_no_descriptors() {
        let rct = RctSection {
            table_id_extension_flag: false,
            service_id: 0x0064,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            year_offset: 0x07D3,
            links: Vec::new(),
            descriptors: DescriptorLoop::new(&[]),
        };
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let p = RctSection::parse(&buf).unwrap();
        assert!(!p.table_id_extension_flag);
        assert_eq!(p.service_id, 0x0064);
        assert_eq!(p.year_offset, 0x07D3);
        assert!(p.links.is_empty());
    }

    #[test]
    fn parse_one_link_uri_only() {
        let li = LinkInfo {
            link_type: 0x00,
            how_related: 0x01,
            term_id: 0x123,
            group_id: 0x5,
            precedence: 0x9,
            media_uri: Some(b"http://example.com"),
            dvb_binary_locator: None,
            items: Vec::new(),
            default_icon_flag: false,
            icon_id: 0,
            descriptors: DescriptorLoop::new(&[]),
        };
        let rct = RctSection {
            table_id_extension_flag: false,
            service_id: 0x1234,
            version_number: 7,
            current_next_indicator: true,
            section_number: 1,
            last_section_number: 3,
            year_offset: 2003,
            links: vec![li],
            descriptors: DescriptorLoop::new(&[]),
        };
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let p = RctSection::parse(&buf).unwrap();
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].link_type, 0x00);
        assert_eq!(p.links[0].how_related, 0x01);
        assert_eq!(p.links[0].term_id, 0x123);
        assert_eq!(p.links[0].group_id, 0x5);
        assert_eq!(p.links[0].precedence, 0x9);
        assert_eq!(p.links[0].media_uri.unwrap(), b"http://example.com");
    }

    #[test]
    fn parse_one_link_with_locator_and_items() {
        let loc = DvbBinaryLocator {
            identifier_type: 0x01,
            scheduled_time_reliability: false,
            inline_service: true,
            start_date: 0x0FF,
            service: DvbLocatorService::Full {
                transport_stream_id: 0x1000,
                original_network_id: 0x2000,
                service_id: 0x3000,
            },
            start_time: 0x8000,
            duration: 0x4000,
            identifier: DvbLocatorIdentifier::EventId { event_id: 0xBEEF },
            windows: None,
        };
        let li = LinkInfo {
            link_type: 0x01,
            how_related: 0x02,
            term_id: 0x456,
            group_id: 0x1,
            precedence: 0x2,
            media_uri: None,
            dvb_binary_locator: Some(loc),
            items: vec![LinkItem {
                language_code: LangCode(*b"eng"),
                promotional_text: DvbText::new(b"Promo"),
            }],
            default_icon_flag: true,
            icon_id: 3,
            descriptors: DescriptorLoop::new(&[0x80, 0x00]),
        };
        let rct = RctSection {
            table_id_extension_flag: true,
            service_id: 0xABCD,
            version_number: 15,
            current_next_indicator: false,
            section_number: 2,
            last_section_number: 5,
            year_offset: 2024,
            links: vec![li],
            descriptors: DescriptorLoop::new(&[]),
        };
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let mut buf2 = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
        let p = RctSection::parse(&buf).unwrap();
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].link_type, 0x01);
        let l = p.links[0].dvb_binary_locator.as_ref().unwrap();
        assert_eq!(l.identifier_type, 0x01);
        assert!(l.inline_service);
        assert_eq!(l.start_date, 0x0FF);
        assert_eq!(p.links[0].items.len(), 1);
        assert_eq!(p.links[0].items[0].language_code, LangCode(*b"eng"));
        assert!(p.links[0].default_icon_flag);
        assert_eq!(p.links[0].icon_id, 3);
    }

    #[test]
    fn byte_exact_round_trip_simple() {
        let li = LinkInfo {
            link_type: 0x00,
            how_related: 0x00,
            term_id: 0,
            group_id: 0,
            precedence: 0,
            media_uri: Some(b"uri"),
            dvb_binary_locator: None,
            items: Vec::new(),
            default_icon_flag: false,
            icon_id: 0,
            descriptors: DescriptorLoop::new(&[]),
        };
        let rct = RctSection {
            table_id_extension_flag: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            year_offset: 0x07D3,
            links: vec![li],
            descriptors: DescriptorLoop::new(&[]),
        };
        let mut buf = vec![0u8; rct.serialized_len()];
        rct.serialize_into(&mut buf).unwrap();
        let parsed = RctSection::parse(&buf).unwrap();
        let mut buf2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let rct = RctSection {
            table_id_extension_flag: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            year_offset: 2024,
            links: Vec::new(),
            descriptors: DescriptorLoop::new(&[]),
        };
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
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_rejects_output_buffer_too_small() {
        let rct = RctSection {
            table_id_extension_flag: false,
            service_id: 0x0001,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            year_offset: 0,
            links: Vec::new(),
            descriptors: DescriptorLoop::new(&[]),
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            rct.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
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

    #[test]
    fn parse_handwritten_rct_no_links() {
        let mut bytes: Vec<u8> = vec![
            0x76, 0x80, 0x0E, 0x00, 0x64, 0xC7, 0x00, 0x00, 0x07, 0xD3, 0x00, 0xF0, 0x00,
        ];
        let crc = dvb_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let rct = RctSection::parse(&bytes).unwrap();
        assert_eq!(rct.service_id, 0x0064);
        assert_eq!(rct.year_offset, 0x07D3);
        assert!(rct.links.is_empty());
    }
}
