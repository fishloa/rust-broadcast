//! IP/MAC Notification Table — ETSI EN 301 192 v1.7.1 §8.4.
//!
//! INT is referenced by a `data_broadcast_id_descriptor` (data_broadcast_id 0x000B)
//! in the PMT ES_info loop; there is no fixed PID.  table_id is 0x4C.
//!
//! Section structure:
//!
//! ```text
//!   [0]      table_id (0x4C)
//!   [1..2]   section_syntax_indicator(1) | reserved_future_use(1) | reserved(2) | section_length(12)
//!   [3]      action_type
//!   [4]      platform_id_hash
//!   [5]      reserved(2) | version_number(5) | current_next_indicator(1)
//!   [6]      section_number
//!   [7]      last_section_number
//!   [8..10]  platform_id (24-bit, big-endian, stored in u32)
//!   [11]     processing_order
//!   [12..]   platform_descriptor_loop  (4-bit reserved | 12-bit length | descriptors)
//!            then N × (target_descriptor_loop | operational_descriptor_loop)
//!   [...-4]  CRC_32
//! ```

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for IP/MAC Notification Table.
pub const TABLE_ID: u8 = 0x4C;

/// PID on which INT is carried.
///
/// INT does not have a fixed PID.  It is discovered through a
/// `data_broadcast_id_descriptor` (data_broadcast_id 0x000B) inside the PMT
/// ES_info loop.  This constant is therefore 0x0000 (unknown/variable), matching
/// the convention used by other tables with no fixed PID in this crate.
pub const PID: u16 = 0x0000;

/// action_type value: IP/MAC stream announcement or location (§8.4.4.1 Table 14).
pub const ACTION_TYPE_STREAM_ANNOUNCEMENT: u8 = 0x01;

// ── layout constants ────────────────────────────────────────────────────────

/// Bytes 0-2: table_id + section_length field.
const OUTER_HEADER_LEN: usize = 3;

/// Bytes 3-11 (after the outer header): fixed INT-specific fields.
///
/// action_type(1) + platform_id_hash(1) + version_byte(1) +
/// section_number(1) + last_section_number(1) + platform_id(3) + processing_order(1)
const INT_FIXED_LEN: usize = 9;

/// Minimum length of a descriptor-loop length header (4-bit reserved + 12-bit length).
const LOOP_LEN_FIELD: usize = 2;

/// CRC_32 at end of section.
const CRC_LEN: usize = 4;

/// Minimum complete section size:
/// outer_header + INT_fixed + platform_descriptor_loop_len_field + CRC.
const MIN_SECTION_LEN: usize = OUTER_HEADER_LEN + INT_FIXED_LEN + LOOP_LEN_FIELD + CRC_LEN;

// ── platform_descriptor_loop byte-offset inside the section ─────────────────

/// Byte offset of the first INT-fixed field (action_type).
const OFF_ACTION_TYPE: usize = 3;
/// Byte offset of platform_id_hash.
const OFF_PLATFORM_ID_HASH: usize = 4;
/// Byte offset of the version_number / current_next_indicator byte.
const OFF_VERSION_BYTE: usize = 5;
/// Byte offset of section_number.
const OFF_SECTION_NUMBER: usize = 6;
/// Byte offset of last_section_number.
const OFF_LAST_SECTION_NUMBER: usize = 7;
/// First byte of the 24-bit platform_id.
const OFF_PLATFORM_ID: usize = 8;
/// Byte offset of processing_order.
const OFF_PROCESSING_ORDER: usize = 11;
/// Start of the platform_descriptor_loop length field.
const OFF_PLATFORM_DESC_LEN: usize = 12;

// ── public types ─────────────────────────────────────────────────────────────

