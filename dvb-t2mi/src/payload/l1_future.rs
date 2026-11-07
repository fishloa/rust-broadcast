//! T2-MI payload type 0x11: L1-future signalling — §5.2.5.
//!
//! L1-future carries "dynamic, next frame" and "dynamic, next-but-one"
//! signalling data. Per §5.5, this is always the **last** T2-MI packet
//! for a given `frame_idx` (if used in current frame).

use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

use super::l1::L1PostDynamic;

/// L1-future payload (type 0x11) per ETSI TS 102 773 §5.2.5.
///
/// Layout:
/// - byte 0: frame_idx (8 bits)
/// - byte 1: rfu (8 bits) — must be 0
/// - bytes 2..: l1_future_data (variable)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct L1FuturePayload<'a> {
    /// FRAME_IDX of T2 frame.
    pub frame_idx: u8,
    /// L1-future data containing L1DYN_NEXT, L1DYN_NEXT2, and optional in-band loops.
    pub l1_future_data: &'a [u8],
}

const L1_FUTURE_HEADER_LEN: usize = 2;

impl L1FuturePayload<'_> {
    /// Parse the "dynamic, next frame" block from `l1_future_data`.
    ///
    /// `num_plp` and `num_aux` must be supplied by the caller from the
    /// configurable L1-post block (obtained from a preceding L1-current payload),
    /// because the L1-future payload does not repeat them.
    ///
    /// Returns `None` when `L1DYN_NEXT_LEN == 0` (block absent).
    ///
    /// # Errors
    /// [`crate::Error::BufferTooShort`] if the data is truncated.
    /// [`crate::Error::L1Bits`] on bit-field errors.
    pub fn l1_dynamic_next(
        &self,
        num_plp: u8,
        num_aux: u8,
    ) -> crate::error::Result<Option<L1PostDynamic>> {
        let data = self.l1_future_data;
        let (b, _) = data
            .split_first_chunk::<2>()
            .ok_or(crate::Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: 2,
                    have: data.len(),
                    what: "L1DYN_NEXT_LEN",
                },
            ))?;
        let len_bits = u16::from_be_bytes(*b) as usize;
        if len_bits == 0 {
            return Ok(None);
        }
        let len_bytes = len_bits.div_ceil(8);
        need_in(data, 2 + len_bytes, "L1DYN_NEXT")?;
        Ok(Some(L1PostDynamic::parse(
            &data[2..2 + len_bytes],
            num_plp,
            num_aux,
        )?))
    }
}

impl<'a> Parse<'a> for L1FuturePayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        need_in(bytes, L1_FUTURE_HEADER_LEN, "L1FuturePayload header")?;

        if bytes[1] != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "byte 1 RFU",
                reason: "Must be zero (ETSI TS 102 773 §5.2.5)",
            });
        }

        Ok(L1FuturePayload {
            frame_idx: bytes[0],
            l1_future_data: &bytes[L1_FUTURE_HEADER_LEN..],
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for L1FuturePayload<'a> {
    const PACKET_TYPE: u8 = 0x11;
    const NAME: &'static str = "L1_FUTURE";
}

impl Serialize for L1FuturePayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        L1_FUTURE_HEADER_LEN + self.l1_future_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        need_out(buf, self.serialized_len())?;

        buf[0] = self.frame_idx;
        buf[1] = 0; // RFU

        if !self.l1_future_data.is_empty() {
            buf[L1_FUTURE_HEADER_LEN..L1_FUTURE_HEADER_LEN + self.l1_future_data.len()]
                .copy_from_slice(self.l1_future_data);
        }

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_frame_idx() {
        let buf = [0xABu8, 0x00, 0x12, 0x34];
        let result = L1FuturePayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0xAB);
        assert_eq!(result.l1_future_data, &[0x12, 0x34]);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        let buf = [0x00u8, 0x01, 0x00];
        assert!(matches!(
            &(L1FuturePayload::parse(&buf)),
            Err(crate::error::Error::ReservedBitsViolation { .. })
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            &(L1FuturePayload::parse(&[0x00])),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let orig = L1FuturePayload {
            frame_idx: 0x42,
            l1_future_data: &[0xCA, 0xFE, 0xBA, 0xBE],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = L1FuturePayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn empty_future_data() {
        let buf = [0x05u8, 0x00];
        let result = L1FuturePayload::parse(&buf).unwrap();
        assert!(result.l1_future_data.is_empty());
    }
}
