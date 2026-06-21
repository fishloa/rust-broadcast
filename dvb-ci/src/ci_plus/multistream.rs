//! Multi-stream resource objects — ETSI TS 103 205 V1.4.1 §6.4.2, Tables 2-5
//! (PDF pp. 31-33). See `docs/ts_103_205/multi-stream-resource.md`.
//!
//! Resource ID `0x00900041` (Class 144, Type 1, Version 1) — a **new** CI Plus
//! resource with no EN 50221 equivalent. The CICAM advertises its multi-stream
//! capabilities and the Host/CICAM negotiate PID selection in the Local TSs.
//!
//! - `CICAM_multistream_capability` (`9F 92 00`, Table 3) — CICAM → Host.
//! - `PID_select_req` (`9F 92 01`, Table 4) — CICAM → Host.
//! - `PID_select_reply` (`9F 92 02`, Table 5) — Host → CICAM.
//!
//! These apdu_tags live in the CI Plus `0x9F92xx` namespace and are dispatched
//! resource-scoped by [`crate::ci_plus::CiPlusApdu`].

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Multi-stream resource (Table 2).
pub mod tag {
    use crate::tag::ApduTag;
    /// `CICAM_multistream_capability_tag` = `9F 92 00`.
    pub const CICAM_MULTISTREAM_CAPABILITY: ApduTag = ApduTag::from_bytes(0x9F, 0x92, 0x00);
    /// `PID_select_req_tag` = `9F 92 01`.
    pub const PID_SELECT_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x92, 0x01);
    /// `PID_select_reply_tag` = `9F 92 02`.
    pub const PID_SELECT_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x92, 0x02);
}

/// The reserved PID value `0x1FFF` — the Host shall ignore a request for it
/// (§6.4.2.3, Table 4 semantics).
pub const NULL_PID: u16 = 0x1FFF;

// --- CICAM_multistream_capability (Table 3) ---

/// `CICAM_multistream_capability()` (Table 3): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CicamMultistreamCapability {
    /// `max_local_TS` (8) — maximum number of Local TSs the CICAM can receive
    /// concurrently.
    pub max_local_ts: u8,
    /// `max_descramblers` (16) — total number of descramblers the CICAM can
    /// provide concurrently across all Local TSs.
    pub max_descramblers: u16,
}

// max_local_TS(1) + max_descramblers(2).
const CAPABILITY_BODY: usize = 1 + 2;

impl<'a> Parse<'a> for CicamMultistreamCapability {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(
            bytes,
            tag::CICAM_MULTISTREAM_CAPABILITY,
            "CICAM_multistream_capability",
        )?;
        if body.len() < CAPABILITY_BODY {
            return Err(Error::BufferTooShort {
                need: CAPABILITY_BODY,
                have: body.len(),
                what: "CICAM_multistream_capability",
            });
        }
        Ok(Self {
            max_local_ts: body[0],
            max_descramblers: u16::from_be_bytes([body[1], body[2]]),
        })
    }
}
impl Serialize for CicamMultistreamCapability {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CAPABILITY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos =
            objects::write_apdu_header(tag::CICAM_MULTISTREAM_CAPABILITY, CAPABILITY_BODY, buf)?;
        buf[pos] = self.max_local_ts;
        buf[pos + 1..pos + 3].copy_from_slice(&self.max_descramblers.to_be_bytes());
        Ok(pos + CAPABILITY_BODY)
    }
}

// --- PID_select_req (Table 4) ---

/// One entry of the `PID_select_req` loop (Table 4): `reserved(2)` +
/// `critical_for_descrambling_flag(1)` + `PID(13)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PidSelectRequest {
    /// `critical_for_descrambling_flag` (1) — `true` if the PID is critical for
    /// descrambling.
    pub critical_for_descrambling: bool,
    /// `PID` (13) — requested PID value (the Host ignores [`NULL_PID`]).
    pub pid: u16,
}

