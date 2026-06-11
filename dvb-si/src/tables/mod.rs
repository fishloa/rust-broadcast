//! SI + PSI table-section parsers.
//!
//! Each `*Section` type parses and serializes one wire section. Use
//! [`crate::collect`] to assemble complete logical tables that span multiple
//! sections.

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
