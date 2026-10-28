//! SI + PSI table-section parsers.
//!
//! Each `*Section` type parses and serializes one wire section. Use
//! [`crate::collect`] to assemble complete logical tables that span multiple
//! sections.
//!
//! # What is here
//!
//! | Family | Modules (table_id) | Standard |
//! |---|---|---|
//! | MPEG-2 PSI | [`pat`] (0x00), [`cat`] (0x01), [`pmt`] (0x02), [`tsdt`] (0x03) | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 |
//! | DSM-CC | [`dsmcc`] (0x3A–0x3F), [`mpe`] (0x3E typed MPE view) | ISO/IEC 13818-6, ETSI EN 301 192 |
//! | DVB SI | [`nit`] (0x40/0x41), [`sdt`] (0x42/0x46), [`bat`] (0x4A), [`sat`] (0x4D), [`eit`] (0x4E–0x6F), [`tdt`] (0x70), [`rst`] (0x71), [`st`] (0x72), [`tot`] (0x73), [`dit`] (0x7E), [`sit`] (0x7F) | ETSI EN 300 468 |
//! | Data broadcasting / SSU | [`unt`] (0x4B), [`int`] (0x4C), [`mpe_fec`] (0x78), [`mpe_ifec`] (0x7A) | ETSI TS 102 006, EN 301 192, TS 102 772 |
//! | Applications | [`ait`] (0x74), [`protection_message`] (0x7B) | ETSI TS 102 809 |
//! | TV-Anytime | [`container`] (0x75), [`rct`] (0x76), [`cit`] (0x77), [`rnt`] (0x79) | ETSI TS 102 323 |
//! | Subtitling fonts | [`downloadable_font_info`] (0x7C) | ETSI EN 303 560 |
//!
//! [`AnyTableSection`] dispatches by table_id (generated from one
//! `declare_tables!` list); [`TableRegistry`] adds private table_ids. The
//! per-table_id reference with verified clauses, and the table_ids no module
//! implements yet, is generated into
//! [`dvb-si/docs/COVERAGE.md`](https://github.com/fishloa/rust-broadcast/blob/main/dvb-si/docs/COVERAGE.md#tables-table_id).
//!
//! # Reserved-bit policy (parse tolerant, serialize re-emits)
//!
//! Every `reserved` / `reserved_future_use` bit of a section header and body
//! is captured into a typed field on parse and written back unchanged on
//! serialize, so a real sender's section re-serializes byte-identically (CRC is
//! recomputed). Constructed values (`new()` / `Default`) carry the spec fill
//! value of the edition the crate is transcribed from
//! (`dvb-si/docs/reserved-bits-edition-audit.md` is the authority):
//!
//! * the bit after `section_syntax_indicator` is **`private_indicator`** in
//!   ITU-T H.222.0 (all nine editions: "a 1-bit user definable flag that shall
//!   not be specified by ITU-T | ISO/IEC in the future") and `reserved_future_use`
//!   (=`1`) in EN 300 468. It is modelled as [`SectionBits::private_indicator`];
//!   PSI sections ([`SectionBits::psi`]) default to `0` (the H.222.0 literal),
//!   EN 300 468 tables ([`SectionBits::dvb`]) to `1`;
//! * `section_syntax_indicator` itself is [`SectionBits::section_syntax_indicator`]:
//!   `1` for the long-form tables, `0` for TDT/TOT/RST/DIT
//!   ([`SectionBits::short_form`]); a received section keeps what it carried;
//! * a value wider than its field (`version_number` over 31, a `reserved*`
//!   wider than its width) is `Error::FieldOverflow` on serialize, never masked;
//! * the `reserved(2)` after it and the `reserved(2)` in front of
//!   `version_number` are [`SectionBits::reserved_b1`] / [`SectionBits::reserved_b5`]
//!   (default `0b11`);
//! * body `reserved(3)` before a PID, `reserved(4)` before a descriptor-loop
//!   length and each table's own `reserved_future_use` runs are typed
//!   `reserved*` fields on the entry/section they belong to (default all ones).
//!
//! The reserved fields never take part in serde output (serialize-only,
//! decoded view) and every changed public struct is `#[non_exhaustive]` with a
//! `new(..)` constructor so future fields stay non-breaking.

