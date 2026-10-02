//! TS packet helpers — scramble/descramble 188-byte MPEG-2 TS packets.
//!
//! Handles adaptation-field offset and transport_scrambling_control bits
//! per ISO/IEC 13818-1 §2.4.3.3/§2.4.3.4 (ETSI TS 100 289 scrambles the
//! payload located this way, but does not itself redefine TS framing).
use crate::csa;
use crate::error::Error;
use crate::key::ControlWord;
use mpeg_ts::ts::{SCRAMBLING_MASK, ScramblingControl, TS_PACKET_SIZE, TsHeader};

/// One byte: the `adaptation_field_length` field itself, immediately after
/// the 4-byte TS header when `adaptation_field_control` signals a present
/// adaptation field (ISO/IEC 13818-1 §2.4.3.4).
const ADAPTATION_FIELD_LENGTH_SIZE: usize = 1;

/// Which of DVB common scrambling's two swappable control words a
/// [`ControlWord`] is — ETSI TS 100 289 V1.1.1 §5.1 Table 1
/// (`transport_scrambling_control` `10`/`11`). A real head-end holds an
/// even and an odd control word concurrently and cross-fades between them
/// (EN 50221 CA_PMT/SimulCrypt hand out both) so a receiver already
/// descrambling under one has time to fetch the other before it takes over;
/// naming which one a given [`ControlWord`] is lets
/// [`scramble_ts_packet`]/[`descramble_ts_packet`] catch a packet/key
/// mismatch instead of silently (de)scrambling with the wrong key.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyParity {
    /// `transport_scrambling_control = 10`.
    Even,
    /// `transport_scrambling_control = 11`.
    Odd,
}

impl KeyParity {
    /// Static label (issue #204 convention). Hand-written rather than
    /// `broadcast_common::impl_spec_display!`: this crate carries no
    /// dependency on `broadcast-common` (nothing else in it needs one).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Even => "even",
            Self::Odd => "odd",
        }
    }

    fn to_scrambling_control(self) -> ScramblingControl {
        match self {
            Self::Even => ScramblingControl::EvenKey,
            Self::Odd => ScramblingControl::OddKey,
        }
    }
}

impl core::fmt::Display for KeyParity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// Scramble the payload of a 188-byte TS packet in-place.
///
/// Skips the 4-byte header and any adaptation field bytes. Sets
/// `transport_scrambling_control` to `parity`'s bits (`10`/`11`).
///
/// Fails with [`Error::AlreadyScrambled`] if the packet is not already `00`
/// (not scrambled) — scrambling it again would corrupt whatever the
/// existing scrambling protects rather than yield a legally descramblable
/// packet.
///
/// ```
/// use dvb_csa::ts::{self, KeyParity};
/// use dvb_csa::ControlWord;
///
/// let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
/// let mut packet = [0xAA_u8; 188];
/// packet[..4].copy_from_slice(&[0x47, 0x01, 0x00, 0x10]); // sync, PID 0x100, payload only, clear
/// let clear = packet;
///
/// ts::scramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
/// assert_ne!(packet[4..], clear[4..]); // payload scrambled
/// ts::descramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
/// assert_eq!(packet, clear); // and back
/// ```
pub fn scramble_ts_packet(
    cw: &ControlWord,
    parity: KeyParity,
    packet: &mut [u8; TS_PACKET_SIZE],
) -> Result<(), Error> {
    let found = current_scrambling(packet);
    if found != ScramblingControl::NotScrambled {
        return Err(Error::AlreadyScrambled { found });
    }
    let payload = ts_payload_mut(packet)?;
    csa::scramble(cw, payload);
    packet[3] = (packet[3] & !SCRAMBLING_MASK) | (parity.to_scrambling_control().to_bits() << 6);
    Ok(())
}

