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
use crate::reserved::ReservedZero;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use mpeg_ts::pid::Pid;

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
pub const NULL_PID: Pid = Pid::new(0x1FFF);

// --- CICAM_multistream_capability (Table 3) ---

/// `CICAM_multistream_capability()` (Table 3): CICAM → Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CicamMultistreamCapability {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

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
        let (length_form, body) = objects::parse_apdu_header(
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
        crate::objects::reject_trailing_body(
            body,
            CAPABILITY_BODY,
            "CICAM_multistream_capability",
        )?;
        Ok(Self {
            length_form,
            max_local_ts: body[0],
            max_descramblers: u16::from_be_bytes([body[1], body[2]]),
        })
    }
}
impl Serialize for CicamMultistreamCapability {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CAPABILITY_BODY, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(
            tag::CICAM_MULTISTREAM_CAPABILITY,
            CAPABILITY_BODY,
            self.length_form,
            buf,
        )?;
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
    pub pid: Pid,
    /// The 2-bit `reserved` field in front of `critical_for_descrambling_flag` as received, preserved so the
    /// object re-serializes byte-for-byte; `0` when hand-built.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedZero<2>,
}

/// `PID_select_req()` (Table 4): CICAM → Host. PIDs are listed in descending
/// priority order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PidSelectReq {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

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
        let (length_form, body) =
            objects::parse_apdu_header(bytes, tag::PID_SELECT_REQ, "PID_select_req")?;
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
            let pid = Pid::new(u16::from_be_bytes([chunk[0], chunk[1]]) & PID_MASK);
            pids.push(PidSelectRequest {
                critical_for_descrambling: critical,
                pid,
                reserved: ReservedZero::from_bits(chunk[0] >> 6),
            });
        }
        objects::reject_trailing_body(loop_bytes, num_pid * PID_ENTRY_LEN, "PID_select_req")?;
        Ok(Self {
            length_form,
            lts_id,
            pids,
        })
    }
}
impl Serialize for PidSelectReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(2 + self.pids.len() * PID_ENTRY_LEN, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::fit_bits(self.pids.len() as u64, 8, "PID_select_req num_PID")?;
        let body_len = 2 + self.pids.len() * PID_ENTRY_LEN;
        let mut pos =
            objects::write_apdu_header(tag::PID_SELECT_REQ, body_len, self.length_form, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.pids.len() as u8;
        pos += 2;
        for entry in &self.pids {
            let mut hi = (entry.reserved.get() << 6) | (entry.pid.value() >> 8) as u8;
            if entry.critical_for_descrambling {
                hi |= CRITICAL_FLAG_BIT;
            }
            buf[pos] = hi;
            buf[pos + 1] = entry.pid.value() as u8;
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
    pub pid: Pid,
    /// The 2-bit `reserved` field in front of `PID_selected_flag` as received, preserved so the
    /// object re-serializes byte-for-byte; `0` when hand-built.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedZero<2>,
}

/// `PID_select_reply()` (Table 5): Host → CICAM.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PidSelectReply {
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// `LTS_id` (8) — Local TS identifier.
    pub lts_id: u8,
    /// `PID_selection_flag` (1) — `false` = whole TS sent (`num_PID` shall be 0);
    /// `true` = PID selection applied and the loop lists the selected PIDs.
    pub pid_selection: bool,
    /// The per-PID selection results (loop count = `num_PID`).
    pub pids: Vec<PidSelectedEntry>,
    /// The 7-bit `reserved` field in front of `PID_selection_flag` as received, preserved so the
    /// object re-serializes byte-for-byte; `0` when hand-built.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedZero<7>,
}

// The PID_selected_flag bit within the high byte (bit 5 of the 16-bit word).
const PID_SELECTED_FLAG_BIT: u8 = 0x20;
// The PID_selection_flag bit (LSB of the reserved(7)+flag(1) byte).
const PID_SELECTION_FLAG_BIT: u8 = 0x01;

impl<'a> Parse<'a> for PidSelectReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, body) =
            objects::parse_apdu_header(bytes, tag::PID_SELECT_REPLY, "PID_select_reply")?;
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
            let pid = Pid::new(u16::from_be_bytes([chunk[0], chunk[1]]) & PID_MASK);
            pids.push(PidSelectedEntry {
                pid_selected: selected,
                pid,
                reserved: ReservedZero::from_bits(chunk[0] >> 6),
            });
        }
        objects::reject_trailing_body(loop_bytes, num_pid * PID_ENTRY_LEN, "PID_select_reply")?;
        Ok(Self {
            length_form,
            lts_id,
            pid_selection,
            pids,
            reserved: ReservedZero::from_bits(body[1] >> 1),
        })
    }
}
impl Serialize for PidSelectReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(3 + self.pids.len() * PID_ENTRY_LEN, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        objects::fit_bits(self.pids.len() as u64, 8, "PID_select_reply num_PID")?;
        let body_len = 3 + self.pids.len() * PID_ENTRY_LEN;
        let mut pos =
            objects::write_apdu_header(tag::PID_SELECT_REPLY, body_len, self.length_form, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = (self.reserved.get() << 1)
            | if self.pid_selection {
                PID_SELECTION_FLAG_BIT
            } else {
                0
            };
        buf[pos + 2] = self.pids.len() as u8;
        pos += 3;
        for entry in &self.pids {
            let mut hi = (entry.reserved.get() << 6) | (entry.pid.value() >> 8) as u8;
            if entry.pid_selected {
                hi |= PID_SELECTED_FLAG_BIT;
            }
            buf[pos] = hi;
            buf[pos + 1] = entry.pid.value() as u8;
            pos += PID_ENTRY_LEN;
        }
        Ok(pos)
    }
}