/// Running status of an event or service — EN 300 468 Table 6.
///
/// Codes 6-7 are reserved for future use and round-tripped transparently via
/// [`Reserved`](RunningStatus::Reserved).
///
/// # Examples
/// ```
/// use dvb_si::tables::RunningStatus;
///
/// let s = RunningStatus::from_u8(4);
/// assert_eq!(s.name(), "running");
/// assert_eq!(s.to_u8(), 4); // lossless back to the wire value
///
/// // Unallocated codes are preserved verbatim for byte-identical round-trip.
/// assert_eq!(RunningStatus::from_u8(6).to_u8(), 6);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RunningStatus {
    /// Value 0 — undefined.
    Undefined,
    /// Value 1 — not running.
    NotRunning,
    /// Value 2 — starts in a few seconds (e.g. for video recording).
    StartsInAFewSeconds,
    /// Value 3 — pausing.
    Pausing,
    /// Value 4 — running.
    Running,
    /// Value 5 — service off-air.
    ServiceOffAir,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl RunningStatus {
    /// Map any 3-bit value to a `RunningStatus`.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::Undefined,
            1 => Self::NotRunning,
            2 => Self::StartsInAFewSeconds,
            3 => Self::Pausing,
            4 => Self::Running,
            5 => Self::ServiceOffAir,
            r => Self::Reserved(r),
        }
    }

    /// Return the 3-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Undefined => 0,
            Self::NotRunning => 1,
            Self::StartsInAFewSeconds => 2,
            Self::Pausing => 3,
            Self::Running => 4,
            Self::ServiceOffAir => 5,
            Self::Reserved(v) => v & 0x07,
        }
    }

    /// Human-readable spec name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::NotRunning => "not running",
            Self::StartsInAFewSeconds => "starts in a few seconds",
            Self::Pausing => "pausing",
            Self::Running => "running",
            Self::ServiceOffAir => "service off-air",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(RunningStatus, Reserved);

#[cfg(test)]
mod running_status_tests {
    use super::*;

    #[test]
    fn from_u8_maps_known_values() {
        assert_eq!(RunningStatus::from_u8(0), RunningStatus::Undefined);
        assert_eq!(RunningStatus::from_u8(1), RunningStatus::NotRunning);
        assert_eq!(
            RunningStatus::from_u8(2),
            RunningStatus::StartsInAFewSeconds
        );
        assert_eq!(RunningStatus::from_u8(3), RunningStatus::Pausing);
        assert_eq!(RunningStatus::from_u8(4), RunningStatus::Running);
        assert_eq!(RunningStatus::from_u8(5), RunningStatus::ServiceOffAir);
        assert_eq!(RunningStatus::from_u8(6), RunningStatus::Reserved(6));
        assert_eq!(RunningStatus::from_u8(7), RunningStatus::Reserved(7));
    }

    #[test]
    fn to_u8_from_u8_round_trips_all_byte_values() {
        for v in 0u8..=0xFFu8 {
            assert_eq!(RunningStatus::to_u8(RunningStatus::from_u8(v)), v & 0x07);
        }
    }

    #[test]
    fn name_returns_known_strings() {
        assert_eq!(RunningStatus::Undefined.name(), "undefined");
        assert_eq!(RunningStatus::NotRunning.name(), "not running");
        assert_eq!(RunningStatus::Running.name(), "running");
        assert_eq!(RunningStatus::ServiceOffAir.name(), "service off-air");
        assert_eq!(RunningStatus::Reserved(6).name(), "reserved");
    }

    #[test]
    fn running_status_wire_to_name() {
        assert_eq!(RunningStatus::from_u8(4).name(), "running");
        assert_eq!(RunningStatus::from_u8(2).name(), "starts in a few seconds");
        assert_eq!(RunningStatus::from_u8(0).name(), "undefined");
    }
}

/// Byte 1 flags nibble for MPEG-2 PSI long-form sections.
///
/// Layout: `section_syntax_indicator(1) | '0'(1) | reserved(2)`.
/// Per ISO/IEC 13818-1 §2.4.4.10, the second bit is a spec-mandated
/// zero in PSI tables (PAT, PMT, CAT, TSDT, DSM-CC).
#[cfg(test)]
pub(crate) const SECTION_B1_FLAGS_PSI: u8 = 0xB0;