/// Descramble the payload of a 188-byte TS packet in-place.
///
/// Skips the 4-byte header and any adaptation field bytes. Clears
/// `transport_scrambling_control` to `00`.
///
/// A packet whose `transport_scrambling_control` is already `00` (not
/// scrambled — e.g. a PSI/PCR-only packet interleaved with a scrambled
/// stream) is left untouched and returned as `Ok(())`: descrambling it
/// anyway would corrupt data that was never encrypted. A packet scrambled
/// under the *other* parity than `parity` names fails with
/// [`Error::ParityMismatch`] rather than silently running `cw` (the wrong
/// control word) over it.
pub fn descramble_ts_packet(
    cw: &ControlWord,
    parity: KeyParity,
    packet: &mut [u8; TS_PACKET_SIZE],
) -> Result<(), Error> {
    let found = current_scrambling(packet);
    if found == ScramblingControl::NotScrambled {
        return Ok(());
    }
    if found != parity.to_scrambling_control() {
        return Err(Error::ParityMismatch {
            expected: parity,
            found,
        });
    }
    let payload = ts_payload_mut(packet)?;
    csa::descramble(cw, payload);
    packet[3] &= !SCRAMBLING_MASK;
    Ok(())
}

/// Typed view of `packet`'s current `transport_scrambling_control` field.
fn current_scrambling(packet: &[u8; TS_PACKET_SIZE]) -> ScramblingControl {
    ScramblingControl::from_bits((packet[3] & SCRAMBLING_MASK) >> 6)
}