/// `PID_select_req()` (Table 4): CICAM → Host. PIDs are listed in descending
/// priority order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PidSelectReq {
    /// `LTS_id` (8) — Local TS identifier.
    pub lts_id: u8,
    /// The requested PIDs (loop count = `num_PID`).
    pub pids: Vec<PidSelectRequest>,
}

// Each loop entry is 2 bytes.
const PID_ENTRY_LEN: usize = 2;
// Mask for the 13-bit PID field.
const PID_MASK: u16 = 0x1FFF;
// The critical_for_descrambling_flag bit within the high byte.
const CRITICAL_FLAG_BIT: u8 = 0x20;

impl<'a> Parse<'a> for PidSelectReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::PID_SELECT_REQ, "PID_select_req")?;
        // LTS_id(1) + num_PID(1).
        if body.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: body.len(),
                what: "PID_select_req",
            });
        }
        let lts_id = body[0];
        let num_pid = body[1] as usize;
        let loop_bytes = &body[2..];
        if loop_bytes.len() < num_pid * PID_ENTRY_LEN {
            return Err(Error::BufferTooShort {
                need: num_pid * PID_ENTRY_LEN,
                have: loop_bytes.len(),
                what: "PID_select_req loop",
            });
        }
        let mut pids = Vec::with_capacity(num_pid);
        for chunk in loop_bytes[..num_pid * PID_ENTRY_LEN].chunks_exact(PID_ENTRY_LEN) {
            let critical = chunk[0] & CRITICAL_FLAG_BIT != 0;
            let pid = u16::from_be_bytes([chunk[0], chunk[1]]) & PID_MASK;
            pids.push(PidSelectRequest {
                critical_for_descrambling: critical,
                pid,
            });
        }
        Ok(Self { lts_id, pids })
    }
}
impl Serialize for PidSelectReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(2 + self.pids.len() * PID_ENTRY_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 2 + self.pids.len() * PID_ENTRY_LEN;
        let mut pos = objects::write_apdu_header(tag::PID_SELECT_REQ, body_len, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.pids.len() as u8;
        pos += 2;
        for entry in &self.pids {
            let mut hi = (entry.pid >> 8) as u8 & (PID_MASK >> 8) as u8;
            if entry.critical_for_descrambling {
                hi |= CRITICAL_FLAG_BIT;
            }
            buf[pos] = hi;
            buf[pos + 1] = entry.pid as u8;
            pos += PID_ENTRY_LEN;
        }
        Ok(pos)
    }
}

// --- PID_select_reply (Table 5) ---

/// One entry of the `PID_select_reply` loop (Table 5): `reserved(2)` +
/// `PID_selected_flag(1)` + `PID(13)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PidSelectedEntry {
    /// `PID_selected_flag` (1) — `true` if the PID could be selected successfully.
    pub pid_selected: bool,
    /// `PID` (13) — PID value to which `PID_selected_flag` applies.
    pub pid: u16,
}

/// `PID_select_reply()` (Table 5): Host → CICAM.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PidSelectReply {
    /// `LTS_id` (8) — Local TS identifier.
    pub lts_id: u8,
    /// `PID_selection_flag` (1) — `false` = whole TS sent (`num_PID` shall be 0);
    /// `true` = PID selection applied and the loop lists the selected PIDs.
    pub pid_selection: bool,
    /// The per-PID selection results (loop count = `num_PID`).
    pub pids: Vec<PidSelectedEntry>,
}

// The PID_selected_flag bit within the high byte (bit 5 of the 16-bit word).
const PID_SELECTED_FLAG_BIT: u8 = 0x20;
// The PID_selection_flag bit (LSB of the reserved(7)+flag(1) byte).
const PID_SELECTION_FLAG_BIT: u8 = 0x01;