/// Byte 1 flags nibble for EN 300 468 (DVB) long-form sections.
///
/// Layout: `section_syntax_indicator(1) | reserved_future_use(1) | reserved(2)`.
/// Per ETSI EN 300 468 §5.1.1, the top nibble must be `F` — all four
/// bits set (SSI=1, rfu=1, reserved=11).
#[cfg(test)]
pub(crate) const SECTION_B1_FLAGS_DVB: u8 = 0xF0;

/// `section_syntax_indicator` bit in long-form section byte 1.
pub(crate) const SECTION_B1_SSI: u8 = 0x80;

/// `private_indicator` / `reserved_future_use` bit in section byte 1 (bit 6;
/// ISO/IEC 13818-1 §2.4.4.10, ETSI EN 300 468 §5.1.1).
pub(crate) const SECTION_B1_PRIVATE_INDICATOR: u8 = 0x40;

/// The non-length header bits of a section that are not decoded fields:
/// `private_indicator` / `reserved_future_use` (byte 1 bit 6), `reserved(2)`
/// (byte 1 bits `[5:4]`) and, for long-form sections, the `reserved(2)` in
/// front of `version_number` (byte 5 bits `[7:6]`).
///
/// See the module docs for the policy and
/// `dvb-si/docs/reserved-bits-edition-audit.md` §3.6 for the per-edition
/// evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SectionBits {
    /// Byte 1 bit 7: `section_syntax_indicator`. Spec value `1` for every
    /// long-form section (the default of [`psi`](Self::psi) /
    /// [`dvb`](Self::dvb)), `0` for the short-form TDT/RST/DIT/TOT
    /// ([`short_form`](Self::short_form)); a received section keeps whatever
    /// it carried (parse tolerant, serialize preserves).
    pub section_syntax_indicator: bool,
    /// Byte 1 bit 6: `private_indicator` (ISO/IEC 13818-1 §2.4.4.10, user
    /// definable) = `reserved_future_use` (ETSI EN 300 468 §5.1.1).
    pub private_indicator: bool,
    /// Byte 1 bits `[5:4]`: `reserved(2)`.
    pub reserved_b1: u8,
    /// Byte 5 bits `[7:6]`: `reserved(2)` before `version_number` (long-form
    /// sections only; unused by short-form sections).
    pub reserved_b5: u8,
}

/// Value of an all-ones 2-bit reserved field.
pub(crate) const RESERVED_2: u8 = 0x03;

impl SectionBits {
    /// MPEG-2 PSI sections (PAT/PMT/CAT/TSDT, ISO/IEC 13818-1 §2.4.4): the
    /// bit after `section_syntax_indicator` is the literal `'0'`.
    #[must_use]
    pub const fn psi() -> Self {
        Self {
            section_syntax_indicator: true,
            private_indicator: false,
            reserved_b1: RESERVED_2,
            reserved_b5: RESERVED_2,
        }
    }

    /// ETSI EN 300 468 (DVB) sections: `reserved_future_use` = `1` after
    /// `section_syntax_indicator` (§5.1.1).
    #[must_use]
    pub const fn dvb() -> Self {
        Self {
            section_syntax_indicator: true,
            private_indicator: true,
            reserved_b1: RESERVED_2,
            reserved_b5: RESERVED_2,
        }
    }

    /// The same bits with `section_syntax_indicator` = `0`: the short-form
    /// sections (TDT, TOT, RST, DIT; EN 300 468 §5.2.5-§5.2.6, §7.2.2).
    #[must_use]
    pub const fn short_form(mut self) -> Self {
        self.section_syntax_indicator = false;
        self
    }

    /// Read the bits from byte 1 (and byte 5 for long-form sections; pass
    /// `None` for short-form sections, which take the all-ones fill).
    #[must_use]
    pub(crate) fn parse(b1: u8, b5: Option<u8>) -> Self {
        Self {
            section_syntax_indicator: b1 & SECTION_B1_SSI != 0,
            private_indicator: b1 & B1_PRIVATE_INDICATOR != 0,
            reserved_b1: (b1 >> B1_RESERVED_SHIFT) & RESERVED_2,
            reserved_b5: b5.map_or(RESERVED_2, |b| b >> B5_RESERVED_SHIFT),
        }
    }

    /// Byte 1 flags (everything above `section_length[11:8]`).
    pub(crate) fn b1(&self) -> crate::Result<u8> {
        b1_of(
            self.section_syntax_indicator,
            self.private_indicator,
            self.reserved_b1,
        )
    }