crate::dispatch::declare_resource_apdus! {
    /// Resource-scoped dispatch over the Multi-stream resource objects.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    #[non_exhaustive]
    pub enum MultistreamApdu ("multistream") {
        /// `CICAM_multistream_capability` (`9F 92 00`).
        CicamMultistreamCapability(CicamMultistreamCapability) = tag::CICAM_MULTISTREAM_CAPABILITY,
        /// `PID_select_req` (`9F 92 01`).
        PidSelectReq(PidSelectReq) = tag::PID_SELECT_REQ,
        /// `PID_select_reply` (`9F 92 02`).
        PidSelectReply(PidSelectReply) = tag::PID_SELECT_REPLY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_round_trips_and_bites() {
        let cap = CicamMultistreamCapability {
            length_form: crate::length::LengthForm::Minimal,
            max_local_ts: 0x04,
            max_descramblers: 0x0102,
        };
        let bytes = cap.try_to_bytes().unwrap();
        // tag(3) + len(0x03) + max_local_TS(0x04) + max_descramblers(01 02).
        assert_eq!(bytes, [0x9F, 0x92, 0x00, 0x03, 0x04, 0x01, 0x02]);
        assert_eq!(CicamMultistreamCapability::parse(&bytes).unwrap(), cap);
        let mut other = cap;
        other.max_descramblers = 0x0103;
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn pid_select_req_round_trips_and_bites() {
        let req = PidSelectReq {
            length_form: crate::length::LengthForm::Minimal,
            lts_id: 0x07,
            pids: alloc::vec![
                PidSelectRequest {
                    reserved: Default::default(),
                    critical_for_descrambling: true,
                    pid: Pid::new(0x0123),
                },
                PidSelectRequest {
                    reserved: Default::default(),
                    critical_for_descrambling: false,
                    pid: Pid::new(0x1FFE),
                },
            ],
        };
        let bytes = req.try_to_bytes().unwrap();
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
        assert_ne!(bytes, other.try_to_bytes().unwrap());
        assert_eq!(other.try_to_bytes().unwrap()[6], 0x01);
    }

    #[test]
    fn pid_select_req_empty_loop() {
        let req = PidSelectReq {
            length_form: crate::length::LengthForm::Minimal,
            lts_id: 0x00,
            pids: Vec::new(),
        };
        let bytes = req.try_to_bytes().unwrap();
        assert_eq!(bytes, [0x9F, 0x92, 0x01, 0x02, 0x00, 0x00]);
        assert_eq!(PidSelectReq::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn pid_select_reply_round_trips_with_two_entries() {
        let reply = PidSelectReply {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            lts_id: 0x05,
            pid_selection: true,
            pids: alloc::vec![
                PidSelectedEntry {
                    reserved: Default::default(),
                    pid_selected: true,
                    pid: Pid::new(0x0064),
                },
                PidSelectedEntry {
                    reserved: Default::default(),
                    pid_selected: false,
                    pid: Pid::new(0x00C8),
                },
            ],
        };
        let bytes = reply.try_to_bytes().unwrap();
        // tag(3) + len(0x07) + LTS_id(05) + flag-byte(0x01) + num_PID(02)
        //   entry0: selected=1 -> 0x20 | 0x00 = 0x20, lo=0x64
        //   entry1: selected=0 -> 0x00, lo=0xC8
        assert_eq!(
            bytes,
            [
                0x9F, 0x92, 0x02, 0x07, 0x05, 0x01, 0x02, 0x20, 0x64, 0x00, 0xC8
            ]
        );
        assert_eq!(PidSelectReply::parse(&bytes).unwrap(), reply);
        // Field-mutation: clear selection flag changes the wire byte.
        let mut other = reply.clone();
        other.pid_selection = false;
        assert_eq!(other.try_to_bytes().unwrap()[5], 0x00);
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn pid_select_reply_whole_ts() {
        let reply = PidSelectReply {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            lts_id: 0x01,
            pid_selection: false,
            pids: Vec::new(),
        };
        let bytes = reply.try_to_bytes().unwrap();
        assert_eq!(bytes, [0x9F, 0x92, 0x02, 0x03, 0x01, 0x00, 0x00]);
        assert_eq!(PidSelectReply::parse(&bytes).unwrap(), reply);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let cap = CicamMultistreamCapability {
            length_form: crate::length::LengthForm::Minimal,
            max_local_ts: 1,
            max_descramblers: 1,
        }
        .try_to_bytes()
        .unwrap();
        assert!(matches!(
            MultistreamApdu::parse(&cap).unwrap(),
            MultistreamApdu::CicamMultistreamCapability(_)
        ));
        let reply = PidSelectReply {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            lts_id: 0,
            pid_selection: false,
            pids: Vec::new(),
        }
        .try_to_bytes()
        .unwrap();
        let parsed = MultistreamApdu::parse(&reply).unwrap();
        assert!(matches!(parsed, MultistreamApdu::PidSelectReply(_)));
        assert_eq!(parsed.try_to_bytes().unwrap(), reply);
    }

    #[test]
    fn max_req_pid_still_serializes_and_round_trips() {
        let req = PidSelectReq {
            length_form: crate::length::LengthForm::Minimal,
            lts_id: 0,
            pids: alloc::vec![PidSelectRequest {
                reserved: Default::default(),
                critical_for_descrambling: false,
                pid: Pid::new(0x1FFF),
            }],
        };
        let bytes = req.try_to_bytes().unwrap();
        assert_eq!(PidSelectReq::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn oversized_num_pid_req_is_rejected_not_wrapped() {
        // 256 entries silently wrapped num_PID (u8) to 0 before the fix.
        let req = PidSelectReq {
            length_form: crate::length::LengthForm::Minimal,
            lts_id: 0,
            pids: (0..256)
                .map(|_| PidSelectRequest {
                    reserved: Default::default(),
                    critical_for_descrambling: false,
                    pid: Pid::new(0x0100),
                })
                .collect(),
        };
        let mut buf = alloc::vec![0u8; 4096];
        assert!(matches!(
            req.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn oversized_pids_are_unrepresentable() {
        // The 13-bit PID fields are typed `Pid`: a value that previously had
        // to be rejected at serialize time cannot be constructed at all.
        assert!(Pid::try_new(0x2064).is_err());
        assert!(Pid::try_new(0x1FFF).is_ok());
    }
}
