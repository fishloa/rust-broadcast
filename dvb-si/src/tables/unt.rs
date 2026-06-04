//! Update Notification Table — ETSI TS 102 006 v1.4.1 §9.4.
//!
//! The UNT delivers software-update instructions for DVB receivers. It is
//! carried on a PID that is **signalled** — there is no fixed PID. The PMT
//! ES_info loop for the update data carousel contains a
//! `data_broadcast_id_descriptor` (tag 0x66) with `data_broadcast_id = 0x000A`;
//! the associated elementary PID is the one carrying UNT sections.
//!
//! Structure (long-form section):
//! - 3-byte section header (table_id + section_length)
//! - action_type (8 bit)
//! - OUI_hash   (8 bit)
//! - reserved(2) | version_number(5) | current_next_indicator(1)
//! - section_number (8 bit)
//! - last_section_number (8 bit)
//! - OUI (24 bit, big-endian)
//! - processing_order (8 bit)
//! - common_descriptor_loop() — reserved(4) + length(12) + raw descriptors
//! - platform_loop — zero or more platform entries, each containing a
//!   `compatibilityDescriptor()` (ISO/IEC 13818-6 groupInfo form, NOT a
//!   standard tag/length SI descriptor) followed by
//!   `platform_loop_length(16)` then target and operational descriptor loops
//! - CRC_32 (32 bit)

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// `table_id` for the Update Notification Table.
pub const TABLE_ID: u8 = 0x4B;

/// Well-known PID for UNT: **none** — the UNT has no fixed PID.
///
/// The carrying PID is signalled via a `data_broadcast_id_descriptor`
/// (`data_broadcast_id = 0x000A`) in the PMT ES_info loop. This constant is
/// set to `0x0000` (the value `Table::PID` returns for tables with no fixed
/// PID) so that callers can detect the special case.
pub const PID: u16 = 0x0000;

/// Minimum byte length of a valid UNT section (3-byte header + 9-byte
/// fixed body + 2-byte common_descriptor_loop_length field + 4-byte CRC).
const MIN_SECTION_LEN: usize = HEADER_LEN + FIXED_BODY_LEN + COMMON_DESC_LEN_FIELD + CRC_LEN;

/// 3-byte outer header: table_id(8) + section_syntax_indicator(1) +
/// reserved_future_use(1) + reserved(2) + section_length(12).
const HEADER_LEN: usize = 3;

/// Fixed portion after the header and before the common_descriptor_loop:
/// action_type(8) + OUI_hash(8) + flags_byte(8) + section_number(8) +
/// last_section_number(8) + OUI(24) + processing_order(8) = 9 bytes.
const FIXED_BODY_LEN: usize = 9;

/// Width of the `reserved(4) | common_descriptor_loop_length(12)` length
/// field, in bytes.
const COMMON_DESC_LEN_FIELD: usize = 2;

/// CRC_32 trailer, 4 bytes.
const CRC_LEN: usize = 4;

/// Byte offset of `action_type` inside the raw section buffer.
const OFFSET_ACTION_TYPE: usize = HEADER_LEN;

/// Byte offset of `OUI_hash` inside the raw section buffer.
const OFFSET_OUI_HASH: usize = HEADER_LEN + 1;

/// Byte offset of the flags byte (reserved(2) | version_number(5) |
/// current_next_indicator(1)) inside the raw section buffer.
const OFFSET_FLAGS: usize = HEADER_LEN + 2;

/// Byte offset of `section_number`.
const OFFSET_SECTION_NUMBER: usize = HEADER_LEN + 3;

/// Byte offset of `last_section_number`.
const OFFSET_LAST_SECTION_NUMBER: usize = HEADER_LEN + 4;

/// Byte offset of the first byte of the 3-byte OUI.
const OFFSET_OUI: usize = HEADER_LEN + 5;

/// Byte offset of `processing_order`.
const OFFSET_PROCESSING_ORDER: usize = HEADER_LEN + 8;

/// Byte offset of the `reserved(4) | common_descriptor_loop_length(12)` field.
const OFFSET_COMMON_DESC_LEN: usize = HEADER_LEN + FIXED_BODY_LEN;

