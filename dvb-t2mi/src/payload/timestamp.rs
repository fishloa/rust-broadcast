//! T2-MI payload type 0x20: DVB-T2 timestamp — §5.2.7.
//!
//! Absolute or relative emission time.
//! Emission time = seconds_since_2000 + subseconds * Tsub (where Tsub depends on bandwidth).
//! Null timestamp: all bits of seconds_since_2000, subseconds, utco = 1.
//!
//! Civil UTC conversion (applying the `utco` leap-second offset) is intentionally not
//! provided yet; `utco` is exposed as a field. `emission_offset` is in the timestamp's
//! own time base relative to 2000-01-01T00:00:00.

use num_enum::TryFromPrimitive;

use dvb_common::{Parse, Serialize};

/// Bandwidth per §5.2.7 Table 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum Bandwidth {
    /// 1.7 MHz bandwidth.
    Mhz1_7 = 0,
    /// 5 MHz bandwidth.
    Mhz5 = 1,
    /// 6 MHz bandwidth.
    Mhz6 = 2,
    /// 7 MHz bandwidth.
    Mhz7 = 3,
    /// 8 MHz bandwidth.
    Mhz8 = 4,
    /// 10 MHz bandwidth.
    Mhz10 = 5,
}

impl From<Bandwidth> for u8 {
    fn from(bw: Bandwidth) -> Self {
        bw as u8
    }
}

impl From<num_enum::TryFromPrimitiveError<Bandwidth>> for crate::error::Error {
    fn from(_: num_enum::TryFromPrimitiveError<Bandwidth>) -> Self {
        crate::error::Error::ReservedBitsViolation {
            field: "bw",
            reason: "Must be 0..=5 per ETSI TS 102 773 §5.2.7 Table 3",
        }
    }
}

/// Subsecond denominator D for 1.7 MHz bandwidth.
/// ETSI TS 102 773 §5.2.7 Table 4: Tsub = 1/D µs, D = 131.
const SUBSEC_DENOM_1_7MHZ: u64 = 131;

/// Subsecond denominator D for 5 MHz bandwidth.
/// ETSI TS 102 773 §5.2.7 Table 4: Tsub = 1/D µs, D = 40.
const SUBSEC_DENOM_5MHZ: u64 = 40;

/// Subsecond denominator D for 6 MHz bandwidth.
/// ETSI TS 102 773 §5.2.7 Table 4: Tsub = 1/D µs, D = 48.
const SUBSEC_DENOM_6MHZ: u64 = 48;

/// Subsecond denominator D for 7 MHz bandwidth.
/// ETSI TS 102 773 §5.2.7 Table 4: Tsub = 1/D µs, D = 56.
const SUBSEC_DENOM_7MHZ: u64 = 56;

/// Subsecond denominator D for 8 MHz bandwidth.
/// ETSI TS 102 773 §5.2.7 Table 4: Tsub = 1/D µs, D = 64.
const SUBSEC_DENOM_8MHZ: u64 = 64;

/// Subsecond denominator D for 10 MHz bandwidth.
/// ETSI TS 102 773 §5.2.7 Table 4: Tsub = 1/D µs, D = 80.
const SUBSEC_DENOM_10MHZ: u64 = 80;

impl Bandwidth {
    /// Return the number of subsecond ticks per second for this bandwidth.
    ///
    /// Equals D × 1_000_000, where D is the denominator from
    /// ETSI TS 102 773 §5.2.7 Table 4 (Tsub = 1/D µs).
    pub fn subseconds_per_second(self) -> u64 {
        match self {
            Bandwidth::Mhz1_7 => SUBSEC_DENOM_1_7MHZ * 1_000_000,
            Bandwidth::Mhz5 => SUBSEC_DENOM_5MHZ * 1_000_000,
            Bandwidth::Mhz6 => SUBSEC_DENOM_6MHZ * 1_000_000,
            Bandwidth::Mhz7 => SUBSEC_DENOM_7MHZ * 1_000_000,
            Bandwidth::Mhz8 => SUBSEC_DENOM_8MHZ * 1_000_000,
            Bandwidth::Mhz10 => SUBSEC_DENOM_10MHZ * 1_000_000,
        }
    }
}

/// Maximum value for the 40-bit `seconds_since_2000` field.
const SECONDS_SINCE_2000_MAX: u64 = 0xFF_FFFF_FFFF;

