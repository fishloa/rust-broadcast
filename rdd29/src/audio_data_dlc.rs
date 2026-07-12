//! `AudioDataDLC` element — RDD 29:2019 §2.4/§4.5/§5.5.
//!
//! Contains the audio essence for one track (channel or object): a
//! proprietary lossless-compressed-PCM codec (linear-predictive + Rice-
//! Golomb entropy-coded residual). This crate never decodes it — see
//! `docs/rdd29.md` scope decision 3 for the exact boundary and why.

use broadcast_common::bits::{BitReader, BitWriter};
use broadcast_common::{Parse, Serialize};

use crate::error::{BitResultExt, Error, Result};
use crate::plex::{plex_bits, read_plex, write_plex};

/// The `AudioDataDLC` element (§4.5/§5.5): one track's audio essence,
/// referenced by [`crate::BedDefinition1`]/[`crate::ObjectDefinition1`] via
/// `AudioDataID`.
///
/// `payload` is the opaque remainder of the element after `AudioDataID` and
/// `DLCSize` — the codec's own bit-packed `DLCSampleRate`/`ShiftBits`/
/// predictor/residual data (§5.5.2 onward), never parsed by this crate (see
/// `docs/rdd29.md` scope decision 3: this is the audio-essence bitstream
/// itself, not a config header).
// `payload` is a borrowed `&'a [u8]`: serde_json's default array-of-numbers
// representation cannot round-trip back into a borrowed byte slice (it
// requires the deserializer's "bytes" hint, which a plain JSON array does
// not satisfy) -- so, like `st337::Burst` (which has the same
// `burst_payload: &'a [u8]` shape), this type derives `Serialize` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AudioDataDlc<'a> {
    /// `AudioDataID` — unique identifier referenced by bed channels/objects
    /// (§5.3.4/§5.4: `0` means "no audio asset" where used as a reference).
    pub audio_data_id: u32,
    /// The opaque codec payload — exactly `DLCSize` bytes (§5.5.1),
    /// verbatim.
    pub payload: &'a [u8],
}

impl<'a> AudioDataDlc<'a> {
    /// Build a new `AudioDataDLC` element around an opaque `payload`.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] if `payload` is longer than `DLCSize`'s
    /// 16-bit field can address (§5.5.1: max 65535 bytes).
    pub fn new(audio_data_id: u32, payload: &'a [u8]) -> Result<Self> {
        if payload.len() > usize::from(u16::MAX) {
            return Err(Error::InvalidValue {
                field: "AudioDataDLC.DLCSize",
                value: payload.len() as u64,
                reason: "DLCSize is a 16-bit field, max 65535 bytes",
            });
        }
        Ok(Self {
            audio_data_id,
            payload,
        })
    }
}

impl<'a> Parse<'a> for AudioDataDlc<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut r = BitReader::new(bytes);
        let audio_data_id = read_plex(&mut r, 8, "AudioDataDLC.AudioDataID")? as u32;
        let dlc_size = r.read_bits(16).ctx("AudioDataDLC.DLCSize")? as usize;
        debug_assert!(r.is_byte_aligned());
        let payload_start = r.bits_read() / 8;
        let payload_end = payload_start
            .checked_add(dlc_size)
            .ok_or(Error::InvalidValue {
                field: "AudioDataDLC.DLCSize",
                value: dlc_size as u64,
                reason: "DLCSize overflowed usize",
            })?;
        if payload_end > bytes.len() {
            return Err(Error::BufferTooShort {
                need: payload_end,
                have: bytes.len(),
                what: "AudioDataDLC.payload",
            });
        }
        if payload_end != bytes.len() {
            // §5.5.1: "DLCSize shall indicate the size in bytes of the
            // remainder of the AudioDataDLC Element" — i.e. it must consume
            // exactly the rest of this element's ElementSize-bounded body.
            return Err(Error::InvalidValue {
                field: "AudioDataDLC.DLCSize",
                value: dlc_size as u64,
                reason: "must equal exactly the remainder of the element body (§5.5.1)",
            });
        }
        Ok(Self {
            audio_data_id,
            payload: &bytes[payload_start..payload_end],
        })
    }
}

impl Serialize for AudioDataDlc<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let header_bits = plex_bits(u64::from(self.audio_data_id), 8) + 16;
        (header_bits as usize).div_ceil(8) + self.payload.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: buf.len(),
                what: "AudioDataDLC",
            });
        }
        let header_len;
        {
            let mut w = BitWriter::new(&mut buf[..need]);
            write_plex(
                &mut w,
                u64::from(self.audio_data_id),
                8,
                "AudioDataDLC.AudioDataID",
            )?;
            w.write_bits(self.payload.len() as u64, 16)
                .ctx("AudioDataDLC.DLCSize")?;
            debug_assert!(w.is_byte_aligned());
            header_len = w.bits_written() / 8;
        }
        buf[header_len..header_len + self.payload.len()].copy_from_slice(self.payload);
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let payload = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        let dlc = AudioDataDlc::new(9, &payload).unwrap();
        let bytes = dlc.to_bytes();
        let parsed = AudioDataDlc::parse(&bytes).unwrap();
        assert_eq!(parsed, dlc);
        assert_eq!(parsed.payload, &payload);
    }

    #[test]
    fn empty_payload_round_trips() {
        let dlc = AudioDataDlc::new(0, &[]).unwrap();
        let bytes = dlc.to_bytes();
        let parsed = AudioDataDlc::parse(&bytes).unwrap();
        assert_eq!(parsed, dlc);
    }

    #[test]
    fn payload_too_large_is_rejected() {
        let max_ok = alloc::vec![0u8; usize::from(u16::MAX)];
        assert!(AudioDataDlc::new(0, &max_ok).is_ok());

        let too_big = alloc::vec![0u8; usize::from(u16::MAX) + 1];
        let err = AudioDataDlc::new(0, &too_big).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }));
    }

    #[test]
    fn dlc_size_mismatch_is_rejected() {
        let payload = [1u8, 2, 3];
        let dlc = AudioDataDlc::new(1, &payload).unwrap();
        let mut bytes = dlc.to_bytes();
        // Truncate: now the element body is shorter than DLCSize claims.
        let short = bytes.len() - 1;
        bytes.truncate(short);
        let err = AudioDataDlc::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn mutating_audio_data_id_changes_only_header() {
        let payload = [1u8, 2, 3];
        let mut dlc = AudioDataDlc::new(5, &payload).unwrap();
        let original = dlc.to_bytes();
        dlc.audio_data_id = 6; // still Plex(8)-direct
        let mutated = dlc.to_bytes();
        assert_ne!(original[0], mutated[0]);
        assert_eq!(&original[1..], &mutated[1..]);
    }
}