/// Mask to extract the 5-bit version_number from the flags byte.
const VERSION_NUMBER_MASK: u8 = 0x3E;

/// Bit shift for version_number inside the flags byte.
const VERSION_NUMBER_SHIFT: u8 = 1;

/// Mask for current_next_indicator in the flags byte.
const CURRENT_NEXT_MASK: u8 = 0x01;

/// Mask for the high-4 of a 12-bit length field in its first byte.
const LENGTH_HIGH_NIBBLE_MASK: u8 = 0x0F;

/// Serialize flag byte: reserved(2) = 0b11, rest provided by caller.
const FLAGS_RESERVED_BITS: u8 = 0xC0;

/// Syntax indicator + reserved in the section_length byte: long-form
/// (section_syntax_indicator=1, reserved_future_use=1, reserved=11).
const SECTION_LEN_BYTE1_FLAGS: u8 = 0xB0;

/// Reserved nibble for `common_descriptor_loop_length` and
/// `platform_loop_length` high-nibble: 0xF0 (4 reserved bits set to 1).
const RESERVED_NIBBLE: u8 = 0xF0;

/// Update Notification Table (UNT).
///
/// Typed fields cover the fixed header (action_type through processing_order).
/// Variable-length regions are kept as raw `&[u8]` borrows to avoid pulling in
/// the full ISO/IEC 13818-6 `compatibilityDescriptor` parser:
///
/// - `common_descriptors` — the body of the `common_descriptor_loop()`, i.e.
///   the bytes AFTER the 12-bit length field (standard SI descriptor format).
/// - `platform_loop` — the entire remaining payload between the
///   `common_descriptor_loop` and the CRC.  This region contains zero or more
///   platform entries, each starting with a `compatibilityDescriptor()` (an
///   ISO/IEC 13818-6 groupInfo block — **not** a standard tag/length SI
///   descriptor) followed by a 16-bit `platform_loop_length` and the
///   corresponding target / operational descriptor loops. Callers that need to
///   walk individual platform entries must parse this field manually.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Unt<'a> {
    /// Action type (Table 12 of ETSI TS 102 006):
    /// 0x01 = System Software Update, 0x80–0xFF = user defined.
    pub action_type: u8,

    /// OUI hash: `OUI[23:16] ^ OUI[15:8] ^ OUI[7:0]` (XOR of the three OUI
    /// bytes, used as a quick equality check before comparing the full OUI).
    pub oui_hash: u8,

    /// 5-bit version_number of this sub-table.
    pub version_number: u8,

    /// `current_next_indicator`: `true` means this section is currently
    /// applicable; `false` means it applies starting from the next version.
    pub current_next_indicator: bool,

    /// Index of this section within the sub-table.
    pub section_number: u8,

    /// Index of the last section in the sub-table.
    pub last_section_number: u8,

    /// 24-bit IEEE Organizationally Unique Identifier.
    ///
    /// Stored in the low 24 bits of a `u32` (high byte is always zero).
    /// The DVB-reserved generic OUI `0x00015A` means the receiver should
    /// analyse the UNT payload to determine applicability.
    pub oui: u32,

    /// Processing order (Table 13): 0x00 = first action, 0x01–0xFE =
    /// subsequent (ascending), 0xFF = no ordering implied.
    pub processing_order: u8,

    /// Raw body of `common_descriptor_loop()` — the bytes AFTER the
    /// 12-bit length field.  Contains zero or more standard SI descriptors
    /// (tag + length + payload), as defined in §9.4.2.1.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub common_descriptors: &'a [u8],

    /// Raw bytes of the entire platform loop region — everything after
    /// `common_descriptor_loop()` up to (but not including) the CRC_32.
    ///
    /// Each platform entry starts with a `compatibilityDescriptor()` block
    /// (ISO/IEC 13818-6 §11 groupInfo form — a 2-byte length prefix +
    /// descriptor list, **not** a standard SI tag/length descriptor), followed
    /// by a 16-bit `platform_loop_length` then zero or more platform entries
    /// each containing target and operational descriptor loops.
    ///
    /// To walk platform entries, parse this field according to
    /// ETSI TS 102 006 §9.4.2.2–9.4.2.4.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub platform_loop: &'a [u8],
}

