//! T2-MI payload type 0x10: L1-current signalling — §5.2.4.
//!
//! L1-current carries the complete L1 signalling for the current T2 frame:
//! L1PRE + L1CONF + L1DYN_CURR + optionally L1EXT.

use num_enum::TryFromPrimitive;

use dvb_common::{Parse, Serialize};

/// Frequency source per §5.2.4 Table 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum FrequencySource {
    /// Use L1-current data field.
    UseL1CurrentData = 0b00,
    /// Use individual addressing frequency function.
    UseIndividualAddressing = 0b01,
    /// Manually set per modulator.
    ManualPerModulator = 0b10,
}

impl From<FrequencySource> for u8 {
    fn from(fs: FrequencySource) -> Self {
        fs as u8
    }
}

impl From<num_enum::TryFromPrimitiveError<FrequencySource>> for crate::error::Error {
    fn from(_: num_enum::TryFromPrimitiveError<FrequencySource>) -> Self {
        crate::error::Error::ReservedBitsViolation {
            field: "freq_source",
            reason: "Must be 0b00, 0b01, or 0b10 (ETSI TS 102 773 §5.2.4)",
        }
    }
}

/// L1-current payload (type 0x10) per ETSI TS 102 773 §5.2.4.
///
/// Layout:
/// - byte 0: frame_idx (8 bits) — T2 frame where L1 is carried
/// - byte 1 `[7:6]`: freq_source (2 bits) — Table 2
/// - byte 1 `[5:0]`: rfu (6 bits) — must be 0
/// - bytes 2..: l1_current_data (variable bytes)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct L1CurrentPayload<'a> {
    /// FRAME_IDX of T2 frame where L1 is carried.
    pub frame_idx: u8,
    /// Frequency source per §5.2.4 Table 2.
    pub freq_source: FrequencySource,
    /// L1-current data: L1PRE + L1CONF + L1DYN_CURR + L1EXT (all per EN 302 755).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub l1_current_data: &'a [u8],
}

const L1_CURRENT_HEADER_LEN: usize = 2;

impl<'a> Parse<'a> for L1CurrentPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < L1_CURRENT_HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: L1_CURRENT_HEADER_LEN,
                have: bytes.len(),
                what: "L1CurrentPayload header",
            });
        }

        let frame_idx = bytes[0];
        let freq_source = FrequencySource::try_from(bytes[1] >> 6)?;

        // RFU: byte 1 bottom 6 bits — must be 0
        let rfu = bytes[1] & 0x3F;
        if rfu != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "6-bit RFU after freq_source",
                reason: "Must be zero (ETSI TS 102 773 §5.2.4)",
            });
        }

        Ok(L1CurrentPayload {
            frame_idx,
            freq_source,
            l1_current_data: &bytes[L1_CURRENT_HEADER_LEN..],
        })
    }
}

impl Serialize for L1CurrentPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        L1_CURRENT_HEADER_LEN + self.l1_current_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        buf[0] = self.frame_idx;
        buf[1] = (u8::from(self.freq_source) << 6) & 0xC0; // freq_source in top 2 bits, RFU = 0

        if !self.l1_current_data.is_empty() {
            buf[L1_CURRENT_HEADER_LEN..L1_CURRENT_HEADER_LEN + self.l1_current_data.len()]
                .copy_from_slice(self.l1_current_data);
        }

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_source_try_from_valid() {
        assert_eq!(
            FrequencySource::try_from(0b00),
            Ok(FrequencySource::UseL1CurrentData)
        );
        assert_eq!(
            FrequencySource::try_from(0b01),
            Ok(FrequencySource::UseIndividualAddressing)
        );
        assert_eq!(
            FrequencySource::try_from(0b10),
            Ok(FrequencySource::ManualPerModulator)
        );
    }

    #[test]
    fn frequency_source_try_from_rejects_11() {
        assert!(FrequencySource::try_from(0b11).is_err());
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = FrequencySource::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 3, "expected 3 matched variants");
    }

    #[test]
    fn parse_extracts_frame_idx_and_freq_source() {
        let buf = [0x42u8, 0x80, 0xDE, 0xAD]; // frame=0x42, freq_src=0b10 (Manual), rfu=0
        let result = L1CurrentPayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0x42);
        assert_eq!(result.freq_source, FrequencySource::ManualPerModulator);
        assert_eq!(result.l1_current_data, &[0xDE, 0xAD]);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        let buf = [0x00u8, 0x01, 0x00]; // freq_source=00, bottom 6 RFU bits nonzero
        assert!(L1CurrentPayload::parse(&buf).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = L1CurrentPayload {
            frame_idx: 0xAB,
            freq_source: FrequencySource::UseL1CurrentData,
            l1_current_data: &[0x12, 0x34, 0x56],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = L1CurrentPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_zeros_rfu_bits() {
        let payload = L1CurrentPayload {
            frame_idx: 0x10,
            freq_source: FrequencySource::UseIndividualAddressing,
            l1_current_data: &[],
        };
        let mut buf = [0xFFu8; 2];
        payload.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[1] & 0x3F, 0x00);
    }
}
