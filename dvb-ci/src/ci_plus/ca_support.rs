//! Conditional Access Support — CI Plus multi-stream `ca_pmt` / `ca_pmt_reply` —
//! ETSI TS 103 205 V1.4.1 §6.4.4, Tables 14/16 (PDF pp. 38-40). See
//! `docs/ts_103_205/ca-support.md`.
//!
//! For multi-stream functionality the CA Support resource gets a new
//! `resource_type` (= 2, version = 1) in which `ca_pmt()` and `ca_pmt_reply()`
//! are extended with a leading `LTS_id`; `ca_pmt()` additionally carries a
//! `PMT_PID`. The apdu_tags are unchanged (`ca_pmt` `0x9F8032`, `ca_pmt_reply`
//! `0x9F8033`).
//!
//! ## Deferred resource_id
//!
//! TS 103 205 §6.4.4.1 (render-verified) does **not** print a full 32-bit
//! `resource_identifier` for this resource_type — only the prose
//! "resource_type = 2, version = 1". The authoritative value is in CI Plus V1.3
//! \[3\]. So these objects are provided as standalone, directly-constructible /
//! parseable typed structs (full `Parse`/`Serialize`) and are **not** wired into
//! [`crate::ci_plus::CiPlusApdu`]'s resource dispatch — there is no invented
//! resource_id constant. A small [`CaSupportApdu::parse`] helper dispatches on
//! the apdu_tag for callers that already know they are in a multi-stream CA
//! Support session.
//!
//! These extended bodies reuse the EN 50221 value enums (`ca_pmt_list_management`,
//! `ca_pmt_cmd_id`, `CA_enable`); Table 15 narrows `ca_pmt_list_management` to the
//! `{Only=0x03, Update=0x05}` subset in multi-stream mode (a usage constraint —
//! any value still round-trips via the full enum).

use crate::error::{Error, Result};
use crate::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement};
use crate::objects::ca_pmt_reply::CaEnable;
use crate::tag::{ApduTag, CA_PMT, CA_PMT_REPLY};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

// Re-export the shared value enums so callers needn't reach into `objects`.
pub use crate::objects::ca_pmt::{
    CaPmtCmdId as MsCaPmtCmdId, CaPmtListManagement as MsCaPmtListManagement,
};
pub use crate::objects::ca_pmt_reply::CaEnable as MsCaEnable;

// ---------------------------------------------------------------------------
// ca_pmt (Table 14)
// ---------------------------------------------------------------------------

/// One elementary-stream entry of a CI Plus multi-stream `ca_pmt` (Table 14).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MsCaPmtStream<'a> {
    /// MPEG-2 `stream_type`.
    pub stream_type: u8,
    /// 13-bit `elementary_PID`.
    pub elementary_pid: u16,
    /// `ca_pmt_cmd_id` for this ES — present only when `ES_info_length != 0`.
    pub cmd_id: Option<CaPmtCmdId>,
    /// ES-level `CA_descriptor()` loop, verbatim wire bytes (no `cmd_id`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub ca_descriptors: &'a [u8],
}

/// CI Plus multi-stream `ca_pmt()` (Table 14): Host → CICAM. Differs from the
/// EN 50221 `ca_pmt` by the leading `LTS_id` and the added `PMT_PID`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MsCaPmt<'a> {
    /// `LTS_id` (8) — Local TS identifier.
    pub lts_id: u8,
    /// `ca_pmt_list_management` (Table 15 narrows to `{Only, Update}`).
    pub list_management: CaPmtListManagement,
    /// `program_number` (16).
    pub program_number: u16,
    /// `PMT_PID` (13) — PID of the selected service's PMT.
    pub pmt_pid: u16,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// `current_next_indicator`.
    pub current_next_indicator: bool,
    /// Programme-level `ca_pmt_cmd_id` — present only when `program_info_length != 0`.
    pub cmd_id: Option<CaPmtCmdId>,
    /// Programme-level `CA_descriptor()` loop, verbatim wire bytes (no `cmd_id`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub program_ca_descriptors: &'a [u8],
    /// Elementary streams in wire order.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub streams: Vec<MsCaPmtStream<'a>>,
}

// LTS_id(1) + list_management(1) + program_number(2) + reserved/PMT_PID(2) +
// reserved/version/cni(1) + reserved/program_info_length(2).
const MS_CA_PMT_PREFIX: usize = 1 + 1 + 2 + 2 + 1 + 2;
// Per-ES: stream_type(1) + reserved/elem_pid(2) + reserved/ES_info_length(2).
const MS_ES_PREFIX: usize = 5;

