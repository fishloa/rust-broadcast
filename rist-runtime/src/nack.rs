//! NACK message types for RIST Simple Profile (VSF TR-06-1:2020).
//!
//! Two NACK formats are defined:
//!
//! - [`GenericNack`] — RFC 4585 §6.2.1, PT 205 (Transport-Layer Feedback),
//!   FMT 1. Bitmask-based: each FCI entry names one lost packet and a 16-bit
//!   bitmask covering the next 16 sequence numbers.
//!
//! - [`RangeNack`] — RIST-specific, RTCP APP (PT 204), subtype 0,
//!   name `"RIST"` (`0x52495354`). Range-based: each entry names a starting
//!   sequence number and a count of additional consecutive missing packets.
//!   TR-06-1 §5.3.2.2 limits each packet to at most 16 range entries.

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::{
    FMT_GENERIC_NACK, PT_RTPFB, RIST_APP_NAME, RIST_APP_NAME_U32, RTCP_COUNT_MASK,
    SUBTYPE_RANGE_NACK,
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
/// APP name field length in bytes.
const APP_NAME_LEN: usize = 4;
/// PT for RTCP APP (RFC 3550 §6.7).
const PT_APP: u8 = 204;
/// Maximum number of range entries per Range NACK (TR-06-1 §5.3.2.2).
const MAX_RANGE_ENTRIES: usize = 16;

/// Minimum GenericNack packet length: header(4) + SSRC sender(4) +
/// SSRC media(4) = 12 bytes. At least one FCI is required.
const GENERIC_NACK_MIN_LEN: usize = RTCP_HEADER_LEN + WORD_LEN + WORD_LEN;
/// Size of one Generic NACK FCI entry: PID(2) + BLP(2) = 4 bytes.
const NACK_FCI_LEN: usize = 4;

/// Minimum RangeNack packet length: header(4) + SSRC(4) + name(4) = 12 bytes.
const RANGE_NACK_MIN_LEN: usize = RTCP_HEADER_LEN + WORD_LEN + APP_NAME_LEN;
/// Size of one Range NACK entry: start(2) + additional(2) = 4 bytes.
const RANGE_ENTRY_LEN: usize = 4;

// ---------------------------------------------------------------------------
// GenericNack — RFC 4585 §6.2.1, PT 205, FMT 1
// ---------------------------------------------------------------------------

/// A single Generic NACK Feedback Control Information entry (RFC 4585 §6.2.1).
///
/// Each entry identifies a lost RTP packet (`pid`) and a bitmask of up to 16
/// following lost packets (`blp`): bit *i* (1-indexed, i = 1..16) set means
/// `pid + i` is also lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NackFci {
    /// Packet ID — the RTP sequence number of the lost packet.
    pub pid: u16,
    /// Bitmask of following Lost Packets. Bit *i* (1..16) set means the
    /// packet with sequence number `pid + i` is also lost.
    pub blp: u16,
}

/// Generic NACK — RFC 4585 §6.2.1, RTCP Transport-Layer Feedback (PT 205,
/// FMT 1).
///
/// Used by RIST receivers to request retransmission of lost packets via
/// bitmask-based loss indication.
///
/// # Wire format
///
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |V=2|P| FMT=1  |   PT=205     |          length               |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                  SSRC of packet sender                       |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                  SSRC of media source                        |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |            PID               |             BLP               |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericNack {
    /// SSRC of the packet sender (the RIST receiver sending the NACK).
    pub ssrc_sender: u32,
    /// SSRC of the media source whose packets are lost. In RIST, LSB
    /// distinguishes the original (0) from the retransmission (1) flow.
    pub ssrc_media: u32,
    /// One or more FCI entries, each identifying a lost packet and a bitmask.
    pub nacks: Vec<NackFci>,
}

impl<'a> Parse<'a> for GenericNack {
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

        // Validate FMT=1 (low 5 bits of byte 0).
        let fmt = bytes[0] & RTCP_COUNT_MASK;
        if fmt != FMT_GENERIC_NACK {
            return Err(Error::InvalidFmt {
                expected: FMT_GENERIC_NACK,
                got: fmt,
            });
        }

