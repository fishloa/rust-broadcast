//! RIST compound RTCP packet builders — VSF TR-06-1:2020 §5.2.1.
//!
//! RIST mandates that RTCP compound packets follow the RFC 3550 §6.1 structure
//! (SR or RR first, then SDES) with RIST-specific extensions appended:
//! retransmission NACKs and RTT Echo messages.
//!
//! - [`RistSenderCompound`] — sender-side compound: SR (or empty RR) +
//!   SDES(CNAME) + optional RTT Echo.
//! - [`RistReceiverCompound`] — receiver-side compound: RR + SDES(CNAME) +
//!   optional Generic/Range NACKs + optional RTT Echo.
//!
//! Both types implement [`Parse`]/[`Serialize`] for byte-exact round-trip.

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};
use rtcp_packet::{
    ReceiverReport, SdesChunk, SdesItem, SdesItemType, SenderReport, SourceDescription,
};

use crate::error::{Error, Result};
use crate::nack::{GenericNack, RangeNack};
use crate::rtt_echo::RttEcho;
use crate::{
    PT_RTPFB, RTCP_COUNT_MASK, SUBTYPE_RANGE_NACK, SUBTYPE_RTT_ECHO_REQUEST,
    SUBTYPE_RTT_ECHO_RESPONSE,
};

// ---------------------------------------------------------------------------
// Wire constants
// ---------------------------------------------------------------------------

/// Common-header length in bytes.
const RTCP_HEADER_LEN: usize = 4;
/// One 32-bit word, in bytes.
const WORD_LEN: usize = 4;
/// PT for RTCP APP (RFC 3550 §6.7).
const PT_APP: u8 = 204;

/// Read the total wire length (bytes) of the RTCP sub-packet at the front of
/// `bytes`, from its 4-byte common header `length` field (RFC 3550 §6.1):
/// `(length + 1) * 4`.
fn peek_total_len(bytes: &[u8]) -> Result<usize> {
    if bytes.len() < RTCP_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: RTCP_HEADER_LEN,
            have: bytes.len(),
        });
    }
    let length_field = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    Ok((length_field + 1) * WORD_LEN)
}

/// Parse one RTCP sub-packet from the front of `bytes`, returning the decoded
/// value and the number of bytes it consumed per its own common-header
/// `length` field (RFC 3550 §6.1). Works for any sub-packet type, whether its
/// `Parse::Error` is `rist_runtime::Error` (the RIST-specific types) or
/// `rtcp_packet::Error` (the underlying SR/RR/SDES types) — both convert into
/// [`Error`] via `?`.
fn parse_one<'a, T>(bytes: &'a [u8]) -> Result<(T, usize)>
where
    T: Parse<'a>,
    Error: From<T::Error>,
{
    let total = peek_total_len(bytes)?;
    if bytes.len() < total {
        return Err(Error::BufferTooShort {
            need: total,
            have: bytes.len(),
        });
    }
    let value = T::parse(&bytes[..total]).map_err(Error::from)?;
    Ok((value, total))
}

/// Extract the CNAME text from a parsed SDES packet — every RIST compound
/// packet carries exactly one (TR-06-1:2020 §5.2.1).
fn extract_cname(sdes: &SourceDescription) -> Result<String> {
    sdes.chunks
        .iter()
        .flat_map(|chunk| chunk.items.iter())
        .find(|item| item.item_type == SdesItemType::CName)
        .map(|item| item.text.clone())
        .ok_or(Error::MissingCname)
}

/// Build a RIST sender compound RTCP packet (TR-06-1 §5.2.1).
///
/// Structure: SR (or empty RR) + SDES(CNAME) + optional RTT Echo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RistSenderCompound {
    /// The Sender Report.
    pub sr: SenderReport,
    /// The CNAME string for the SDES chunk.
    pub cname: String,
    /// Optional RTT Echo Request or Response.
    pub rtt_echo: Option<RttEcho>,
}

