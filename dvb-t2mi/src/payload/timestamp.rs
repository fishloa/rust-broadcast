//! T2-MI payload type 0x20: DVB-T2 timestamp — ETSI TS 102 773 §5.2.7.
//!
//! Carries an absolute or relative emission time for the T2 transmitter.
//!
//! ## Wire layout (88 bits = 11 bytes)
//!
//! - byte 0 `[7:4]`: rfu (must be 0)
//! - byte 0 `[3:0]`: `bw` — bandwidth code, Table 4
//! - bytes 1–5: `seconds_since_2000` (40 bits) — whole seconds since
//!   2000-01-01T00:00:00 in the timestamp's own time base
//! - bytes 6–9 `[7:5]`: `subseconds` (27 bits) — sub-second count in units of Tsub
//! - byte 9 `[4:0]` + byte 10: `utco` (13 bits) — leap-second offset to subtract
//!   to obtain civil UTC
//!
//! ## Emission-time formula (Table 4)
//!
//! ```text
//! emission_offset = seconds_since_2000 + subseconds × Tsub
//! civil_utc       = epoch_2000 + emission_offset − utco
//! ```
//!
//! where `Tsub` per bandwidth (ETSI TS 102 773 §5.2.7, Table 4):
//!
//! | Bandwidth | bw | Tsub        |
//! |-----------|----|-------------|
//! | 1.7 MHz   | 0  | 1/131 µs    |
//! | 5 MHz     | 1  | 1/40 µs     |
//! | 6 MHz     | 2  | 1/48 µs     |
//! | 7 MHz     | 3  | 1/56 µs     |
//! | 8 MHz     | 4  | 1/64 µs     |
//! | 10 MHz    | 5  | 1/80 µs     |
//!
//! ## Special values
//!
//! - **Null timestamp**: all 80 data bits (seconds + subseconds + utco) are 1.
//! - **Relative timestamp**: `seconds_since_2000 == 0` (and not null).

use broadcast_common::len::{need_in, need_out};
use num_enum::TryFromPrimitive;

use broadcast_common::{Parse, Serialize};

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
    fn from(e: num_enum::TryFromPrimitiveError<Bandwidth>) -> Self {
        crate::error::Error::InvalidValue(broadcast_common::error::InvalidValue {
            field: "bw",
            value: u64::from(e.number),
            reason: "must be 0..=5 (Table 3)",
        })
    }
}

impl Bandwidth {
    /// Human-readable spec label (ETSI TS 102 773 Table 3).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mhz1_7 => "1.7 MHz",
            Self::Mhz5 => "5 MHz",
            Self::Mhz6 => "6 MHz",
            Self::Mhz7 => "7 MHz",
            Self::Mhz8 => "8 MHz",
            Self::Mhz10 => "10 MHz",
        }
    }
}
broadcast_common::impl_spec_display!(Bandwidth);

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
    /// Return the elementary period Tsub as a rational `(numerator, denominator)`
    /// in **nanoseconds**, exact and without floating-point.
    ///
    /// ETSI TS 102 773 §5.2.7 Table 4 gives `Tsub = 1/D µs` (where D is the
    /// bandwidth-dependent denominator), so in nanoseconds: `Tsub = 1000/D ns`.
    ///
    /// The returned value `(numer, denom)` satisfies `Tsub_ns = numer / denom`.
    /// The fraction is **not** reduced (numer is always 1000, denom is D), so
    /// callers can multiply `subseconds × 1000` and divide by `denom` to get
    /// the total subsecond nanoseconds without any floating-point.
    ///
    /// # Example
    /// ```
    /// use dvb_t2mi::payload::timestamp::Bandwidth;
    ///
    /// // 8 MHz: Tsub = 1/64 µs = 1000/64 ns.
    /// let (n, d) = Bandwidth::Mhz8.t_sub();
    /// assert_eq!((n, d), (1000, 64));
    /// // subseconds = 32_000_000 → total nanos = 32_000_000 × 1000 / 64 = 500_000_000 ns = 0.5 s.
    /// let nanos = 32_000_000u128 * u128::from(n) / u128::from(d);
    /// assert_eq!(nanos, 500_000_000);
    /// ```
    #[must_use]
    pub fn t_sub(self) -> (u32, u32) {
        // Tsub = 1/D µs = 1000/D ns.  numer = 1000, denom = D.
        let denom = match self {
            Bandwidth::Mhz1_7 => SUBSEC_DENOM_1_7MHZ,
            Bandwidth::Mhz5 => SUBSEC_DENOM_5MHZ,
            Bandwidth::Mhz6 => SUBSEC_DENOM_6MHZ,
            Bandwidth::Mhz7 => SUBSEC_DENOM_7MHZ,
            Bandwidth::Mhz8 => SUBSEC_DENOM_8MHZ,
            Bandwidth::Mhz10 => SUBSEC_DENOM_10MHZ,
        };
        (1000, denom as u32)
    }

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
        need_in(bytes, TIMESTAMP_HEADER_LEN, "T2TimestampPayload header")?;

        // byte 0 [7:4] = rfu
        if bytes[0] & 0xF0 != 0 {
            return Err(crate::Error::ReservedBits(
                broadcast_common::error::ReservedBits {
                    field: "4-bit RFU",
                    expected: 0,
                    found: u64::from(bytes[0] >> 4),
                },
            ));
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

impl crate::traits::PayloadDef<'_> for T2TimestampPayload {
    const PACKET_TYPE: u8 = 0x20;
    const NAME: &'static str = "TIMESTAMP";
}

