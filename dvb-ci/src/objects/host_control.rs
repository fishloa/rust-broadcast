//! Host Control objects — ETSI EN 50221 §8.5.1, Tables 27-30 (PDF pp. 33-34).
//!
//! - `tune` (`9F 84 00`, Table 27) — retune to a different service.
//! - `replace` (`9F 84 01`, Table 28) — temporarily replace one PID with another.
//! - `clear_replace` (`9F 84 02`, Table 29) — undo Replace operations by reference.
//! - `ask_release` (`9F 84 03`, Table 30) — header-only release request.

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use broadcast_common::{Parse, Serialize};

/// `tune()` object (Table 27): retune to a different service. Parameters are the
/// EN 300 468 identifiers, each 16 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Tune {
    /// `network_id`.
    pub network_id: u16,
    /// `original_network_id`.
    pub original_network_id: u16,
    /// `transport_stream_id`.
    pub transport_stream_id: u16,
    /// `service_id`.
    pub service_id: u16,
}

// network_id(2) + original_network_id(2) + transport_stream_id(2) + service_id(2).
const TUNE_BODY: usize = 8;

impl<'a> Parse<'a> for Tune {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::TUNE, "tune")?;
        if body.len() < TUNE_BODY {
            return Err(Error::BufferTooShort {
                need: TUNE_BODY,
                have: body.len(),
                what: "tune",
            });
        }
        Ok(Self {
            network_id: u16::from_be_bytes([body[0], body[1]]),
            original_network_id: u16::from_be_bytes([body[2], body[3]]),
            transport_stream_id: u16::from_be_bytes([body[4], body[5]]),
            service_id: u16::from_be_bytes([body[6], body[7]]),
        })
    }
}

impl Serialize for Tune {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(TUNE_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::TUNE, TUNE_BODY, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.network_id.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[pos + 4..pos + 6].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[pos + 6..pos + 8].copy_from_slice(&self.service_id.to_be_bytes());
        pos += TUNE_BODY;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for Tune {
    const TAG: ApduTag = tag::TUNE;
    const NAME: &'static str = "TUNE";
}

/// `replace()` object (Table 28): temporarily replace one component PID with
/// another from the same multiplex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Replace {
    /// `replacement_ref` — application-allocated reference matched by Clear Replace.
    pub replacement_ref: u8,
    /// 13-bit `replaced_PID` — PID of the component to replace.
    pub replaced_pid: u16,
    /// 13-bit `replacement_PID` — PID to replace it with.
    pub replacement_pid: u16,
}

// replacement_ref(1) + reserved/replaced_PID(2) + reserved/replacement_PID(2).
const REPLACE_BODY: usize = 5;

impl<'a> Parse<'a> for Replace {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::REPLACE, "replace")?;
        if body.len() < REPLACE_BODY {
            return Err(Error::BufferTooShort {
                need: REPLACE_BODY,
                have: body.len(),
                what: "replace",
            });
        }
        let replacement_ref = body[0];
        let replaced_pid = (((body[1] & 0x1F) as u16) << 8) | body[2] as u16;
        let replacement_pid = (((body[3] & 0x1F) as u16) << 8) | body[4] as u16;
        Ok(Self {
            replacement_ref,
            replaced_pid,
            replacement_pid,
        })
    }
}

impl Serialize for Replace {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(REPLACE_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::REPLACE, REPLACE_BODY, buf)?;
        buf[pos] = self.replacement_ref;
        // reserved(3)='111', replaced_PID(13).
        buf[pos + 1] = 0xE0 | ((self.replaced_pid >> 8) as u8 & 0x1F);
        buf[pos + 2] = self.replaced_pid as u8;
        // reserved(3)='111', replacement_PID(13).
        buf[pos + 3] = 0xE0 | ((self.replacement_pid >> 8) as u8 & 0x1F);
        buf[pos + 4] = self.replacement_pid as u8;
        pos += REPLACE_BODY;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for Replace {
    const TAG: ApduTag = tag::REPLACE;
    const NAME: &'static str = "REPLACE";
}

/// `clear_replace()` object (Table 29): undo all Replace operations sharing a
/// `replacement_ref`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ClearReplace {
    /// `replacement_ref` — matches the value used in one or more Replace objects.
    pub replacement_ref: u8,
}

impl<'a> Parse<'a> for ClearReplace {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::CLEAR_REPLACE, "clear_replace")?;
        let replacement_ref = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "clear_replace replacement_ref",
        })?;
        Ok(Self { replacement_ref })
    }
}

impl Serialize for ClearReplace {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::CLEAR_REPLACE, 1, buf)?;
        buf[pos] = self.replacement_ref;
        pos += 1;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for ClearReplace {
    const TAG: ApduTag = tag::CLEAR_REPLACE;
    const NAME: &'static str = "CLEAR_REPLACE";
}

/// `ask_release()` object (Table 30): header-only release request from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AskRelease;

impl<'a> Parse<'a> for AskRelease {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        super::parse_empty_apdu(bytes, tag::ASK_RELEASE, "ask_release")?;
        Ok(Self)
    }
}

impl Serialize for AskRelease {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::empty_apdu_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        super::serialize_empty_apdu(tag::ASK_RELEASE, buf)
    }
}

impl<'a> ApduDef<'a> for AskRelease {
    const TAG: ApduTag = tag::ASK_RELEASE;
    const NAME: &'static str = "ASK_RELEASE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tune_round_trips_and_bites() {
        let t = Tune {
            network_id: 0x1122,
            original_network_id: 0x3344,
            transport_stream_id: 0x5566,
            service_id: 0x7788,
        };
        let bytes = t.to_bytes();
        assert_eq!(
            bytes,
            [0x9F, 0x84, 0x00, 0x08, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
        );
        assert_eq!(Tune::parse(&bytes).unwrap(), t);
        let mut other = t;
        other.service_id = 0x9999;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn replace_round_trips_and_bites() {
        let r = Replace {
            replacement_ref: 0x07,
            replaced_pid: 0x0123,    // 13-bit
            replacement_pid: 0x01FF, // 13-bit
        };
        let bytes = r.to_bytes();
        // body: 07, reserved(111)+0x0123 => 0xE1 0x23, reserved+0x01FF => 0xE1 0xFF
        assert_eq!(
            bytes,
            [0x9F, 0x84, 0x01, 0x05, 0x07, 0xE1, 0x23, 0xE1, 0xFF]
        );
        let parsed = Replace::parse(&bytes).unwrap();
        assert_eq!(parsed, r);
        let mut other = r;
        other.replacement_pid = 0x0001;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn clear_replace_round_trips_and_bites() {
        let c = ClearReplace {
            replacement_ref: 0x42,
        };
        let bytes = c.to_bytes();
        assert_eq!(bytes, [0x9F, 0x84, 0x02, 0x01, 0x42]);
        assert_eq!(ClearReplace::parse(&bytes).unwrap(), c);
        let other = ClearReplace {
            replacement_ref: 0x43,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn ask_release_round_trips() {
        let bytes = AskRelease.to_bytes();
        assert_eq!(bytes, [0x9F, 0x84, 0x03, 0x00]);
        assert_eq!(AskRelease::parse(&bytes).unwrap(), AskRelease);
    }
}