        // Validate PT=205.
        let pt = bytes[1];
        if pt != PT_RTPFB {
            return Err(Error::InvalidPacketType {
                expected: PT_RTPFB,
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

        if total_len < GENERIC_NACK_MIN_LEN {
            return Err(Error::BufferTooShort {
                need: GENERIC_NACK_MIN_LEN,
                have: total_len,
            });
        }

        let ssrc_sender = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let ssrc_media = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);

        // Parse FCI entries.
        let fci_bytes = total_len - GENERIC_NACK_MIN_LEN;
        let fci_count = fci_bytes / NACK_FCI_LEN;
        let mut nacks = Vec::with_capacity(fci_count);
        let mut off = GENERIC_NACK_MIN_LEN;
        for _ in 0..fci_count {
            let pid = u16::from_be_bytes([bytes[off], bytes[off + 1]]);
            let blp = u16::from_be_bytes([bytes[off + 2], bytes[off + 3]]);
            nacks.push(NackFci { pid, blp });
            off += NACK_FCI_LEN;
        }

        Ok(GenericNack {
            ssrc_sender,
            ssrc_media,
            nacks,
        })
    }
}

impl Serialize for GenericNack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        GENERIC_NACK_MIN_LEN + self.nacks.len() * NACK_FCI_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        // Header: V=2, P=0, FMT=1.
        let length_field = (len / WORD_LEN - 1) as u16;
        buf[0] = (RTCP_VERSION << 6) | FMT_GENERIC_NACK;
        buf[1] = PT_RTPFB;
        buf[2..4].copy_from_slice(&length_field.to_be_bytes());

        // SSRC fields.
        buf[4..8].copy_from_slice(&self.ssrc_sender.to_be_bytes());
        buf[8..12].copy_from_slice(&self.ssrc_media.to_be_bytes());

        // FCI entries.
        let mut off = GENERIC_NACK_MIN_LEN;
        for fci in &self.nacks {
            buf[off..off + 2].copy_from_slice(&fci.pid.to_be_bytes());
            buf[off + 2..off + 4].copy_from_slice(&fci.blp.to_be_bytes());
            off += NACK_FCI_LEN;
        }

        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// RangeNack — TR-06-1 §5.3.2.2, RTCP APP (PT 204), Subtype 0, name "RIST"
// ---------------------------------------------------------------------------

/// A single Range NACK entry (TR-06-1 §5.3.2.2).
///
/// Identifies a contiguous run of lost packets: `start` is the RTP sequence
/// number of the first lost packet, `additional` is the number of *additional*
/// consecutive lost packets (0 means a single packet is lost).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketRange {
    /// RTP sequence number of the first missing packet.
    pub start: u16,
    /// Number of additional consecutive missing packets (0 = single packet).
    pub additional: u16,
}

/// Range NACK — RIST-specific RTCP APP (PT 204, subtype 0, name `"RIST"`).
///
/// An alternative NACK format to [`GenericNack`] that uses sequence-number
/// ranges instead of bitmasks, more efficient for large contiguous losses.
/// TR-06-1 §5.3.2.2 limits each packet to at most 16 range entries.
///
/// # Wire format
///
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |V=2|P| Sub=0  |   PT=204     |          length               |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                  SSRC of media source                        |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                  name = 0x52495354 ("RIST")                  |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |  Missing Pkt Seq Start       | Num addtl missing pkts       |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeNack {
    /// SSRC of the media source whose packets are lost.
    pub ssrc_media: u32,
    /// Range entries, each identifying a contiguous run of lost packets.
    /// At most 16 per packet (TR-06-1 §5.3.2.2).
    pub ranges: Vec<PacketRange>,
}

impl<'a> Parse<'a> for RangeNack {
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

        // Validate Subtype=0 (low 5 bits of byte 0).
        let subtype = bytes[0] & RTCP_COUNT_MASK;
        if subtype != SUBTYPE_RANGE_NACK {
            return Err(Error::InvalidSubtype(subtype));
        }

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