/// Build an SDES packet containing a single chunk with one CNAME item.
fn build_sdes(ssrc: u32, cname: &str) -> SourceDescription {
    SourceDescription {
        chunks: alloc::vec![SdesChunk {
            source: ssrc,
            items: alloc::vec![SdesItem {
                item_type: SdesItemType::CName,
                text: String::from(cname),
            }],
        }],
    }
}

impl Serialize for RistSenderCompound {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let sdes = build_sdes(self.sr.ssrc, &self.cname);
        let mut len = self.sr.serialized_len() + sdes.serialized_len();
        if let Some(ref echo) = self.rtt_echo {
            len += echo.serialized_len();
        }
        len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let mut off = 0;

        // 1. SR
        let n = self.sr.serialize_into(&mut buf[off..])?;
        off += n;

        // 2. SDES(CNAME)
        let sdes = build_sdes(self.sr.ssrc, &self.cname);
        let n = sdes.serialize_into(&mut buf[off..])?;
        off += n;

        // 3. Optional RTT Echo
        if let Some(ref echo) = self.rtt_echo {
            let n = echo.serialize_into(&mut buf[off..])?;
            off += n;
        }

        Ok(off)
    }
}

impl<'a> Parse<'a> for RistSenderCompound {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut off = 0;

        // 1. SR
        let (sr, n) = parse_one::<SenderReport>(&bytes[off..])?;
        off += n;

        // 2. SDES(CNAME)
        let (sdes, n) = parse_one::<SourceDescription>(&bytes[off..])?;
        off += n;
        let cname = extract_cname(&sdes)?;

        // 3. Optional RTT Echo — at most one, and it must be the last thing
        // in the compound packet.
        let rtt_echo = if off < bytes.len() {
            let (echo, n) = parse_one::<RttEcho>(&bytes[off..])?;
            off += n;
            Some(echo)
        } else {
            None
        };

        if off != bytes.len() {
            return Err(Error::TrailingData(bytes.len() - off));
        }

        Ok(RistSenderCompound {
            sr,
            cname,
            rtt_echo,
        })
    }
}

/// Build a RIST receiver compound RTCP packet (TR-06-1 §5.2.1).
///
/// Structure: RR (with 0 or 1 report blocks) + SDES(CNAME) + optional
/// Generic/Range NACKs + optional RTT Echo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RistReceiverCompound {
    /// The Receiver Report (0 or 1 report blocks).
    pub rr: ReceiverReport,
    /// The CNAME string for the SDES chunk.
    pub cname: String,
    /// Optional Generic NACKs (RFC 4585, PT 205).
    pub nacks: Vec<GenericNack>,
    /// Optional Range NACKs (RIST APP, PT 204).
    pub range_nacks: Vec<RangeNack>,
    /// Optional RTT Echo Request or Response.
    pub rtt_echo: Option<RttEcho>,
}

impl Serialize for RistReceiverCompound {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let sdes = build_sdes(self.rr.ssrc, &self.cname);
        let mut len = self.rr.serialized_len() + sdes.serialized_len();
        for nack in &self.nacks {
            len += nack.serialized_len();
        }
        for rn in &self.range_nacks {
            len += rn.serialized_len();
        }
        if let Some(ref echo) = self.rtt_echo {
            len += echo.serialized_len();
        }
        len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let mut off = 0;

        // 1. RR
        let n = self.rr.serialize_into(&mut buf[off..])?;
        off += n;

        // 2. SDES(CNAME)
        let sdes = build_sdes(self.rr.ssrc, &self.cname);
        let n = sdes.serialize_into(&mut buf[off..])?;
        off += n;

        // 3. Generic NACKs
        for nack in &self.nacks {
            let n = nack.serialize_into(&mut buf[off..])?;
            off += n;
        }

        // 4. Range NACKs
        for rn in &self.range_nacks {
            let n = rn.serialize_into(&mut buf[off..])?;
            off += n;
        }

        // 5. Optional RTT Echo
        if let Some(ref echo) = self.rtt_echo {
            let n = echo.serialize_into(&mut buf[off..])?;
            off += n;
        }