    /// Byte 5: `reserved(2) | version_number(5) | current_next_indicator(1)`.
    pub(crate) fn b5(&self, version_number: u8, current_next_indicator: bool) -> crate::Result<u8> {
        b5_of(self.reserved_b5, version_number, current_next_indicator)
    }
}

/// Assemble section byte 1 (everything above `section_length[11:8]`) from its
/// parts, for tables that keep `private_indicator` / `reserved_b1` as their
/// own fields.
///
/// An over-range `reserved_b1` is a typed `FieldOverflow`, never masked.
pub(crate) fn b1_of(
    section_syntax_indicator: bool,
    private_indicator: bool,
    reserved_b1: u8,
) -> crate::Result<u8> {
    Ok((u8::from(section_syntax_indicator) * SECTION_B1_SSI)
        | (u8::from(private_indicator) * B1_PRIVATE_INDICATOR)
        | (broadcast_common::len::fit_mask(reserved_b1, RESERVED_2, "reserved")?
            << B1_RESERVED_SHIFT))
}

/// Assemble the long-form byte 5 with a caller-supplied `reserved(2)`.
///
/// An over-range `reserved_b5` or `version_number` is a typed
/// `FieldOverflow`, never masked into a different valid-looking section.
pub(crate) fn b5_of(
    reserved_b5: u8,
    version_number: u8,
    current_next_indicator: bool,
) -> crate::Result<u8> {
    Ok(
        (broadcast_common::len::fit_mask(reserved_b5, RESERVED_2, "reserved")?
            << B5_RESERVED_SHIFT)
            | (broadcast_common::len::fit_mask(version_number, B5_VERSION_MASK, "version_number")?
                << 1)
            | u8::from(current_next_indicator),
    )
}

/// The `reserved(2)` bits in section byte 1 (`[5:4]`).
#[must_use]
pub(crate) fn b1_reserved_of(b1: u8) -> u8 {
    (b1 >> B1_RESERVED_SHIFT) & RESERVED_2
}

/// The `reserved(2)` bits in the long-form version byte (`[7:6]`).
#[must_use]
pub(crate) fn b5_reserved_of(b5: u8) -> u8 {
    b5 >> B5_RESERVED_SHIFT
}

impl Default for SectionBits {
    /// The EN 300 468 fill ([`SectionBits::dvb`]); PSI tables default to
    /// [`SectionBits::psi`] in their own `Default`.
    fn default() -> Self {
        Self::dvb()
    }
}

/// `private_indicator` mask in section byte 1 (bit 6).
const B1_PRIVATE_INDICATOR: u8 = 0x40;
/// Shift of the `reserved(2)` in section byte 1 (bits `[5:4]`).
const B1_RESERVED_SHIFT: u32 = 4;
/// Shift of the `reserved(2)` in the version byte (bits `[7:6]`).
const B5_RESERVED_SHIFT: u32 = 6;

/// Validate a section_length field and compute the total encoded length.
///
/// Returns `total` (= `header_len + section_length`) on success, or
/// `Err(SectionLengthOverflow)` when the declared `section_length` would
/// make `total` smaller than `min_total` or larger than `bytes_len`.
///
/// Every table's `Parse` implementation should call this immediately after
/// extracting `section_length` from bytes 1-2, passing the appropriate
/// constants for that table type.
pub(crate) fn check_section_length(
    bytes_len: usize,
    header_len: usize,
    section_length: usize,
    min_total: usize,
) -> crate::Result<usize> {
    let total = header_len + section_length;
    if bytes_len < total || total < min_total {
        return Err(crate::error::Error::SectionLengthOverflow {
            declared: section_length,
            available: bytes_len.saturating_sub(header_len),
        });
    }
    Ok(total)
}

/// Length of the CRC-32 trailer of a long-form section (ISO/IEC 13818-1
/// Annex A); one definition for every table module.
pub(crate) const CRC_LEN: usize = 4;

/// A trailing loop whose declared length is the last field before the CRC
/// (NIT/BAT `transport_stream_loop_length`, AIT `application_loop_length`) must
/// end exactly at the CRC. A shorter declared length leaves bytes that belong
/// to no field, which a parse would accept and a serialize would silently
/// drop; reject them (never a byte-identical round trip otherwise).
pub(crate) fn check_loop_reaches_crc(
    loop_end: usize,
    crc_pos: usize,
    what: &'static str,
) -> crate::Result<()> {
    if loop_end != crc_pos {
        return Err(crate::error::Error::BufferTooShort {
            need: crc_pos.saturating_sub(loop_end),
            have: 0,
            what,
        });
    }
    Ok(())
}

