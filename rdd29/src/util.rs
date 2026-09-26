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

/// Read the `AlignBits` byte-alignment padding at the reader's current
/// position, returning its raw value (`0` if already aligned).
///
/// Unlike RDD 29's other reserved fields, `AlignBits` carries no documented
/// "(set to X)" value anywhere in the syntax tables (it is genuinely
/// "VARIABLE" width with no stated content) — the same "no fixed value
/// given" category as `st337`'s `Pf` (`docs/rdd29.md` scope decision 4's own
/// contrast). So this crate preserves it verbatim rather than discarding it
/// (RD-W2, #1114): forcing it to `0` on serialize would silently corrupt a
/// real producer's padding value, and byte-exact round-trip requires
/// capturing whatever was actually there.
pub(crate) fn read_align_bits(r: &mut BitReader<'_>, field: &'static str) -> Result<u8> {
    let pad = (8 - r.bits_read() % 8) % 8;
    if pad == 0 {
        return Ok(0);
    }
    Ok(r.read_bits(pad as u32).ctx(field)? as u8)
}

/// Write `align_bits` as the `AlignBits` byte-alignment padding at the
/// writer's current position.
///
/// # Errors
/// [`Error::InvalidValue`] if `align_bits` does not fit in the number of
/// padding bits actually needed here (it may differ, e.g. `0` bits, from
/// whatever context `align_bits` was originally read in).
pub(crate) fn write_align_bits(
    w: &mut BitWriter<'_>,
    align_bits: u8,
    field: &'static str,
) -> Result<()> {
    let pad = (8 - w.bits_written() % 8) % 8;
    if pad == 0 {
        return if align_bits == 0 {
            Ok(())
        } else {
            Err(Error::InvalidValue {
                field,
                value: u64::from(align_bits),
                reason: "no padding bits are needed here, but align_bits is nonzero",
            })
        };
    }
    let max = (1u16 << pad) - 1;
    if u16::from(align_bits) > max {
        return Err(Error::InvalidValue {
            field,
            value: u64::from(align_bits),
            reason: "does not fit in the number of padding bits needed here",
        });
    }
    w.write_bits(u64::from(align_bits), pad as u32).ctx(field)
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