        Ok(off)
    }
}

impl<'a> Parse<'a> for RistReceiverCompound {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut off = 0;

        // 1. RR
        let (rr, n) = parse_one::<ReceiverReport>(&bytes[off..])?;
        off += n;

        // 2. SDES(CNAME)
        let (sdes, n) = parse_one::<SourceDescription>(&bytes[off..])?;
        off += n;
        let cname = extract_cname(&sdes)?;

        // 3. Zero or more Generic/Range NACKs, then an optional single RTT
        // Echo — classified by (PT, subtype), consumed in wire order.
        let mut nacks = Vec::new();
        let mut range_nacks = Vec::new();
        let mut rtt_echo = None;

        while off < bytes.len() {
            let pt = *bytes.get(off + 1).ok_or(Error::BufferTooShort {
                need: off + 2,
                have: bytes.len(),
            })?;
            match pt {
                PT_RTPFB => {
                    let (nack, n) = parse_one::<GenericNack>(&bytes[off..])?;
                    nacks.push(nack);
                    off += n;
                }
                PT_APP => {
                    let subtype = bytes[off] & RTCP_COUNT_MASK;
                    match subtype {
                        SUBTYPE_RANGE_NACK => {
                            let (rn, n) = parse_one::<RangeNack>(&bytes[off..])?;
                            range_nacks.push(rn);
                            off += n;
                        }
                        SUBTYPE_RTT_ECHO_REQUEST | SUBTYPE_RTT_ECHO_RESPONSE => {
                            if rtt_echo.is_some() {
                                return Err(Error::DuplicateRttEcho);
                            }
                            let (echo, n) = parse_one::<RttEcho>(&bytes[off..])?;
                            rtt_echo = Some(echo);
                            off += n;
                        }
                        other => return Err(Error::InvalidSubtype(other)),
                    }
                }
                other => return Err(Error::UnexpectedPacketType(other)),
            }
        }

        Ok(RistReceiverCompound {
            rr,
            cname,
            nacks,
            range_nacks,
            rtt_echo,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtcp_packet::ReportBlock;

    #[test]
    fn sender_compound_sr_sdes() {
        let compound = RistSenderCompound {
            sr: SenderReport {
                ssrc: 0x1122_3344,
                ntp_msw: 0xE0E1_E2E3,
                ntp_lsw: 0x1020_3040,
                rtp_timestamp: 0x0009_0000,
                packet_count: 100,
                octet_count: 50_000,
                report_blocks: Vec::new(),
            },
            cname: String::from("sender@example.com"),
            rtt_echo: None,
        };
        let bytes = compound.to_bytes();
        // Should start with SR (PT 200).
        assert_eq!(bytes[1], 200);
        // SR length from header.
        let sr_len = (u16::from_be_bytes([bytes[2], bytes[3]]) as usize + 1) * 4;
        // Next packet should be SDES (PT 202).
        assert_eq!(bytes[sr_len + 1], 202);
    }

    #[test]
    fn receiver_compound_rr_sdes_nack() {
        let compound = RistReceiverCompound {
            rr: ReceiverReport {
                ssrc: 0xAAAA_BBBB,
                report_blocks: alloc::vec![ReportBlock {
                    ssrc: 0xCCCC_DDDD,
                    fraction_lost: 10,
                    cumulative_lost: 5,
                    ext_highest_seq: 0x0000_1000,
                    jitter: 100,
                    lsr: 0,
                    dlsr: 0,
                }],
            },
            cname: String::from("receiver@example.com"),
            nacks: alloc::vec![GenericNack {
                ssrc_sender: 0xAAAA_BBBB,
                ssrc_media: 0xCCCC_DDDD,
                nacks: alloc::vec![crate::nack::NackFci { pid: 500, blp: 0 }],
            }],
            range_nacks: Vec::new(),
            rtt_echo: None,
        };
        let bytes = compound.to_bytes();
        // Should start with RR (PT 201).
        assert_eq!(bytes[1], 201);
    }
}