impl Serialize for T2TimestampPayload {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        TIMESTAMP_HEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        need_out(buf, self.serialized_len())?;

        if self.seconds_since_2000 > 0xFF_FFFF_FFFF {
            return Err(crate::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "seconds_since_2000",
                    value: self.seconds_since_2000,
                    max: 0xFF_FFFF_FFFF,
                },
            ));
        }
        if self.subseconds > 0x7FFFFFF {
            return Err(crate::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "subseconds",
                    value: u64::from(self.subseconds),
                    max: 0x7FF_FFFF,
                },
            ));
        }
        if self.utco > 0x1FFF {
            return Err(crate::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "utco",
                    value: u64::from(self.utco),
                    max: 0x1FFF,
                },
            ));
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

    /// Returns `true` if this is a relative timestamp: `seconds_since_2000` is
    /// zero, so the emission time is defined only by `subseconds` (§5.2.7).
    pub fn is_relative(&self) -> bool {
        self.seconds_since_2000 == 0
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
    /// using the current `bw`; the inverse of [`emission_offset`](Self::emission_offset),
    /// with the sub-second part truncated to whole `Tsub` units. `bw` and `utco`
    /// are left unchanged.
    ///
    /// # Errors
    ///
    /// [`FieldOverflow`](crate::error::Error::FieldOverflow) if
    /// the duration does not fit the 40-bit seconds or 27-bit subseconds field;
    /// `self` is unchanged.
    pub fn set_emission_offset(
        &mut self,
        offset: core::time::Duration,
    ) -> Result<(), crate::error::Error> {
        let seconds_since_2000 = offset.as_secs();
        if seconds_since_2000 > SECONDS_SINCE_2000_MAX {
            return Err(crate::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "seconds_since_2000",
                    value: seconds_since_2000,
                    max: SECONDS_SINCE_2000_MAX,
                },
            ));
        }
        let sps = u128::from(self.bw.subseconds_per_second());
        let subseconds = u128::from(offset.subsec_nanos()) * sps / 1_000_000_000u128;
        let subseconds = u32::try_from(subseconds)
            .ok()
            .filter(|&s| s <= SUBSECONDS_MAX)
            .ok_or(crate::error::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "subseconds",
                    value: u64::try_from(subseconds).unwrap_or(u64::MAX),
                    max: u64::from(SUBSECONDS_MAX),
                },
            ))?;
        self.seconds_since_2000 = seconds_since_2000;
        self.subseconds = subseconds;
        Ok(())
    }
}

