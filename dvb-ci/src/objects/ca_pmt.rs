//! CA PMT object (`ca_pmt`) — ETSI EN 50221 §8.4.3.4, Table 25 (PDF pp. 30-31).
//!
//! `ca_pmt` (`9F 80 32`, host → app) is a CA-only projection of the MPEG-2 PMT:
//! the host strips every non-CA descriptor and keeps only the `CA_descriptor()`s
//! (ISO/IEC 13818-1 §2.6.16, tag `0x09`) at programme and elementary-stream
//! level, prefixing each surviving descriptor loop with a `ca_pmt_cmd_id` byte.
//!
//! This module carries the surviving CA descriptor loops as their verbatim wire
//! bytes (`&[u8]`); see [`crate::builder`] for the projection from a `dvb-si`
//! `PmtSection`.

use crate::error::{Error, Result};
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// MPEG-2 `CA_descriptor` tag (ISO/IEC 13818-1 §2.6.16); the only descriptor a
/// `ca_pmt` carries.
pub const CA_DESCRIPTOR_TAG: u8 = 0x09;

/// `ca_pmt_list_management` values (Table, p. 31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CaPmtListManagement {
    /// `00` — neither first nor last of the list.
    More,
    /// `01` — first of a new list (replaces previous selections).
    First,
    /// `02` — last of the list.
    Last,
    /// `03` — the list is a single CA PMT.
    Only,
    /// `04` — a newly selected programme; previous selections retained.
    Add,
    /// `05` — a programme already in the list re-sent (version change).
    Update,
    /// Any other value (reserved).
    Reserved(u8),
}

impl CaPmtListManagement {
    /// Decode a `ca_pmt_list_management` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::More,
            0x01 => Self::First,
            0x02 => Self::Last,
            0x03 => Self::Only,
            0x04 => Self::Add,
            0x05 => Self::Update,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::More => 0x00,
            Self::First => 0x01,
            Self::Last => 0x02,
            Self::Only => 0x03,
            Self::Add => 0x04,
            Self::Update => 0x05,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::More => "more",
            Self::First => "first",
            Self::Last => "last",
            Self::Only => "only",
            Self::Add => "add",
            Self::Update => "update",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(CaPmtListManagement, Reserved);

/// `ca_pmt_cmd_id` values (Table, p. 31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CaPmtCmdId {
    /// `01` — application may start descrambling / MMI immediately.
    OkDescrambling,
    /// `02` — application may start MMI but not descrambling yet.
    OkMmi,
    /// `03` — host expects a `ca_pmt_reply`.
    Query,
    /// `04` — host no longer needs this application to descramble.
    NotSelected,
    /// Any other value (RFU).
    Rfu(u8),
}

impl CaPmtCmdId {
    /// Decode a `ca_pmt_cmd_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::OkDescrambling,
            0x02 => Self::OkMmi,
            0x03 => Self::Query,
            0x04 => Self::NotSelected,
            other => Self::Rfu(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::OkDescrambling => 0x01,
            Self::OkMmi => 0x02,
            Self::Query => 0x03,
            Self::NotSelected => 0x04,
            Self::Rfu(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::OkDescrambling => "ok_descrambling",
            Self::OkMmi => "ok_mmi",
            Self::Query => "query",
            Self::NotSelected => "not_selected",
            Self::Rfu(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(CaPmtCmdId, Rfu);

/// One elementary-stream entry in a `ca_pmt`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaPmtStream<'a> {
    /// MPEG-2 `stream_type`.
    pub stream_type: u8,
    /// 13-bit `elementary_PID`.
    pub elementary_pid: u16,
    /// `ca_pmt_cmd_id` for this ES — present only when the ES has CA info
    /// (i.e. when `ca_descriptors` is non-empty), per the `ES_info_length != 0`
    /// guard in Table 25.
    pub cmd_id: Option<CaPmtCmdId>,
    /// The ES-level `CA_descriptor()` loop, verbatim wire bytes (no `cmd_id`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub ca_descriptors: &'a [u8],
}

/// `ca_pmt()` object (Table 25).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaPmt<'a> {
    /// `ca_pmt_list_management`.
    pub list_management: CaPmtListManagement,
    /// `program_number`.
    pub program_number: u16,
    /// 5-bit `version_number`.
    pub version_number: u8,
    /// `current_next_indicator`.
    pub current_next_indicator: bool,
    /// Programme-level `ca_pmt_cmd_id` — present only when there is programme-level
    /// CA info (`program_info_length != 0`).
    pub cmd_id: Option<CaPmtCmdId>,
    /// Programme-level `CA_descriptor()` loop, verbatim wire bytes (no `cmd_id`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub program_ca_descriptors: &'a [u8],
    /// Elementary streams in wire order.
    pub streams: Vec<CaPmtStream<'a>>,
}

// Fixed prefix after the header: list_management(1) + program_number(2) +
// reserved/version/cni(1) + reserved/program_info_length(2).
const CA_PMT_PREFIX: usize = 6;
// Per-ES fixed prefix: stream_type(1) + reserved/elem_pid(2) +
// reserved/ES_info_length(2).
const ES_PREFIX: usize = 5;

impl<'a> Parse<'a> for CaPmt<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::CA_PMT, "ca_pmt")?;
        if body.len() < CA_PMT_PREFIX {
            return Err(Error::BufferTooShort {
                need: CA_PMT_PREFIX,
                have: body.len(),
                what: "ca_pmt prefix",
            });
        }
        let list_management = CaPmtListManagement::from_u8(body[0]);
        let program_number = u16::from_be_bytes([body[1], body[2]]);
        let version_number = (body[3] >> 1) & 0x1F;
        let current_next_indicator = (body[3] & 0x01) != 0;
        let program_info_length = (((body[4] & 0x0F) as usize) << 8) | body[5] as usize;

