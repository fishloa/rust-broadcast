//! Generic PSI/SI section framing — ITU-T H.222.0 §2.4 (= ISO/IEC 13818-1) and ETSI EN 300 468 §5.1.1.
//!
//! Every PSI and SI table is carried in one or more sections. This module
//! parses the **section header** and exposes the payload for table-specific
//! parsers to consume.
//!
//! # Section layout
//!
//! ```text
//! byte 0:       table_id (8 bits)
//! byte 1 bit 7: section_syntax_indicator (1 bit)
//! byte 1 bit 6: private_indicator (1 bit)
//! byte 1 bits 5-4: reserved (2 bits)
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
//! path folds the CRC into `payload` — use the table-specific `TotSection` parser instead.
//!
//! `section_length` counts bytes *after* the 3-byte section header, so the
//! total section size is `section_length + 3`. It is derived from the payload
//! on serialize, never stored, so a section whose payload is edited cannot
//! carry a stale count or CRC.

use crate::error::{Error, Result};
use broadcast_common::crc32_mpeg2 as crc;
use broadcast_common::len::{fit_bits, fit_u8, need_in, need_out};
use broadcast_common::{Parse, Serialize};

const MIN_HEADER_LEN: usize = 3;

// Long-form header adds: extension_id(2) + version/cni(1) + sec_num(1) +
// last_sec_num(1) = 5 bytes.
const LONG_FORM_EXTRA: usize = 5;

// CRC occupies the last 4 bytes of every long-form section.
const CRC_LEN: usize = 4;

// W15 (#1074): named bit-field masks for the section header, used by both
// `parse` and `serialize_into` — ISO/IEC 13818-1 §2.4.4.1.

/// `section_syntax_indicator` — byte 1 bit 7.
const SSI_MASK: u8 = 0x80;
/// `private_indicator` (byte 1 bit 6). Meaning is table-specific; DVB SI uses
/// this position for the DVB `private_indicator` (ETSI EN 300 468 §5.1.1).
const PRIVATE_INDICATOR_MASK: u8 = 0x40;
/// Byte 1 bits 5-4: reserved (ISO/IEC 13818-1 §2.4.4.1).
const BYTE1_RESERVED_BITS: u8 = 0x30;
/// Number of bits in the `section_length` field (12, ISO/IEC 13818-1 §2.4.4.1).
const SECTION_LENGTH_BITS: u32 = 12;
/// `version_number` — 5 bits (byte 5 bits 5-1 of the long-form extension header).
const VERSION_NUMBER_MASK: u8 = 0x1F;
/// Number of bits in `version_number` (5, ISO/IEC 13818-1 §2.4.4.1).
const VERSION_NUMBER_BITS: u32 = 5;
/// Byte 5 bits 7-6: reserved (ISO/IEC 13818-1 §2.4.4.1).
const BYTE5_RESERVED_BITS: u8 = 0xC0;
/// The 2 reserved bits sit at bits 7..6 of byte 5.
const BYTE5_RESERVED_SHIFT: u32 = 6;
/// `current_next_indicator` — byte 5 bit 0 of the long-form extension header.
const CURRENT_NEXT_MASK: u8 = 0x01;

/// A parsed PSI/SI section header, borrowing the raw input buffer for payload.
///
/// Created via `Section::parse(bytes)`. Does **not** validate the CRC on
/// construction — call [`Section::validate_crc`] (or the raw-byte
/// [`validate_crc`]) explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
    pub payload: &'a [u8],
    /// The 2 `reserved` bits of byte 1 (spec convention `0b11`) — kept so a
    /// section that does not use the convention still round-trips.
    pub byte1_reserved: u8,
    /// The 2 `reserved` bits of byte 5 (spec convention `0b11`); only
    /// meaningful for long-form sections.
    pub byte5_reserved: u8,
    /// The CRC_32 the section carried when it was parsed; `None` for
    /// short-form sections. Never written back: [`Serialize`] recomputes the
    /// CRC from the current fields.
    parsed_crc32: Option<u32>,
}