/// IP/MAC Notification Table (INT), ETSI EN 301 192 v1.7.1 §8.4.
///
/// All variable-length regions are borrowed as raw bytes from the source
/// slice.  The per-target-and-operational-descriptor loops are exposed as a
/// single `loops` slice covering everything after the `platform_descriptor_loop`
/// and before the CRC; callers that need to iterate individual entries must
/// walk the raw bytes using the same 4-bit-reserved + 12-bit-length framing
/// described by the spec.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Int<'a> {
    /// Semantics of this INT announcement — 0x01 = stream announcement/location.
    pub action_type: u8,

    /// 8-bit XOR hash over the 24-bit platform_id.
    /// Used for fast section filtering; not unique — always verify against
    /// the full `platform_id`.
    pub platform_id_hash: u8,

    /// 5-bit version_number.
    pub version_number: u8,

    /// current_next_indicator bit.  `true` means this section is currently
    /// applicable; `false` means it will become applicable at the next update.
    pub current_next_indicator: bool,

    /// section_number within this sub-table.
    pub section_number: u8,

    /// last_section_number in this sub-table.
    pub last_section_number: u8,

    /// 24-bit platform identifier (TS 101 162) stored in the low 24 bits of
    /// a `u32`.  The high byte is always zero on the wire.
    pub platform_id: u32,

    /// Processing order relative to other INT sections for the same platform_id.
    /// 0x00 means no ordering constraint.
    pub processing_order: u8,

    /// The `platform_descriptor_loop` (descriptors only, not the 2-byte length
    /// field). Serializes as the typed descriptor sequence; `.raw()` yields the
    /// wire bytes.
    pub platform_descriptors: DescriptorLoop<'a>,

    /// Raw bytes of all `N × (target_descriptor_loop | operational_descriptor_loop)`
    /// entries that follow the platform_descriptor_loop and precede the CRC.
    ///
    /// Each iteration starts with a target_descriptor_loop length field (4-bit
    /// reserved + 12-bit length) followed by target descriptors, then an
    /// operational_descriptor_loop length field followed by operational
    /// descriptors.  Callers iterate this by walking the 2-byte length headers
    /// in sequence.
    pub loops: &'a [u8],
}

// ── Parse ────────────────────────────────────────────────────────────────────

impl<'a> Parse<'a> for Int<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // 1. Absolute minimum length check.
        if bytes.len() < MIN_SECTION_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_SECTION_LEN,
                have: bytes.len(),
                what: "Int",
            });
        }

        // 2. table_id guard.
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "Int",
                expected: &[TABLE_ID],
            });
        }

        // 3. section_length: bits [1] 3-0 (high) and [2] (low).
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = OUTER_HEADER_LEN + section_length;
        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len() - OUTER_HEADER_LEN,
            });
        }

        // 4. Fixed INT fields.
        let action_type = bytes[OFF_ACTION_TYPE];
        let platform_id_hash = bytes[OFF_PLATFORM_ID_HASH];
        let version_byte = bytes[OFF_VERSION_BYTE];
        let version_number = (version_byte >> 1) & 0x1F;
        let current_next_indicator = (version_byte & 0x01) != 0;
        let section_number = bytes[OFF_SECTION_NUMBER];
        let last_section_number = bytes[OFF_LAST_SECTION_NUMBER];
        let platform_id = ((bytes[OFF_PLATFORM_ID] as u32) << 16)
            | ((bytes[OFF_PLATFORM_ID + 1] as u32) << 8)
            | bytes[OFF_PLATFORM_ID + 2] as u32;
        let processing_order = bytes[OFF_PROCESSING_ORDER];

        // 5. platform_descriptor_loop length field (4-bit reserved | 12-bit length).
        let plat_desc_len = (((bytes[OFF_PLATFORM_DESC_LEN] & 0x0F) as usize) << 8)
            | bytes[OFF_PLATFORM_DESC_LEN + 1] as usize;

        let plat_desc_start = OFF_PLATFORM_DESC_LEN + LOOP_LEN_FIELD;
        let plat_desc_end = plat_desc_start + plat_desc_len;

        // Ensure platform_descriptors fit within the section (before CRC).
        if plat_desc_end > total - CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: plat_desc_len,
                available: (total - CRC_LEN).saturating_sub(plat_desc_start),
            });
        }

        let platform_descriptors = DescriptorLoop::new(&bytes[plat_desc_start..plat_desc_end]);

        // 6. Remainder (target/operational loops) — everything between end of
        //    platform_descriptors and CRC.
        let loops_start = plat_desc_end;
        let loops_end = total - CRC_LEN;
        let loops = &bytes[loops_start..loops_end];

        Ok(Int {
            action_type,
            platform_id_hash,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            platform_id,
            processing_order,
            platform_descriptors,
            loops,
        })
    }
}

