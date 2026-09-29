//! SI + PSI table-section parsers.
//!
//! Each `*Section` type parses and serializes one wire section. Use
//! [`crate::collect`] to assemble complete logical tables that span multiple
//! sections.

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
pub(crate) const SECTION_B1_FLAGS_PSI: u8 = 0xB0;

/// Byte 1 flags nibble for EN 300 468 (DVB) long-form sections.
///
/// Layout: `section_syntax_indicator(1) | reserved_future_use(1) | reserved(2)`.
/// Per ETSI EN 300 468 §5.1.1, the top nibble must be `F` — all four
/// bits set (SSI=1, rfu=1, reserved=11).
pub(crate) const SECTION_B1_FLAGS_DVB: u8 = 0xF0;

/// `section_syntax_indicator` bit in long-form section byte 1.
pub(crate) const SECTION_B1_SSI: u8 = 0x80;

/// Reserved bits `[5:4]` in long-form section byte 1, set to `11`.
pub(crate) const SECTION_B1_RESERVED_HI: u8 = 0x30;

/// Byte 1 flags nibble for short-form sections (no extension header, no CRC).
///
/// Layout: `section_syntax_indicator(0) | reserved_future_use(1) | reserved(11)`.
/// Top nibble `0b0111` = `0x70`. Used by RST, ST, DIT, TDT, TOT.
pub(crate) const SECTION_B1_FLAGS_SHORT: u8 = 0x70;

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

/// `reserved(2) = '11'` fill in the extension-header flags byte (byte 5):
/// bits `[7:6]` (ISO/IEC 13818-1 §2.4.4.10, ETSI EN 300 468 §5.1.1).
pub(crate) const B5_RESERVED_HI: u8 = 0xC0;

/// `version_number` field mask in the extension-header flags byte (byte 5):
/// bits `[5:1]`.
pub(crate) const B5_VERSION_MASK: u8 = 0x1F;

/// `current_next_indicator` bit in the extension-header flags byte (byte 5):
/// bit `[0]`.
pub(crate) const B5_CNI: u8 = 0x01;

/// `pid(13)` field mask inside a PID's first wire byte (the 5 low bits).
pub(crate) const PID_LO_MASK: u8 = 0x1F;

/// The three `reserved = '111'` bits that precede a 13-bit PID high byte
/// (ISO/IEC 13818-1 §2.4.4.8/§2.4.4.9: PAT `reserved`, PMT `pcr_pid`, ES
/// `elementary_pid`; ETSI EN 300 468 §5.3 CAT-style `ca_pid` uses the same
/// form): bits `[7:5]` of the PID's first wire byte.
pub(crate) const PID_HI_RESERVED_BITS: u8 = 0xE0;

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
/// The 5-bit version is masked defensively so a constructed value wider than
/// the field can never bleed into the reserved bits.
#[must_use]
pub(crate) fn version_byte(version_number: u8, current_next_indicator: bool) -> u8 {
    B5_RESERVED_HI | ((version_number & B5_VERSION_MASK) << 1) | u8::from(current_next_indicator)
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
pub(crate) fn pid_of(hi: u8, lo: u8) -> u16 {
    ((u16::from(hi & PID_LO_MASK)) << 8) | u16::from(lo)
}

/// Write a 13-bit PID carrying the three `reserved='111'` bits.
pub(crate) fn write_pid(buf: &mut [u8], pid: u16) {
    buf[0] = PID_HI_RESERVED_BITS | ((pid >> 8) as u8 & PID_LO_MASK);
    buf[1] = (pid & 0xFF) as u8;
}

/// Decode a 12-bit descriptor-loop length from its two wire bytes (the top
/// four bits of `hi` are reserved and masked off).
#[must_use]
pub(crate) fn desc_loop_len_of(hi: u8, lo: u8) -> usize {
    (((hi & !DESC_LOOP_LEN_HI_RESERVED_BITS) as usize) << 8) | usize::from(lo)
}

/// Write a 12-bit descriptor-loop length carrying the four
/// `reserved='1111'` bits; errors instead of silently wrapping (cf. #1129).
pub(crate) fn write_desc_loop_len(buf: &mut [u8], len: usize) -> crate::Result<()> {
    let fitted = broadcast_common::len::fit_bits(len as u64, 12, "descriptors_length")?;
    buf[0] = DESC_LOOP_LEN_HI_RESERVED_BITS | ((fitted >> 8) as u8);
    buf[1] = fitted as u8;
    Ok(())
}

/// Position of the `private_indicator(1)` bit within byte 1 of a section
/// whose flags byte is built from typed fields (private-section forms,
/// ISO/IEC 13818-1 §2.4.4.10 / ETSI EN 301 192 §8.4): bit `[6]`. Only the
/// shift is named — the bit is always written through
/// [`private_section_b1`], so a standalone mask constant would be dead code.
pub(crate) const B1_PRIVATE_INDICATOR_SHIFT: u32 = 6;

/// The two `reserved = '11'` bits that follow `private_indicator` in byte 1
/// of a private-section flags byte (ISO/IEC 13818-1 §2.4.4.10): bits
/// `[5:4]`.
pub(crate) const B1_PRIVATE_RESERVED: u8 = 0x30;

/// The five `reserved_future_use = '11111'` bits that precede a
/// `running_status(3)` field (ETSI EN 300 468 §5.4 RST entry): bits `[7:3]`.
pub(crate) const RESERVED_FUTURE_USE_5: u8 = 0xF8;

/// Mask of the low nibble of a byte: a sub-byte field's width when the byte
/// also carries reserved bits above it (r02-W16).
pub(crate) const LOW_NIBBLE_MASK: u8 = 0x0F;

/// Assemble byte 1 of a section that carries a `private_indicator` flag:
/// `section_syntax_indicator(1) | private_indicator(1) | reserved(2)='11' |
/// section_length[11:8]` (ISO/IEC 13818-1 §2.4.4.10 / ETSI EN 301 192
/// §8.4.4). `length_hi` is masked defensively so a wider value cannot bleed
/// into the reserved bits.
#[must_use]
pub(crate) fn private_section_b1(private_indicator: bool, length_hi: u8) -> u8 {
    SECTION_B1_SSI
        | (u8::from(private_indicator) << B1_PRIVATE_INDICATOR_SHIFT)
        | B1_PRIVATE_RESERVED
        | (length_hi & LOW_NIBBLE_MASK)
}

/// Push the two `SECTION_B1_FLAGS_* | section_length` header bytes of a
/// section; test/fixture helper counterpart of [`write_section_length`].
#[cfg(test)]
pub(crate) fn push_section_header(v: &mut Vec<u8>, flags: u8, len: usize) {
    v.push(flags | ((len >> 8) as u8 & SECTION_LENGTH_HI_MASK));
    v.push((len & 0xFF) as u8);
}

/// Push the two `reserved(4) | descriptors_loop_length(12)` bytes of a
/// descriptor loop length; test/fixture helper counterpart of
/// [`write_desc_loop_len`].
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
pub use real_time_parameters::TargetOperationalLoop;

pub mod ait;
pub mod bat;
pub mod cat;
pub mod cit;
pub mod container;
pub mod dit;
pub mod downloadable_font_info;
pub mod dsmcc;
pub mod eit;
pub mod int;
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
