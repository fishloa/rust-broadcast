//! T2-MI payload type 0x11: L1-future signalling — §5.2.5.
//!
//! L1-future carries "dynamic, next frame" and "dynamic, next-but-one"
//! signalling data. Per §5.5, this is always the **last** T2-MI packet
//! for a given `frame_idx` (if used in current frame).

use dvb_common::{Parse, Serialize};

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

impl<'a> L1FuturePayload<'a> {
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
        if data.len() < 2 {
            return Err(crate::Error::BufferTooShort {
                need: 2,
                have: data.len(),
                what: "L1DYN_NEXT_LEN",
            });
        }
        let len_bits = u16::from_be_bytes([data[0], data[1]]) as usize;
        if len_bits == 0 {
            return Ok(None);
        }
        let len_bytes = len_bits.div_ceil(8);
        if data.len() < 2 + len_bytes {
            return Err(crate::Error::BufferTooShort {
                need: 2 + len_bytes,
                have: data.len(),
                what: "L1DYN_NEXT",
            });
        }
        Ok(Some(L1PostDynamic::parse(
            &data[2..2 + len_bytes],
            num_plp,
            num_aux,
        )?))
    }

    /// Parse the "dynamic, next-but-one frame" block from `l1_future_data`.
    ///
    /// `num_plp` and `num_aux` must be supplied by the caller (same reason as
    /// [`l1_dynamic_next`](Self::l1_dynamic_next)).
    ///
    /// Returns `None` when `L1DYN_NEXT2_LEN == 0` (block absent, e.g. non-TFS).
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
        // Skip over the first block
        if data.len() < 2 {
            return Err(crate::Error::BufferTooShort {
                need: 2,
                have: data.len(),
                what: "L1DYN_NEXT_LEN (scanning for NEXT2)",
            });
        }
        let next_len_bits = u16::from_be_bytes([data[0], data[1]]) as usize;
        let next_len_bytes = next_len_bits.div_ceil(8);
        let offset = 2 + next_len_bytes;
        if data.len() < offset + 2 {
            return Err(crate::Error::BufferTooShort {
                need: offset + 2,
                have: data.len(),
                what: "L1DYN_NEXT2_LEN",
            });
        }
        let len_bits = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        if len_bits == 0 {
            return Ok(None);
        }
        let len_bytes = len_bits.div_ceil(8);
        if data.len() < offset + 2 + len_bytes {
            return Err(crate::Error::BufferTooShort {
                need: offset + 2 + len_bytes,
                have: data.len(),
                what: "L1DYN_NEXT2",
            });
        }
        Ok(Some(L1PostDynamic::parse(
            &data[offset + 2..offset + 2 + len_bytes],
            num_plp,
            num_aux,
        )?))
    }

    /// Parse the in-band signalling loop from `l1_future_data` (Table 3).
    ///
    /// Returns a `Vec` of `(plp_id, inband_bytes)` pairs.
    ///
    /// # Errors
    /// [`crate::Error::BufferTooShort`] if the data is truncated.
    pub fn inband_loop(&self) -> crate::error::Result<Vec<(u8, Vec<u8>)>> {
        let data = self.l1_future_data;
        // Skip over DYN_NEXT and DYN_NEXT2
        if data.len() < 2 {
            return Err(crate::Error::BufferTooShort {
                need: 2,
                have: data.len(),
                what: "L1DYN_NEXT_LEN (scanning for INBAND)",
            });
        }
        let next_len_bits = u16::from_be_bytes([data[0], data[1]]) as usize;
        let next_len_bytes = next_len_bits.div_ceil(8);
        let mut pos = 2 + next_len_bytes;
        if data.len() < pos + 2 {
            return Err(crate::Error::BufferTooShort {
                need: pos + 2,
                have: data.len(),
                what: "L1DYN_NEXT2_LEN (scanning for INBAND)",
            });
        }
        let next2_len_bits = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        let next2_len_bytes = next2_len_bits.div_ceil(8);
        pos += 2 + next2_len_bytes;

        // NUM_INBAND (8 bits)
        if data.len() < pos + 1 {
            return Err(crate::Error::BufferTooShort {
                need: pos + 1,
                have: data.len(),
                what: "NUM_INBAND",
            });
        }
        let num_inband = data[pos] as usize;
        pos += 1;

        let mut result = Vec::with_capacity(num_inband);
        for _ in 0..num_inband {
            if data.len() < pos + 3 {
                return Err(crate::Error::BufferTooShort {
                    need: pos + 3,
                    have: data.len(),
                    what: "INBAND entry header",
                });
            }
            let plp_id = data[pos];
            let inband_len_bits = u16::from_be_bytes([data[pos + 1], data[pos + 2]]) as usize;
            let inband_bytes = inband_len_bits.div_ceil(8);
            pos += 3;
            if data.len() < pos + inband_bytes {
                return Err(crate::Error::BufferTooShort {
                    need: pos + inband_bytes,
                    have: data.len(),
                    what: "INBAND data",
                });
            }
            result.push((plp_id, data[pos..pos + inband_bytes].to_vec()));
            pos += inband_bytes;
        }
        Ok(result)
    }
}

impl<'a> Parse<'a> for L1FuturePayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < L1_FUTURE_HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: L1_FUTURE_HEADER_LEN,
                have: bytes.len(),
                what: "L1FuturePayload header",
            });
        }

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
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

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
        assert!(L1FuturePayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(L1FuturePayload::parse(&[0x00]).is_err());
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
