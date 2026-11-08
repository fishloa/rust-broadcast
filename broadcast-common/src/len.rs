//! Range-checked narrowing for wire length/count fields (see #1129).
//!
//! Serializers historically wrote lengths and counts with `as u8` / `as u16` /
//! bit-masks, which silently wrap: a 65 540-byte body became `section_length`
//! 4. These helpers make the range check impossible to forget and return
//! [`FieldOverflow`] instead, so the caller's serializer maps it into its own
//! error type rather than emitting a corrupt frame.

pub use crate::error::{BufferTooShort, FieldOverflow, OutputBufferTooSmall};

/// `Ok` if `buf` holds at least `need` bytes, else [`BufferTooShort`] carrying
/// `what` (the parse-side pre-check).
#[inline]
pub fn need_in(buf: &[u8], need: usize, what: &'static str) -> Result<(), BufferTooShort> {
    let have = buf.len();
    if have < need {
        Err(BufferTooShort { need, have, what })
    } else {
        Ok(())
    }
}

/// `Ok` if `buf` can hold `need` bytes, else [`OutputBufferTooSmall`] (the
/// serialize-side pre-check).
#[inline]
pub fn need_out(buf: &[u8], need: usize) -> Result<(), OutputBufferTooSmall> {
    let have = buf.len();
    if have < need {
        Err(OutputBufferTooSmall { need, have })
    } else {
        Ok(())
    }
}

/// `value` if it fits in an unsigned field of `bits` bits, else `Err`.
///
/// The comparison happens in `u64` before any narrowing, so a value that an
/// `as` cast would silently truncate is reported instead. Panics if `bits` is
/// 0 or greater than 64: that is a programming error, not a wire condition.
pub fn fit_bits(value: u64, bits: u32, field: &'static str) -> Result<u64, FieldOverflow> {
    assert!(
        (1..=64).contains(&bits),
        "fit_bits: bits must be in 1..=64, got {bits}"
    );
    let max = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    if value <= max {
        Ok(value)
    } else {
        Err(FieldOverflow { field, value, max })
    }
}

