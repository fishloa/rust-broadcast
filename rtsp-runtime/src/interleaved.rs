//! Interleaved (`$`-framed) binary data — RFC 2326 §10.12.
//!
//! When RTSP is carried over TCP, media (RTP) and control (RTCP) packets may be
//! interleaved with RTSP messages on the same connection, each wrapped in a
//! 4-byte framing prefix, per [`docs/interleaved-framing.md`](../docs/interleaved-framing.md):
//!
//! ```text
//! | '$' (0x24) | channel id | length (u16 big-endian) | payload (length bytes) |
//! ```
//!
//! [`InterleavedFrame`] models one such block. [`parse_frames`] is the streaming
//! demultiplexer: given a byte buffer that may contain several complete frames
//! followed by a partial tail, it returns the complete frames plus the number of
//! unconsumed bytes, so the caller can retain the partial tail for the next read.

use crate::error::{Error, Result};

/// The `$` magic byte that prefixes an interleaved data block (RFC 2326 §10.12).
pub const MAGIC: u8 = 0x24;

/// Length of the interleaved framing prefix: `$` + channel + u16 length.
pub const HEADER_LEN: usize = 4;

/// One interleaved binary data block (RFC 2326 §10.12): a channel id and a
/// payload (exactly one upper-layer PDU, e.g. one RTP packet).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InterleavedFrame {
    /// Channel identifier, as negotiated by `interleaved=` in the Transport
    /// header. By convention even = RTP, odd = RTCP.
    pub channel: u8,
    /// The framed payload bytes.
    pub payload: Vec<u8>,
}

impl InterleavedFrame {
    /// Creates a frame for `channel` carrying `payload`.
    pub fn new(channel: u8, payload: impl Into<Vec<u8>>) -> Self {
        InterleavedFrame {
            channel,
            payload: payload.into(),
        }
    }

    /// The serialized length of this frame (`4 + payload.len()`).
    pub fn serialized_len(&self) -> usize {
        HEADER_LEN + self.payload.len()
    }

    /// Serializes the frame to a fresh `Vec<u8>` (`$`, channel, u16-be length,
    /// payload).
    ///
    /// Returns an error if the payload exceeds the 16-bit length field.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.serialized_len());
        self.serialize_into(&mut out)?;
        Ok(out)
    }

    /// Appends the serialized frame to `out`.
    ///
    /// Returns an error if the payload exceeds the 16-bit length field.
    pub fn serialize_into(&self, out: &mut Vec<u8>) -> Result<()> {
        let len: u16 = u16::try_from(self.payload.len()).map_err(|_| {
            Error::InterleavedFrame(format!(
                "payload of {} bytes exceeds 16-bit length field",
                self.payload.len()
            ))
        })?;
        out.push(MAGIC);
        out.push(self.channel);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(())
    }

    /// Parses a single frame from the front of `buf`.
    ///
    /// On success returns the frame and the number of bytes consumed
    /// (`4 + payload len`). Returns `Ok(None)` if `buf` does not yet contain a
    /// complete frame (need more bytes). Returns an error if the leading byte is
    /// not the `$` magic.
    pub fn parse(buf: &[u8]) -> Result<Option<(InterleavedFrame, usize)>> {
        if buf.len() < HEADER_LEN {
            return Ok(None);
        }
        if buf[0] != MAGIC {
            return Err(Error::InterleavedFrame(format!(
                "expected '$' (0x{MAGIC:02X}), found 0x{:02X}",
                buf[0]
            )));
        }
        let channel = buf[1];
        let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        let total = HEADER_LEN + len;
        if buf.len() < total {
            return Ok(None);
        }
        let frame = InterleavedFrame {
            channel,
            payload: buf[HEADER_LEN..total].to_vec(),
        };
        Ok(Some((frame, total)))
    }
}

/// Streaming demultiplexer for a run of interleaved frames (RFC 2326 §10.12).
///
/// Parses as many complete frames as `buf` contains, starting at offset 0, and
/// returns them together with the count of trailing bytes that form an
/// incomplete frame (the "remainder"). The caller keeps `buf[buf.len() -
/// remainder ..]` and prepends it to the next chunk.
///
/// This function assumes the buffer starts on a frame boundary (a `$`). It is
/// used by the engine only after it has classified the leading byte as `$`;
/// mixed RTSP-message / `$`-frame streams are dispatched at a higher level.
pub fn parse_frames(buf: &[u8]) -> Result<(Vec<InterleavedFrame>, usize)> {
    let mut frames = Vec::new();
    let mut offset = 0usize;
    while offset < buf.len() {
        match InterleavedFrame::parse(&buf[offset..])? {
            Some((frame, consumed)) => {
                frames.push(frame);
                offset += consumed;
            }
            None => break, // partial frame at the tail
        }
    }
    Ok((frames, buf.len() - offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_frame_round_trip() {
        let payload: Vec<u8> = (0u8..17).collect();
        let frame = InterleavedFrame::new(0, payload.clone());
        let bytes = frame.to_bytes().unwrap();
        assert_eq!(bytes[0], MAGIC);
        assert_eq!(bytes[1], 0);
        assert_eq!(&bytes[2..4], &(payload.len() as u16).to_be_bytes());
        let (parsed, consumed) = InterleavedFrame::parse(&bytes).unwrap().unwrap();
        assert_eq!(parsed, frame);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn two_frames_plus_partial_returns_two_and_remainder() {
        let f0 = InterleavedFrame::new(0, vec![0xAA; 10]);
        let f1 = InterleavedFrame::new(1, vec![0xBB; 4]);
        let mut buf = Vec::new();
        buf.extend_from_slice(&f0.to_bytes().unwrap());
        buf.extend_from_slice(&f1.to_bytes().unwrap());
        // A partial third frame: a full header claiming 8 bytes but only 3 present.
        let partial = [MAGIC, 0x00, 0x00, 0x08, 1, 2, 3];
        buf.extend_from_slice(&partial);

        let (frames, remainder) = parse_frames(&buf).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], f0);
        assert_eq!(frames[1], f1);
        assert_eq!(remainder, partial.len());
        assert_eq!(&buf[buf.len() - remainder..], &partial);
    }

    #[test]
    fn header_only_partial_is_remainder() {
        let buf = [MAGIC, 0x00];
        let (frames, remainder) = parse_frames(&buf).unwrap();
        assert!(frames.is_empty());
        assert_eq!(remainder, 2);
    }

    #[test]
    fn wrong_magic_bites() {
        let buf = [b'R', 0, 0, 1, 9];
        assert!(InterleavedFrame::parse(&buf).is_err());
    }
}