/// Write the MPEG-2 CRC-32 trailer of a long-form section: the CRC of
/// `buf[..crc_pos]` goes into `buf[crc_pos..crc_pos + 4]`
/// (ISO/IEC 13818-1 Annex A / ETSI EN 300 468 §5.1.1). The one definition of
/// "serializer ends by sealing the section", previously repeated in about 25
/// table modules (audit r02-W10).
///
/// The caller has already checked `buf.len() >= crc_pos + CRC_LEN`.
pub(crate) fn write_crc(buf: &mut [u8], crc_pos: usize) {
    let crc = broadcast_common::crc32_mpeg2::compute(&buf[..crc_pos]);
    buf[crc_pos..crc_pos + CRC_LEN].copy_from_slice(&crc.to_be_bytes());
}

/// Write a checked 12-bit `section_length` into `buf[1..3]`, preserving
/// whatever flag bits the caller already wrote into the top nibble of
/// `buf[1]` (set `buf[1]` to the section's flags constant, bottom nibble
/// zero, immediately before calling this).
///
/// `body_len` is the value carried in `section_length`: the byte count
/// from just after this field to the end of the section, CRC included
/// where present (ISO/IEC 13818-1 §2.4.4.10 / ETSI EN 300 468 §5.1.1).
/// Returns `Err(FieldOverflow)` instead of silently wrapping when
/// `body_len` exceeds the 12-bit field (max 4095) — see #1129: every
/// table serializer must route its outer `section_length` write through
/// this one helper so the guard cannot be forgotten again.
pub(crate) fn write_section_length(buf: &mut [u8], body_len: usize) -> crate::Result<()> {
    let fitted = broadcast_common::len::fit_bits(body_len as u64, 12, "section_length")?;
    buf[1] = (buf[1] & 0xF0) | ((fitted >> 8) as u8);
    buf[2] = fitted as u8;
    Ok(())
}

// ── Shared section-header bit fields (r02-W16) ──────────────────────────────
//
// The same masks recurred as bare literals in every table parser and
// serializer (`bytes[1] & 0x0F`, `0xC0 | ((ver & 0x1F) << 1) | cni`,
// `0xE0 | (pid >> 8)`, `0xF0 | (len >> 8)`). They live here once now, with
// the same "route every write through the helper so the guard cannot be
// forgotten" rationale as [`write_section_length`] (#1129).

/// Low 4 bits of long-form section byte 1: the high nibble of the 12-bit
/// `section_length`.
pub(crate) const SECTION_LENGTH_HI_MASK: u8 = 0x0F;

/// `version_number` field mask in the extension-header flags byte (byte 5):
/// bits `[5:1]`.
pub(crate) const B5_VERSION_MASK: u8 = 0x1F;

/// `current_next_indicator` bit in the extension-header flags byte (byte 5):
/// bit `[0]`.
pub(crate) const B5_CNI: u8 = 0x01;

/// `pid(13)` field mask inside a PID's first wire byte (the 5 low bits).
pub(crate) const PID_LO_MASK: u8 = 0x1F;

/// The four `reserved = '1111'` bits that precede a 12-bit descriptor-loop
/// length (ISO/IEC 13818-1 §2.4.4.9, ETSI EN 300 468 §5.1): bits `[7:4]` of
/// the length's first wire byte.
pub(crate) const DESC_LOOP_LEN_HI_RESERVED_BITS: u8 = 0xF0;

/// `running_status(3)` mask inside a status/length byte: bits `[7:5]`
/// (ETSI EN 300 468 §5.2.3/§5.2.4 service and event loops).
pub(crate) const RUNNING_STATUS_MASK: u8 = 0x07;

/// `free_CA_mode` bit inside a status/length byte: bit `[4]`
/// (ETSI EN 300 468 §5.2.3/§5.2.4 service and event loops).
pub(crate) const FREE_CA_MODE: u8 = 0x10;

/// Decode the `running_status(3)` field from a status/length byte.
#[must_use]
pub(crate) fn running_status_of(status_and_len_hi: u8) -> u8 {
    (status_and_len_hi >> 5) & RUNNING_STATUS_MASK
}