impl<'a> Parse<'a> for Unt<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // ── 1. Minimum-length guard ──────────────────────────────────────────
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "Unt",
            });
        }

        // ── 2. table_id check ────────────────────────────────────────────────
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Unt",
                expected: &[TABLE_ID],
            });
        }

        // ── 3. section_length → total byte count ─────────────────────────────
        let section_length =
            (((bytes[1] & LENGTH_HIGH_NIBBLE_MASK) as usize) << 8) | bytes[2] as usize;
        let total = HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - HEADER_LEN,
            });
        }

        // ── 4. Fixed header fields ────────────────────────────────────────────
        let action_type = bytes[OFFSET_ACTION_TYPE];
        let oui_hash = bytes[OFFSET_OUI_HASH];
        let flags_byte = bytes[OFFSET_FLAGS];
        let version_number = (flags_byte & VERSION_NUMBER_MASK) >> VERSION_NUMBER_SHIFT;
        let current_next_indicator = (flags_byte & CURRENT_NEXT_MASK) != 0;
        let section_number = bytes[OFFSET_SECTION_NUMBER];
        let last_section_number = bytes[OFFSET_LAST_SECTION_NUMBER];
        // OUI is a 24-bit big-endian value packed into bytes [OFFSET_OUI..OFFSET_OUI+3].
        let oui = ((bytes[OFFSET_OUI] as u32) << 16)
            | ((bytes[OFFSET_OUI + 1] as u32) << 8)
            | (bytes[OFFSET_OUI + 2] as u32);
        let processing_order = bytes[OFFSET_PROCESSING_ORDER];

        // ── 5. common_descriptor_loop ────────────────────────────────────────
        // reserved(4) | common_descriptor_loop_length(12)
        let cdl = (((bytes[OFFSET_COMMON_DESC_LEN] & LENGTH_HIGH_NIBBLE_MASK) as usize) << 8)
            | bytes[OFFSET_COMMON_DESC_LEN + 1] as usize;
        let common_desc_start = OFFSET_COMMON_DESC_LEN + COMMON_DESC_LEN_FIELD;
        let common_desc_end = common_desc_start + cdl;
        if common_desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: cdl,
                available: (total - CRC_LEN).saturating_sub(common_desc_start),
            });
        }
        let common_descriptors = &bytes[common_desc_start..common_desc_end];

        // ── 6. platform_loop ─────────────────────────────────────────────────
        let platform_loop_start = common_desc_end;
        let platform_loop_end = total - CRC_LEN;
        let platform_loop = &bytes[platform_loop_start..platform_loop_end];

        Ok(Unt {
            action_type,
            oui_hash,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            oui,
            processing_order,
            common_descriptors,
            platform_loop,
        })
    }
}