        let mut pos = CA_PMT_PREFIX;
        let (cmd_id, program_ca_descriptors) =
            parse_cmd_and_descriptors(body, &mut pos, program_info_length, "ca_pmt program_info")?;

        let mut streams = Vec::new();
        while pos < body.len() {
            if pos + ES_PREFIX > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + ES_PREFIX,
                    have: body.len(),
                    what: "ca_pmt ES prefix",
                });
            }
            let stream_type = body[pos];
            let elementary_pid = (((body[pos + 1] & 0x1F) as u16) << 8) | body[pos + 2] as u16;
            let es_info_length = (((body[pos + 3] & 0x0F) as usize) << 8) | body[pos + 4] as usize;
            pos += ES_PREFIX;
            let (es_cmd, ca_descriptors) =
                parse_cmd_and_descriptors(body, &mut pos, es_info_length, "ca_pmt ES_info")?;
            streams.push(CaPmtStream {
                stream_type,
                elementary_pid,
                cmd_id: es_cmd,
                ca_descriptors,
            });
        }

        Ok(Self {
            list_management,
            program_number,
            version_number,
            current_next_indicator,
            cmd_id,
            program_ca_descriptors,
            streams,
        })
    }
}

/// Read an `info_length`-byte block at `*pos`: when non-zero it starts with a
/// `ca_pmt_cmd_id` byte followed by `info_length - 1` descriptor bytes. Advances
/// `*pos` past the block.
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

/// Wire length of an info block (cmd_id + descriptors), or 0 when neither is
/// present.
fn info_block_len(cmd_id: Option<CaPmtCmdId>, descriptors: &[u8]) -> usize {
    if cmd_id.is_some() || !descriptors.is_empty() {
        1 + descriptors.len()
    } else {
        0
    }
}

impl Serialize for CaPmt<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut body = CA_PMT_PREFIX + info_block_len(self.cmd_id, self.program_ca_descriptors);
        for s in &self.streams {
            body += ES_PREFIX + info_block_len(s.cmd_id, s.ca_descriptors);
        }
        super::apdu_len(body)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let program_info_length = info_block_len(self.cmd_id, self.program_ca_descriptors);
        let mut body = CA_PMT_PREFIX + program_info_length;
        for s in &self.streams {
            body += ES_PREFIX + info_block_len(s.cmd_id, s.ca_descriptors);
        }
        let mut pos = super::write_apdu_header(tag::CA_PMT, body, buf)?;

        buf[pos] = self.list_management.to_u8();
        buf[pos + 1..pos + 3].copy_from_slice(&self.program_number.to_be_bytes());
        // reserved(2)='11', version(5), current_next(1).
        buf[pos + 3] =
            0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        // reserved(4)='1111', program_info_length(12).
        buf[pos + 4] = 0xF0 | ((program_info_length >> 8) as u8 & 0x0F);
        buf[pos + 5] = program_info_length as u8;
        pos += CA_PMT_PREFIX;
        pos += write_info_block(self.cmd_id, self.program_ca_descriptors, &mut buf[pos..])?;

        for s in &self.streams {
            let es_info_length = info_block_len(s.cmd_id, s.ca_descriptors);
            buf[pos] = s.stream_type;
            // reserved(3)='111', elementary_PID(13).
            buf[pos + 1] = 0xE0 | ((s.elementary_pid >> 8) as u8 & 0x1F);
            buf[pos + 2] = s.elementary_pid as u8;
            // reserved(4)='1111', ES_info_length(12).
            buf[pos + 3] = 0xF0 | ((es_info_length >> 8) as u8 & 0x0F);
            buf[pos + 4] = es_info_length as u8;
            pos += ES_PREFIX;
            pos += write_info_block(s.cmd_id, s.ca_descriptors, &mut buf[pos..])?;
        }
        Ok(pos)
    }
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
    // cmd_id defaults to ok_descrambling if a descriptor block exists without an
    // explicit id (shouldn't occur via the builder, but keeps serialize total).
    buf[0] = cmd_id.unwrap_or(CaPmtCmdId::OkDescrambling).to_u8();
    buf[1..len].copy_from_slice(descriptors);
    Ok(len)
}