/// Validate the CRC-32 of a complete long-form section held in `raw`: the
/// header, the payload and the 4 trailing CRC bytes.
///
/// Every byte is on the wire, so this needs no parsed [`Section`], and a
/// section whose payload was edited is validated against what it holds now
/// rather than against a stale stored value.
///
/// Returns `Ok(())` when `raw` cannot hold a long-form section at all
/// (fewer than the 3-byte header plus the 5-byte extension header plus the 4
/// CRC bytes): such a section carries no CRC.
///
/// # Errors
///
/// Returns [`Error::CrcMismatch`] when the computed CRC over
/// `raw[..raw.len() - 4]` does not match the declared value at
/// `raw[raw.len()-4..]`.
pub fn validate_crc(raw: &[u8]) -> Result<()> {
    if raw.len() < MIN_HEADER_LEN {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: MIN_HEADER_LEN,
                have: raw.len(),
                what: "section header in validate_crc",
            },
        ));
    }
    let section_syntax_indicator = (raw[1] & SSI_MASK) != 0;
    if !section_syntax_indicator || raw.len() < MIN_HEADER_LEN + LONG_FORM_EXTRA + CRC_LEN {
        return Ok(());
    }
    let computed = crc::compute(&raw[..raw.len() - CRC_LEN]);
    let tail = &raw[raw.len() - CRC_LEN..];
    let expected = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]);
    if computed != expected {
        return Err(Error::CrcMismatch(broadcast_common::error::CrcMismatch {
            what: "section",
            computed,
            expected,
        }));
    }
    Ok(())
}

