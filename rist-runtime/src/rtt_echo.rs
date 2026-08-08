//! RTT Echo Request/Response — VSF TR-06-1:2020 §5.2.6.
//!
//! RIST uses a pair of RTCP APP packets (PT 204, name `"RIST"`) to measure
//! round-trip time. The sender issues an [`RttEcho`] with
//! [`RttEchoKind::Request`] (subtype 2) carrying an arbitrary timestamp; the
//! receiver echoes the timestamp back in an [`RttEcho`] with
//! [`RttEchoKind::Response`] (subtype 3), adding the processing delay in
//! microseconds. Variable-length padding (always a multiple of 4 bytes) can
//! be appended for path-MTU probing.

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::{
    RIST_APP_NAME, RIST_APP_NAME_U32, RTCP_COUNT_MASK, SUBTYPE_RTT_ECHO_REQUEST,
    SUBTYPE_RTT_ECHO_RESPONSE,
};

// ---------------------------------------------------------------------------
// Wire constants
// ---------------------------------------------------------------------------

/// RTCP protocol version — always 2 (RFC 3550 §6.4.1).
const RTCP_VERSION: u8 = 2;
/// Common-header length in bytes.
const RTCP_HEADER_LEN: usize = 4;
/// One 32-bit word, in bytes.
const WORD_LEN: usize = 4;
/// PT for RTCP APP (RFC 3550 §6.7).
const PT_APP: u8 = 204;

/// Fixed-payload length of an RTT Echo packet (excluding the common header):
/// SSRC(4) + name(4) + timestamp(8) + processing_delay(4) = 20 bytes.
const RTT_ECHO_BODY_LEN: usize = 20;
/// Total minimum RTT Echo packet length (header + body, no padding):
/// 4 + 20 = 24 bytes = 6 words.
const RTT_ECHO_MIN_LEN: usize = RTCP_HEADER_LEN + RTT_ECHO_BODY_LEN;

// ---------------------------------------------------------------------------
// RttEchoKind — the subtype discriminant
// ---------------------------------------------------------------------------

/// Discriminates an RTT Echo Request (subtype 2) from an RTT Echo Response
/// (subtype 3) — TR-06-1 §5.2.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RttEchoKind {
    /// RTT Echo Request (subtype 2). The sender populates `timestamp` with
    /// an arbitrary value and sets `processing_delay_us` to 0.
    Request,
    /// RTT Echo Response (subtype 3). The responder echoes the sender's
    /// `timestamp` and fills in `processing_delay_us` (microseconds).
    Response,
}

impl RttEchoKind {
    /// The wire subtype value.
    pub fn subtype(self) -> u8 {
        match self {
            RttEchoKind::Request => SUBTYPE_RTT_ECHO_REQUEST,
            RttEchoKind::Response => SUBTYPE_RTT_ECHO_RESPONSE,
        }
    }

    /// Decode the subtype byte; returns `Err` for unrecognised values.
    fn from_subtype(subtype: u8) -> Result<Self> {
        match subtype {
            SUBTYPE_RTT_ECHO_REQUEST => Ok(RttEchoKind::Request),
            SUBTYPE_RTT_ECHO_RESPONSE => Ok(RttEchoKind::Response),
            other => Err(Error::InvalidSubtype(other)),
        }
    }

    /// Spec token.
    pub fn name(self) -> &'static str {
        match self {
            RttEchoKind::Request => "RTT Echo Request",
            RttEchoKind::Response => "RTT Echo Response",
        }
    }
}

broadcast_common::impl_spec_display!(RttEchoKind);

// ---------------------------------------------------------------------------
// RttEcho — TR-06-1 §5.2.6
// ---------------------------------------------------------------------------

/// RTT Echo Request/Response — RTCP APP (PT 204, name `"RIST"`,
/// subtype 2 or 3). TR-06-1 §5.2.6.
///
/// # Wire format
///
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |V=2|0| Subtype |   PT=204     |          Length               |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                  SSRC of media source                        |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                  name = 0x52495354 ("RIST")                  |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |         Timestamp, most significant word                     |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |         Timestamp, least significant word                    |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |           Processing Delay (microseconds)                    |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |              Padding bytes (0 or more x 4)                   |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RttEcho {
    /// Whether this is a Request or Response.
    pub kind: RttEchoKind,
    /// SSRC of the media source.
    pub ssrc_media: u32,
    /// Arbitrary timestamp: set by the requester, echoed by the responder.
    pub timestamp: u64,
    /// Processing delay in microseconds. SHALL be 0 in a Request.
    pub processing_delay_us: u32,
    /// Optional padding for path-MTU probing. Must be a multiple of 4 bytes.
    pub padding: Vec<u8>,
}

