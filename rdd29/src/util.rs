//! Shared bit-level parse/serialize helpers used across element modules.

use broadcast_common::bits::{BitReader, BitWriter};

use crate::error::{BitResultExt, Error, Result};

/// Read and validate a "Reserved (set to `expected`)" field. RDD 29 gives an
/// explicit literal value for every reserved field it defines (unlike, e.g.,
/// `st337`'s "reserved for future use" `Pf`), so every one of them is
/// hard-validated here rather than round-tripped as an unexamined raw value
/// — see `docs/rdd29.md` scope decision 4.
///
/// # Errors
/// [`Error::InvalidReserved`] if the field's value does not equal `expected`.
pub(crate) fn read_reserved(
    r: &mut BitReader<'_>,
    width: u32,
    expected: u64,
    field: &'static str,
) -> Result<()> {
    let found = r.read_bits(width).ctx(field)?;
    if found != expected {
        return Err(Error::InvalidReserved {
            field,
            expected,
            found,
        });
    }
    Ok(())
}

/// Write a "Reserved (set to `expected`)" field's documented literal value.
pub(crate) fn write_reserved(
    w: &mut BitWriter<'_>,
    width: u32,
    expected: u64,
    field: &'static str,
) -> Result<()> {
    w.write_bits(expected, width).ctx(field)
}

/// Assert a body [`BitReader`] has been fully consumed (no trailing bytes
/// left unaccounted for) — a cheap but real spec-fidelity check that the
/// element's declared `ElementSize`/`DLCSize` was fully, correctly parsed.
pub(crate) fn expect_fully_consumed(r: &BitReader<'_>, what: &'static str) -> Result<()> {
    let remaining = r.bits_remaining();
    if remaining != 0 {
        return Err(Error::InvalidValue {
            field: what,
            value: remaining as u64,
            reason: "trailing unparsed bits remain in the element body",
        });
    }
    Ok(())
}