fn info_block_len(cmd_id: Option<CaPmtCmdId>, descriptors: &[u8]) -> usize {
    if cmd_id.is_some() || !descriptors.is_empty() {
        1 + descriptors.len()
    } else {
        0
    }
}

fn parse_cmd_and_descriptors<'a>(
    body: &'a [u8],
    pos: &mut usize,
    info_length: usize,
    what: &'static str,
) -> Result<(Option<CaPmtCmdId>, &'a [u8])> {
    if info_length == 0 {
        return Ok((None, &body[..0]));
    }
    let end = *pos + info_length;
    if end > body.len() {
        return Err(Error::LengthMismatch {
            what,
            declared: info_length,
            actual: body.len().saturating_sub(*pos),
        });
    }
    let cmd_id = CaPmtCmdId::from_u8(body[*pos]);
    let descriptors = &body[*pos + 1..end];
    *pos = end;
    Ok((Some(cmd_id), descriptors))
}

fn write_info_block(
    cmd_id: Option<CaPmtCmdId>,
    descriptors: &[u8],
    buf: &mut [u8],
) -> Result<usize> {
    let len = info_block_len(cmd_id, descriptors);
    if len == 0 {
        return Ok(0);
    }
    if buf.len() < len {
        return Err(Error::OutputBufferTooSmall {
            need: len,
            have: buf.len(),
        });
    }
    buf[0] = cmd_id.unwrap_or(CaPmtCmdId::OkDescrambling).to_u8();
    buf[1..len].copy_from_slice(descriptors);
    Ok(len)
}

impl<'a> Parse<'a> for MsCaPmt<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = crate::objects::parse_apdu_header(bytes, CA_PMT, "ms ca_pmt")?;
        if body.len() < MS_CA_PMT_PREFIX {
            return Err(Error::BufferTooShort {
                need: MS_CA_PMT_PREFIX,
                have: body.len(),
                what: "ms ca_pmt prefix",
            });
        }
        let lts_id = body[0];
        let list_management = CaPmtListManagement::from_u8(body[1]);
        let program_number = u16::from_be_bytes([body[2], body[3]]);
        // reserved(3) + PMT_PID(13).
        let pmt_pid = (((body[4] & 0x1F) as u16) << 8) | body[5] as u16;
        // reserved(2) + version(5) + cni(1).
        let version_number = (body[6] >> 1) & 0x1F;
        let current_next_indicator = (body[6] & 0x01) != 0;
        // reserved(4) + program_info_length(12).
        let program_info_length = (((body[7] & 0x0F) as usize) << 8) | body[8] as usize;

        let mut pos = MS_CA_PMT_PREFIX;
        let (cmd_id, program_ca_descriptors) = parse_cmd_and_descriptors(
            body,
            &mut pos,
            program_info_length,
            "ms ca_pmt program_info",
        )?;

        let mut streams = Vec::new();
        while pos < body.len() {
            if pos + MS_ES_PREFIX > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + MS_ES_PREFIX,
                    have: body.len(),
                    what: "ms ca_pmt ES prefix",
                });
            }
            let stream_type = body[pos];
            let elementary_pid = (((body[pos + 1] & 0x1F) as u16) << 8) | body[pos + 2] as u16;
            let es_info_length = (((body[pos + 3] & 0x0F) as usize) << 8) | body[pos + 4] as usize;
            pos += MS_ES_PREFIX;
            let (es_cmd, ca_descriptors) =
                parse_cmd_and_descriptors(body, &mut pos, es_info_length, "ms ca_pmt ES_info")?;
            streams.push(MsCaPmtStream {
                stream_type,
                elementary_pid,
                cmd_id: es_cmd,
                ca_descriptors,
            });
        }

        Ok(Self {
            lts_id,
            list_management,
            program_number,
            pmt_pid,
            version_number,
            current_next_indicator,
            cmd_id,
            program_ca_descriptors,
            streams,
        })
    }
}