impl<'a> Parse<'a> for PidSelectReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::PID_SELECT_REPLY, "PID_select_reply")?;
        // LTS_id(1) + reserved(7)+flag(1) byte + num_PID(1).
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "PID_select_reply",
            });
        }
        let lts_id = body[0];
        let pid_selection = body[1] & PID_SELECTION_FLAG_BIT != 0;
        let num_pid = body[2] as usize;
        let loop_bytes = &body[3..];
        if loop_bytes.len() < num_pid * PID_ENTRY_LEN {
            return Err(Error::BufferTooShort {
                need: num_pid * PID_ENTRY_LEN,
                have: loop_bytes.len(),
                what: "PID_select_reply loop",
            });
        }
        let mut pids = Vec::with_capacity(num_pid);
        for chunk in loop_bytes[..num_pid * PID_ENTRY_LEN].chunks_exact(PID_ENTRY_LEN) {
            let selected = chunk[0] & PID_SELECTED_FLAG_BIT != 0;
            let pid = u16::from_be_bytes([chunk[0], chunk[1]]) & PID_MASK;
            pids.push(PidSelectedEntry {
                pid_selected: selected,
                pid,
            });
        }
        Ok(Self {
            lts_id,
            pid_selection,
            pids,
        })
    }
}
impl Serialize for PidSelectReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(3 + self.pids.len() * PID_ENTRY_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 3 + self.pids.len() * PID_ENTRY_LEN;
        let mut pos = objects::write_apdu_header(tag::PID_SELECT_REPLY, body_len, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = if self.pid_selection {
            PID_SELECTION_FLAG_BIT
        } else {
            0
        };
        buf[pos + 2] = self.pids.len() as u8;
        pos += 3;
        for entry in &self.pids {
            let mut hi = (entry.pid >> 8) as u8 & (PID_MASK >> 8) as u8;
            if entry.pid_selected {
                hi |= PID_SELECTED_FLAG_BIT;
            }
            buf[pos] = hi;
            buf[pos + 1] = entry.pid as u8;
            pos += PID_ENTRY_LEN;
        }
        Ok(pos)
    }
}

/// Resource-scoped dispatch over the Multi-stream resource objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MultistreamApdu {
    /// `CICAM_multistream_capability` (`9F 92 00`).
    CicamMultistreamCapability(CicamMultistreamCapability),
    /// `PID_select_req` (`9F 92 01`).
    PidSelectReq(PidSelectReq),
    /// `PID_select_reply` (`9F 92 02`).
    PidSelectReply(PidSelectReply),
}

impl MultistreamApdu {
    /// Parse a Multi-stream APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "multistream apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::CICAM_MULTISTREAM_CAPABILITY => Ok(Self::CicamMultistreamCapability(
                CicamMultistreamCapability::parse(body)?,
            )),
            tag::PID_SELECT_REQ => Ok(Self::PidSelectReq(PidSelectReq::parse(body)?)),
            tag::PID_SELECT_REPLY => Ok(Self::PidSelectReply(PidSelectReply::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::CICAM_MULTISTREAM_CAPABILITY.as_u24(),
                what: "multistream",
            }),
        }
    }
}

