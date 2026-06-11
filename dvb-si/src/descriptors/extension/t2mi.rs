//! T2-MI Descriptor — ETSI EN 300 468 §6.4.14 (tag_extension 0x11).
//!
//! The `reserved_tail` field holds trailing `reserved_zero_future_use` bytes
//! verbatim; future spec growth is surfaced via additive typed accessors.
use super::*;

impl<'a> ExtensionBodyDef<'a> for T2mi<'a> {
    const TAG_EXTENSION: u8 = 0x11;
    const NAME: &'static str = "T2MI";
}
/// T2MI body (Table 158) — fully typed, fixed 3-byte lead-in + reserved tail.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct T2mi<'a> {
    /// t2mi_stream_id(3) — byte 0 low bits.
    pub t2mi_stream_id: u8,
    /// num_t2mi_streams_minus_one(3) — byte 1 low bits.
    pub num_t2mi_streams_minus_one: u8,
    /// pcr_iscr_common_clock_flag(1) — byte 2 low bit.
    pub pcr_iscr_common_clock_flag: bool,
    /// Trailing reserved_zero_future_use byte loop (Table 158 inner `for`).
    pub reserved_tail: &'a [u8],
}

impl<'a> Parse<'a> for T2mi<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < T2MI_MIN_LEN {
            return Err(Error::BufferTooShort {
                need: T2MI_MIN_LEN,
                have: sel.len(),
                what: "T2MI body",
            });
        }
        Ok(T2mi {
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

impl Serialize for T2mi<'_> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_t2mi_round_trip() {
        // t2mi_stream_id=5, num_t2mi_streams_minus_one=2,
        // pcr_iscr_common_clock_flag=true, 2-byte reserved tail.
        let sel = [0x05, 0x02, 0x01, 0x00, 0x00];
        let bytes = wrap(0x11, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::T2mi));
        match &d.body {
            ExtensionBody::T2mi(b) => {
                assert_eq!(b.t2mi_stream_id, 5);
                assert_eq!(b.num_t2mi_streams_minus_one, 2);
                assert!(b.pcr_iscr_common_clock_flag);
                assert_eq!(b.reserved_tail, &[0x00, 0x00]);
            }
            other => panic!("expected T2mi, got {other:?}"),
        }
        // parse → serialize → parse round-trip (byte-identical)
        round_trip(&d);
        // Also verify that serialize → parse round-trips the flag false variant
        let sel2 = [0x07, 0x00, 0x00, 0xFF];
        let bytes2 = wrap(0x11, &sel2);
        let d2 = ExtensionDescriptor::parse(&bytes2).unwrap();
        round_trip(&d2);
    }

    #[test]
    fn parse_t2mi_minimal() {
        // Only the 3 mandatory bytes, no reserved tail.
        let sel = [0x01, 0x03, 0x01];
        let bytes = wrap(0x11, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::T2mi(b) => {
                assert_eq!(b.t2mi_stream_id, 1);
                assert_eq!(b.num_t2mi_streams_minus_one, 3);
                assert!(b.pcr_iscr_common_clock_flag);
                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected T2mi, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_t2mi_rejects_truncated() {
        let sel = [0xAA, 0xBB]; // only 2 bytes, need >= 3
        let bytes = wrap(0x11, &sel);
        assert!(matches!(
            ExtensionDescriptor::parse(&bytes).unwrap_err(),
            crate::error::Error::BufferTooShort { .. }
        ));
    }
}