// ── Serialize ─────────────────────────────────────────────────────────────────

impl Serialize for Int<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        OUTER_HEADER_LEN
            + INT_FIXED_LEN
            + LOOP_LEN_FIELD           // platform_descriptor_loop length field
            + self.platform_descriptors.len()
            + self.loops.len()
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

        // section_length = everything after the 3-byte outer header.
        let section_length = (len - OUTER_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        // section_syntax_indicator=1, reserved_future_use=1, reserved=11 → top nibble 0xF.
        buf[1] = 0xF0 | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;

        // INT fixed fields.
        buf[OFF_ACTION_TYPE] = self.action_type;
        buf[OFF_PLATFORM_ID_HASH] = self.platform_id_hash;
        buf[OFF_VERSION_BYTE] =
            0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[OFF_SECTION_NUMBER] = self.section_number;
        buf[OFF_LAST_SECTION_NUMBER] = self.last_section_number;
        // platform_id: 24 bits big-endian.
        buf[OFF_PLATFORM_ID] = ((self.platform_id >> 16) & 0xFF) as u8;
        buf[OFF_PLATFORM_ID + 1] = ((self.platform_id >> 8) & 0xFF) as u8;
        buf[OFF_PLATFORM_ID + 2] = (self.platform_id & 0xFF) as u8;
        buf[OFF_PROCESSING_ORDER] = self.processing_order;

        // platform_descriptor_loop length field (top nibble reserved = 1111).
        let pdl = self.platform_descriptors.len() as u16;
        buf[OFF_PLATFORM_DESC_LEN] = 0xF0 | ((pdl >> 8) as u8 & 0x0F);
        buf[OFF_PLATFORM_DESC_LEN + 1] = (pdl & 0xFF) as u8;

        // platform_descriptors.
        let plat_start = OFF_PLATFORM_DESC_LEN + LOOP_LEN_FIELD;
        let plat_end = plat_start + self.platform_descriptors.len();
        buf[plat_start..plat_end].copy_from_slice(self.platform_descriptors.raw());

        // loops (raw target/operational iterations).
        let loops_start = plat_end;
        let loops_end = loops_start + self.loops.len();
        buf[loops_start..loops_end].copy_from_slice(self.loops);

        // CRC: compute over everything up to (but not including) the CRC slot.
        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());

        Ok(len)
    }
}

// ── Table trait ──────────────────────────────────────────────────────────────

impl<'a> Table<'a> for Int<'a> {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
}

