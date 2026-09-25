//! Range-checked narrowing for wire length/count fields (see #1129).
//!
//! Serializers historically wrote lengths and counts with `as u8` / `as u16` /
//! bit-masks, which silently wrap: a 65 540-byte body became `section_length`
//! 4. These helpers make the range check impossible to forget and return
//! [`FieldOverflow`] instead, so the caller's serializer maps it into its own
//! error type rather than emitting a corrupt frame.

/// A value did not fit the wire field it is written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldOverflow {
    /// Spec name of the field, e.g. "section_length".
    pub field: &'static str,
    /// The value that did not fit.
    pub value: u64,
    /// The largest value the field can hold.
    pub max: u64,
}

impl core::fmt::Display for FieldOverflow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} = {} exceeds the field maximum {}",
            self.field, self.value, self.max
        )
    }
}

impl core::error::Error for FieldOverflow {}

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

/// `value` as a `u8`, or [`FieldOverflow`] with `max` 255.
pub fn fit_u8(value: usize, field: &'static str) -> Result<u8, FieldOverflow> {
    let fitted = fit_bits(u64::try_from(value).expect("usize fits in u64"), 8, field)?;
    Ok(u8::try_from(fitted).expect("a value that fits in 8 bits is a u8"))
}

/// `value` as a `u16`, or [`FieldOverflow`] with `max` 65 535.
pub fn fit_u16(value: usize, field: &'static str) -> Result<u16, FieldOverflow> {
    let fitted = fit_bits(u64::try_from(value).expect("usize fits in u64"), 16, field)?;
    Ok(u16::try_from(fitted).expect("a value that fits in 16 bits is a u16"))
}

/// 24-bit field (e.g. FLV/RTMP UI24), returned in a u32.
pub fn fit_u24(value: usize, field: &'static str) -> Result<u32, FieldOverflow> {
    let fitted = fit_bits(u64::try_from(value).expect("usize fits in u64"), 24, field)?;
    Ok(u32::try_from(fitted).expect("a value that fits in 24 bits is a u32"))
}

/// `value` as a `u32`, or [`FieldOverflow`] with `max` 4 294 967 295.
pub fn fit_u32(value: usize, field: &'static str) -> Result<u32, FieldOverflow> {
    let fitted = fit_bits(u64::try_from(value).expect("usize fits in u64"), 32, field)?;
    Ok(u32::try_from(fitted).expect("a value that fits in 32 bits is a u32"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