impl Serialize for MsCaPmt<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut body = MS_CA_PMT_PREFIX + info_block_len(self.cmd_id, self.program_ca_descriptors);
        for s in &self.streams {
            body += MS_ES_PREFIX + info_block_len(s.cmd_id, s.ca_descriptors);
        }
        crate::objects::apdu_len(body)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let program_info_length = info_block_len(self.cmd_id, self.program_ca_descriptors);
        let mut body = MS_CA_PMT_PREFIX + program_info_length;
        for s in &self.streams {
            body += MS_ES_PREFIX + info_block_len(s.cmd_id, s.ca_descriptors);
        }
        let mut pos = crate::objects::write_apdu_header(CA_PMT, body, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1] = self.list_management.to_u8();
        buf[pos + 2..pos + 4].copy_from_slice(&self.program_number.to_be_bytes());
        // reserved(3)='111', PMT_PID(13).
        buf[pos + 4] = 0xE0 | ((self.pmt_pid >> 8) as u8 & 0x1F);
        buf[pos + 5] = self.pmt_pid as u8;
        // reserved(2)='11', version(5), current_next(1).
        buf[pos + 6] =
            0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        // reserved(4)='1111', program_info_length(12).
        buf[pos + 7] = 0xF0 | ((program_info_length >> 8) as u8 & 0x0F);
        buf[pos + 8] = program_info_length as u8;
        pos += MS_CA_PMT_PREFIX;
        pos += write_info_block(self.cmd_id, self.program_ca_descriptors, &mut buf[pos..])?;

        for s in &self.streams {
            let es_info_length = info_block_len(s.cmd_id, s.ca_descriptors);
            buf[pos] = s.stream_type;
            buf[pos + 1] = 0xE0 | ((s.elementary_pid >> 8) as u8 & 0x1F);
            buf[pos + 2] = s.elementary_pid as u8;
            buf[pos + 3] = 0xF0 | ((es_info_length >> 8) as u8 & 0x0F);
            buf[pos + 4] = es_info_length as u8;
            pos += MS_ES_PREFIX;
            pos += write_info_block(s.cmd_id, s.ca_descriptors, &mut buf[pos..])?;
        }
        Ok(pos)
    }
}

// ---------------------------------------------------------------------------
// ca_pmt_reply (Table 16)
// ---------------------------------------------------------------------------

/// One ES entry of a CI Plus multi-stream `ca_pmt_reply` (Table 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MsCaPmtReplyStream {
    /// 13-bit `elementary_PID`.
    pub elementary_pid: u16,
    /// ES-level `CA_enable` — `Some` iff the `CA_enable_flag` bit was set.
    pub ca_enable: Option<CaEnable>,
}

/// CI Plus multi-stream `ca_pmt_reply()` (Table 16): CICAM → Host. Differs from
/// the EN 50221 `ca_pmt_reply` by the leading `LTS_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MsCaPmtReply {
    /// `LTS_id` (8) — Local TS identifier.
    pub lts_id: u8,
    /// `program_number` (16).
    pub program_number: u16,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// `current_next_indicator`.
    pub current_next_indicator: bool,
    /// Programme-level `CA_enable` — `Some` iff the programme `CA_enable_flag` bit
    /// was set.
    pub ca_enable: Option<CaEnable>,
    /// Per-ES entries in wire order.
    pub streams: Vec<MsCaPmtReplyStream>,
}

// LTS_id(1) + program_number(2) + reserved/version/cni(1) + flag/enable(1).
const MS_REPLY_PREFIX: usize = 1 + 2 + 1 + 1;
const MS_REPLY_ES_LEN: usize = 3; // reserved/elem_pid(2) + flag/enable(1)

/// Encode a `CA_enable_flag` + 7-bit `CA_enable`/reserved byte (absent → `0x7F`).
fn encode_enable_byte(enable: Option<CaEnable>) -> u8 {
    match enable {
        Some(e) => 0x80 | (e.to_u8() & 0x7F),
        None => 0x7F,
    }
}

impl<'a> Parse<'a> for MsCaPmtReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = crate::objects::parse_apdu_header(bytes, CA_PMT_REPLY, "ms ca_pmt_reply")?;
        if body.len() < MS_REPLY_PREFIX {
            return Err(Error::BufferTooShort {
                need: MS_REPLY_PREFIX,
                have: body.len(),
                what: "ms ca_pmt_reply prefix",
            });
        }
        let lts_id = body[0];
        let program_number = u16::from_be_bytes([body[1], body[2]]);
        let version_number = (body[3] >> 1) & 0x1F;
        let current_next_indicator = (body[3] & 0x01) != 0;
        let ca_enable_flag = (body[4] & 0x80) != 0;
        let ca_enable = if ca_enable_flag {
            Some(CaEnable::from_u8(body[4] & 0x7F))
        } else {
            None
        };

        let mut pos = MS_REPLY_PREFIX;
        let mut streams = Vec::new();
        while pos < body.len() {
            if pos + MS_REPLY_ES_LEN > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + MS_REPLY_ES_LEN,
                    have: body.len(),
                    what: "ms ca_pmt_reply ES",
                });
            }
            let elementary_pid = (((body[pos] & 0x1F) as u16) << 8) | body[pos + 1] as u16;
            let es_flag = (body[pos + 2] & 0x80) != 0;
            let es_enable = if es_flag {
                Some(CaEnable::from_u8(body[pos + 2] & 0x7F))
            } else {
                None
            };
            streams.push(MsCaPmtReplyStream {
                elementary_pid,
                ca_enable: es_enable,
            });
            pos += MS_REPLY_ES_LEN;
        }

        Ok(Self {
            lts_id,
            program_number,
            version_number,
            current_next_indicator,
            ca_enable,
            streams,
        })
    }
}

