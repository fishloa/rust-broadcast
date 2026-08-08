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
//! Both types implement [`Serialize`] to produce the compound RTCP bytes.

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::Serialize;
use rtcp_packet::{
    ReceiverReport, SdesChunk, SdesItem, SdesItemType, SenderReport, SourceDescription,
};

use crate::error::{Error, Result};
use crate::nack::{GenericNack, RangeNack};
use crate::rtt_echo::RttEcho;

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
