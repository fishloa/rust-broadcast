//! Generic PSI/SI section framing — ETSI EN 300 468 §5.1.1.
//!
//! Every PSI and SI table is carried in one or more sections. This module
//! parses the **section header** and exposes the payload + CRC for
//! table-specific parsers to consume.
//!
//! # Section layout
//!
//! ```text
//! byte 0:       table_id (8 bits)
//! byte 1 bit 7: section_syntax_indicator (1 bit)
//! byte 1 bit 6: private_indicator (1 bit)
//! byte 1 bits 5-4: reserved (2 bits — ignored)
//! byte 1 bits 3-0 + byte 2: section_length (12 bits)
//!
//! Long-form (section_syntax_indicator == 1):
//!   byte 3-4:   table_id_extension (16 bits)
//!   byte 5:     reserved(2) | version_number(5) | current_next_indicator(1)
//!   byte 6:     section_number (8 bits)
//!   byte 7:     last_section_number (8 bits)
//!   byte 8..(total-4): payload
//!   last 4 bytes: CRC_32
//!
//! Short-form (section_syntax_indicator == 0, e.g. TDT):
//!   byte 3..(3+section_length): payload (no extension header, no CRC)
//! ```
//!
//! NOTE the TOT exception: the TOT (0x73) also sets SSI=0 but DOES end with a
//! CRC_32 (EN 300 468 §5.2.6). Parsing it through this generic short-form
//! path folds the CRC into `payload` — use [`crate::tables::tot::Tot`].
//!
//! `section_length` counts bytes *after* the 3-byte section header, so the
//! total section size is `section_length + 3`.

use crate::error::{Error, Result};
use dvb_common::crc32_mpeg2 as crc;
use dvb_common::{Parse, Serialize};

// Minimum bytes to read the section header (table_id + section_syntax_indicator
// + section_length field = 3 bytes).
const MIN_HEADER_LEN: usize = 3;

// Long-form header adds: extension_id(2) + version/cni(1) + sec_num(1) +
// last_sec_num(1) = 5 bytes.
const LONG_FORM_EXTRA: usize = 5;

// CRC occupies the last 4 bytes of every long-form section.
const CRC_LEN: usize = 4;

/// A parsed PSI/SI section header, borrowing the raw input buffer for payload.
///
/// Created via `Section::parse(bytes)`. Does **not** validate the CRC on
/// construction — call [`Section::validate_crc`] explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Section<'a> {
    /// Table identifier.
    pub table_id: u8,
    /// When `true` the section uses the long-form syntax (has extension header
    /// and CRC). When `false` only the 3-byte header is present (short form,
    /// e.g. TDT — but see the module docs for the TOT exception: SSI=0 yet
    /// CRC present; parse TOT via `tables::tot`, not this path).
    pub section_syntax_indicator: bool,
    /// Private indicator bit (meaning is table-specific).
    pub private_indicator: bool,
    /// Number of bytes following byte 2 of the section header.
    pub section_length: u16,
    /// Table ID extension (aka `table_id_extension`). Present only for
    /// long-form sections; zero for short-form.
    pub extension_id: u16,
    /// Version number (5 bits). Present only for long-form sections.
    pub version_number: u8,
    /// `current_next_indicator` flag. Present only for long-form sections.
    pub current_next_indicator: bool,
    /// Section number within the table sub-table.
    pub section_number: u8,
    /// Number of the last section in the table sub-table.
    pub last_section_number: u8,
    /// Section payload: excludes the header bytes and the trailing CRC for
    /// long-form sections. For short-form sections this is bytes
    /// `3..(section_length + 3)`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub payload: &'a [u8],
    /// Declared CRC value (last 4 bytes, big-endian). `None` for short-form
    /// sections which carry no CRC.
    pub crc32: Option<u32>,
}

impl<'a> Section<'a> {
    /// Return the payload slice (same as the `payload` field — convenience
    /// getter for code that has a `&Section` reference).
    #[inline]
    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// Validate the CRC-32 of the section against `raw` — the complete section
    /// bytes (including header and CRC suffix).
    ///
    /// For short-form sections (`section_syntax_indicator == false`) this
    /// returns `Ok(())` immediately because no CRC is present.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CrcMismatch`] when the computed CRC over
    /// `raw[..raw.len() - 4]` does not match the declared value at
    /// `raw[raw.len()-4..]`.
    pub fn validate_crc(&self, raw: &[u8]) -> Result<()> {
        let expected = match self.crc32 {
            None => return Ok(()), // short-form — no CRC
            Some(v) => v,
        };

        // Guard: raw must be at least CRC_LEN bytes for a valid long-form section.
        if raw.len() < CRC_LEN {
            return Err(Error::BufferTooShort {
                need: CRC_LEN,
                have: raw.len(),
                what: "CRC suffix in validate_crc",
            });
        }

        // The CRC covers everything up to (but not including) the 4 CRC bytes.
        let covered = &raw[..raw.len() - CRC_LEN];
        let computed = crc::compute(covered);

        if computed != expected {
            return Err(Error::CrcMismatch { computed, expected });
        }
        Ok(())
    }
}

