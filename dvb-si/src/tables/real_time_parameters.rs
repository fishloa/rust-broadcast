//! Shared `real_time_parameters(32)` bit codec (r02-W22).
//!
//! MPE-FEC (ETSI EN 301 192 v1.7.1 §9.10 Table 46) and MPE-IFEC
//! (ETSI TS 102 772 §5.3 Table 3) carry the *same* 32-bit block —
//! `delta_t(12) | boundary_flag(1) | frame_boundary(1) | tail(18)` — under
//! different field names (`table_boundary`/`address` vs
//! `mpe_boundary`/`prev_burst_size`). The bit packing lived twice; it lives
//! here once, and both tables' public structs keep their spec-named fields.

/// Byte offset of the middle `tail` byte.
const RTP_TAIL_MID_OFFSET: usize = 2;
/// Byte offset of the low `tail` byte.
const RTP_TAIL_LO_OFFSET: usize = 3;
/// Wire width of the `real_time_parameters` block.
pub(crate) const RTP_LEN: usize = 4;

/// `delta_t` occupies the top 12 bits of the block.
const DELTA_T_BITS: u32 = 12;
#[cfg(test)]
const DELTA_T_MAX: u16 = 0x0FFF;
/// The boundary flags occupy bits `[19:18]` of the block.
const TABLE_BOUNDARY_BIT: u8 = 0x08;
const FRAME_BOUNDARY_BIT: u8 = 0x04;
/// The 18-bit tail field (`address` / `prev_burst_size`).
const TAIL_BITS: u32 = 18;
#[cfg(test)]
const TAIL_MASK: u32 = 0x0003_FFFF;

/// Shift of the `delta_t` high nibble and of the tail low bytes.
const NIBBLE_BITS: u32 = 4;
const TAIL_HI_SHIFT: u32 = 16;
const TAIL_HI_MASK: u8 = 0x03;
const TABLE_BOUNDARY_SHIFT: u32 = 3;
const FRAME_BOUNDARY_SHIFT: u32 = 2;

/// The shared 32-bit bit layout behind both tables' `RealTimeParameters`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RealTimeParametersBits {
    /// 12-bit `delta_t`.
    pub delta_t: u16,
    /// First boundary flag (MPE-FEC `table_boundary`, MPE-IFEC `mpe_boundary`).
    pub boundary: bool,
    /// Second boundary flag (`frame_boundary` in both specs).
    pub frame_boundary: bool,
    /// 18-bit tail (MPE-FEC `address`, MPE-IFEC `prev_burst_size`).
    pub tail: u32,
}

impl RealTimeParametersBits {
    /// Decode the 4-byte real_time_parameters block.
    pub(crate) fn from_bytes(b: [u8; RTP_LEN]) -> Self {
        // delta_t(12) = b[0] | top 4 bits of b[1]
        let delta_t = ((u16::from(b[0])) << NIBBLE_BITS) | ((b[1] >> NIBBLE_BITS) as u16);
        let boundary = (b[1] & TABLE_BOUNDARY_BIT) != 0;
        let frame_boundary = (b[1] & FRAME_BOUNDARY_BIT) != 0;
        // tail(18) = bottom 2 bits of b[1] | b[2] | b[3]
        let tail = ((u32::from(b[1] & TAIL_HI_MASK)) << TAIL_HI_SHIFT)
            | (u32::from(b[RTP_TAIL_MID_OFFSET]) << u8::BITS)
            | u32::from(b[RTP_TAIL_LO_OFFSET]);
        RealTimeParametersBits {
            delta_t,
            boundary,
            frame_boundary,
            tail,
        }
    }

    /// Encode into the 4-byte real_time_parameters block.
    ///
    /// `delta_t` (12 bits) and `tail` (18 bits) are range-checked: a wider
    /// value is an `Err`, never silently masked into the neighbouring fields
    /// (#1129).
    pub(crate) fn to_bytes(
        self,
    ) -> core::result::Result<[u8; RTP_LEN], broadcast_core::len::FieldOverflow> {
        let dt = broadcast_core::len::fit_bits(
            u64::from(self.delta_t),
            DELTA_T_BITS,
            "real_time_parameters.delta_t",
        )? as u16;
        let tail = broadcast_core::len::fit_bits(
            u64::from(self.tail),
            TAIL_BITS,
            "real_time_parameters.address_or_prev_burst_size",
        )? as u32;
        Ok([
            (dt >> NIBBLE_BITS) as u8,
            (((dt & u16::from(crate::tables::LOW_NIBBLE_MASK)) as u8) << NIBBLE_BITS)
                | (u8::from(self.boundary) << TABLE_BOUNDARY_SHIFT)
                | (u8::from(self.frame_boundary) << FRAME_BOUNDARY_SHIFT)
                | ((tail >> TAIL_HI_SHIFT) as u8 & TAIL_HI_MASK),
            ((tail >> u8::BITS) & u32::from(u8::MAX)) as u8,
            (tail & u32::from(u8::MAX)) as u8,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_matches_legacy_packing() {
        // Same bit layout both tables' tests asserted against historically:
        // delta_t=0x0ABC, table_boundary=1, frame_boundary=0, tail=0x00012345.
        let bits = RealTimeParametersBits {
            delta_t: 0x0ABC,
            boundary: true,
            frame_boundary: false,
            tail: 0x0001_2345,
        };
        let bytes = bits.to_bytes().unwrap();
        assert_eq!(bytes, [0xAB, 0xC8 | 0x01, 0x23, 0x45]);
        assert_eq!(RealTimeParametersBits::from_bytes(bytes), bits);
    }

    #[test]
    fn wide_fields_are_rejected_not_masked() {
        let ok = RealTimeParametersBits {
            delta_t: DELTA_T_MAX,
            boundary: false,
            frame_boundary: false,
            tail: TAIL_MASK,
        };
        let decoded = RealTimeParametersBits::from_bytes(ok.to_bytes().unwrap());
        assert_eq!(decoded, ok);

        let wide_dt = RealTimeParametersBits {
            delta_t: DELTA_T_MAX + 1,
            ..ok
        };
        assert_eq!(wide_dt.to_bytes().unwrap_err().max, u64::from(DELTA_T_MAX));
        let wide_tail = RealTimeParametersBits {
            tail: TAIL_MASK + 1,
            ..ok
        };
        assert_eq!(wide_tail.to_bytes().unwrap_err().max, u64::from(TAIL_MASK));
    }
}
