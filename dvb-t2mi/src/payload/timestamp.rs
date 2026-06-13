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

#[cfg(feature = "chrono")]
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
impl T2TimestampPayload {
    /// Decode this timestamp to a civil [`chrono::DateTime<chrono::Utc>`],
    /// applying the `utco` leap-second correction.
    ///
    /// Per ETSI TS 102 773 §5.2.7:
    /// ```text
    /// civil_utc = epoch_2000 + seconds_since_2000 + subseconds × Tsub − utco
    /// ```
    /// where `utco` is the number of leap seconds to subtract.
    ///
    /// Returns `None` for a Null timestamp or if the value is out of range.
    #[must_use]
    pub fn emission_time_utc(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let offset = self.emission_offset()?;
        dvb_common::time::decode_seconds_since_2000_utc(
            offset.as_secs(),
            offset.subsec_nanos(),
            self.utco,
        )
    }

    /// Set `seconds_since_2000`, `subseconds`, and `utco` from a civil UTC
    /// [`chrono::DateTime<chrono::Utc>`] and a `utco` leap-second offset.
    ///
    /// # Errors
    ///
    /// Returns [`ReservedBitsViolation`](crate::error::Error::ReservedBitsViolation)
    /// if the resulting `seconds_since_2000` would not fit in 40 bits (i.e. the
    /// date is before 2000 or more than ~34657 years in the future).
    pub fn set_emission_time_utc(
        &mut self,
        dt: chrono::DateTime<chrono::Utc>,
        utco: u16,
    ) -> Result<(), crate::error::Error> {
        let (secs, nanos) = dvb_common::time::encode_seconds_since_2000_utc(dt, utco).ok_or(
            crate::error::Error::ReservedBitsViolation {
                field: "seconds_since_2000",
                reason: "date before 2000 epoch or exceeds 40-bit range",
            },
        )?;
        let sps = self.bw.subseconds_per_second();
        let subseconds = (u128::from(nanos) * sps as u128 / 1_000_000_000u128) as u32;
        if subseconds > SUBSECONDS_MAX {
            return Err(crate::error::Error::ReservedBitsViolation {
                field: "subseconds",
                reason: "nanosecond component exceeds 27-bit subseconds range",
            });
        }
        self.seconds_since_2000 = secs;
        self.subseconds = subseconds;
        self.utco = utco;
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

    #[cfg(feature = "chrono")]
    #[test]
    fn emission_time_utc_known_value() {
        use chrono::{Datelike, Timelike};
        // seconds_since_2000 = 0, subseconds = 0, utco = 0 → 2000-01-01T00:00:00 UTC.
        let p = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: 0,
            subseconds: 0,
            utco: 0,
        };
        let dt = p.emission_time_utc().expect("should decode");
        assert_eq!((dt.year(), dt.month(), dt.day()), (2000, 1, 1));
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (0, 0, 0));

        // utco = 37: epoch_2000 + 0s − 37s = 1999-12-31T23:59:23 UTC.
        let p2 = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: 0,
            subseconds: 0,
            utco: 37,
        };
        let dt2 = p2.emission_time_utc().expect("should decode");
        assert_eq!((dt2.year(), dt2.month(), dt2.day()), (1999, 12, 31));
        assert_eq!((dt2.hour(), dt2.minute(), dt2.second()), (23, 59, 23));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn set_emission_time_utc_round_trips() {
        use chrono::TimeZone;
        let dt = chrono::Utc
            .with_ymd_and_hms(2023, 6, 8, 12, 34, 56)
            .unwrap();
        let mut p = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: 0,
            subseconds: 0,
            utco: 0,
        };
        p.set_emission_time_utc(dt, 37).unwrap();
        let decoded = p.emission_time_utc().expect("decodes");
        assert_eq!(decoded, dt);
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn emission_time_utc_null_returns_none() {
        let p = T2TimestampPayload {
            bw: Bandwidth::Mhz8,
            seconds_since_2000: SECONDS_SINCE_2000_MAX,
            subseconds: SUBSECONDS_MAX,
            utco: UTCO_MAX,
        };
        assert!(p.emission_time_utc().is_none());
    }
}