/// Maximum value for the 27-bit `subseconds` field.
const SUBSECONDS_MAX: u32 = 0x7FF_FFFF;

/// Maximum value for the 13-bit `utco` field.
const UTCO_MAX: u16 = 0x1FFF;

/// DVB-T2 timestamp payload (type 0x20) per ETSI TS 102 773 §5.2.7.
///
/// Layout (88 bits = 11 bytes):
/// - byte 0 `[7:4]`: rfu (4 bits) — must be 0
/// - byte 0 `[3:0]`: bw (4 bits) — Table 3
/// - bytes 1-5: seconds_since_2000 (40 bits)
/// - subseconds (27 bits): bytes 6-8 + byte 9 `[7:5]`
/// - utco (13 bits): byte 9 `[4:0]` + byte 10 — UTC offset in seconds
///
/// Civil UTC conversion (applying the `utco` leap-second offset) is intentionally not
/// provided yet; `utco` is exposed as a field. `emission_offset` is in the timestamp's
/// own time base relative to 2000-01-01T00:00:00.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct T2TimestampPayload {
    /// Bandwidth (determines Tsub units).
    pub bw: Bandwidth,
    /// Seconds since 2000-01-01T00:00:00Z. 0 = relative timestamp.
    /// If all bits are 1 along with subseconds + utco, this is a Null timestamp.
    pub seconds_since_2000: u64,
    /// Subsecond count (27 bits).
    pub subseconds: u32,
    /// UTC offset in seconds (e.g. 34 for leap seconds as of 2016).
    pub utco: u16,
}

const TIMESTAMP_HEADER_LEN: usize = 11;

