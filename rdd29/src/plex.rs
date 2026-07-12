//! `Plex(n)` variable-length integer coding — RDD 29:2019 §3.4.
//!
//! See `docs/rdd29.md` §3.4 for the full transcription and "Scope decisions"
//! #1 for why this implementation escalates on "the field is all-ones for
//! its own current width" rather than the literally-printed (and internally
//! inconsistent) `Plex(8)` pseudocode.

use broadcast_common::bits::{BitReader, BitWriter};

use crate::error::{BitResultExt, Error, Result};

/// Largest value a `Plex`-coded symbol may hold (RDD 29 §3.4: "symbols to be
/// Plex encoded shall have a value less than or equal to `0xFFFFFFFE`").
pub const PLEX_MAX_VALUE: u64 = 0xFFFF_FFFE;

/// Read a `Plex(start_width)`-coded unsigned integer.
///
/// Reads `start_width` bits; if the field reads as all-ones, reads
/// `2 * start_width` bits in its place; if *that* field is also all-ones,
/// reads `4 * start_width` bits (terminal: no further escalation is
/// defined, matching the `PLEX_MAX_VALUE` cap — the one unrepresentable
/// 32-bit value is `0xFFFF_FFFF` itself).
pub(crate) fn read_plex(
    r: &mut BitReader<'_>,
    start_width: u32,
    field: &'static str,
) -> Result<u64> {
    let mut width = start_width;
    loop {
        let value = r.read_bits(width).ctx(field)?;
        let all_ones = all_ones_for_width(width);
        if value != all_ones {
            return Ok(value);
        }
        if width >= 32 {
            return Err(Error::PlexUnrepresentable { field });
        }
        width *= 2;
    }
}

/// Write `value` as a `Plex(start_width)`-coded symbol, using the smallest
/// container width that can represent it (escalating through escape codes
/// as needed), per RDD 29 §3.4's "smallest container possible" rule.
pub(crate) fn write_plex(
    w: &mut BitWriter<'_>,
    value: u64,
    start_width: u32,
    field: &'static str,
) -> Result<()> {
    if value > PLEX_MAX_VALUE {
        return Err(Error::InvalidValue {
            field,
            value,
            reason: "exceeds the Plex-encodable maximum 0xFFFFFFFE",
        });
    }
    let mut width = start_width;
    loop {
        let all_ones = all_ones_for_width(width);
        // Largest value this width can carry directly (all-ones is reserved
        // as the escape code, so the direct range tops out one below it).
        let max_direct = all_ones - 1;
        if width >= 32 || value <= max_direct {
            w.write_bits(value, width).ctx(field)?;
            return Ok(());
        }
        w.write_bits(all_ones, width).ctx(field)?;
        width *= 2;
    }
}

/// Total bits a `Plex(start_width)`-coded `value` will occupy on the wire.
///
/// Each escalation level writes its own full `width` bits — either an
/// escape code (if escalating further) or the final value — so the total is
/// the *sum* of every width visited, not just the terminal level's width
/// (e.g. `Plex(8)` value `0x1234` is `8 + 16 = 24` bits: the `0xFF` escape
/// byte plus the 16-bit value, matching the spec's own `0xFF1234` example).
pub(crate) fn plex_bits(value: u64, start_width: u32) -> u32 {
    let mut width = start_width;
    let mut total = 0u32;
    loop {
        total += width;
        let all_ones = all_ones_for_width(width);
        if width >= 32 || value < all_ones {
            return total;
        }
        width *= 2;
    }
}

fn all_ones_for_width(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: u64, start_width: u32) {
        let bits = plex_bits(value, start_width);
        let bytes_needed = bits.div_ceil(8) as usize;
        let mut buf = vec![0u8; bytes_needed];
        let mut w = BitWriter::new(&mut buf);
        write_plex(&mut w, value, start_width, "test").unwrap();
        assert_eq!(w.bits_written(), bits as usize);

        let mut r = BitReader::new(&buf);
        assert_eq!(read_plex(&mut r, start_width, "test").unwrap(), value);
    }

    #[test]
    fn worked_example_0x1234_is_encoded_as_0xff1234() {
        // RDD 29 §3.4: "0x1234 is to be Plex(8) encoded as 0xFF1234".
        let mut buf = [0u8; 3];
        let mut w = BitWriter::new(&mut buf);
        write_plex(&mut w, 0x1234, 8, "test").unwrap();
        assert_eq!(buf, [0xFF, 0x12, 0x34]);
    }

    #[test]
    fn plex8_direct_small_values_round_trip() {
        for v in [0, 1, 0xFE] {
            round_trip(v, 8);
        }
    }

    #[test]
    fn plex8_escalates_to_16_bits() {
        for v in [0xFF, 0x100, 0x1234, 0xFFFE] {
            round_trip(v, 8);
        }
    }

    #[test]
    fn plex8_escalates_to_32_bits() {
        for v in [0xFFFF, 0x1_0000, 0xDEAD_BEEF, PLEX_MAX_VALUE] {
            round_trip(v, 8);
        }
    }

    #[test]
    fn plex4_escalates_through_all_widths() {
        for v in [0, 0xE, 0xF, 0xFF, 0x1234, PLEX_MAX_VALUE] {
            round_trip(v, 4);
        }
    }

    #[test]
    fn value_above_max_is_rejected() {
        let mut buf = [0u8; 8];
        let mut w = BitWriter::new(&mut buf);
        let err = write_plex(&mut w, PLEX_MAX_VALUE + 1, 8, "test").unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }));
    }

    #[test]
    fn top_level_all_ones_is_unrepresentable() {
        // 8 -> 16 -> 32, then 32-bit field itself all-ones: 0xFFFFFFFF.
        let buf = [0xFFu8; 8];
        let mut r = BitReader::new(&buf);
        let err = read_plex(&mut r, 8, "test").unwrap_err();
        assert!(matches!(err, Error::PlexUnrepresentable { .. }));
    }
}
