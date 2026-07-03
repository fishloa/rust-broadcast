//! Close MMI object — ETSI EN 50221 §8.6.2.1, Table 33 (PDF p. 36).
//!
//! `close_mmi` (`9F 88 00`): a `close_mmi_cmd_id` byte, plus a `delay` byte when
//! the command id is `delay` (`01`).

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use broadcast_common::{Parse, Serialize};

/// `close_mmi_cmd_id` values (Table, p. 36).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CloseMmiCmdId {
    /// `00` — return to the previous display immediately.
    Immediate,
    /// `01` — delay the return; the `delay` byte gives the delay in seconds.
    Delay,
    /// Any other value (reserved).
    Reserved(u8),
}

impl CloseMmiCmdId {
    /// Decode a `close_mmi_cmd_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Immediate,
            0x01 => Self::Delay,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte for this command id.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Immediate => 0x00,
            Self::Delay => 0x01,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::Delay => "delay",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(CloseMmiCmdId, Reserved);

/// `close_mmi()` object (Table 33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CloseMmi {
    /// `close_mmi_cmd_id`.
    pub cmd_id: CloseMmiCmdId,
    /// `delay` (seconds) — present only when `cmd_id == delay`.
    pub delay: Option<u8>,
}

impl<'a> Parse<'a> for CloseMmi {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::CLOSE_MMI, "close_mmi")?;
        let cmd_byte = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "close_mmi cmd_id",
        })?;
        let cmd_id = CloseMmiCmdId::from_u8(cmd_byte);
        let delay = if cmd_id == CloseMmiCmdId::Delay {
            if body.len() < 2 {
                return Err(Error::InvalidObject {
                    what: "close_mmi",
                    reason: "cmd_id=delay requires a delay byte",
                });
            }
            Some(body[1])
        } else {
            None
        };
        Ok(Self { cmd_id, delay })
    }
}

impl Serialize for CloseMmi {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let body = 1 + usize::from(self.delay.is_some());
        super::apdu_len(body)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + usize::from(self.delay.is_some());
        let mut pos = super::write_apdu_header(tag::CLOSE_MMI, body_len, buf)?;
        buf[pos] = self.cmd_id.to_u8();
        pos += 1;
        if let Some(d) = self.delay {
            buf[pos] = d;
            pos += 1;
        }
        Ok(pos)
    }
}

impl ApduDef<'_> for CloseMmi {
    const TAG: ApduTag = tag::CLOSE_MMI;
    const NAME: &'static str = "CLOSE_MMI";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_round_trip() {
        let c = CloseMmi {
            cmd_id: CloseMmiCmdId::Immediate,
            delay: None,
        };
        let bytes = c.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x00, 0x01, 0x00]);
        assert_eq!(CloseMmi::parse(&bytes).unwrap(), c);
    }

    #[test]
    fn delay_round_trip() {
        let c = CloseMmi {
            cmd_id: CloseMmiCmdId::Delay,
            delay: Some(5),
        };
        let bytes = c.to_bytes();
        assert_eq!(bytes, [0x9F, 0x88, 0x00, 0x02, 0x01, 0x05]);
        let parsed = CloseMmi::parse(&bytes).unwrap();
        assert_eq!(parsed, c);
        assert_eq!(parsed.cmd_id.name(), "delay");
    }

    #[test]
    fn mutating_changes_bytes() {
        let c = CloseMmi {
            cmd_id: CloseMmiCmdId::Delay,
            delay: Some(5),
        };
        let a = c.to_bytes();
        let mut other = c;
        other.delay = Some(10);
        assert_ne!(a, other.to_bytes());
    }
}