impl<'a> Parse<'a> for T2TimestampPayload {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < TIMESTAMP_HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: TIMESTAMP_HEADER_LEN,
                have: bytes.len(),
                what: "T2TimestampPayload header",
            });
        }

        // byte 0 [7:4] = rfu
        if bytes[0] & 0xF0 != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "4-bit RFU",
                reason: "Must be zero (ETSI TS 102 773 §5.2.7)",
            });
        }

        let bw = Bandwidth::try_from(bytes[0] & 0x0F)?;

        // bytes 1-5: seconds_since_2000 (40 bits)
        let seconds_since_2000 = (bytes[1] as u64) << 32
            | (bytes[2] as u64) << 24
            | (bytes[3] as u64) << 16
            | (bytes[4] as u64) << 8
            | (bytes[5] as u64);

        // bytes 6-8 [31:5]: subseconds (27 bits)
        // bytes 6-7-8 = 24 bits, but subseconds extends into byte 9
        // 27 bits: bytes 6-8 (24 bits) + byte 9 [7:5] (3 bits)
        let subseconds = (bytes[6] as u32) << 19
            | (bytes[7] as u32) << 11
            | (bytes[8] as u32) << 3
            | ((bytes[9] >> 5) as u32 & 0x7);

        // byte 9 [4:0] + byte 10: utco (13 bits)
        let utco = ((bytes[9] as u16 & 0x1F) << 8) | (bytes[10] as u16);

        Ok(T2TimestampPayload {
            bw,
            seconds_since_2000,
            subseconds,
            utco,
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for T2TimestampPayload {
    const PACKET_TYPE: u8 = 0x20;
    const NAME: &'static str = "TIMESTAMP";
}

impl Serialize for T2TimestampPayload {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        TIMESTAMP_HEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        if self.seconds_since_2000 > 0xFF_FFFF_FFFF {
            return Err(crate::Error::ReservedBitsViolation {
                field: "seconds_since_2000",
                reason: "Must fit in 40 bits",
            });
        }
        if self.subseconds > 0x7FFFFFF {
            return Err(crate::Error::ReservedBitsViolation {
                field: "subseconds",
                reason: "Must fit in 27 bits",
            });
        }
        if self.utco > 0x1FFF {
            return Err(crate::Error::ReservedBitsViolation {
                field: "utco",
                reason: "Must fit in 13 bits",
            });
        }

        buf[0] = u8::from(self.bw) & 0x0F; // RFU = 0
        buf[1] = (self.seconds_since_2000 >> 32 & 0xFF) as u8;
        buf[2] = (self.seconds_since_2000 >> 24 & 0xFF) as u8;
        buf[3] = (self.seconds_since_2000 >> 16 & 0xFF) as u8;
        buf[4] = (self.seconds_since_2000 >> 8 & 0xFF) as u8;
        buf[5] = (self.seconds_since_2000 & 0xFF) as u8;
        buf[6] = (self.subseconds >> 19 & 0xFF) as u8;
        buf[7] = (self.subseconds >> 11 & 0xFF) as u8;
        buf[8] = (self.subseconds >> 3 & 0xFF) as u8;
        buf[9] = ((self.subseconds & 0x7) as u8) << 5 | ((self.utco >> 8) as u8 & 0x1F);
        buf[10] = (self.utco & 0xFF) as u8;

        Ok(self.serialized_len())
    }
}

impl T2TimestampPayload {
    /// Returns `true` if this is a Null timestamp (all bits of
    /// `seconds_since_2000`, `subseconds`, and `utco` are 1).
    pub fn is_null(&self) -> bool {
        self.seconds_since_2000 == SECONDS_SINCE_2000_MAX
            && self.subseconds == SUBSECONDS_MAX
            && self.utco == UTCO_MAX
    }

    /// Returns `true` if this is a relative timestamp
    /// (`seconds_since_2000` is 0 and is not null).
    pub fn is_relative(&self) -> bool {
        self.seconds_since_2000 == 0 && !self.is_null()
    }

    /// Time elapsed since 2000-01-01T00:00:00 in the timestamp's own time base.
    ///
    /// Returns `None` for a Null timestamp.
    ///
    /// Civil UTC conversion (applying the `utco` leap-second offset) is
    /// intentionally not provided yet; `utco` is exposed as a field.
    pub fn emission_offset(&self) -> Option<core::time::Duration> {
        if self.is_null() {
            return None;
        }
        let sps = self.bw.subseconds_per_second();
        let total_nanos: u128 = self.subseconds as u128 * 1_000_000_000u128 / sps as u128;
        let secs = self.seconds_since_2000 + (total_nanos / 1_000_000_000) as u64;
        let sub_nanos = (total_nanos % 1_000_000_000) as u32;
        Some(core::time::Duration::new(secs, sub_nanos))
    }

    /// Set `seconds_since_2000` and `subseconds` from a [`core::time::Duration`]
    /// using the current `bw`. Leaves `bw` and `utco` unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`ReservedBitsViolation`](crate::error::Error::ReservedBitsViolation)
    /// if the duration exceeds the 40-bit seconds or 27-bit subseconds range.
    pub fn set_emission_offset(
        &mut self,
        offset: core::time::Duration,
    ) -> Result<(), crate::error::Error> {
        let secs = offset.as_secs();
        if secs > SECONDS_SINCE_2000_MAX {
            return Err(crate::error::Error::ReservedBitsViolation {
                field: "seconds_since_2000",
                reason: "exceeds 40 bits",
            });
        }
        let sps = self.bw.subseconds_per_second();
        let subseconds = (offset.subsec_nanos() as u128 * sps as u128 / 1_000_000_000u128) as u32;
        if subseconds > SUBSECONDS_MAX {
            return Err(crate::error::Error::ReservedBitsViolation {
                field: "subseconds",
                reason: "exceeds 27 bits",
            });
        }
        self.seconds_since_2000 = secs;
        self.subseconds = subseconds;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bandwidth_try_from_valid() {
        assert_eq!(Bandwidth::try_from(0), Ok(Bandwidth::Mhz1_7));
        assert_eq!(Bandwidth::try_from(5), Ok(Bandwidth::Mhz10));
    }

    #[test]
    fn bandwidth_try_from_rejects_6() {
        assert!(Bandwidth::try_from(6).is_err());
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = Bandwidth::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 6, "expected 6 matched variants");
    }

    #[test]
    fn parse_extracts_all_fields() {
        let mut buf = [0u8; 11];
        buf[0] = 0x02; // bw = 6MHz
        buf[1] = 0x00;
        buf[2] = 0x00;
        buf[3] = 0x01; // seconds_since_2000 = 65536 + 256 + ... let me just set simple values
        buf[6] = 0x00;
        buf[7] = 0x00;
        buf[8] = 0x00;
        buf[9] = 0x00; // subseconds=0, utco=0
        buf[10] = 0x00;

        let result = T2TimestampPayload::parse(&buf).unwrap();
        assert_eq!(result.bw, Bandwidth::Mhz6);
        assert_eq!(result.seconds_since_2000, 0x00_00_01_00_00);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        let buf = [
            0x80u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(T2TimestampPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(T2TimestampPayload::parse(&[0x00; 10]).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: 0x00_00_01_02_03,
            subseconds: 0x0123456,
            utco: 0x7FF,
        };
        let mut buf = [0u8; 11];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = T2TimestampPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn null_timestamp_all_ones() {
        let mut buf = [0xFFu8; 11];
        buf[0] = 0x0F; // bw = max valid (5), rest RFU = 1 (should fail)
                       // Actually for null timestamp, RFU bits should still be 0
        buf[0] = 0x0F; // rfu=0(4 bits) + bw=1111=0xF (but F=15 is invalid per TryFrom)
                       // Let me set bw=0, rest=1
        buf[0] = 0x00; // rfu=0, bw=0 — but then seconds_since_2000 bits...
                       // For null timestamp: all bits of seconds, subseconds, utco = 1, but bw + rfu normal
        buf[0] = 0x02; // bw=6MHz
        buf[1..11].fill(0xFF);
        let result = T2TimestampPayload::parse(&buf);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.seconds_since_2000, 0xFFFFFFFFFF); // 40 bits all 1
        assert_eq!(parsed.subseconds, 0x7FFFFFF); // 27 bits all 1
        assert_eq!(parsed.utco, 0x1FFF); // 13 bits all 1
    }

    #[test]
    fn subseconds_per_second_per_table4() {
        assert_eq!(
            Bandwidth::Mhz1_7.subseconds_per_second(),
            131_000_000,
            "1.7 MHz: D=131"
        );
        assert_eq!(
            Bandwidth::Mhz5.subseconds_per_second(),
            40_000_000,
            "5 MHz: D=40"
        );
        assert_eq!(
            Bandwidth::Mhz6.subseconds_per_second(),
            48_000_000,
            "6 MHz: D=48"
        );
        assert_eq!(
            Bandwidth::Mhz7.subseconds_per_second(),
            56_000_000,
            "7 MHz: D=56"
        );
        assert_eq!(
            Bandwidth::Mhz8.subseconds_per_second(),
            64_000_000,
            "8 MHz: D=64"
        );
        assert_eq!(
            Bandwidth::Mhz10.subseconds_per_second(),
            80_000_000,
            "10 MHz: D=80"
        );
    }

    #[test]
    fn emission_offset_known_values() {
        // 8 MHz: D=64, sps=64_000_000.
        // subseconds=32_000_000 = half of sps => 0.5 s subsecond component.
        // total_nanos = 32_000_000 * 1_000_000_000 / 64_000_000 = 500_000_000.
        let p = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: 100,
            subseconds: 32_000_000,
            utco: 0,
        };
        assert_eq!(
            p.emission_offset(),
            Some(core::time::Duration::new(100, 500_000_000))
        );

        // 6 MHz: D=48, sps=48_000_000.
        // subseconds=12_000_000 = 1/4 of sps => 0.25 s subsecond component.
        // total_nanos = 12_000_000 * 1_000_000_000 / 48_000_000 = 250_000_000.
        let p2 = T2TimestampPayload {
            bw: Bandwidth::Mhz6,
            seconds_since_2000: 200,
            subseconds: 12_000_000,
            utco: 0,
        };
        assert_eq!(
            p2.emission_offset(),
            Some(core::time::Duration::new(200, 250_000_000))
        );
    }

    #[test]
    fn set_emission_offset_round_trips() {
        let mut p = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: 0,
            subseconds: 0,
            utco: 0,
        };
        let dur = core::time::Duration::new(12345, 500_000_000);
        p.set_emission_offset(dur).unwrap();
        assert_eq!(p.emission_offset(), Some(dur));
    }

    #[test]
    fn null_timestamp_offset_is_none() {
        let p = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: SECONDS_SINCE_2000_MAX,
            subseconds: SUBSECONDS_MAX,
            utco: UTCO_MAX,
        };
        assert!(p.is_null());
        assert_eq!(p.emission_offset(), None);
    }

    #[test]
    fn relative_timestamp_flag() {
        let p = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: 0,
            subseconds: 1000,
            utco: 0,
        };
        assert!(p.is_relative());
        assert!(!p.is_null());
        assert!(p.emission_offset().is_some());
    }
}