impl<'a> ApduDef<'a> for CaPmt<'a> {
    const TAG: ApduTag = tag::CA_PMT;
    const NAME: &'static str = "CA_PMT";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ca_descriptor(ca_system_id: u16, pid: u16) -> [u8; 6] {
        // CA_descriptor: tag 0x09, len 4, CA_system_id(2), reserved(3)+CA_PID(13).
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
    fn empty_ca_pmt_round_trips() {
        let pmt = CaPmt {
            list_management: CaPmtListManagement::Only,
            program_number: 0x1234,
            version_number: 5,
            current_next_indicator: true,
            cmd_id: None,
            program_ca_descriptors: &[],
            streams: Vec::new(),
        };
        let bytes = pmt.to_bytes();
        assert_eq!(&bytes[..4], &[0x9F, 0x80, 0x32, 0x06]);
        assert_eq!(CaPmt::parse(&bytes).unwrap(), pmt);
    }

    #[test]
    fn multi_es_round_trips_and_bites() {
        let prog_desc = sample_ca_descriptor(0x0500, 0x0100);
        let es0_desc = sample_ca_descriptor(0x0500, 0x0101);
        let es1_desc = sample_ca_descriptor(0x0B00, 0x0201);
        let pmt = CaPmt {
            list_management: CaPmtListManagement::Only,
            program_number: 0x0001,
            version_number: 1,
            current_next_indicator: true,
            cmd_id: Some(CaPmtCmdId::OkDescrambling),
            program_ca_descriptors: &prog_desc,
            streams: alloc::vec![
                CaPmtStream {
                    stream_type: 0x02,
                    elementary_pid: 0x0200,
                    cmd_id: Some(CaPmtCmdId::OkDescrambling),
                    ca_descriptors: &es0_desc,
                },
                CaPmtStream {
                    stream_type: 0x03,
                    elementary_pid: 0x0201,
                    cmd_id: Some(CaPmtCmdId::OkDescrambling),
                    ca_descriptors: &es1_desc,
                },
            ],
        };
        let bytes = pmt.to_bytes();
        let parsed = CaPmt::parse(&bytes).unwrap();
        assert_eq!(parsed, pmt);
        assert_eq!(parsed.streams.len(), 2);
        assert_eq!(parsed.list_management.name(), "only");

        // bite: mutate the program_number.
        let mut other = pmt.clone();
        other.program_number = 0x0002;
        assert_ne!(bytes, other.to_bytes());

        // bite: mutate a stream's pid.
        let mut other2 = pmt.clone();
        other2.streams[0].elementary_pid = 0x0300;
        assert_ne!(bytes, other2.to_bytes());
    }

    #[test]
    fn es_without_ca_info_omits_cmd_id() {
        let pmt = CaPmt {
            list_management: CaPmtListManagement::Add,
            program_number: 7,
            version_number: 0,
            current_next_indicator: true,
            cmd_id: None,
            program_ca_descriptors: &[],
            streams: alloc::vec![CaPmtStream {
                stream_type: 0x1B,
                elementary_pid: 0x00FF,
                cmd_id: None,
                ca_descriptors: &[],
            }],
        };
        let bytes = pmt.to_bytes();
        let parsed = CaPmt::parse(&bytes).unwrap();
        assert_eq!(parsed, pmt);
        assert!(parsed.streams[0].cmd_id.is_none());
    }
}
