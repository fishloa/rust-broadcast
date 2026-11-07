//! 40-bit MJD + BCD UTC timestamp field — ETSI EN 300 468 Annex C.
//!
//! The "16 bits giving the 16 LSBs of MJD followed by 24 bits coded as 6
//! digits in 4-bit BCD" encoding used by the UNT `scheduling_descriptor`
//! (ETSI TS 102 006 §9.5.2.9 Table 31) and the RNT RAR descriptors (ETSI
//! TS 102 323 §5.3.5/§5.3.6 Tables 12/13), as well as the TDT/TOT/EIT. The
//! value keeps the wire bytes (a non-decimal BCD nibble from a sloppy sender
//! must round-trip) and decodes on demand.

use crate::error::{Error, Result};

/// Wire width of the field in bytes (16-bit MJD + 24-bit BCD `HHMMSS`).
pub const MJD_BCD_LEN: usize = 5;

/// A 40-bit MJD + BCD UTC timestamp, kept as received.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MjdBcdTime([u8; MJD_BCD_LEN]);

impl MjdBcdTime {
    /// Wrap the 5 wire bytes of the field.
    #[must_use]
    pub const fn from_raw(raw: [u8; MJD_BCD_LEN]) -> Self {
        Self(raw)
    }

    /// The 5 wire bytes of the field, verbatim.
    #[must_use]
    pub const fn raw(&self) -> [u8; MJD_BCD_LEN] {
        self.0
    }

    /// Decode to `(mjd, hours, minutes, seconds)` without `chrono`. Each BCD
    /// component is `None` if its nibbles are not decimal digits.
    #[must_use]
    pub fn parts(&self) -> (u16, Option<u8>, Option<u8>, Option<u8>) {
        (
            u16::from_be_bytes([self.0[0], self.0[1]]),
            broadcast_common::bcd::from_bcd_byte(self.0[2]),
            broadcast_common::bcd::from_bcd_byte(self.0[3]),
            broadcast_common::bcd::from_bcd_byte(self.0[4]),
        )
    }

    /// Decode to a UTC datetime. `None` if a field is out of range.
    #[cfg(feature = "chrono")]
    #[must_use]
    pub fn to_datetime(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        broadcast_common::time::decode_mjd_bcd_utc(self.0)
    }

    /// Encode a UTC datetime.
    ///
    /// # Errors
    /// [`Error::ValueOutOfRange`] if the date is outside the 16-bit MJD range.
    #[cfg(feature = "chrono")]
    pub fn from_datetime(dt: chrono::DateTime<chrono::Utc>) -> Result<Self> {
        broadcast_common::time::encode_mjd_bcd_utc(dt)
            .map(Self)
            .ok_or(Error::ValueOutOfRange {
                field: "MjdBcdTime",
                reason: "date not representable in 16-bit MJD",
            })
    }

    /// Read the field from `bytes[off..off + 5]`, or `BufferTooShort` naming
    /// `what`.
    pub(crate) fn read(bytes: &[u8], off: usize, what: &'static str) -> Result<Self> {
        match bytes
            .get(off..)
            .and_then(|s| s.first_chunk::<MJD_BCD_LEN>())
        {
            Some(b) => Ok(Self(*b)),
            None => Err(Error::BufferTooShort {
                need: off.saturating_add(MJD_BCD_LEN),
                have: bytes.len(),
                what,
            }),
        }
    }

    /// Write the field to `buf[off..off + 5]` (caller checked the length).
    pub(crate) fn write(&self, buf: &mut [u8], off: usize) {
        buf[off..off + MJD_BCD_LEN].copy_from_slice(&self.0);
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for MjdBcdTime {
    fn serialize<S: serde::Serializer>(&self, s: S) -> core::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let (mjd, hour, minute, second) = self.parts();
        let mut st = s.serialize_struct("MjdBcdTime", 4)?;
        st.serialize_field("mjd", &mjd)?;
        st.serialize_field("hour", &hour)?;
        st.serialize_field("minute", &minute)?;
        st.serialize_field("second", &second)?;
        st.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_decode_and_raw_round_trip() {
        // EN 300 468 Annex C example: 1993-10-13 12:45:00 = MJD 0xC079 12 45 00.
        let t = MjdBcdTime::from_raw([0xC0, 0x79, 0x12, 0x45, 0x00]);
        assert_eq!(t.parts(), (0xC079, Some(12), Some(45), Some(0)));
        assert_eq!(MjdBcdTime::from_raw(t.raw()), t);
    }

    #[test]
    fn non_decimal_bcd_is_kept_not_rejected() {
        let t = MjdBcdTime::from_raw([0xC0, 0x79, 0xFF, 0x45, 0x00]);
        assert_eq!(t.parts().1, None);
        assert_eq!(t.raw()[2], 0xFF);
    }

    #[test]
    fn read_rejects_short_buffer() {
        assert!(matches!(
            MjdBcdTime::read(&[0; 4], 0, "t"),
            Err(Error::BufferTooShort {
                need: 5,
                have: 4,
                what: "t"
            })
        ));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn datetime_round_trip() {
        let t = MjdBcdTime::from_raw([0xC0, 0x79, 0x12, 0x45, 0x00]);
        let dt = t.to_datetime().unwrap();
        assert_eq!(MjdBcdTime::from_datetime(dt).unwrap(), t);
    }
}
