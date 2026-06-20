//! CA PMT Reply object (`ca_pmt_reply`) — ETSI EN 50221 §8.4.3.5, Table 26
//! (PDF p. 32).
//!
//! `ca_pmt_reply` (`9F 80 33`, app → host) reports descrambling capability. It
//! has one optional programme-level `CA_enable` (gated by a `CA_enable_flag`
//! bit) followed by a per-ES list, each ES with its own `CA_enable_flag` +
//! optional 7-bit `CA_enable`.

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// `CA_enable` 7-bit value (Table, p. 32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CaEnable {
    /// `01` — descrambling possible.
    Possible,
    /// `02` — possible under conditions (purchase dialogue).
    PossiblePurchaseDialogue,
    /// `03` — possible under conditions (technical dialogue).
    PossibleTechnicalDialogue,
    /// `71` — not possible (no entitlement).
    NotPossibleNoEntitlement,
    /// `73` — not possible (technical reasons).
    NotPossibleTechnical,
    /// Any other 7-bit value (RFU).
    Rfu(u8),
}

impl CaEnable {
    /// Decode the low 7 bits of a `CA_enable` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x7F {
            0x01 => Self::Possible,
            0x02 => Self::PossiblePurchaseDialogue,
            0x03 => Self::PossibleTechnicalDialogue,
            0x71 => Self::NotPossibleNoEntitlement,
            0x73 => Self::NotPossibleTechnical,
            other => Self::Rfu(other),
        }
    }
    /// The 7-bit wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Possible => 0x01,
            Self::PossiblePurchaseDialogue => 0x02,
            Self::PossibleTechnicalDialogue => 0x03,
            Self::NotPossibleNoEntitlement => 0x71,
            Self::NotPossibleTechnical => 0x73,
            Self::Rfu(v) => v & 0x7F,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Possible => "descrambling_possible",
            Self::PossiblePurchaseDialogue => "possible_purchase_dialogue",
            Self::PossibleTechnicalDialogue => "possible_technical_dialogue",
            Self::NotPossibleNoEntitlement => "not_possible_no_entitlement",
            Self::NotPossibleTechnical => "not_possible_technical",
            Self::Rfu(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(CaEnable, Rfu);

/// One ES entry in a `ca_pmt_reply`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaPmtReplyStream {
    /// 13-bit `elementary_PID`.
    pub elementary_pid: u16,
    /// ES-level `CA_enable` — `Some` iff the `CA_enable_flag` bit was set.
    pub ca_enable: Option<CaEnable>,
}

/// `ca_pmt_reply()` object (Table 26).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaPmtReply {
    /// `program_number`.
    pub program_number: u16,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// `current_next_indicator`.
    pub current_next_indicator: bool,
    /// Programme-level `CA_enable` — `Some` iff the programme `CA_enable_flag` bit
    /// was set.
    pub ca_enable: Option<CaEnable>,
    /// Per-ES entries in wire order.
    pub streams: Vec<CaPmtReplyStream>,
}

const REPLY_PREFIX: usize = 4; // program_number(2) + version/cni/flag/enable(2)
const ES_LEN: usize = 3; // reserved/elem_pid(2) + flag/enable(1)

impl<'a> Parse<'a> for CaPmtReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::CA_PMT_REPLY, "ca_pmt_reply")?;
        if body.len() < REPLY_PREFIX {
            return Err(Error::BufferTooShort {
                need: REPLY_PREFIX,
                have: body.len(),
                what: "ca_pmt_reply prefix",
            });
        }
        let program_number = u16::from_be_bytes([body[0], body[1]]);
        let version_number = (body[2] >> 1) & 0x1F;
        let current_next_indicator = (body[2] & 0x01) != 0;
        let ca_enable_flag = (body[3] & 0x80) != 0;
        let ca_enable = if ca_enable_flag {
            Some(CaEnable::from_u8(body[3] & 0x7F))
        } else {
            None
        };

        let mut pos = REPLY_PREFIX;
        let mut streams = Vec::new();
        while pos < body.len() {
            if pos + ES_LEN > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + ES_LEN,
                    have: body.len(),
                    what: "ca_pmt_reply ES",
                });
            }
            let elementary_pid = (((body[pos] & 0x1F) as u16) << 8) | body[pos + 1] as u16;
            let es_flag = (body[pos + 2] & 0x80) != 0;
            let es_enable = if es_flag {
                Some(CaEnable::from_u8(body[pos + 2] & 0x7F))
            } else {
                None
            };
            streams.push(CaPmtReplyStream {
                elementary_pid,
                ca_enable: es_enable,
            });
            pos += ES_LEN;
        }

        Ok(Self {
            program_number,
            version_number,
            current_next_indicator,
            ca_enable,
            streams,
        })
    }
}