impl Serialize for Unt<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FIXED_BODY_LEN
            + COMMON_DESC_LEN_FIELD
            + self.common_descriptors.len()
            + self.platform_loop.len()
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

        // ── Header ───────────────────────────────────────────────────────────
        let section_length = (len - HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        buf[1] = SECTION_LEN_BYTE1_FLAGS | ((section_length >> 8) as u8 & LENGTH_HIGH_NIBBLE_MASK);
        buf[2] = (section_length & 0xFF) as u8;

        // ── Fixed body ───────────────────────────────────────────────────────
        buf[OFFSET_ACTION_TYPE] = self.action_type;
        buf[OFFSET_OUI_HASH] = self.oui_hash;
        buf[OFFSET_FLAGS] = FLAGS_RESERVED_BITS
            | ((self.version_number & 0x1F) << VERSION_NUMBER_SHIFT)
            | u8::from(self.current_next_indicator);
        buf[OFFSET_SECTION_NUMBER] = self.section_number;
        buf[OFFSET_LAST_SECTION_NUMBER] = self.last_section_number;
        // OUI — 24 bits, big-endian.
        buf[OFFSET_OUI] = ((self.oui >> 16) & 0xFF) as u8;
        buf[OFFSET_OUI + 1] = ((self.oui >> 8) & 0xFF) as u8;
        buf[OFFSET_OUI + 2] = (self.oui & 0xFF) as u8;
        buf[OFFSET_PROCESSING_ORDER] = self.processing_order;

        // ── common_descriptor_loop length field ──────────────────────────────
        let cdl = self.common_descriptors.len() as u16;
        buf[OFFSET_COMMON_DESC_LEN] =
            RESERVED_NIBBLE | ((cdl >> 8) as u8 & LENGTH_HIGH_NIBBLE_MASK);
        buf[OFFSET_COMMON_DESC_LEN + 1] = (cdl & 0xFF) as u8;

        // ── common_descriptors body ──────────────────────────────────────────
        let common_start = OFFSET_COMMON_DESC_LEN + COMMON_DESC_LEN_FIELD;
        let common_end = common_start + self.common_descriptors.len();
        buf[common_start..common_end].copy_from_slice(self.common_descriptors);

        // ── platform_loop ────────────────────────────────────────────────────
        let plat_end = common_end + self.platform_loop.len();
        buf[common_end..plat_end].copy_from_slice(self.platform_loop);

        // ── CRC_32 — compute over everything up to (but not including) the CRC slot.
        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

impl<'a> Table<'a> for Unt<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but syntactically valid UNT section byte buffer.
    ///
    /// `common_descs`  — raw bytes to place inside the common_descriptor_loop body.
    /// `platform_loop` — raw bytes for the entire platform_loop region.
    #[allow(clippy::too_many_arguments)]
    fn build_unt(
        action_type: u8,
        oui_hash: u8,
        version_number: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        oui: u32,
        processing_order: u8,
        common_descs: &[u8],
        platform_loop: &[u8],
    ) -> Vec<u8> {
        // section_length covers everything after the 3-byte outer header up to
        // and including the CRC_32.
        let section_length = FIXED_BODY_LEN
            + COMMON_DESC_LEN_FIELD
            + common_descs.len()
            + platform_loop.len()
            + CRC_LEN;

        let mut v: Vec<u8> = Vec::with_capacity(HEADER_LEN + section_length);

        // Header.
        v.push(TABLE_ID);
        v.push(SECTION_LEN_BYTE1_FLAGS | ((section_length >> 8) as u8 & LENGTH_HIGH_NIBBLE_MASK));
        v.push((section_length & 0xFF) as u8);

        // Fixed body.
        v.push(action_type);
        v.push(oui_hash);
        let flags = FLAGS_RESERVED_BITS
            | ((version_number & 0x1F) << VERSION_NUMBER_SHIFT)
            | u8::from(current_next_indicator);
        v.push(flags);
        v.push(section_number);
        v.push(last_section_number);
        v.push(((oui >> 16) & 0xFF) as u8);
        v.push(((oui >> 8) & 0xFF) as u8);
        v.push((oui & 0xFF) as u8);
        v.push(processing_order);

        // common_descriptor_loop length + body.
        let cdl = common_descs.len() as u16;
        v.push(RESERVED_NIBBLE | ((cdl >> 8) as u8 & LENGTH_HIGH_NIBBLE_MASK));
        v.push((cdl & 0xFF) as u8);
        v.extend_from_slice(common_descs);

        // Platform loop.
        v.extend_from_slice(platform_loop);

        // CRC_32 placeholder.
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

        v
    }

    /// Verify all typed fields are parsed correctly on a happy-path input.
    #[test]
    fn parse_happy_path() {
        // OUI = 0x00015A (DVB generic), hash = 0x00 ^ 0x01 ^ 0x5A = 0x5B.
        let oui: u32 = 0x00_01_5A;
        let oui_hash: u8 = 0x01 ^ 0x5A;

        // A minimal SSU-compatible descriptor: data_broadcast_id_descriptor
        // tag 0x66, length 4, data_broadcast_id 0x000A, selector_len 0x00.
        let common_descs: &[u8] = &[0x66, 0x04, 0x00, 0x0A, 0x00, 0x00];

        let bytes = build_unt(
            0x01, // action_type: System Software Update
            oui_hash,
            7,    // version_number (5-bit)
            true, // current_next_indicator
            0,    // section_number
            0,    // last_section_number
            oui,
            0x00, // processing_order: first
            common_descs,
            &[], // empty platform loop
        );

        let unt = Unt::parse(&bytes).expect("parse must succeed");

        assert_eq!(unt.action_type, 0x01);
        assert_eq!(unt.oui_hash, oui_hash);
        assert_eq!(unt.version_number, 7);
        assert!(unt.current_next_indicator);
        assert_eq!(unt.section_number, 0);
        assert_eq!(unt.last_section_number, 0);
        assert_eq!(unt.oui, oui);
        assert_eq!(unt.processing_order, 0x00);
        assert_eq!(unt.common_descriptors, common_descs);
        assert_eq!(unt.platform_loop, &[] as &[u8]);
    }

    /// current_next_indicator = false must parse correctly.
    #[test]
    fn parse_current_next_false() {
        let bytes = build_unt(0x01, 0x5B, 1, false, 1, 2, 0x00015A, 0x01, &[], &[]);
        let unt = Unt::parse(&bytes).unwrap();
        assert!(!unt.current_next_indicator);
        assert_eq!(unt.section_number, 1);
        assert_eq!(unt.last_section_number, 2);
    }

    /// Platform loop bytes are preserved verbatim.
    #[test]
    fn parse_preserves_platform_loop() {
        // Minimal compatibilityDescriptor: length=0x0004, descriptorCount=0x0000,
        // then platform_loop_length=0x0000.
        let plat: &[u8] = &[0x00, 0x04, 0x00, 0x00, 0x00, 0x00];
        let bytes = build_unt(0x01, 0x5B, 3, true, 0, 0, 0x00015A, 0xFF, &[], plat);
        let unt = Unt::parse(&bytes).unwrap();
        assert_eq!(unt.platform_loop, plat);
        assert_eq!(unt.processing_order, 0xFF);
    }

    /// Wrong table_id must produce `Error::UnexpectedTableId`.
    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_unt(0x01, 0x5B, 0, true, 0, 0, 0x00015A, 0x00, &[], &[]);
        bytes[0] = 0x4A; // BAT table_id — not 0x4B
        let err = Unt::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::UnexpectedTableId { table_id: 0x4A, .. }),
            "expected UnexpectedTableId(0x4A), got {err:?}"
        );
    }

    /// Buffer shorter than the minimum section size must produce
    /// `Error::BufferTooShort`.
    #[test]
    fn parse_rejects_short_buffer() {
        let err = Unt::parse(&[TABLE_ID, 0x00]).unwrap_err();
        assert!(
            matches!(err, Error::BufferTooShort { .. }),
            "expected BufferTooShort, got {err:?}"
        );
    }

    /// `serialize_into` on a buffer that is one byte too small must return
    /// `Error::OutputBufferTooSmall`.
    #[test]
    fn serialize_rejects_small_output_buffer() {
        let unt = Unt {
            action_type: 0x01,
            oui_hash: 0x5B,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            oui: 0x00015A,
            processing_order: 0x00,
            common_descriptors: &[],
            platform_loop: &[],
        };
        let mut buf = vec![0u8; unt.serialized_len() - 1];
        let err = unt.serialize_into(&mut buf).unwrap_err();
        assert!(
            matches!(err, Error::OutputBufferTooSmall { .. }),
            "expected OutputBufferTooSmall, got {err:?}"
        );
    }

    /// Serialize a `Unt` → parse → assert structural equality (round-trip).
    #[test]
    fn serialize_round_trip() {
        let common_descs: &[u8] = &[0x66, 0x04, 0x00, 0x0A, 0x00, 0x00];
        // Minimal compatibilityDescriptor + empty platform_loop_length.
        let plat: &[u8] = &[0x00, 0x04, 0x00, 0x00, 0x00, 0x00];

        let original = Unt {
            action_type: 0x01,
            oui_hash: 0x5B,
            version_number: 15,
            current_next_indicator: true,
            section_number: 2,
            last_section_number: 5,
            oui: 0x00015A,
            processing_order: 0x02,
            common_descriptors: common_descs,
            platform_loop: plat,
        };

        let mut buf = vec![0u8; original.serialized_len()];
        original
            .serialize_into(&mut buf)
            .expect("serialize must succeed");

        let reparsed = Unt::parse(&buf).expect("reparse must succeed");
        assert_eq!(original, reparsed);
    }
}