impl<'a> Section<'a> {
    /// The same section owning its payload, so it can outlive the input
    /// buffer it was parsed from.
    #[must_use]
    pub fn into_owned(self) -> Section<'static> {
        Section {
            payload: alloc::boxed::Box::leak(self.payload.to_vec().into_boxed_slice()),
            ..self
        }
    }

    /// The section as its own bytes: header, payload and, for a long-form
    /// section, a freshly computed CRC_32.
    ///
    /// # Errors
    ///
    /// [`Error::FieldOverflow`] when a header field does not fit its wire
    /// width (see [`section_length`](Self::section_length)).
    pub fn serialize(&self) -> Result<alloc::vec::Vec<u8>> {
        let mut buf = alloc::vec![0u8; self.serialized_len()];
        self.serialize_into(&mut buf)?;
        Ok(buf)
    }

    /// Return the payload slice (same as the `payload` field — convenience
    /// getter for code that has a `&Section` reference).
    #[inline]
    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// `section_length` implied by [`payload`](Self::payload) and the syntax
    /// indicator (ISO/IEC 13818-1 §2.4.4.1): the byte count after the 3-byte
    /// header.
    #[must_use]
    pub fn section_length(&self) -> usize {
        self.payload.len() + self.overhead()
    }

    /// Header bytes the syntax adds around the payload: the 5-byte extension
    /// header plus the 4-byte CRC for long form, nothing for short form.
    fn overhead(&self) -> usize {
        if self.section_syntax_indicator {
            LONG_FORM_EXTRA + CRC_LEN
        } else {
            0
        }
    }

    /// The CRC_32 the section carried when it was parsed; `None` for
    /// short-form sections.
    #[must_use]
    pub fn crc32(&self) -> Option<u32> {
        self.parsed_crc32
    }

    fn header_bytes(&self) -> Result<[u8; MIN_HEADER_LEN]> {
        let length = fit_bits(
            self.section_length() as u64,
            SECTION_LENGTH_BITS,
            "section_length",
        )? as u16;
        Ok([
            self.table_id,
            (u8::from(self.section_syntax_indicator) << 7)
                | (u8::from(self.private_indicator) << 6)
                | self.byte1_reserved
                | ((length >> 8) as u8 & crate::ts::SECTION_LENGTH_HI_MASK),
            length.to_be_bytes()[1],
        ])
    }

    fn extension_header_bytes(&self) -> Result<[u8; LONG_FORM_EXTRA]> {
        let version = fit_bits(
            u64::from(self.version_number),
            VERSION_NUMBER_BITS,
            "version_number",
        )? as u8;
        Ok([
            (self.extension_id >> 8) as u8,
            (self.extension_id & 0xFF) as u8,
            fit_u8(
                usize::from(self.byte5_reserved) << BYTE5_RESERVED_SHIFT
                    | usize::from(version) << 1
                    | usize::from(self.current_next_indicator),
                "section byte 5",
            )?,
            self.section_number,
            self.last_section_number,
        ])
    }

    /// Validate the CRC_32 this section currently carries, recomputed from
    /// its fields.
    ///
    /// The CRC is never stored: [`Serialize`] always recomputes it, so this
    /// is the value a serialized section will carry, even after the payload
    /// was edited.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CrcMismatch`] when the recomputed CRC is not the one
    /// the section was parsed from (only possible for bytes that carried a
    /// wrong CRC). Always `Ok(())` for short-form sections.
    pub fn validate_crc(&self) -> Result<()> {
        let Some(expected) = self.parsed_crc32 else {
            return Ok(());
        };
        let mut covered = alloc::vec::Vec::with_capacity(MIN_HEADER_LEN + LONG_FORM_EXTRA);
        covered.extend_from_slice(&self.header_bytes()?);
        covered.extend_from_slice(&self.extension_header_bytes()?);
        covered.extend_from_slice(self.payload);
        let computed = crc::compute(&covered);
        if computed != expected {
            return Err(Error::CrcMismatch(broadcast_common::error::CrcMismatch {
                what: "section",
                computed,
                expected,
            }));
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
        need_in(bytes, MIN_HEADER_LEN, "section header")?;

        let table_id = bytes[0];
        let section_syntax_indicator = (bytes[1] & SSI_MASK) != 0;
        let private_indicator = (bytes[1] & PRIVATE_INDICATOR_MASK) != 0;
        let byte1_reserved = (bytes[1] & BYTE1_RESERVED_BITS) & !crate::ts::SECTION_LENGTH_HI_MASK;

        // Total section size is section_length + 3 (the 3-byte header itself
        // is not counted by section_length). W14 (#1074): shared with the
        // other three sites that used to decode this field separately.
        // `need_in(bytes, MIN_HEADER_LEN, ..)` passed above, so the header is
        // present and `section_total_len` cannot return `None`.
        let total = match crate::ts::section_total_len(bytes) {
            Some(t) => t,
            None => return Err(Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                need: MIN_HEADER_LEN,
                have: bytes.len(),
                what: "section header",
            })),
        };

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
                extension_id: 0,
                version_number: 0,
                current_next_indicator: false,
                section_number: 0,
                last_section_number: 0,
                payload,
                byte1_reserved,
                byte5_reserved: BYTE5_RESERVED_BITS >> BYTE5_RESERVED_SHIFT,
                parsed_crc32: None,
            });
        }

        // ── Long-form section ───────────────────────────────────────────────
        // Minimum size for a valid long-form section:
        //   3 (common header) + 5 (extension header) + 4 (CRC) = 12 bytes.
        let min_long = MIN_HEADER_LEN + LONG_FORM_EXTRA + CRC_LEN;
        need_in(
            section_bytes,
            min_long,
            "long-form section extension header + CRC",
        )?;

        let extension_id = ((bytes[3] as u16) << 8) | (bytes[4] as u16);
        let byte5_reserved = (bytes[5] & BYTE5_RESERVED_BITS) >> BYTE5_RESERVED_SHIFT;
        let version_number = (bytes[5] >> 1) & VERSION_NUMBER_MASK;
        let current_next_indicator = (bytes[5] & CURRENT_NEXT_MASK) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        // Payload: bytes[8 .. total-4]  (excludes 5-byte extension header
        // counting from offset 3, and the 4-byte CRC at the end).
        let payload_start = MIN_HEADER_LEN + LONG_FORM_EXTRA;
        let payload_end = total - CRC_LEN;
        let payload = &section_bytes[payload_start..payload_end];

        let crc_offset = total - CRC_LEN;
        let parsed_crc32 = Some(
            ((section_bytes[crc_offset] as u32) << 24)
                | ((section_bytes[crc_offset + 1] as u32) << 16)
                | ((section_bytes[crc_offset + 2] as u32) << 8)
                | (section_bytes[crc_offset + 3] as u32),
        );

        Ok(Section {
            table_id,
            section_syntax_indicator,
            private_indicator,
            extension_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            payload,
            byte1_reserved,
            byte5_reserved,
            parsed_crc32,
        })
    }
}