/// Decode the `free_CA_mode(1)` field from a status/length byte.
#[must_use]
pub(crate) fn free_ca_mode_of(status_and_len_hi: u8) -> bool {
    (status_and_len_hi & FREE_CA_MODE) != 0
}

/// Decode the low 8 bits of `descriptors_loop_length` carried in the low
/// nibble of a status/length byte, combined with its length-low byte.
#[must_use]
pub(crate) fn dll_from_status(status_and_len_hi: u8, lo: u8) -> usize {
    (((status_and_len_hi & SECTION_LENGTH_HI_MASK) as usize) << 8) | usize::from(lo)
}

/// Assemble the SDT/EIT `running_status(3) | free_CA_mode(1) |
/// descriptors_loop_length[11:8]` status byte from typed fields.
#[must_use]
pub(crate) fn status_byte(running_status: u8, free_ca_mode: bool, dll_hi: u8) -> u8 {
    ((running_status & RUNNING_STATUS_MASK) << 5)
        | (u8::from(free_ca_mode) * FREE_CA_MODE)
        | (dll_hi & SECTION_LENGTH_HI_MASK)
}

/// Assemble the extension-header flags byte (byte 5) of a long-form section:
/// `reserved(2)='11' | version_number(5) | current_next_indicator(1)`.
///
/// A version wider than the 5-bit field is a [`FieldOverflow`], never wrapped.
#[cfg(test)]
pub(crate) fn version_byte(version_number: u8, current_next_indicator: bool) -> u8 {
    b5_of(RESERVED_2, version_number, current_next_indicator).expect("test version fits 5 bits")
}

/// Decode the 12-bit `section_length` from bytes 1–2 of a section header.
#[must_use]
pub(crate) fn section_length_of(bytes: &[u8]) -> usize {
    ((usize::from(bytes[1] & SECTION_LENGTH_HI_MASK)) << 8) | usize::from(bytes[2])
}

/// Decode the `version_number` from the extension-header flags byte (byte 5).
#[must_use]
pub(crate) fn version_number_of(flags_byte: u8) -> u8 {
    (flags_byte >> 1) & B5_VERSION_MASK
}

/// Decode the `current_next_indicator` from the extension-header flags byte
/// (byte 5).
#[must_use]
pub(crate) fn current_next_of(flags_byte: u8) -> bool {
    (flags_byte & B5_CNI) != 0
}

/// Decode a 13-bit PID from its two wire bytes.
#[must_use]
pub(crate) fn pid_of(hi: u8, lo: u8) -> mpeg_ts::pid::Pid {
    mpeg_ts::pid::Pid::new(((u16::from(hi & PID_LO_MASK)) << 8) | u16::from(lo))
}

/// Write a 13-bit PID carrying the three `reserved='111'` bits.
#[cfg(test)]
pub(crate) fn write_pid(buf: &mut [u8], pid: mpeg_ts::pid::Pid) {
    write_pid_r(buf, pid, RESERVED_3).expect("RESERVED_3 fits 3 bits");
}

/// All-ones `reserved(3)` fill (ISO/IEC 13818-1 §2.4.4.8 etc.).
pub const RESERVED_3: u8 = 0x07;
/// All-ones `reserved(4)` / `reserved_future_use(4)` fill.
pub const RESERVED_4: u8 = 0x0F;
/// All-ones `reserved_future_use(5)` fill (RST entry, MPE-FEC section).
pub(crate) const RESERVED_5: u8 = 0x1F;
/// All-ones `reserved_future_use(6)` fill (SDT service entry).
pub(crate) const RESERVED_6: u8 = 0x3F;
/// Shift of the `reserved(3)` above a 13-bit PID.
const PID_RESERVED_SHIFT: u32 = 5;
/// Shift of the `reserved(4)` above a 12-bit length.
const LEN_RESERVED_SHIFT: u32 = 4;

/// The `reserved(3)` bits above a 13-bit PID's first wire byte.
#[must_use]
pub(crate) fn pid_reserved_of(hi: u8) -> u8 {
    hi >> PID_RESERVED_SHIFT
}