/// `value` unchanged if every set bit lies inside `mask`, else
/// [`FieldOverflow`] (with `max` = `mask`).
///
/// This is the checked replacement for the `(value & MASK)` idiom in a
/// serializer: `& MASK` silently drops over-range high bits (a 4-bit field
/// given `0x13` is written as `3`), whereas this reports the overflow. For the
/// usual contiguous low-bit mask (`2^n - 1`) it is exactly [`fit_bits`]; the
/// value is returned in its own type so the packing expression around it is
/// unchanged.
pub fn fit_mask<T>(value: T, mask: T, field: &'static str) -> Result<T, FieldOverflow>
where
    T: Copy + Into<u64>,
{
    let v: u64 = value.into();
    let m: u64 = mask.into();
    if v & !m == 0 {
        Ok(value)
    } else {
        Err(FieldOverflow {
            field,
            value: v,
            max: m,
        })
    }
}

/// `value` as a `u8`, or [`FieldOverflow`] with `max` 255.
pub fn fit_u8(value: usize, field: &'static str) -> Result<u8, FieldOverflow> {
    // `usize` is at most 64 bits on every supported target, so the widening
    // cast is lossless; the narrowing one is guarded by `fit_bits`.
    let fitted = fit_bits(value as u64, 8, field)?;
    Ok(fitted as u8)
}

/// `value` as a `u16`, or [`FieldOverflow`] with `max` 65 535.
pub fn fit_u16(value: usize, field: &'static str) -> Result<u16, FieldOverflow> {
    // `usize` is at most 64 bits on every supported target, so the widening
    // cast is lossless; the narrowing one is guarded by `fit_bits`.
    let fitted = fit_bits(value as u64, 16, field)?;
    Ok(fitted as u16)
}

/// 24-bit field (e.g. FLV/RTMP UI24), returned in a u32.
pub fn fit_u24(value: usize, field: &'static str) -> Result<u32, FieldOverflow> {
    let fitted = fit_bits(value as u64, 24, field)?;
    Ok(fitted as u32)
}

/// `value` as a `u32`, or [`FieldOverflow`] with `max` 4 294 967 295.
pub fn fit_u32(value: usize, field: &'static str) -> Result<u32, FieldOverflow> {
    // `usize` is at most 64 bits on every supported target, so the widening
    // cast is lossless; the narrowing one is guarded by `fit_bits`.
    let fitted = fit_bits(value as u64, 32, field)?;
    Ok(fitted as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn need_in_boundaries() {
        assert_eq!(need_in(&[0; 4], 4, "x"), Ok(()));
        assert_eq!(need_in(&[0; 5], 4, "x"), Ok(()));
        assert_eq!(need_in(&[], 0, "x"), Ok(()));
        assert_eq!(
            need_in(&[0; 3], 4, "pat"),
            Err(BufferTooShort {
                need: 4,
                have: 3,
                what: "pat"
            })
        );
    }

    #[test]
    fn need_out_boundaries() {
        let b = [0u8; 4];
        assert_eq!(need_out(&b, 4), Ok(()));
        assert_eq!(need_out(&b, 0), Ok(()));
        assert_eq!(
            need_out(&b, 5),
            Err(OutputBufferTooSmall { need: 5, have: 4 })
        );
    }

    #[test]
    fn displays_match_the_crate_error_texts() {
        assert_eq!(
            BufferTooShort {
                need: 4,
                have: 3,
                what: "pat"
            }
            .to_string(),
            "buffer too short: need 4 bytes, have 3 (while parsing pat)"
        );
        assert_eq!(
            OutputBufferTooSmall { need: 5, have: 4 }.to_string(),
            "output buffer too small: need 5, have 4"
        );
    }

    #[test]
    fn fit_mask_accepts_in_range_and_rejects_over_range() {
        assert_eq!(fit_mask(0x0Fu8, 0x0F, "x"), Ok(0x0F));
        assert_eq!(fit_mask(0u8, 0x0F, "x"), Ok(0));
        assert_eq!(
            fit_mask(0x13u8, 0x0F, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 0x13,
                max: 0x0F
            })
        );
        assert_eq!(fit_mask(0x3FFu16, 0x3FF, "lcn"), Ok(0x3FF));
        assert!(matches!(
            fit_mask(0x400u16, 0x3FF, "lcn"),
            Err(FieldOverflow { .. })
        ));
        // Non-contiguous mask: every set bit must lie inside it.
        assert!(matches!(
            fit_mask(0b0100u8, 0b1010, "x"),
            Err(FieldOverflow { .. })
        ));
        assert_eq!(fit_mask(0b1010u8, 0b1010, "x"), Ok(0b1010));
    }

    #[test]
    fn fit_u8_boundaries() {
        assert_eq!(fit_u8(255, "x"), Ok(255));
        assert_eq!(
            fit_u8(256, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 256,
                max: 255
            })
        );
    }

    #[test]
    fn fit_u16_boundaries() {
        assert_eq!(fit_u16(65_535, "x"), Ok(65_535));
        assert_eq!(
            fit_u16(65_536, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 65_536,
                max: 65_535
            })
        );
    }

    #[test]
    fn fit_u24_boundaries() {
        assert_eq!(fit_u24(0xFF_FFFF, "x"), Ok(0xFF_FFFF));
        assert_eq!(
            fit_u24(0x100_0000, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 0x100_0000,
                max: 0xFF_FFFF
            })
        );
    }

    #[test]
    fn fit_u32_boundaries() {
        assert_eq!(fit_u32(u32::MAX as usize, "x"), Ok(u32::MAX));
        assert_eq!(
            fit_u32(u32::MAX as usize + 1, "x"),
            Err(FieldOverflow {
                field: "x",
                value: u64::from(u32::MAX) + 1,
                max: u64::from(u32::MAX)
            })
        );
    }

    #[test]
    fn fit_bits_arbitrary_widths() {
        assert_eq!(fit_bits(4095, 12, "x"), Ok(4095));
        assert_eq!(
            fit_bits(4096, 12, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 4096,
                max: 4095
            })
        );
        assert_eq!(fit_bits(63, 6, "x"), Ok(63));
        assert_eq!(
            fit_bits(64, 6, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 64,
                max: 63
            })
        );
        assert_eq!(fit_bits(u64::MAX, 64, "x"), Ok(u64::MAX));
        assert_eq!(fit_bits(1, 1, "x"), Ok(1));
        assert_eq!(
            fit_bits(2, 1, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 2,
                max: 1
            })
        );
    }

    /// The bug class these helpers exist to prevent: `as u16` silently wraps
    /// 65 540 down to 4, emitting a corrupt length. `fit_u16` rejects it.
    #[test]
    fn fit_u16_rejects_65540_that_as_u16_would_wrap_to_4() {
        assert_eq!(65540usize as u16, 4);
        assert_eq!(
            fit_u16(65_540, "x"),
            Err(FieldOverflow {
                field: "x",
                value: 65_540,
                max: 65_535
            })
        );
    }

    #[test]
    #[should_panic(expected = "bits must be in 1..=64")]
    fn fit_bits_rejects_zero_bits() {
        let _ = fit_bits(1, 0, "x");
    }

    #[test]
    #[should_panic(expected = "bits must be in 1..=64")]
    fn fit_bits_rejects_65_bits() {
        let _ = fit_bits(1, 65, "x");
    }

    #[test]
    fn display_text_is_exact() {
        let err = FieldOverflow {
            field: "section_length",
            value: 4096,
            max: 4095,
        };
        assert_eq!(
            err.to_string(),
            "section_length = 4096 exceeds the field maximum 4095"
        );
    }
}
