//! Shared `real_time_parameters(32)` bit codec (r02-W22).
//!
//! MPE-FEC (ETSI EN 301 192 v1.7.1 §9.10 Table 46) and MPE-IFEC
//! (ETSI TS 102 772 §5.3 Table 3) carry the *same* 32-bit block —
//! `delta_t(12) | boundary_flag(1) | frame_boundary(1) | tail(18)` — under
//! different field names (`table_boundary`/`address` vs
//! `mpe_boundary`/`prev_burst_size`). The bit packing lived twice; it lives
//! here once, and both tables' public structs keep their spec-named fields.

/// Wire width of the `real_time_parameters` block.
pub(crate) const RTP_LEN: usize = 4;

/// `delta_t` occupies the top 12 bits of the block.
const DELTA_T_MAX: u16 = 0x0FFF;
/// The boundary flags occupy bits `[19:18]` of the block.
const TABLE_BOUNDARY_BIT: u8 = 0x08;
const FRAME_BOUNDARY_BIT: u8 = 0x04;
/// The 18-bit tail field (`address` / `prev_burst_size`).
const TAIL_MASK: u32 = 0x0003_FFFF;

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
        let delta_t = ((u16::from(b[0])) << 4) | ((b[1] >> 4) as u16);
        let boundary = (b[1] & TABLE_BOUNDARY_BIT) != 0;
        let frame_boundary = (b[1] & FRAME_BOUNDARY_BIT) != 0;
        // tail(18) = bottom 2 bits of b[1] | b[2] | b[3]
        let tail = ((u32::from(b[1] & 0x03)) << 16) | (u32::from(b[2]) << 8) | u32::from(b[3]);
        RealTimeParametersBits {
            delta_t,
            boundary,
            frame_boundary,
            tail,
        }
    }

    /// Encode into the 4-byte real_time_parameters block. Field values wider
    /// than their bit positions are masked so they can never bleed into the
    /// neighbouring fields.
    pub(crate) fn to_bytes(self) -> [u8; RTP_LEN] {
        let dt = self.delta_t & DELTA_T_MAX;
        let tail = self.tail & TAIL_MASK;
        [
            (dt >> 4) as u8,
            (((dt & u16::from(crate::tables::LOW_NIBBLE_MASK)) as u8) << 4)
                | (u8::from(self.boundary) << 3)
                | (u8::from(self.frame_boundary) << 2)
                | ((tail >> 16) as u8 & 0x03),
            ((tail >> 8) & 0xFF) as u8,
            (tail & 0xFF) as u8,
        ]
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
        let bytes = bits.to_bytes();
        assert_eq!(bytes, [0xAB, 0xC8 | 0x01, 0x23, 0x45]);
        assert_eq!(RealTimeParametersBits::from_bytes(bytes), bits);
    }

    #[test]
    fn wide_fields_do_not_bleed() {
        // delta_t wider than 12 bits must not shift the flags; tail wider
        // than 18 bits must not set the flags either.
        let bits = RealTimeParametersBits {
            delta_t: 0xFFFF,
            boundary: false,
            frame_boundary: false,
            tail: 0xFFFF_FFFF,
        };
        let decoded = RealTimeParametersBits::from_bytes(bits.to_bytes());
        assert_eq!(decoded.delta_t, DELTA_T_MAX);
        assert!(!decoded.boundary);
        assert!(!decoded.frame_boundary);
        assert_eq!(decoded.tail, TAIL_MASK);
    }
}

/// A target/operational descriptor-loop pair — the loop element shared by the
/// INT body (ETSI EN 301 192 §8.4.4.1 Tables 17/18) and the UNT platform loop
/// (ETSI TS 102 006 §9.4 Table 11). Both specs define the identical
/// `target_descriptor_loop` + `operational_descriptor_loop` pair; the INT
/// module used to own a duplicate struct and the UNT module an anonymous
/// tuple (r02-W22).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TargetOperationalLoop<'a> {
    /// Target descriptor loop — raw descriptor bytes (after the 12-bit length
    /// field).  Serializes as the typed descriptor sequence; `.raw()` yields the
    /// wire bytes.
    pub target_descriptors: crate::descriptors::DescriptorLoop<'a>,
    /// Operational descriptor loop — raw descriptor bytes (after the 12-bit
    /// length field).  Serializes as the typed descriptor sequence; `.raw()`
    /// yields the wire bytes.
    pub operational_descriptors: crate::descriptors::DescriptorLoop<'a>,
}

/// Wire width of the 12-bit descriptor-loop length field.
pub(crate) const DESC_LOOP_LEN_FIELD: usize = 2;

impl TargetOperationalLoop<'_> {
    /// Wire length of the pair: two 12-bit length fields plus both loops.
    pub(crate) fn serialized_len(&self) -> usize {
        DESC_LOOP_LEN_FIELD
            + self.target_descriptors.len()
            + DESC_LOOP_LEN_FIELD
            + self.operational_descriptors.len()
    }
}