/// Write a 13-bit PID with a caller-supplied `reserved(3)`.
pub(crate) fn write_pid_r(
    buf: &mut [u8],
    pid: mpeg_ts::pid::Pid,
    reserved: u8,
) -> crate::Result<()> {
    let pid = pid.value();
    buf[0] = (broadcast_common::len::fit_mask(reserved, RESERVED_3, "reserved")?
        << PID_RESERVED_SHIFT)
        | ((pid >> 8) as u8 & PID_LO_MASK);
    buf[1] = (pid & 0xFF) as u8;
    Ok(())
}

/// The `reserved(4)` bits above a 12-bit length's first wire byte.
#[must_use]
pub(crate) fn len_reserved_of(hi: u8) -> u8 {
    hi >> LEN_RESERVED_SHIFT
}

/// Write a checked 12-bit length with a caller-supplied `reserved(4)`.
pub(crate) fn write_desc_loop_len_r(buf: &mut [u8], len: usize, reserved: u8) -> crate::Result<()> {
    let fitted = broadcast_common::len::fit_bits(len as u64, 12, "descriptors_length")?;
    buf[0] = (broadcast_common::len::fit_mask(reserved, RESERVED_4, "reserved")?
        << LEN_RESERVED_SHIFT)
        | ((fitted >> 8) as u8);
    buf[1] = fitted as u8;
    Ok(())
}

/// Decode a 12-bit descriptor-loop length from its two wire bytes (the top
/// four bits of `hi` are reserved and masked off).
#[must_use]
pub(crate) fn desc_loop_len_of(hi: u8, lo: u8) -> usize {
    (((hi & !DESC_LOOP_LEN_HI_RESERVED_BITS) as usize) << 8) | usize::from(lo)
}

/// Mask of the low nibble of a byte: a sub-byte field's width when the byte
/// also carries reserved bits above it (r02-W16).
pub(crate) const LOW_NIBBLE_MASK: u8 = 0x0F;

/// Push the two `SECTION_B1_FLAGS_* | section_length` header bytes of a
/// section; test/fixture helper counterpart of [`write_section_length`].
#[cfg(test)]
pub(crate) fn push_section_header(v: &mut Vec<u8>, flags: u8, len: usize) {
    v.push(flags | ((len >> 8) as u8 & SECTION_LENGTH_HI_MASK));
    v.push((len & 0xFF) as u8);
}

/// Push the two `reserved(4) | descriptors_loop_length(12)` bytes of a
/// descriptor loop length; test/fixture helper counterpart of
/// [`write_desc_loop_len_r`].
#[cfg(test)]
pub(crate) fn push_desc_loop_len(v: &mut Vec<u8>, len: usize) {
    v.push(DESC_LOOP_LEN_HI_RESERVED_BITS | ((len >> 8) as u8 & SECTION_LENGTH_HI_MASK));
    v.push((len & 0xFF) as u8);
}

/// Push the SDT/EIT service/event `running_status | free_CA_mode | len_hi`
/// status byte plus the length's low byte; test/fixture helper counterpart
/// of [`status_byte`].
#[cfg(test)]
pub(crate) fn push_status_and_len(v: &mut Vec<u8>, running_status: u8, free_ca: bool, len: usize) {
    v.push(status_byte(running_status, free_ca, (len >> 8) as u8));
    v.push((len & 0xFF) as u8);
}

pub mod any;
pub use any::AnyTableSection;

pub mod registry;
pub use registry::{TableObject, TableRegistry};

/// Shared `real_time_parameters(32)` bit codec used by MPE-FEC and MPE-IFEC
/// (r02-W22); the public field-named structs stay in each table's module.
mod real_time_parameters;

/// Shared target/operational descriptor-loop pair (INT body, UNT platform loop).
mod target_operational_loop;
pub use target_operational_loop::TargetOperationalLoop;

pub mod ait;
pub mod bat;
pub mod cat;
pub mod cit;
pub mod container;
pub mod dit;
pub mod downloadable_font_info;
pub mod dsmcc;
pub mod eit;
pub mod green_access_unit_section;
pub mod int;
pub mod iso14496_section;
pub mod metadata_section;
pub mod mpe;
pub mod mpe_fec;
pub mod mpe_ifec;
pub mod nit;
pub mod pat;
pub mod pmt;
pub mod protection_message;
pub mod rct;
pub mod rnt;
pub mod rst;
pub mod sat;
pub mod sdt;
pub mod sit;
pub mod st;
pub mod tdt;
pub mod tot;
pub mod tsdt;
pub mod unt;