impl Serialize for CaPmtReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(REPLY_PREFIX + self.streams.len() * ES_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body = REPLY_PREFIX + self.streams.len() * ES_LEN;
        let mut pos = super::write_apdu_header(tag::CA_PMT_REPLY, body, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.program_number.to_be_bytes());
        // reserved(2)='11', version(5), current_next(1).
        buf[pos + 2] =
            0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[pos + 3] = encode_enable_byte(self.ca_enable);
        pos += REPLY_PREFIX;
        for s in &self.streams {
            // reserved(3)='111', elementary_PID(13).
            buf[pos] = 0xE0 | ((s.elementary_pid >> 8) as u8 & 0x1F);
            buf[pos + 1] = s.elementary_pid as u8;
            buf[pos + 2] = encode_enable_byte(s.ca_enable);
            pos += ES_LEN;
        }
        Ok(pos)
    }
}

/// Encode a `CA_enable_flag` + 7-bit `CA_enable`/reserved byte. When absent the
/// flag is 0 and the 7 reserved bits are set (`0x7F`) per the reserved-bit
/// convention.
fn encode_enable_byte(enable: Option<CaEnable>) -> u8 {
    match enable {
        Some(e) => 0x80 | (e.to_u8() & 0x7F),
        None => 0x7F,
    }
}

impl<'a> ApduDef<'a> for CaPmtReply {
    const TAG: ApduTag = tag::CA_PMT_REPLY;
    const NAME: &'static str = "CA_PMT_REPLY";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_with_prog_and_es_enable_round_trips() {
        let r = CaPmtReply {
            program_number: 0x0001,
            version_number: 1,
            current_next_indicator: true,
            ca_enable: Some(CaEnable::Possible),
            streams: alloc::vec![
                CaPmtReplyStream {
                    elementary_pid: 0x0200,
                    ca_enable: Some(CaEnable::Possible),
                },
                CaPmtReplyStream {
                    elementary_pid: 0x0201,
                    ca_enable: Some(CaEnable::NotPossibleNoEntitlement),
                },
            ],
        };
        let bytes = r.to_bytes();
        assert_eq!(&bytes[..3], &[0x9F, 0x80, 0x33]);
        let parsed = CaPmtReply::parse(&bytes).unwrap();
        assert_eq!(parsed, r);
        assert_eq!(parsed.streams.len(), 2);
        assert_eq!(parsed.ca_enable.unwrap().name(), "descrambling_possible");
    }

    #[test]
    fn reply_without_enable_flags() {
        let r = CaPmtReply {
            program_number: 9,
            version_number: 0,
            current_next_indicator: true,
            ca_enable: None,
            streams: alloc::vec![CaPmtReplyStream {
                elementary_pid: 0x00FF,
                ca_enable: None,
            }],
        };
        let bytes = r.to_bytes();
        let parsed = CaPmtReply::parse(&bytes).unwrap();
        assert_eq!(parsed, r);
        assert!(parsed.ca_enable.is_none());
        assert!(parsed.streams[0].ca_enable.is_none());
    }

    #[test]
    fn mutating_enable_changes_bytes() {
        let r = CaPmtReply {
            program_number: 1,
            version_number: 1,
            current_next_indicator: true,
            ca_enable: Some(CaEnable::Possible),
            streams: Vec::new(),
        };
        let a = r.to_bytes();
        let mut other = r.clone();
        other.ca_enable = Some(CaEnable::NotPossibleNoEntitlement);
        assert_ne!(a, other.to_bytes());
    }
}