impl<'a> Parse<'a> for RttEcho {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < RTCP_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: RTCP_HEADER_LEN,
                have: bytes.len(),
            });
        }

        // Validate V=2.
        let version = bytes[0] >> 6;
        if version != RTCP_VERSION {
            return Err(Error::InvalidVersion(version));
        }

        // Subtype from low 5 bits of byte 0.
        let subtype = bytes[0] & RTCP_COUNT_MASK;
        let kind = RttEchoKind::from_subtype(subtype)?;

        // Validate PT=204.
        let pt = bytes[1];
        if pt != PT_APP {
            return Err(Error::InvalidPacketType {
                expected: PT_APP,
                got: pt,
            });
        }

        // Total length from header.
        let length_field = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        let total_len = (length_field + 1) * WORD_LEN;
        if bytes.len() < total_len {
            return Err(Error::BufferTooShort {
                need: total_len,
                have: bytes.len(),
            });
        }

        if total_len < RTT_ECHO_MIN_LEN {
            return Err(Error::BufferTooShort {
                need: RTT_ECHO_MIN_LEN,
                have: total_len,
            });
        }

        // SSRC of media source.
        let ssrc_media = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

        // Validate APP name = "RIST".
        let name = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if name != RIST_APP_NAME_U32 {
            return Err(Error::InvalidAppName(name));
        }

        // Timestamp (8 bytes, big-endian u64).
        let ts_msw = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let ts_lsw = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let timestamp = ((ts_msw as u64) << 32) | (ts_lsw as u64);

        // Processing delay (4 bytes).
        let processing_delay_us = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);

        // Padding (remaining bytes, if any).
        let padding = if total_len > RTT_ECHO_MIN_LEN {
            bytes[RTT_ECHO_MIN_LEN..total_len].to_vec()
        } else {
            Vec::new()
        };

        Ok(RttEcho {
            kind,
            ssrc_media,
            timestamp,
            processing_delay_us,
            padding,
        })
    }
}

impl Serialize for RttEcho {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        RTT_ECHO_MIN_LEN + self.padding.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.padding.len() % WORD_LEN != 0 {
            return Err(Error::InvalidPaddingLength(self.padding.len()));
        }

        // Header: V=2, P=0, Subtype.
        let length_field = (len / WORD_LEN - 1) as u16;
        buf[0] = (RTCP_VERSION << 6) | self.kind.subtype();
        buf[1] = PT_APP;
        buf[2..4].copy_from_slice(&length_field.to_be_bytes());

        // SSRC of media source.
        buf[4..8].copy_from_slice(&self.ssrc_media.to_be_bytes());

        // APP name = "RIST".
        buf[8..12].copy_from_slice(&RIST_APP_NAME);

        // Timestamp (8 bytes, big-endian).
        buf[12..16].copy_from_slice(&((self.timestamp >> 32) as u32).to_be_bytes());
        buf[16..20].copy_from_slice(&(self.timestamp as u32).to_be_bytes());

        // Processing delay.
        buf[20..24].copy_from_slice(&self.processing_delay_us.to_be_bytes());

        // Padding.
        if !self.padding.is_empty() {
            buf[24..24 + self.padding.len()].copy_from_slice(&self.padding);
        }

        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtt_echo_request_no_padding() {
        let echo = RttEcho {
            kind: RttEchoKind::Request,
            ssrc_media: 0xAABB_CC00,
            timestamp: 0x0102_0304_0506_0708,
            processing_delay_us: 0,
            padding: Vec::new(),
        };
        let bytes = echo.to_bytes();
        assert_eq!(bytes.len(), RTT_ECHO_MIN_LEN);
        // V=2, P=0, Subtype=2 -> 0x82
        assert_eq!(bytes[0], 0x82);
        // PT=204
        assert_eq!(bytes[1], PT_APP);
        // APP name = "RIST"
        assert_eq!(&bytes[8..12], b"RIST");
        let parsed = RttEcho::parse(&bytes).unwrap();
        assert_eq!(parsed, echo);
    }

    #[test]
    fn rtt_echo_response_with_padding() {
        let echo = RttEcho {
            kind: RttEchoKind::Response,
            ssrc_media: 0x1234_5678,
            timestamp: 0xDEAD_BEEF_CAFE_BABE,
            processing_delay_us: 42_000,
            padding: alloc::vec![0u8; 8],
        };
        let bytes = echo.to_bytes();
        assert_eq!(bytes.len(), RTT_ECHO_MIN_LEN + 8);
        // V=2, P=0, Subtype=3 -> 0x83
        assert_eq!(bytes[0], 0x83);
        let parsed = RttEcho::parse(&bytes).unwrap();
        assert_eq!(parsed, echo);
    }

    #[test]
    fn rtt_echo_rejects_bad_subtype() {
        let echo = RttEcho {
            kind: RttEchoKind::Request,
            ssrc_media: 1,
            timestamp: 0,
            processing_delay_us: 0,
            padding: Vec::new(),
        };
        let mut bytes = echo.to_bytes();
        // Change subtype to 5.
        bytes[0] = (RTCP_VERSION << 6) | 5;
        let err = RttEcho::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidSubtype(5)));
    }

    #[test]
    fn rtt_echo_rejects_non_aligned_padding() {
        let echo = RttEcho {
            kind: RttEchoKind::Request,
            ssrc_media: 1,
            timestamp: 0,
            processing_delay_us: 0,
            padding: alloc::vec![0u8; 3], // Not a multiple of 4
        };
        let mut buf = alloc::vec![0u8; echo.serialized_len()];
        let err = echo.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::InvalidPaddingLength(3)));
    }

    #[test]
    fn rtt_echo_kind_display() {
        assert_eq!(RttEchoKind::Request.to_string(), "RTT Echo Request");
        assert_eq!(RttEchoKind::Response.to_string(), "RTT Echo Response");
    }
}