/// Get a mutable slice to the TS packet payload, skipping the header and
/// adaptation field.
///
/// The adaptation_field_control decode reuses `mpeg_ts::ts::TsHeader::parse`
/// (ISO/IEC 13818-1 §2.4.3.3) rather than re-deriving the same bit masks a
/// second time — an overrun/bit-order fix in that parser now reaches this
/// crate too. `TsHeader::parse` does not itself locate the payload byte
/// offset (that also needs the `adaptation_field_length` byte, §2.4.3.4,
/// which is not a header field), and `mpeg_ts::ts::TsPacket` — which does
/// compute that offset — only exposes an **immutable** `payload: &[u8]`
/// borrowed from its input, with no in-place-mutable equivalent. CSA
/// (de)scrambling must write the descrambled bytes back into the caller's
/// own buffer, so this function still computes the offset and takes the
/// `&mut` slice itself rather than depending on a mutable view `mpeg-ts`
/// does not provide.
fn ts_payload_mut(packet: &mut [u8; TS_PACKET_SIZE]) -> Result<&mut [u8], Error> {
    let header = TsHeader::parse(&packet[..TsHeader::serialized_len()])
        .expect("packet[..TsHeader::serialized_len()] is always exactly 4 bytes");

    if !header.has_payload {
        return Err(Error::NoPayload);
    }

    let mut payload_start = TsHeader::serialized_len();

    if header.has_adaptation {
        let af_len = packet[payload_start] as usize; // adaptation_field_length, §2.4.3.4
        payload_start += ADAPTATION_FIELD_LENGTH_SIZE + af_len;
    }

    if payload_start >= TS_PACKET_SIZE {
        return Err(Error::NoPayload);
    }

    Ok(&mut packet[payload_start..TS_PACKET_SIZE])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal TS packet: sync byte, PID 0x0100, no adaptation field, and a
    /// non-zero payload — `tsc_bits` sets `transport_scrambling_control`.
    fn payload_packet(tsc_bits: u8) -> [u8; TS_PACKET_SIZE] {
        let mut packet = [0u8; TS_PACKET_SIZE];
        packet[0] = 0x47; // sync byte
        packet[1] = 0x41; // TEI=0, PUSI=1, priority=0, PID high=0x0100>>8
        packet[2] = 0x00; // PID low=0x00
        packet[3] = 0x10 | (tsc_bits << 6); // adaptation_field_control=01 (payload only), CC=0
        for i in 0..184 {
            packet[4 + i] = (i % 256) as u8;
        }
        packet
    }

    #[test]
    fn roundtrip_ts_packet() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut packet = payload_packet(0b00);
        let original = packet;
        scramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
        assert_ne!(packet[4..], original[4..]);
        assert_eq!(packet[3] & 0xc0, 0x80); // scrambling bits set (even key)

        descramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
        // Compare payload only (skip header)
        assert_eq!(packet[4..], original[4..]);
        assert_eq!(packet[3] & 0xc0, 0x00); // scrambling bits cleared
    }

    #[test]
    fn roundtrip_ts_packet_odd_parity() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut packet = payload_packet(0b00);
        let original = packet;
        scramble_ts_packet(&cw, KeyParity::Odd, &mut packet).unwrap();
        assert_eq!(packet[3] & 0xc0, 0xc0); // scrambling bits set (odd key)
        descramble_ts_packet(&cw, KeyParity::Odd, &mut packet).unwrap();
        assert_eq!(packet[4..], original[4..]);
    }

    /// r11-W-CSA-1: descrambling a packet that carries no scrambling at all
    /// (`transport_scrambling_control = 00`, e.g. a clear PSI packet
    /// interleaved with a scrambled stream) must leave its payload
    /// untouched rather than corrupt it by running the cipher over
    /// already-clear data. Pre-fix, `descramble_ts_packet` ran the cipher
    /// unconditionally regardless of the TSC bits.
    #[test]
    fn descramble_is_a_no_op_on_an_unscrambled_packet() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut packet = payload_packet(0b00);
        let original = packet;
        descramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
        assert_eq!(packet, original, "clear packet must be left byte-identical");
    }

    /// r11-W-CSA-1: descrambling with the wrong parity's control word (the
    /// packet is scrambled `Odd` but the caller supplies `Even`) must be
    /// rejected, not silently run the wrong key over the payload. Pre-fix,
    /// there was no parity parameter at all, so this mismatch could not
    /// even be detected.
    #[test]
    fn descramble_rejects_wrong_parity() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut packet = payload_packet(0b00);
        scramble_ts_packet(&cw, KeyParity::Odd, &mut packet).unwrap();
        let scrambled = packet;
        let err = descramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap_err();
        assert!(matches!(err, Error::ParityMismatch { .. }), "got: {err:?}");
        assert_eq!(
            packet, scrambled,
            "a rejected call must not touch the payload"
        );
    }

    /// r11-W-CSA-2: scrambling a packet that is already scrambled (any
    /// non-`00` TSC) must be rejected outright, not silently re-scrambled —
    /// pre-fix, `scramble_ts_packet` always ran the cipher and stamped
    /// `10` (even), with no check of the packet's existing TSC bits at all.
    #[test]
    fn scramble_rejects_an_already_scrambled_packet() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut packet = payload_packet(0b00);
        scramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap();
        let scrambled_once = packet;
        let err = scramble_ts_packet(&cw, KeyParity::Odd, &mut packet).unwrap_err();
        assert!(
            matches!(err, Error::AlreadyScrambled { .. }),
            "got: {err:?}"
        );
        assert_eq!(
            packet, scrambled_once,
            "a rejected re-scramble must not touch the payload"
        );
    }

    /// r11-W-CSA-3: a packet with no payload at all (adaptation-field-only,
    /// `adaptation_field_control = 10`) must fail with the dedicated
    /// [`Error::NoPayload`], not a fabricated [`Error::BufferTooShort`] —
    /// the 188-byte buffer is not short; there is simply nothing to
    /// (de)scramble.
    #[test]
    fn no_payload_packet_reports_no_payload_not_buffer_too_short() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut packet = [0u8; TS_PACKET_SIZE];
        packet[0] = 0x47;
        packet[1] = 0x41;
        packet[2] = 0x00;
        // TSC=10 (even) + adaptation_field_control=10 (adaptation only, no
        // payload) — non-zero TSC so `descramble_ts_packet` doesn't take its
        // "already clear" early-return before reaching the payload lookup.
        packet[3] = 0xA0;
        packet[4] = 183; // adaptation_field_length fills the rest of the packet

        let err = scramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap_err();
        assert!(
            matches!(err, Error::AlreadyScrambled { .. }),
            "scramble must reject the already-non-00 TSC before reaching the payload lookup: {err:?}"
        );
        let err = descramble_ts_packet(&cw, KeyParity::Even, &mut packet).unwrap_err();
        assert!(matches!(err, Error::NoPayload), "got: {err:?}");
    }
}
