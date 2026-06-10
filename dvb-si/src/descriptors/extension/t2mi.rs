use super::*;

impl ExtensionBodyDef for T2miDescriptor<'_> {
    const TAG_EXTENSION: u8 = 0x11;
    const NAME: &'static str = "T2MI";
}

// ===========================================================================
//  Section 0x11 — T2MI_descriptor (Table 158, §6.4.14)
// ===========================================================================
/// T2MI body (Table 158) — fully typed, fixed 3-byte lead-in + reserved tail.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct T2miDescriptor<'a> {
    /// t2mi_stream_id(3) — byte 0 low bits.
    pub t2mi_stream_id: u8,
    /// num_t2mi_streams_minus_one(3) — byte 1 low bits.
    pub num_t2mi_streams_minus_one: u8,
    /// pcr_iscr_common_clock_flag(1) — byte 2 low bit.
    pub pcr_iscr_common_clock_flag: bool,
    /// Trailing reserved_zero_future_use byte loop (Table 158 inner `for`).
    pub reserved_tail: &'a [u8],
}

impl<'a> Parse<'a> for T2miDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < T2MI_MIN_LEN {
            return Err(invalid("T2MI: body truncated"));
        }
        Ok(T2miDescriptor {
            // Table 158 bytes 0-2:
            //   byte 0: reserved_zero_future_use(5) | t2mi_stream_id(3)
            //   byte 1: reserved_zero_future_use(5) | num_t2mi_streams_minus_one(3)
            //   byte 2: reserved_zero_future_use(7) | pcr_iscr_common_clock_flag(1)
            t2mi_stream_id: sel[0] & 0x07,
            num_t2mi_streams_minus_one: sel[1] & 0x07,
            pcr_iscr_common_clock_flag: (sel[2] & 0x01) != 0,
            reserved_tail: &sel[T2MI_MIN_LEN..],
        })
    }
}

impl Serialize for T2miDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        T2MI_MIN_LEN + self.reserved_tail.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // Table 158 bytes 0-2:
        // reserved_zero_future_use(5)=0 | t2mi_stream_id(3)
        buf[0] = self.t2mi_stream_id & 0x07;
        // reserved_zero_future_use(5)=0 | num_t2mi_streams_minus_one(3)
        buf[1] = self.num_t2mi_streams_minus_one & 0x07;
        // reserved_zero_future_use(7)=0 | pcr_iscr_common_clock_flag(1)
        buf[2] = u8::from(self.pcr_iscr_common_clock_flag);
        buf[T2MI_MIN_LEN..len].copy_from_slice(self.reserved_tail);
        Ok(len)
    }
}
