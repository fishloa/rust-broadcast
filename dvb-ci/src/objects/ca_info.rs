//! CA Info objects — ETSI EN 50221 §8.4.3.1-8.4.3.2, Tables 23-24 (PDF p. 29).
//!
//! - `ca_info_enq` (`9F 80 30`, Table 23) — header-only enquiry.
//! - `ca_info` (`9F 80 31`, Table 24) — list of supported `CA_system_id`s.

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// `ca_info_enq()` — header-only enquiry (Table 23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaInfoEnq;

/// `ca_info()` reply — the `CA_system_id`s this application supports (Table 24).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaInfo {
    /// Supported `CA_system_id` values (ETSI TS 101 162), in wire order.
    pub ca_system_ids: Vec<u16>,
}

impl<'a> Parse<'a> for CaInfoEnq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        super::parse_empty_apdu(bytes, tag::CA_INFO_ENQ, "ca_info_enq")?;
        Ok(Self)
    }
}
impl Serialize for CaInfoEnq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::serialize_empty_apdu(tag::CA_INFO_ENQ, buf)
    }
}
impl<'a> ApduDef<'a> for CaInfoEnq {
    const TAG: ApduTag = tag::CA_INFO_ENQ;
    const NAME: &'static str = "CA_INFO_ENQ";
}

impl<'a> Parse<'a> for CaInfo {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::CA_INFO, "ca_info")?;
        if body.len() % 2 != 0 {
            return Err(Error::InvalidObject {
                what: "ca_info",
                reason: "body length is not a multiple of 2",
            });
        }
        let ca_system_ids = body
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        Ok(Self { ca_system_ids })
    }
}

impl Serialize for CaInfo {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(self.ca_system_ids.len() * 2)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.ca_system_ids.len() * 2;
        let mut pos = super::write_apdu_header(tag::CA_INFO, body_len, buf)?;
        for id in &self.ca_system_ids {
            buf[pos..pos + 2].copy_from_slice(&id.to_be_bytes());
            pos += 2;
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for CaInfo {
    const TAG: ApduTag = tag::CA_INFO;
    const NAME: &'static str = "CA_INFO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enq_round_trip() {
        let bytes = CaInfoEnq.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x30, 0x00]);
        assert_eq!(CaInfoEnq::parse(&bytes).unwrap(), CaInfoEnq);
    }

    #[test]
    fn ca_info_multi_round_trips() {
        let info = CaInfo {
            ca_system_ids: alloc::vec![0x0500, 0x0B00, 0x1801],
        };
        let bytes = info.to_bytes();
        assert_eq!(&bytes[..4], &[0x9F, 0x80, 0x31, 0x06]); // len 6
        let parsed = CaInfo::parse(&bytes).unwrap();
        assert_eq!(parsed, info);
        assert_eq!(parsed.ca_system_ids.len(), 3);
    }

    #[test]
    fn mutating_id_changes_bytes() {
        let info = CaInfo {
            ca_system_ids: alloc::vec![0x0500],
        };
        let a = info.to_bytes();
        let mut other = info.clone();
        other.ca_system_ids[0] = 0x0B00;
        assert_ne!(a, other.to_bytes());
    }
}
