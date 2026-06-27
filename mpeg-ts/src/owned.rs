//! Owned 188-byte TS packet with pre-parsed header fields.
//!
//! [`OwnedTsPacket`] complements the zero-copy [`crate::ts::TsPacket`] (which holds
//! a borrowed `&[u8; 188]`) with an **owned** `[u8; 188]` suitable for queuing,
//! cloning, and in-place mutation — e.g. for mux pipelines that must rewrite the
//! continuity counter or splice in a new payload.
//!
//! Header parsing delegates to [`crate::ts::TsHeader::parse`]; no bit-twiddling
//! is duplicated here.

use crate::error::{Error, Result};
use crate::ts::{
    AdaptationFieldControl, ScramblingControl, TsHeader, TS_PACKET_SIZE, TS_SYNC_BYTE,
};

/// Owned 188-byte TS packet with pre-parsed header fields.
///
/// The raw bytes are stored in `raw`; the parsed flags (`pid`, `pusi`, etc.) are
/// pre-extracted at construction time so hot paths avoid repeated byte masking.
///
/// # Payload access
///
/// Use [`payload`](Self::payload) / [`payload_mut`](Self::payload_mut) to obtain
/// a slice that correctly skips the 4-byte header **and** any adaptation field.
///
/// # Building packets
///
/// [`serialize_with_payload`](Self::serialize_with_payload) constructs a plain
/// payload-only packet (no adaptation field) filled with 0xFF stuffing.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OwnedTsPacket {
    /// The raw 188 bytes (serialized as a byte sequence).
    #[cfg_attr(feature = "serde", serde(serialize_with = "serialize_raw_bytes"))]
    pub raw: [u8; TS_PACKET_SIZE],
    /// 13-bit PID extracted from bytes 1–2.
    pub pid: u16,
    /// Payload Unit Start Indicator (byte 1 bit 6).
    pub pusi: bool,
    /// Adaptation field present flag (byte 3 bit 5).
    pub has_adaptation: bool,
    /// Payload present flag (byte 3 bit 4).
    pub has_payload: bool,
    /// Transport Error Indicator (byte 1 bit 7).
    pub tei: bool,
    /// 2-bit transport_scrambling_control (byte 3 bits 7–6).
    pub scrambling: u8,
    /// 4-bit continuity_counter (byte 3 bits 3–0).
    pub continuity_counter: u8,
    /// Discontinuity flag: `true` if the adaptation-field `discontinuity_indicator`
    /// was set in the source packet, or if the caller marks this as a
    /// continuity-counter discontinuity boundary. Defaults to `false` on parse.
    pub discontinuity: bool,
}

/// Serialize a `[u8; N]` as a variable-length byte sequence so serde's
/// blanket impls (only up to `[u8; 32]`) are not required.
#[cfg(feature = "serde")]
fn serialize_raw_bytes<S: serde::Serializer>(
    bytes: &[u8; TS_PACKET_SIZE],
    s: S,
) -> core::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(bytes.len()))?;
    for b in bytes {
        seq.serialize_element(b)?;
    }
    seq.end()
}

impl OwnedTsPacket {
    /// Parse a 188-byte owned TS packet.
    ///
    /// Returns [`Error::InvalidSyncByte`] if `raw[0] != 0x47`.
    /// Header bit-parsing is delegated to [`TsHeader::parse`].
    /// The `discontinuity` field defaults to `false`; set it manually if needed.
    pub fn parse(raw: [u8; TS_PACKET_SIZE]) -> Result<Self> {
        if raw[0] != TS_SYNC_BYTE {
            return Err(Error::InvalidSyncByte { found: raw[0] });
        }
        let hdr = TsHeader::parse(&raw[..4])?;
        Ok(Self {
            raw,
            pid: hdr.pid,
            pusi: hdr.pusi,
            has_adaptation: hdr.has_adaptation,
            has_payload: hdr.has_payload,
            tei: hdr.tei,
            scrambling: hdr.scrambling,
            continuity_counter: hdr.continuity_counter,
            discontinuity: false,
        })
    }

    /// Typed view of the 2-bit `transport_scrambling_control` field.
    ///
    /// See [`ScramblingControl`] for the spec citation (H.222.0 Table 2-4 +
    /// ETSI TS 100 289 §5.1 Table 1).
    pub fn scrambling_control(&self) -> ScramblingControl {
        ScramblingControl::from_bits(self.scrambling)
    }

    /// Typed view of the `adaptation_field_control` 2-bit field, derived from the
    /// stored `has_adaptation`/`has_payload` booleans.
    ///
    /// See [`AdaptationFieldControl`] for the spec citation (H.222.0 Table 2-5).
    pub fn adaptation_field_control(&self) -> AdaptationFieldControl {
        AdaptationFieldControl::from_flags(self.has_adaptation, self.has_payload)
    }