        if total_len < RANGE_NACK_MIN_LEN {
            return Err(Error::BufferTooShort {
                need: RANGE_NACK_MIN_LEN,
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

        // Parse range entries.
        let data_bytes = total_len - RANGE_NACK_MIN_LEN;
        let range_count = data_bytes / RANGE_ENTRY_LEN;
        if range_count > MAX_RANGE_ENTRIES {
            return Err(Error::TooManyRanges {
                max: MAX_RANGE_ENTRIES,
                got: range_count,
            });
        }
        let mut ranges = Vec::with_capacity(range_count);
        let mut off = RANGE_NACK_MIN_LEN;
        for _ in 0..range_count {
            let start = u16::from_be_bytes([bytes[off], bytes[off + 1]]);
            let additional = u16::from_be_bytes([bytes[off + 2], bytes[off + 3]]);
            ranges.push(PacketRange { start, additional });
            off += RANGE_ENTRY_LEN;
        }

        Ok(RangeNack { ssrc_media, ranges })
    }
}

impl Serialize for RangeNack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        RANGE_NACK_MIN_LEN + self.ranges.len() * RANGE_ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.ranges.len() > MAX_RANGE_ENTRIES {
            return Err(Error::TooManyRanges {
                max: MAX_RANGE_ENTRIES,
                got: self.ranges.len(),
            });
        }

        // Header: V=2, P=0, Subtype=0.
        let length_field = (len / WORD_LEN - 1) as u16;
        buf[0] = (RTCP_VERSION << 6) | SUBTYPE_RANGE_NACK;
        buf[1] = PT_APP;
        buf[2..4].copy_from_slice(&length_field.to_be_bytes());

        // SSRC of media source.
        buf[4..8].copy_from_slice(&self.ssrc_media.to_be_bytes());

        // APP name = "RIST".
        buf[8..12].copy_from_slice(&RIST_APP_NAME);

        // Range entries.
        let mut off = RANGE_NACK_MIN_LEN;
        for range in &self.ranges {
            buf[off..off + 2].copy_from_slice(&range.start.to_be_bytes());
            buf[off + 2..off + 4].copy_from_slice(&range.additional.to_be_bytes());
            off += RANGE_ENTRY_LEN;
        }

        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nack_fci_single() {
        let nack = GenericNack {
            ssrc_sender: 0x1111_2222,
            ssrc_media: 0x3333_4444,
            nacks: alloc::vec![NackFci {
                pid: 1000,
                blp: 0x000F,
            }],
        };
        let bytes = nack.to_bytes();
        // V=2, P=0, FMT=1 -> 0x81
        assert_eq!(bytes[0], 0x81);
        // PT=205
        assert_eq!(bytes[1], PT_RTPFB);
        let parsed = GenericNack::parse(&bytes).unwrap();
        assert_eq!(parsed, nack);
    }

    #[test]
    fn range_nack_single() {
        let rn = RangeNack {
            ssrc_media: 0xAABB_CC00,
            ranges: alloc::vec![PacketRange {
                start: 100,
                additional: 0,
            }],
        };
        let bytes = rn.to_bytes();
        // V=2, P=0, Subtype=0 -> 0x80
        assert_eq!(bytes[0], 0x80);
        // PT=204
        assert_eq!(bytes[1], PT_APP);
        // APP name = "RIST"
        assert_eq!(&bytes[8..12], b"RIST");
        let parsed = RangeNack::parse(&bytes).unwrap();
        assert_eq!(parsed, rn);
    }

    #[test]
    fn range_nack_rejects_bad_name() {
        let rn = RangeNack {
            ssrc_media: 0xAABB_CC00,
            ranges: alloc::vec![PacketRange {
                start: 100,
                additional: 0,
            }],
        };
        let mut bytes = rn.to_bytes();
        // Corrupt the name field.
        bytes[8] = b'X';
        let err = RangeNack::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidAppName(_)));
    }

    #[test]
    fn generic_nack_rejects_bad_fmt() {
        let nack = GenericNack {
            ssrc_sender: 1,
            ssrc_media: 2,
            nacks: alloc::vec![NackFci { pid: 1, blp: 0 }],
        };
        let mut bytes = nack.to_bytes();
        // Change FMT from 1 to 2 (keep V=2, P=0).
        bytes[0] = (RTCP_VERSION << 6) | 2;
        let err = GenericNack::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidFmt {
                expected: 1,
                got: 2
            }
        ));
    }

    #[test]
    fn range_nack_too_many_ranges_serialize() {
        let rn = RangeNack {
            ssrc_media: 1,
            ranges: (0..17)
                .map(|i| PacketRange {
                    start: i * 10,
                    additional: 5,
                })
                .collect(),
        };
        let mut buf = alloc::vec![0u8; rn.serialized_len()];
        let err = rn.serialize_into(&mut buf).unwrap_err();
        assert!(matches!(err, Error::TooManyRanges { max: 16, got: 17 }));
    }

    #[test]
    fn range_nack_too_many_ranges_parse() {
        // Build a wire packet with 17 range entries (exceeds the 16 limit).
        let entry_count: usize = 17;
        let total_words = 3 + entry_count; // header(1) + SSRC(1) + name(1) + entries
        let total_bytes = total_words * 4;
        let mut buf = alloc::vec![0u8; total_bytes];
        // V=2, P=0, Subtype=0
        buf[0] = 0x80;
        // PT=204
        buf[1] = PT_APP;
        // length = total_words - 1
        let length_field = (total_words - 1) as u16;
        buf[2..4].copy_from_slice(&length_field.to_be_bytes());
        // SSRC
        buf[4..8].copy_from_slice(&1u32.to_be_bytes());
        // APP name = "RIST"
        buf[8..12].copy_from_slice(b"RIST");
        // 17 range entries (all zeros is fine)
        let err = RangeNack::parse(&buf).unwrap_err();
        assert!(matches!(err, Error::TooManyRanges { max: 16, got: 17 }));
    }
}
