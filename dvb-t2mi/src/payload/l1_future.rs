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
const LENGTH_FIELD_BYTES: usize = 2;

/// One `LEN(16) | block(8 x ceil(LEN/8))` pair of Table 3 starting at
/// `data[pos..]`: the block bytes and the position after them.
fn length_prefixed_block<'d>(
    data: &'d [u8],
    pos: usize,
    len_what: &'static str,
    block_what: &'static str,
) -> crate::error::Result<(&'d [u8], usize)> {
    let len_field = data
        .get(pos..)
        .and_then(|rest| rest.first_chunk::<LENGTH_FIELD_BYTES>())
        .ok_or(crate::Error::BufferTooShort {
            need: pos.saturating_add(LENGTH_FIELD_BYTES),
            have: data.len(),
            what: len_what,
        })?;
    let block_len = usize::from(u16::from_be_bytes(*len_field)).div_ceil(8);
    let start = pos + LENGTH_FIELD_BYTES;
    let end = start.saturating_add(block_len);
    let block = data.get(start..end).ok_or(crate::Error::BufferTooShort {
        need: end,
        have: data.len(),
        what: block_what,
    })?;
    Ok((block, end))
}

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

    /// Parse the "dynamic, next-but-one frame" block from `l1_future_data`
    /// (Table 3 `L1DYN_NEXT2`, TFS mode only).
    ///
    /// `num_plp` and `num_aux` must be supplied by the caller, as for
    /// [`l1_dynamic_next`](Self::l1_dynamic_next).
    ///
    /// Returns `None` when `L1DYN_NEXT2_LEN == 0` (block absent).
    ///
    /// # Errors
    /// [`crate::Error::BufferTooShort`] if the data is truncated.
    /// [`crate::Error::L1Bits`] on bit-field errors.
    pub fn l1_dynamic_next2(
        &self,
        num_plp: u8,
        num_aux: u8,
    ) -> crate::error::Result<Option<L1PostDynamic>> {
        let data = self.l1_future_data;
        let (_, next2_pos) = length_prefixed_block(
            data,
            0,
            "L1DYN_NEXT_LEN (scanning for NEXT2)",
            "L1DYN_NEXT (scanning for NEXT2)",
        )?;
        let (block, _) = length_prefixed_block(data, next2_pos, "L1DYN_NEXT2_LEN", "L1DYN_NEXT2")?;
        if block.is_empty() {
            return Ok(None);
        }
        Ok(Some(L1PostDynamic::parse(block, num_plp, num_aux)?))
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

    const DYNAMIC_BLOCK_79_BITS: [u8; 10] = [0x05, 0, 0, 0, 0, 0, 0x0A, 0xBA, 0, 0];
    const DYNAMIC_BLOCK_LEN_BITS: [u8; 2] = [0x00, 0x4F];

    fn dynamic_block(frame_idx: u8) -> Vec<u8> {
        let mut block = DYNAMIC_BLOCK_79_BITS.to_vec();
        block[0] = frame_idx;
        block
    }

    #[test]
    fn l1_dynamic_next2_decodes_the_block_after_a_present_next_block() {
        let mut data = DYNAMIC_BLOCK_LEN_BITS.to_vec();
        data.extend_from_slice(&dynamic_block(5));
        data.extend_from_slice(&DYNAMIC_BLOCK_LEN_BITS);
        data.extend_from_slice(&dynamic_block(6));
        let pkt = L1FuturePayload {
            frame_idx: 0,
            l1_future_data: &data,
        };
        let next = pkt.l1_dynamic_next(0, 0).unwrap().unwrap();
        let next2 = pkt.l1_dynamic_next2(0, 0).unwrap().unwrap();
        assert_eq!(next.frame_idx, 5);
        assert_eq!(next2.frame_idx, 6);
        assert_eq!(next2.l1_change_counter, 0xAB);
        assert_eq!(next2.start_rf_idx, 5);
    }

    #[test]
    fn l1_dynamic_next2_is_none_when_its_length_is_zero() {
        let mut data = vec![0x00, 0x00, 0x00, 0x00];
        let pkt = L1FuturePayload {
            frame_idx: 0,
            l1_future_data: &data,
        };
        assert_eq!(pkt.l1_dynamic_next2(0, 0).unwrap(), None);
        data.truncate(3);
        let pkt = L1FuturePayload {
            frame_idx: 0,
            l1_future_data: &data,
        };
        assert!(matches!(
            pkt.l1_dynamic_next2(0, 0),
            Err(crate::Error::BufferTooShort {
                what: "L1DYN_NEXT2_LEN",
                ..
            })
        ));
    }

    #[test]
    fn l1_dynamic_next2_oversized_next_len_is_an_error_not_a_panic() {
        let data = [0xFFu8, 0xFF];
        let pkt = L1FuturePayload {
            frame_idx: 0,
            l1_future_data: &data,
        };
        assert!(matches!(
            pkt.l1_dynamic_next2(0, 0),
            Err(crate::Error::BufferTooShort {
                what: "L1DYN_NEXT (scanning for NEXT2)",
                ..
            })
        ));
    }
}