    /// Return the payload bytes (after the 4-byte header and any adaptation field).
    ///
    /// Returns `None` when [`has_payload`](Self::has_payload) is `false` or the
    /// adaptation field consumed all remaining bytes.
    pub fn payload(&self) -> Option<&[u8]> {
        if !self.has_payload {
            return None;
        }
        let offset = self.payload_offset();
        if offset < TS_PACKET_SIZE {
            Some(&self.raw[offset..])
        } else {
            None
        }
    }

    /// Return a mutable slice of the payload bytes.
    ///
    /// Returns `None` when [`has_payload`](Self::has_payload) is `false` or the
    /// adaptation field consumed all remaining bytes.
    pub fn payload_mut(&mut self) -> Option<&mut [u8]> {
        if !self.has_payload {
            return None;
        }
        let offset = self.payload_offset();
        if offset < TS_PACKET_SIZE {
            Some(&mut self.raw[offset..])
        } else {
            None
        }
    }

    /// Compute the byte offset of the first payload byte inside `raw`.
    ///
    /// The 4-byte header is always present; if `has_adaptation` is set, the next
    /// byte is the adaptation-field length, and the payload starts after those
    /// `1 + af_len` bytes.
    #[inline]
    fn payload_offset(&self) -> usize {
        let mut offset = 4;
        if self.has_adaptation {
            // raw[4] is the adaptation_field_length byte
            let af_len = self.raw[4] as usize;
            offset += 1 + af_len;
        }
        offset
    }

    /// Build a 188-byte payload-only TS packet (no adaptation field).
    ///
    /// The packet is initialised to `0xFF` (MPEG-TS stuffing), the 4-byte header
    /// is written via [`TsHeader::serialize_into`], then up to 184 bytes of
    /// `payload` are copied starting at byte 4.  Any unfilled bytes remain `0xFF`.
    ///
    /// # Panics
    ///
    /// Never panics — serializing a 4-byte header into a 188-byte buffer cannot
    /// fail.
    pub fn serialize_with_payload(
        pid: u16,
        pusi: bool,
        cc: u8,
        payload: &[u8],
    ) -> [u8; TS_PACKET_SIZE] {
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        let hdr = TsHeader {
            tei: false,
            pusi,
            pid,
            scrambling: 0,
            has_adaptation: false,
            has_payload: true,
            continuity_counter: cc & 0x0F,
        };
        // Cannot fail: buf is 188 bytes, need 4.
        hdr.serialize_into(&mut pkt)
            .expect("serialize TsHeader into 188-byte buf");
        let copy_len = payload.len().min(184);
        pkt[4..4 + copy_len].copy_from_slice(&payload[..copy_len]);
        pkt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_round_trip_and_payload_mut() {
        let payload = [0xAAu8; 184];
        let mut pkt = OwnedTsPacket::parse(OwnedTsPacket::serialize_with_payload(
            0x0100, true, 7, &payload,
        ))
        .unwrap();
        assert_eq!(pkt.pid, 0x0100);
        assert!(pkt.pusi);
        assert_eq!(pkt.continuity_counter, 7);
        assert_eq!(pkt.payload().unwrap()[..184], payload[..]);
        pkt.payload_mut().unwrap()[0] = 0x55;
        assert_eq!(pkt.payload().unwrap()[0], 0x55);
        // discontinuity defaults to false
        assert!(!pkt.discontinuity);
    }

    #[test]
    fn owned_scrambling_control_accessor() {
        let make = |scrambling_bits: u8| -> OwnedTsPacket {
            let mut raw = OwnedTsPacket::serialize_with_payload(0x0100, false, 0, &[]);
            // byte 3 bits [7:6] = scrambling
            raw[3] = (raw[3] & 0x3F) | (scrambling_bits << 6);
            OwnedTsPacket::parse(raw).unwrap()
        };
        assert_eq!(
            make(0b00).scrambling_control(),
            ScramblingControl::NotScrambled
        );
        assert_eq!(make(0b01).scrambling_control(), ScramblingControl::Reserved);
        assert_eq!(make(0b10).scrambling_control(), ScramblingControl::EvenKey);
        assert_eq!(make(0b11).scrambling_control(), ScramblingControl::OddKey);
    }

    #[test]
    fn owned_adaptation_field_control_accessor() {
        let make = |afc_bits: u8| -> OwnedTsPacket {
            let mut raw = [0xFFu8; TS_PACKET_SIZE];
            raw[0] = TS_SYNC_BYTE;
            raw[1] = 0x00;
            raw[2] = 0x00;
            raw[3] = (afc_bits << 4) & 0x30;
            if afc_bits & 0b10 != 0 {
                raw[4] = 0; // adaptation_field_length = 0
            }
            OwnedTsPacket::parse(raw).unwrap()
        };
        assert_eq!(
            make(0b00).adaptation_field_control(),
            AdaptationFieldControl::Reserved
        );
        assert_eq!(
            make(0b01).adaptation_field_control(),
            AdaptationFieldControl::PayloadOnly
        );
        assert_eq!(
            make(0b10).adaptation_field_control(),
            AdaptationFieldControl::AdaptationOnly
        );
        assert_eq!(
            make(0b11).adaptation_field_control(),
            AdaptationFieldControl::AdaptationAndPayload
        );
    }
}