impl Serialize for MultistreamApdu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::CicamMultistreamCapability(o) => o.serialized_len(),
            Self::PidSelectReq(o) => o.serialized_len(),
            Self::PidSelectReply(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::CicamMultistreamCapability(o) => o.serialize_into(buf),
            Self::PidSelectReq(o) => o.serialize_into(buf),
            Self::PidSelectReply(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_round_trips_and_bites() {
        let cap = CicamMultistreamCapability {
            max_local_ts: 0x04,
            max_descramblers: 0x0102,
        };
        let bytes = cap.to_bytes();
        // tag(3) + len(0x03) + max_local_TS(0x04) + max_descramblers(01 02).
        assert_eq!(bytes, [0x9F, 0x92, 0x00, 0x03, 0x04, 0x01, 0x02]);
        assert_eq!(CicamMultistreamCapability::parse(&bytes).unwrap(), cap);
        let mut other = cap;
        other.max_descramblers = 0x0103;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn pid_select_req_round_trips_and_bites() {
        let req = PidSelectReq {
            lts_id: 0x07,
            pids: alloc::vec![
                PidSelectRequest {
                    critical_for_descrambling: true,
                    pid: 0x0123,
                },
                PidSelectRequest {
                    critical_for_descrambling: false,
                    pid: 0x1FFE,
                },
            ],
        };
        let bytes = req.to_bytes();
        // tag(3) + len(0x06) + LTS_id(07) + num_PID(02)
        //   entry0: critical=1 -> 0x20 | (0x0123>>8=0x01) = 0x21, lo=0x23
        //   entry1: critical=0 -> (0x1FFE>>8=0x1F), lo=0xFE
        assert_eq!(
            bytes,
            [0x9F, 0x92, 0x01, 0x06, 0x07, 0x02, 0x21, 0x23, 0x1F, 0xFE]
        );
        assert_eq!(PidSelectReq::parse(&bytes).unwrap(), req);
        // Field-mutation: flip critical flag changes the wire.
        let mut other = req.clone();
        other.pids[0].critical_for_descrambling = false;
        assert_ne!(bytes, other.to_bytes());
        assert_eq!(other.to_bytes()[6], 0x01);
    }

    #[test]
    fn pid_select_req_empty_loop() {
        let req = PidSelectReq {
            lts_id: 0x00,
            pids: Vec::new(),
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes, [0x9F, 0x92, 0x01, 0x02, 0x00, 0x00]);
        assert_eq!(PidSelectReq::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn pid_select_reply_round_trips_with_two_entries() {
        let reply = PidSelectReply {
            lts_id: 0x05,
            pid_selection: true,
            pids: alloc::vec![
                PidSelectedEntry {
                    pid_selected: true,
                    pid: 0x0064,
                },
                PidSelectedEntry {
                    pid_selected: false,
                    pid: 0x00C8,
                },
            ],
        };
        let bytes = reply.to_bytes();
        // tag(3) + len(0x07) + LTS_id(05) + flag-byte(0x01) + num_PID(02)
        //   entry0: selected=1 -> 0x20 | 0x00 = 0x20, lo=0x64
        //   entry1: selected=0 -> 0x00, lo=0xC8
        assert_eq!(
            bytes,
            [0x9F, 0x92, 0x02, 0x07, 0x05, 0x01, 0x02, 0x20, 0x64, 0x00, 0xC8]
        );
        assert_eq!(PidSelectReply::parse(&bytes).unwrap(), reply);
        // Field-mutation: clear selection flag changes the wire byte.
        let mut other = reply.clone();
        other.pid_selection = false;
        assert_eq!(other.to_bytes()[5], 0x00);
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn pid_select_reply_whole_ts() {
        let reply = PidSelectReply {
            lts_id: 0x01,
            pid_selection: false,
            pids: Vec::new(),
        };
        let bytes = reply.to_bytes();
        assert_eq!(bytes, [0x9F, 0x92, 0x02, 0x03, 0x01, 0x00, 0x00]);
        assert_eq!(PidSelectReply::parse(&bytes).unwrap(), reply);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let cap = CicamMultistreamCapability {
            max_local_ts: 1,
            max_descramblers: 1,
        }
        .to_bytes();
        assert!(matches!(
            MultistreamApdu::parse(&cap).unwrap(),
            MultistreamApdu::CicamMultistreamCapability(_)
        ));
        let reply = PidSelectReply {
            lts_id: 0,
            pid_selection: false,
            pids: Vec::new(),
        }
        .to_bytes();
        let parsed = MultistreamApdu::parse(&reply).unwrap();
        assert!(matches!(parsed, MultistreamApdu::PidSelectReply(_)));
        assert_eq!(parsed.to_bytes(), reply);
    }
}