impl Serialize for Section<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        MIN_HEADER_LEN + self.section_length()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;

        buf[..MIN_HEADER_LEN].copy_from_slice(&self.header_bytes()?);
        if !self.section_syntax_indicator {
            buf[MIN_HEADER_LEN..need].copy_from_slice(self.payload);
            return Ok(need);
        }

        let payload_start = MIN_HEADER_LEN + LONG_FORM_EXTRA;
        buf[MIN_HEADER_LEN..payload_start].copy_from_slice(&self.extension_header_bytes()?);
        let payload_end = payload_start + self.payload.len();
        buf[payload_start..payload_end].copy_from_slice(self.payload);

        let crc = crc::compute(&buf[..payload_end]);
        buf[payload_end..need].copy_from_slice(&crc.to_be_bytes());
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use broadcast_common::crc32_mpeg2::compute as crc32;

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
                matches!(err, Error::BufferTooShort(broadcast_common::error::BufferTooShort { need: 3, have, .. }) if have == bad_len),
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
        assert_eq!(section.section_length(), 10);
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
        section.validate_crc().expect("CRC should match");
        validate_crc(&raw).expect("CRC should match");
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
        // Parsing after the flip captures that old CRC, which the section no
        // longer recomputes to; `validate_crc` detects the mismatch over the
        // corrupted bytes in both forms — which is what we are testing.
        // (Parsing before the flip would immutably borrow `raw` for the lifetime
        // of `section` — the compiler would then reject `raw[8] ^= 0x01` because
        // a mutable borrow conflicts with any live shared borrow.)
        raw[8] ^= 0x01;

        let section = Section::parse(&raw).unwrap();
        let err = section.validate_crc().unwrap_err();
        assert!(
            matches!(err, Error::CrcMismatch(..)),
            "expected CrcMismatch, got {err:?}"
        );
        let err = validate_crc(&raw).unwrap_err();
        assert!(
            matches!(err, Error::CrcMismatch(..)),
            "expected CrcMismatch, got {err:?}"
        );
    }

    // ── Test (Fix 1 TDD) ─────────────────────────────────────────────────────

    #[test]
    fn validate_crc_rejects_raw_slice_shorter_than_the_header() {
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, &[0xDE, 0xAD]);
        // Parse the section first: it must still parse, so the failure below
        // comes from the raw-byte validator alone.
        Section::parse(&raw).unwrap();
        // Pass an empty slice — shorter than the 3-byte header itself.
        let err = validate_crc(&[]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                    need: MIN_HEADER_LEN,
                    ..
                })
            ),
            "expected BufferTooShort(need=MIN_HEADER_LEN), got {err:?}"
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
        assert_eq!(section.crc32(), None);
        // validate_crc on short-form should return Ok(()) vacuously.
        section
            .validate_crc()
            .expect("short-form: no CRC to validate");
        validate_crc(&buf).expect("short-form: no CRC to validate");
        // Payload is the 5 bytes after the 3-byte header.
        assert_eq!(section.payload(), &buf[3..]);
    }

    // ── Serialize derives the length and recomputes the CRC (#2749) ──────────

    #[test]
    fn serialize_grows_with_an_edited_payload() {
        // `section_length` used to be a public field independent of
        // `payload`: a longer payload either errored or was truncated to the
        // stale count. It is derived now, so the edit is simply written.
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, &[0xAA, 0xBB]);
        let mut section = Section::parse(&raw).unwrap();
        let longer = [0xAAu8, 0xBB, 0xCC, 0xDD];
        section.payload = &longer;

        assert_eq!(
            section.section_length(),
            longer.len() + LONG_FORM_EXTRA + CRC_LEN
        );
        let mut buf = vec![0u8; section.serialized_len()];
        let n = section.serialize_into(&mut buf).unwrap();
        assert_eq!(n, raw.len() + 2);
        assert_eq!(&buf[8..12], &longer);
        validate_crc(&buf[..n]).expect("the recomputed CRC must validate");
    }

    #[test]
    fn serialize_shrinks_with_an_edited_payload() {
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, &[0xAA, 0xBB, 0xCC]);
        let mut section = Section::parse(&raw).unwrap();
        let shorter = &[0xAAu8][..];
        section.payload = shorter;

        assert_eq!(
            section.section_length(),
            shorter.len() + LONG_FORM_EXTRA + CRC_LEN
        );
        let mut buf = vec![0u8; section.serialized_len()];
        let n = section.serialize_into(&mut buf).unwrap();
        assert_eq!(n, raw.len() - 2);
        validate_crc(&buf[..n]).expect("the recomputed CRC must validate");
    }

    #[test]
    fn serialize_recomputes_the_crc_after_a_payload_edit() {
        // The parsed section used to write its stored CRC back verbatim, so a
        // payload edit produced a section that failed its own CRC.
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, &[0xAA, 0xBB]);
        let parsed = Section::parse(&raw).unwrap();
        let mut section = parsed.clone();
        let edited = [0x11u8, 0x22];
        section.payload = &edited;

        let mut buf = vec![0u8; section.serialized_len()];
        assert!(matches!(
            section.validate_crc(),
            Err(Error::CrcMismatch(..))
        ));
        section.serialize_into(&mut buf).unwrap();
        assert_ne!(
            buf[buf.len() - CRC_LEN..],
            raw[raw.len() - CRC_LEN..],
            "the CRC must change"
        );
        validate_crc(&buf).expect("the serialized edited section's CRC");
    }

    #[test]
    fn serialize_rejects_a_payload_too_large_for_section_length() {
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, &[0xAA]);
        let mut section = Section::parse(&raw).unwrap();
        let too_long = vec![0u8; (1 << SECTION_LENGTH_BITS) - LONG_FORM_EXTRA - CRC_LEN + 1];
        section.payload = &too_long;

        let mut buf = vec![0u8; section.serialized_len()];
        assert!(matches!(
            section.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));
    }

    #[test]
    fn serialize_accepts_consistent_section_round_trip() {
        // Sanity: a self-consistent Section (as produced by `parse`) still
        // serializes byte-identically after the new validation.
        let raw = make_long_section(0x42, 0x0001, 3, true, 1, 2, &[0x01, 0x02, 0x03]);
        let section = Section::parse(&raw).unwrap();
        let mut buf = vec![0u8; section.serialized_len()];
        let n = section.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[..n], raw.as_slice());
    }

    #[test]
    fn reserved_bits_that_are_not_both_set_round_trip() {
        // Byte 1 and byte 5 reserved bits used to be forced to `11` on
        // serialize, so such a section did not round-trip.
        let mut raw = make_long_section(0x42, 0x0001, 3, true, 0, 0, &[0xAA]);
        raw[1] = (raw[1] & !BYTE1_RESERVED_BITS) | 0x10;
        raw[5] &= !BYTE5_RESERVED_BITS;
        let len = raw.len();
        let crc = crc32(&raw[..len - CRC_LEN]);
        raw[len - CRC_LEN..].copy_from_slice(&crc.to_be_bytes());

        let section = Section::parse(&raw).unwrap();
        assert_eq!(section.byte1_reserved, 0x10);
        assert_eq!(section.byte5_reserved, 0x00);
        let mut buf = vec![0u8; section.serialized_len()];
        section.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, raw);
    }

    #[test]
    fn serialize_rejects_an_over_range_version_number() {
        let raw = make_long_section(0x42, 0x0001, 0, true, 0, 0, &[0xAA]);
        let mut section = Section::parse(&raw).unwrap();
        section.version_number = (1 << VERSION_NUMBER_BITS) as u8;

        let mut buf = vec![0u8; section.serialized_len()];
        assert!(matches!(
            section.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));
    }
}