impl<'a> Parse<'a> for Section<'a> {
    type Error = crate::error::Error;
    /// Parse a complete section from `bytes`.
    ///
    /// # Errors
    ///
    /// - [`Error::BufferTooShort`] — fewer than 3 bytes supplied.
    /// - [`Error::SectionLengthOverflow`] — `section_length` field declares
    ///   more data than `bytes` contains.
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // ── 3-byte common header ────────────────────────────────────────────
        if bytes.len() < MIN_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_HEADER_LEN,
                have: bytes.len(),
                what: "section header",
            });
        }

        let table_id = bytes[0];
        let section_syntax_indicator = (bytes[1] & 0x80) != 0;
        let private_indicator = (bytes[1] & 0x40) != 0;
        let section_length = (((bytes[1] & 0x0F) as u16) << 8) | (bytes[2] as u16);

        // Total section size is section_length + 3 (the 3-byte header itself
        // is not counted by section_length).
        let total = (section_length as usize) + MIN_HEADER_LEN;

        if bytes.len() < total {
            return Err(Error::SectionLengthOverflow {
                declared: total,
                available: bytes.len(),
            });
        }

        // Work only inside the declared section boundary.
        let section_bytes = &bytes[..total];

        if !section_syntax_indicator {
            // ── Short-form section (e.g. TDT) ──────────────────────────────
            // No extension header, no CRC. Payload is everything after the
            // 3-byte header.
            let payload = &section_bytes[MIN_HEADER_LEN..];
            return Ok(Section {
                table_id,
                section_syntax_indicator,
                private_indicator,
                section_length,
                extension_id: 0,
                version_number: 0,
                current_next_indicator: false,
                section_number: 0,
                last_section_number: 0,
                payload,
                crc32: None,
            });
        }

        // ── Long-form section ───────────────────────────────────────────────
        // Minimum size for a valid long-form section:
        //   3 (common header) + 5 (extension header) + 4 (CRC) = 12 bytes.
        let min_long = MIN_HEADER_LEN + LONG_FORM_EXTRA + CRC_LEN;
        if section_bytes.len() < min_long {
            return Err(Error::BufferTooShort {
                need: min_long,
                have: section_bytes.len(),
                what: "long-form section extension header + CRC",
            });
        }

        let extension_id = ((bytes[3] as u16) << 8) | (bytes[4] as u16);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // Payload: bytes[8 .. total-4]  (excludes 5-byte extension header
        // counting from offset 3, and the 4-byte CRC at the end).
        let payload_start = MIN_HEADER_LEN + LONG_FORM_EXTRA;
        let payload_end = total - CRC_LEN;
        let payload = &section_bytes[payload_start..payload_end];

        // Read declared CRC from last 4 bytes of the section (big-endian).
        let crc_offset = total - CRC_LEN;
        let crc32 = Some(
            ((section_bytes[crc_offset] as u32) << 24)
                | ((section_bytes[crc_offset + 1] as u32) << 16)
                | ((section_bytes[crc_offset + 2] as u32) << 8)
                | (section_bytes[crc_offset + 3] as u32),
        );

        Ok(Section {
            table_id,
            section_syntax_indicator,
            private_indicator,
            section_length,
            extension_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            payload,
            crc32,
        })
    }
}