impl Serialize for MsCaPmtReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::objects::apdu_len(MS_REPLY_PREFIX + self.streams.len() * MS_REPLY_ES_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body = MS_REPLY_PREFIX + self.streams.len() * MS_REPLY_ES_LEN;
        let mut pos = crate::objects::write_apdu_header(CA_PMT_REPLY, body, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1..pos + 3].copy_from_slice(&self.program_number.to_be_bytes());
        // reserved(2)='11', version(5), current_next(1).
        buf[pos + 3] =
            0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[pos + 4] = encode_enable_byte(self.ca_enable);
        pos += MS_REPLY_PREFIX;
        for s in &self.streams {
            buf[pos] = 0xE0 | ((s.elementary_pid >> 8) as u8 & 0x1F);
            buf[pos + 1] = s.elementary_pid as u8;
            buf[pos + 2] = encode_enable_byte(s.ca_enable);
            pos += MS_REPLY_ES_LEN;
        }
        Ok(pos)
    }
}

// ---------------------------------------------------------------------------
// apdu-tag dispatch helper (no resource_id — see module doc)
// ---------------------------------------------------------------------------

/// A parsed CI Plus multi-stream CA-support object.
///
/// There is intentionally **no** `resource_id`-keyed entry point for these
/// objects: TS 103 205 does not print the resource_id (see the module doc), so
/// dispatch is on the apdu_tag alone, for callers already in a multi-stream CA
/// Support session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CaSupportApdu<'a> {
    /// `ca_pmt` (`9F 80 32`), CI Plus multi-stream variant.
    CaPmt(MsCaPmt<'a>),
    /// `ca_pmt_reply` (`9F 80 33`), CI Plus multi-stream variant.
    CaPmtReply(MsCaPmtReply),
}

impl<'a> CaSupportApdu<'a> {
    /// Parse a CI Plus multi-stream CA-support APDU by its apdu_tag.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "ca_support apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            CA_PMT => Ok(Self::CaPmt(MsCaPmt::parse(body)?)),
            CA_PMT_REPLY => Ok(Self::CaPmtReply(MsCaPmtReply::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: CA_PMT.as_u24(),
                what: "ca_support",
            }),
        }
    }
}