#[cfg(feature = "jiff")]
#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
impl T2TimestampPayload {
    /// The emission time as civil UTC: `UTC = DVB-T2 time - utco`
    /// (ETSI TS 102 773 §5.2.7 and Annex F).
    ///
    /// `None` for a Null timestamp or a value `jiff::Timestamp` cannot represent.
    #[must_use]
    pub fn emission_time_utc(&self) -> Option<jiff::Timestamp> {
        let offset = self.emission_offset()?;
        broadcast_common::time::decode_seconds_since_2000_utc(
            offset.as_secs(),
            offset.subsec_nanos(),
            self.utco,
        )
    }

    /// Set `seconds_since_2000`, `subseconds` and `utco` from a civil UTC time
    /// and the leap-second offset `utco`; the inverse of
    /// [`emission_time_utc`](Self::emission_time_utc), with the sub-second part
    /// truncated to whole `Tsub` units.
    ///
    /// # Errors
    ///
    /// [`InvalidField`](crate::error::Error::InvalidField) if the date is before
    /// 2000 or its seconds exceed 40 bits, or
    /// [`FieldOverflow`](crate::error::Error::FieldOverflow) if `utco` exceeds 13
    /// bits; `self` is unchanged.
    pub fn set_emission_time_utc(
        &mut self,
        dt: jiff::Timestamp,
        utco: u16,
    ) -> Result<(), crate::error::Error> {
        if utco > UTCO_MAX {
            return Err(crate::error::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "utco",
                    value: u64::from(utco),
                    max: u64::from(UTCO_MAX),
                },
            ));
        }
        let (seconds, nanos) = broadcast_common::time::encode_seconds_since_2000_utc(dt, utco)
            .ok_or(crate::error::Error::InvalidField(
                broadcast_common::error::InvalidField {
                    field: "seconds_since_2000",
                    reason: "date before 2000 or exceeds 40 bits",
                },
            ))?;
        let mut next = self.clone();
        next.utco = utco;
        next.set_emission_offset(core::time::Duration::new(seconds, nanos))?;
        *self = next;
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
        assert_eq!((Bandwidth::try_from(6)).unwrap_err().number, 6);
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
        assert_eq!(
            T2TimestampPayload::parse(&buf).err(),
            Some(crate::Error::ReservedBits(
                broadcast_common::error::ReservedBits {
                    field: "4-bit RFU",
                    expected: 0,
                    found: 8
                }
            ))
        );
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert_eq!(
            T2TimestampPayload::parse(&[0x00; 10]).err(),
            Some(crate::Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: 11,
                    have: 10,
                    what: "T2TimestampPayload header"
                }
            ))
        );
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
        buf[0] = 0x02; // bw = 6 MHz; top 4 rfu bits = 0
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
        assert!(!p.is_null());
        assert!(p.emission_offset().is_some());
    }

    // ── Bandwidth::t_sub() known-value tests ─────────────────────────────────
    // Table 4 (ETSI TS 102 773 §5.2.7): Tsub = 1/D µs = 1000/D ns.
    // We use subseconds = D*1000 (= 1 ms worth of ticks) to verify the arithmetic.

    #[test]
    fn t_sub_1_7mhz() {
        // D = 131: Tsub = 1000/131 ns.
        // subseconds = 131_000 → total nanos = 131_000 × 1000 / 131 = 1_000_000 ns = 1 ms.
        let (n, d) = Bandwidth::Mhz1_7.t_sub();
        assert_eq!((n, d), (1000, 131));
        let nanos = 131_000u128 * u128::from(n) / u128::from(d);
        assert_eq!(nanos, 1_000_000, "1.7 MHz: 131_000 ticks should be 1 ms");
    }

    #[test]
    fn t_sub_5mhz() {
        // D = 40: Tsub = 1000/40 = 25 ns.
        // subseconds = 40_000 → total nanos = 40_000 × 1000 / 40 = 1_000_000 ns = 1 ms.
        let (n, d) = Bandwidth::Mhz5.t_sub();
        assert_eq!((n, d), (1000, 40));
        let nanos = 40_000u128 * u128::from(n) / u128::from(d);
        assert_eq!(nanos, 1_000_000, "5 MHz: 40_000 ticks should be 1 ms");
    }

    #[test]
    fn t_sub_6mhz() {
        // D = 48: Tsub = 1000/48 ns ≈ 20.83 ns.
        // subseconds = 48_000 → total nanos = 48_000 × 1000 / 48 = 1_000_000 ns = 1 ms.
        let (n, d) = Bandwidth::Mhz6.t_sub();
        assert_eq!((n, d), (1000, 48));
        let nanos = 48_000u128 * u128::from(n) / u128::from(d);
        assert_eq!(nanos, 1_000_000, "6 MHz: 48_000 ticks should be 1 ms");
    }

    #[test]
    fn t_sub_7mhz() {
        // D = 56: Tsub = 1000/56 ns ≈ 17.86 ns.
        // subseconds = 56_000 → total nanos = 56_000 × 1000 / 56 = 1_000_000 ns = 1 ms.
        let (n, d) = Bandwidth::Mhz7.t_sub();
        assert_eq!((n, d), (1000, 56));
        let nanos = 56_000u128 * u128::from(n) / u128::from(d);
        assert_eq!(nanos, 1_000_000, "7 MHz: 56_000 ticks should be 1 ms");
    }

    #[test]
    fn t_sub_8mhz() {
        // D = 64: Tsub = 1000/64 = 15.625 ns.
        // subseconds = 64_000 → total nanos = 64_000 × 1000 / 64 = 1_000_000 ns = 1 ms.
        let (n, d) = Bandwidth::Mhz8.t_sub();
        assert_eq!((n, d), (1000, 64));
        let nanos = 64_000u128 * u128::from(n) / u128::from(d);
        assert_eq!(nanos, 1_000_000, "8 MHz: 64_000 ticks should be 1 ms");
    }

    #[test]
    fn t_sub_10mhz() {
        // D = 80: Tsub = 1000/80 = 12.5 ns.
        // subseconds = 80_000 → total nanos = 80_000 × 1000 / 80 = 1_000_000 ns = 1 ms.
        let (n, d) = Bandwidth::Mhz10.t_sub();
        assert_eq!((n, d), (1000, 80));
        let nanos = 80_000u128 * u128::from(n) / u128::from(d);
        assert_eq!(nanos, 1_000_000, "10 MHz: 80_000 ticks should be 1 ms");
    }

    fn payload(
        bw: Bandwidth,
        seconds_since_2000: u64,
        subseconds: u32,
        utco: u16,
    ) -> T2TimestampPayload {
        T2TimestampPayload {
            bw,
            seconds_since_2000,
            subseconds,
            utco,
        }
    }

    #[test]
    fn is_relative_means_zero_seconds_since_2000() {
        assert!(payload(Bandwidth::Mhz8, 0, 1000, 0).is_relative());
        assert!(payload(Bandwidth::Mhz8, 0, 0, 0).is_relative());
        assert!(!payload(Bandwidth::Mhz8, 1, 0, 0).is_relative());
        assert!(
            !payload(
                Bandwidth::Mhz8,
                SECONDS_SINCE_2000_MAX,
                SUBSECONDS_MAX,
                UTCO_MAX
            )
            .is_relative()
        );
    }

    #[test]
    fn set_emission_offset_inverts_emission_offset_in_whole_tsub_units() {
        let mut p = payload(Bandwidth::Mhz8, 0, 0, 5);
        let dur = core::time::Duration::new(12345, 500_000_000);
        p.set_emission_offset(dur).unwrap();
        assert_eq!(p.seconds_since_2000, 12345);
        assert_eq!(p.subseconds, p.bw.subseconds_per_second() as u32 / 2);
        assert_eq!(p.utco, 5);
        assert_eq!(p.emission_offset(), Some(dur));
        let bytes = p.try_to_bytes().unwrap();
        assert_eq!(T2TimestampPayload::parse(&bytes).unwrap(), p);
    }

    #[test]
    fn set_emission_offset_rejects_over_range_and_leaves_self_unchanged() {
        let mut p = payload(Bandwidth::Mhz8, 7, 9, 0);
        let before = p.clone();
        let too_many_seconds = core::time::Duration::from_secs(SECONDS_SINCE_2000_MAX + 1);
        assert_eq!(
            p.set_emission_offset(too_many_seconds).err(),
            Some(crate::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "seconds_since_2000",
                    value: SECONDS_SINCE_2000_MAX + 1,
                    max: SECONDS_SINCE_2000_MAX
                }
            ))
        );
        assert_eq!(p, before);
        assert!(
            p.set_emission_offset(core::time::Duration::from_secs(SECONDS_SINCE_2000_MAX))
                .is_ok()
        );
    }

    #[cfg(feature = "jiff")]
    fn utc(y: i16, mo: i8, d: i8, h: i8, mi: i8, s: i8) -> jiff::Timestamp {
        jiff::tz::TimeZone::UTC
            .to_timestamp(jiff::civil::date(y, mo, d).at(h, mi, s, 0))
            .unwrap()
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn emission_time_utc_subtracts_utco() {
        let civil = |p: T2TimestampPayload| {
            jiff::tz::TimeZone::UTC.to_datetime(p.emission_time_utc().unwrap())
        };
        let at_epoch = civil(payload(Bandwidth::Mhz8, 0, 0, 0));
        assert_eq!(
            (at_epoch.year(), at_epoch.month(), at_epoch.day()),
            (2000, 1, 1)
        );
        assert_eq!(
            (at_epoch.hour(), at_epoch.minute(), at_epoch.second()),
            (0, 0, 0)
        );
        let with_utco = civil(payload(Bandwidth::Mhz8, 0, 0, 37));
        assert_eq!(
            (with_utco.year(), with_utco.month(), with_utco.day()),
            (1999, 12, 31)
        );
        assert_eq!(
            (with_utco.hour(), with_utco.minute(), with_utco.second()),
            (23, 59, 23)
        );
        let half_second = payload(
            Bandwidth::Mhz8,
            3,
            Bandwidth::Mhz8.subseconds_per_second() as u32 / 2,
            0,
        )
        .emission_time_utc()
        .unwrap();
        assert_eq!(half_second.subsec_nanosecond(), 500_000_000);
        assert!(
            payload(
                Bandwidth::Mhz8,
                SECONDS_SINCE_2000_MAX,
                SUBSECONDS_MAX,
                UTCO_MAX
            )
            .emission_time_utc()
            .is_none()
        );
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn set_emission_time_utc_inverts_emission_time_utc() {
        let dt = utc(2023, 6, 8, 12, 34, 56);
        let mut p = payload(Bandwidth::Mhz8, 0, 0, 0);
        p.set_emission_time_utc(dt, 37).unwrap();
        assert_eq!(p.utco, 37);
        assert_eq!(p.emission_time_utc(), Some(dt));
        let expected_seconds = u64::try_from(dt.as_second() - 946_684_800).unwrap() + 37;
        assert_eq!(p.seconds_since_2000, expected_seconds);
    }

    #[cfg(feature = "jiff")]
    #[test]
    fn set_emission_time_utc_rejects_bad_input_and_leaves_self_unchanged() {
        let mut p = payload(Bandwidth::Mhz8, 7, 9, 1);
        let before = p.clone();
        let before_2000 = utc(1999, 1, 1, 0, 0, 0);
        assert_eq!(
            p.set_emission_time_utc(before_2000, 0).err(),
            Some(crate::Error::InvalidField(
                broadcast_common::error::InvalidField {
                    field: "seconds_since_2000",
                    reason: "date before 2000 or exceeds 40 bits"
                }
            ))
        );
        let dt = utc(2023, 1, 1, 0, 0, 0);
        assert_eq!(
            p.set_emission_time_utc(dt, UTCO_MAX + 1).err(),
            Some(crate::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "utco",
                    value: u64::from(UTCO_MAX + 1),
                    max: u64::from(UTCO_MAX)
                }
            ))
        );
        assert_eq!(p, before);
    }
}
