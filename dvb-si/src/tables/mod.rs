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
    pub fn to_u8(self) -> u8 {
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

pub mod any;
pub use any::AnyTableSection;

pub mod registry;
pub use registry::{TableObject, TableRegistry};

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