impl Serialize for CaSupportApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::CaPmt(o) => o.serialized_len(),
            Self::CaPmtReply(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::CaPmt(o) => o.serialize_into(buf),
            Self::CaPmtReply(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::ca_pmt::CA_DESCRIPTOR_TAG;

    fn sample_ca_descriptor(ca_system_id: u16, pid: u16) -> [u8; 6] {
        [
            CA_DESCRIPTOR_TAG,
            0x04,
            (ca_system_id >> 8) as u8,
            ca_system_id as u8,
            0xE0 | ((pid >> 8) as u8 & 0x1F),
            pid as u8,
        ]
    }

    #[test]
    fn ms_ca_pmt_round_trips_and_bites() {
        let prog = sample_ca_descriptor(0x1234, 0x0100);
        let es_desc = sample_ca_descriptor(0x1234, 0x0101);
        let pmt = MsCaPmt {
            lts_id: 0x02,
            list_management: CaPmtListManagement::Only,
            program_number: 0x0001,
            pmt_pid: 0x0064,
            version_number: 1,
            current_next_indicator: true,
            cmd_id: Some(CaPmtCmdId::OkDescrambling),
            program_ca_descriptors: &prog,
            streams: alloc::vec![MsCaPmtStream {
                stream_type: 0x02,
                elementary_pid: 0x0200,
                cmd_id: Some(CaPmtCmdId::OkDescrambling),
                ca_descriptors: &es_desc,
            }],
        };
        let bytes = pmt.to_bytes();
        // Hand-computed body:
        //   LTS_id 02 | list_mgmt 03 | prog_num 00 01 | reserved+PMT_PID E0 64
        //   reserved+ver1+cni1 = 0xC0|0x02|0x01 = 0xC3
        //   reserved+prog_info_len(7) = F0 07 ; cmd_id 01 + 6 desc bytes
        //   ES: stream_type 02 | E0 00 (pid 0x200 -> hi 0x02) wait pid 0x0200
        let pid = 0x0200u16;
        let es_hi = 0xE0 | ((pid >> 8) as u8 & 0x1F);
        let expected = {
            let mut v = alloc::vec![
                0x9F, 0x80, 0x32, // tag
                0x1C, // length (28 body bytes)
                0x02, // LTS_id
                0x03, // list_management Only
                0x00, 0x01, // program_number
                0xE0, 0x64, // reserved + PMT_PID 0x0064
                0xC3, // reserved + version 1 + cni 1
                0xF0, 0x07, // reserved + program_info_length 7
                0x01, // program ca_pmt_cmd_id ok_descrambling
            ];
            v.extend_from_slice(&prog); // 6 bytes
            v.extend_from_slice(&[0x02, es_hi, 0x00, 0xF0, 0x07, 0x01]); // ES prefix + es_info_len + cmd
            v.extend_from_slice(&es_desc); // 6 bytes
            v
        };
        assert_eq!(bytes, expected);
        assert_eq!(MsCaPmt::parse(&bytes).unwrap(), pmt);
        // Field-mutation: change PMT_PID.
        let mut other = pmt.clone();
        other.pmt_pid = 0x0065;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn ms_ca_pmt_no_descriptors() {
        let pmt = MsCaPmt {
            lts_id: 0x00,
            list_management: CaPmtListManagement::Update,
            program_number: 0x0009,
            pmt_pid: 0x1FFF,
            version_number: 0,
            current_next_indicator: true,
            cmd_id: None,
            program_ca_descriptors: &[],
            streams: Vec::new(),
        };
        let bytes = pmt.to_bytes();
        // PMT_PID 0x1FFF -> hi 0xE0|0x1F=0xFF, lo 0xFF; prog_info_len 0 -> F0 00.
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x32, 0x09, 0x00, 0x05, 0x00, 0x09, 0xFF, 0xFF, 0xC1, 0xF0, 0x00]
        );
        assert_eq!(MsCaPmt::parse(&bytes).unwrap(), pmt);
    }

    #[test]
    fn ms_ca_pmt_reply_round_trips_and_bites() {
        let reply = MsCaPmtReply {
            lts_id: 0x03,
            program_number: 0x0001,
            version_number: 1,
            current_next_indicator: true,
            ca_enable: Some(CaEnable::Possible),
            streams: alloc::vec![
                MsCaPmtReplyStream {
                    elementary_pid: 0x0200,
                    ca_enable: Some(CaEnable::Possible),
                },
                MsCaPmtReplyStream {
                    elementary_pid: 0x0201,
                    ca_enable: None,
                },
            ],
        };
        let bytes = reply.to_bytes();
        let expected = [
            0x9F, 0x80, 0x33, // tag
            0x0B, // length (11 body bytes)
            0x03, // LTS_id
            0x00, 0x01, // program_number
            0xC3, // reserved + version 1 + cni 1
            0x81, // flag=1 + CA_enable Possible(0x01)
            0xE2, 0x00, 0x81, // ES pid 0x200 + flag/enable Possible
            0xE2, 0x01, 0x7F, // ES pid 0x201 + no enable (0x7F)
        ];
        assert_eq!(bytes, expected);
        assert_eq!(MsCaPmtReply::parse(&bytes).unwrap(), reply);
        // Field-mutation: change LTS_id.
        let mut other = reply.clone();
        other.lts_id = 0x04;
        assert_ne!(bytes, other.to_bytes());
        assert_eq!(other.to_bytes()[4], 0x04);
    }

    #[test]
    fn dispatch_helper_routes_by_tag() {
        let pmt = MsCaPmt {
            lts_id: 0,
            list_management: CaPmtListManagement::Only,
            program_number: 1,
            pmt_pid: 0x64,
            version_number: 0,
            current_next_indicator: true,
            cmd_id: None,
            program_ca_descriptors: &[],
            streams: Vec::new(),
        };
        let bytes = pmt.to_bytes();
        let parsed = CaSupportApdu::parse(&bytes).unwrap();
        assert!(matches!(parsed, CaSupportApdu::CaPmt(_)));
        assert_eq!(parsed.to_bytes(), bytes);

        let reply = MsCaPmtReply {
            lts_id: 1,
            program_number: 1,
            version_number: 0,
            current_next_indicator: true,
            ca_enable: None,
            streams: Vec::new(),
        };
        let rb = reply.to_bytes();
        assert!(matches!(
            CaSupportApdu::parse(&rb).unwrap(),
            CaSupportApdu::CaPmtReply(_)
        ));
    }
}