impl Serialize for Section<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        // Total size = section_length + 3 (the 3-byte base header precedes
        // the bytes counted by section_length).
        usize::from(self.section_length) + MIN_HEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }

        // Byte 0: table_id
        buf[0] = self.table_id;

        // Byte 1: SSI | PI | 2-bit reserved (set high per spec) | length hi 4 bits
        let length_hi = ((self.section_length >> 8) as u8) & 0x0F;
        let ssi = u8::from(self.section_syntax_indicator) << 7;
        let pi = u8::from(self.private_indicator) << 6;
        // Reserved bits 5..4 are 'reserved' per §5.1.1 — convention is both set.
        buf[1] = ssi | pi | 0x30 | length_hi;

        // Byte 2: length low 8 bits
        buf[2] = (self.section_length & 0xFF) as u8;

        if self.section_syntax_indicator {
            // Long form: 5 bytes of extension header, then payload, then CRC.
            buf[3] = (self.extension_id >> 8) as u8;
            buf[4] = (self.extension_id & 0xFF) as u8;
            // Byte 5: 2-bit reserved (both high) | 5-bit version | 1-bit current_next
            let version = (self.version_number & 0x1F) << 1;
            let cni = u8::from(self.current_next_indicator);
            buf[5] = 0xC0 | version | cni;
            buf[6] = self.section_number;
            buf[7] = self.last_section_number;

            let payload_start = MIN_HEADER_LEN + LONG_FORM_EXTRA;
            let payload_end = payload_start + self.payload.len();
            buf[payload_start..payload_end].copy_from_slice(self.payload);

            // Append CRC — use the declared value to preserve round-trip identity.
            let crc = self.crc32.expect("long-form section must carry a CRC");
            let crc_start = payload_end;
            buf[crc_start..crc_start + CRC_LEN].copy_from_slice(&crc.to_be_bytes());
        } else {
            // Short form: no extension header, no CRC.
            let payload_end = MIN_HEADER_LEN + self.payload.len();
            buf[MIN_HEADER_LEN..payload_end].copy_from_slice(self.payload);
        }

        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::crc32_mpeg2::compute as crc32;

    // ── Helper: build a minimal long-form section with correct CRC ───────────

    /// Build a syntactically valid long-form section byte vector.
    ///
    /// Layout: [table_id, flags+len_hi, len_lo, ext_hi, ext_lo,
    ///          ver_cni, sec_num, last_sec_num, ...payload..., crc(4)]
    fn make_long_section(
        table_id: u8,
        extension_id: u16,
        version: u8,
        current_next: bool,
        section_number: u8,
        last_section_number: u8,
        payload: &[u8],
    ) -> Vec<u8> {
        // section_length = 5 (extension header) + payload.len() + 4 (CRC)
        let section_length: u16 = (5 + payload.len() + 4) as u16;

        // reserved(2) | version(5) | current_next(1)
        let ver_cni = 0xC0u8 | ((version & 0x1F) << 1) | (current_next as u8);
        let mut buf: Vec<u8> = vec![
            table_id,
            // section_syntax_indicator=1, private_indicator=0, reserved=0b11, upper 4 bits of section_length
            0x80 | 0x30 | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            (extension_id >> 8) as u8,
            (extension_id & 0xFF) as u8,
            ver_cni,
            section_number,
            last_section_number,
        ];
        buf.extend_from_slice(payload);

        // Compute CRC over bytes so far, append as big-endian u32.
        let crc = crc32(&buf);
        buf.push((crc >> 24) as u8);
        buf.push((crc >> 16) as u8);
        buf.push((crc >> 8) as u8);
        buf.push(crc as u8);

        buf
    }

    // ── Test 1 ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_rejects_buffer_shorter_than_3_bytes() {
        for bad_len in [0usize, 1, 2] {
            let buf = vec![0x00u8; bad_len];
            let err = Section::parse(&buf).unwrap_err();
            assert!(
                matches!(err, Error::BufferTooShort { need: 3, have, .. } if have == bad_len),
                "expected BufferTooShort for len={bad_len}, got {err:?}"
            );
        }
    }

    // ── Test 2 ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_reads_table_id_syntax_indicator_and_length() {
        // Construct a minimal 13-byte long-form section with no payload.
        // section_length = 5 (extension header) + 0 (payload) + 4 (CRC) = 9
        // BUT we need section_length >= min for a valid section: 9 → total = 12, which is 12 bytes, not 13.
        // Let's use 1 byte of payload so section_length = 10, total = 13.
        let raw = make_long_section(0x42, 0x1234, 3, true, 0, 0, &[0xAB]);
        assert_eq!(raw.len(), 13);

        let section = Section::parse(&raw).unwrap();
        assert_eq!(section.table_id, 0x42);
        assert!(section.section_syntax_indicator);
        // section_length = 5 + 1 + 4 = 10
        assert_eq!(section.section_length, 10);
    }

    // ── Test 3 ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_rejects_when_section_length_exceeds_buffer() {
        // Build a 3-byte header that claims section_length = 100 bytes of data.
        // Buffer is only 3 bytes (just the header), so total = 103 > 3.
        let buf = [
            0x00u8, // table_id
            0x80u8, // section_syntax_indicator=1, section_length upper nibble = 0
            100u8,  // section_length lower byte = 100 → total = 103
        ];
        let err = Section::parse(&buf).unwrap_err();
        assert!(
            matches!(
                err,
                Error::SectionLengthOverflow {
                    declared: 103,
                    available: 3
                }
            ),
            "expected SectionLengthOverflow, got {err:?}"
        );
    }

    // ── Test 4 ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_reads_extension_id_version_current_next_section_numbers() {
        let raw = make_long_section(
            0x02,          // PMT table_id
            0xBEEF,        // extension_id / program_number
            7,             // version_number
            true,          // current_next
            2,             // section_number
            5,             // last_section_number
            &[0x00, 0x00], // dummy payload
        );

        let section = Section::parse(&raw).unwrap();
        assert_eq!(section.extension_id, 0xBEEF);
        assert_eq!(section.version_number, 7);
        assert!(section.current_next_indicator);
        assert_eq!(section.section_number, 2);
        assert_eq!(section.last_section_number, 5);
    }

    #[test]
    fn parse_reads_current_next_indicator_false() {
        // Same as test 4 but with current_next = false (bit 0 of byte 5 cleared).
        let raw = make_long_section(
            0x02,   // PMT table_id
            0xBEEF, // extension_id
            7,      // version_number
            false,  // current_next — the field under test
            2,      // section_number
            5,      // last_section_number
            &[0x00, 0x00],
        );

        let section = Section::parse(&raw).unwrap();
        assert!(!section.current_next_indicator);
        // Confirm the other fields are unaffected by flipping bit 0.
        assert_eq!(section.extension_id, 0xBEEF);
        assert_eq!(section.version_number, 7);
        assert_eq!(section.section_number, 2);
        assert_eq!(section.last_section_number, 5);
    }

    // ── Test 5 ───────────────────────────────────────────────────────────────

    #[test]
    fn payload_slice_excludes_header_and_crc() {
        let inner_payload = &[0x01u8, 0x02, 0x03, 0x04, 0x05];
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, inner_payload);

        let section = Section::parse(&raw).unwrap();
        assert_eq!(section.payload(), inner_payload);
    }

    // ── Test 6 ───────────────────────────────────────────────────────────────

    #[test]
    fn validate_crc_accepts_matching_crc32() {
        let raw = make_long_section(0x00, 0x0001, 1, true, 0, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
        let section = Section::parse(&raw).unwrap();
        section.validate_crc(&raw).expect("CRC should match");
    }

    // ── Test 7 ───────────────────────────────────────────────────────────────

    #[test]
    fn validate_crc_rejects_flipped_bit() {
        let mut raw = make_long_section(0x00, 0x0001, 1, true, 0, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
        // Flip a bit inside the payload (byte 8 is the first payload byte) BEFORE
        // parsing.  The 4-byte CRC at the tail of `raw` was computed over the
        // original (un-flipped) bytes and is NOT updated here, so after the flip:
        //   • `raw[..raw.len()-4]` contains corrupted data, and
        //   • `raw[raw.len()-4..]` still holds the CRC of the original data.
        // Parsing after the flip captures that old CRC into `section.crc32`.
        // `validate_crc` then recomputes over the corrupted bytes and detects the
        // mismatch — which is exactly the invariant we are testing.
        // (Parsing before the flip would immutably borrow `raw` for the lifetime
        // of `section` — the compiler would then reject `raw[8] ^= 0x01` because
        // a mutable borrow conflicts with any live shared borrow.)
        raw[8] ^= 0x01;

        let section = Section::parse(&raw).unwrap();
        let err = section.validate_crc(&raw).unwrap_err();
        assert!(
            matches!(err, Error::CrcMismatch { .. }),
            "expected CrcMismatch, got {err:?}"
        );
    }

    // ── Test (Fix 1 TDD) ─────────────────────────────────────────────────────

    #[test]
    fn validate_crc_rejects_raw_slice_shorter_than_crc_len() {
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, &[0xDE, 0xAD]);
        let section = Section::parse(&raw).unwrap();
        // Pass an empty slice — shorter than CRC_LEN bytes.
        let err = section.validate_crc(&[]).unwrap_err();
        assert!(
            matches!(err, Error::BufferTooShort { need: CRC_LEN, .. }),
            "expected BufferTooShort(need=CRC_LEN), got {err:?}"
        );
    }

    // ── Test 8 ───────────────────────────────────────────────────────────────

    #[test]
    fn short_form_section_has_no_crc() {
        // TDT-style short-form section: section_syntax_indicator = 0.
        // section_length = 5 (5 bytes of payload), total = 8 bytes.
        let buf = [
            0x70u8, // table_id (TDT)
            0x70u8, // SSI=0, private=1, reserved=0b11, upper nibble of section_length=0
            0x05u8, // section_length = 5
            // 5 bytes of "UTC time" payload
            0xE0, 0x00, 0x00, 0x00, 0x00,
        ];

        let section = Section::parse(&buf).unwrap();
        assert!(!section.section_syntax_indicator);
        assert!(section.crc32.is_none());
        // validate_crc on short-form should return Ok(()) vacuously.
        section
            .validate_crc(&buf)
            .expect("short-form: no CRC to validate");
        // Payload is the 5 bytes after the 3-byte header.
        assert_eq!(section.payload(), &buf[3..]);
    }
}