impl<'a> crate::traits::TableDef<'a> for Int<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "IP_MAC_NOTIFICATION";
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but valid INT section byte vector.
    ///
    /// `platform_desc` = raw bytes for the platform_descriptor_loop payload.
    /// `loops`         = raw bytes for the combined target/operational loop region.
    /// CRC is zeroed (the parser does not verify CRC for INT).
    #[allow(clippy::too_many_arguments)]
    fn build_int(
        action_type: u8,
        platform_id_hash: u8,
        version_number: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        platform_id: u32,
        processing_order: u8,
        platform_desc: &[u8],
        loops: &[u8],
    ) -> Vec<u8> {
        let int = Int {
            action_type,
            platform_id_hash,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            platform_id,
            processing_order,
            platform_descriptors: DescriptorLoop::new(platform_desc),
            loops,
        };
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_happy_path() {
        // platform_id 0x001234, typical DVB IP/MAC announcement.
        let bytes = build_int(
            ACTION_TYPE_STREAM_ANNOUNCEMENT,
            /* platform_id_hash = */
            0x12 ^ 0x34, // XOR of low two bytes for illustration
            /* version_number = */ 3,
            /* current_next_indicator = */ true,
            /* section_number = */ 0,
            /* last_section_number = */ 0,
            /* platform_id = */ 0x00_12_34,
            /* processing_order = */ 0x00,
            /* platform_desc = */ &[0x81, 0x02, 0xAB, 0xCD],
            /* loops = */ &[],
        );
        let int = Int::parse(&bytes).unwrap();

        assert_eq!(int.action_type, ACTION_TYPE_STREAM_ANNOUNCEMENT);
        assert_eq!(int.platform_id_hash, 0x12 ^ 0x34);
        assert_eq!(int.version_number, 3);
        assert!(int.current_next_indicator);
        assert_eq!(int.section_number, 0);
        assert_eq!(int.last_section_number, 0);
        assert_eq!(int.platform_id, 0x00_12_34);
        assert_eq!(int.processing_order, 0x00);
        assert_eq!(
            int.platform_descriptors.raw(),
            &[0x81, 0x02, 0xAB, 0xCD][..]
        );
        assert_eq!(int.loops, &[] as &[u8]);
    }

    #[test]
    fn parse_happy_path_with_loops() {
        // Fake target + operational loop pair: two 2-byte length fields, each
        // covering 0 bytes of descriptors.
        let fake_loops: [u8; 4] = [
            0xF0, 0x00, // target_descriptor_loop_length = 0
            0xF0, 0x00, // operational_descriptor_loop_length = 0
        ];
        let bytes = build_int(
            0x01,
            0x56,
            5,
            false,
            1,
            1,
            0x00_56_78,
            0x01,
            &[],
            &fake_loops,
        );
        let int = Int::parse(&bytes).unwrap();
        assert_eq!(int.platform_id, 0x00_56_78);
        assert_eq!(int.version_number, 5);
        assert!(!int.current_next_indicator);
        assert_eq!(int.loops, &fake_loops[..]);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_int(0x01, 0x00, 0, true, 0, 0, 0x000001, 0x00, &[], &[]);
        bytes[0] = 0x4B; // wrong table_id
        let err = Int::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x4B, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_too_short() {
        let err = Int::parse(&[TABLE_ID, 0xF0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { what: "Int", .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let plat_desc = [0x7C, 0x04, 0x01, 0x02, 0x03, 0x04];
        let fake_loops: [u8; 4] = [0xF0, 0x00, 0xF0, 0x00];
        let bytes = build_int(
            ACTION_TYPE_STREAM_ANNOUNCEMENT,
            0xAB,
            15,
            true,
            2,
            3,
            0x00_AB_CD,
            0x00,
            &plat_desc,
            &fake_loops,
        );

        let int = Int::parse(&bytes).unwrap();

        // Re-serialize.
        let mut buf = vec![0u8; int.serialized_len()];
        int.serialize_into(&mut buf).unwrap();

        // Parse again.
        let re = Int::parse(&buf).unwrap();

        assert_eq!(int, re);
        assert_eq!(re.action_type, ACTION_TYPE_STREAM_ANNOUNCEMENT);
        assert_eq!(re.platform_id_hash, 0xAB);
        assert_eq!(re.version_number, 15);
        assert!(re.current_next_indicator);
        assert_eq!(re.section_number, 2);
        assert_eq!(re.last_section_number, 3);
        assert_eq!(re.platform_id, 0x00_AB_CD);
        assert_eq!(re.processing_order, 0x00);
        assert_eq!(re.platform_descriptors.raw(), &plat_desc[..]);
        assert_eq!(re.loops, &fake_loops[..]);
    }

    #[test]
    fn serialize_rejects_too_small_output_buffer() {
        let int = Int {
            action_type: 0x01,
            platform_id_hash: 0x00,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            platform_id: 0,
            processing_order: 0,
            platform_descriptors: DescriptorLoop::new(&[]),
            loops: &[],
        };
        let mut buf = vec![0u8; 2]; // far too small
        let err = int.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn platform_id_24bit_boundary() {
        // Verify max 24-bit value survives a round-trip without high-byte bleed.
        let bytes = build_int(0x01, 0xFF, 0, true, 0, 0, 0x00FF_FFFF, 0x00, &[], &[]);
        let int = Int::parse(&bytes).unwrap();
        assert_eq!(int.platform_id, 0x00FF_FFFF);
    }
}
