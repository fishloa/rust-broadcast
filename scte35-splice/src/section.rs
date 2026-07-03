//! splice_info_section() — ANSI/SCTE 35 2023r1 §9.6, Table 5 (table_id 0xFC).
//!
//! The top-level SCTE 35 message: a short-form MPEG section carrying one splice
//! command and a splice descriptor loop, trailed by an MPEG CRC-32 (the same
//! `crc32_mpeg2` every PSI/SI section uses; verified §9.6.1 to be the MPEG
//! Systems decoder CRC).
//!
//! ## Encryption (§9.6.1, §11.3)
//!
//! When `encrypted_packet == 1` the region from `splice_command_type` through
//! `E_CRC_32` is encrypted; this parser does **not** decrypt. Such a section is
//! kept as [`SpliceInfoSection::encrypted_payload`] (the raw encrypted bytes,
//! verbatim) and the command / descriptor accessors return nothing — but the
//! section still round-trips byte-for-byte and its CRC is recomputed on
//! serialize. Clear sections (`encrypted_packet == 0`, the overwhelming common
//! case) expose the typed command and descriptor loop.

use crate::commands::AnyCommand;
use crate::descriptors::{SpliceDescriptorIter, parse_loop};
use crate::error::{Error, Result};
use crate::time::PTS_MAX;
use broadcast_common::{Parse, Serialize};

/// `table_id` of a splice_info_section (§9.6.1).
pub const TABLE_ID: u8 = 0xFC;

/// Bytes of the fixed header from `table_id` through `splice_command_type`.
const FIXED_HEADER_LEN: usize = 14;

/// Bytes of the trailing CRC_32.
const CRC_LEN: usize = 4;

/// `tier` value (0xFFF) that downstream equipment shall ignore (§9.6.1).
pub const TIER_IGNORE: u16 = 0x0FFF;

/// The clear (decrypted) splice command and its splice descriptor loop, present
/// only when `encrypted_packet == 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ClearPayload<'a> {
    /// The typed splice command (§9.7).
    pub command: AnyCommand<'a>,
    /// Raw splice descriptor loop bytes; walk typed via
    /// [`SpliceInfoSection::descriptors`].
    pub descriptor_loop: &'a [u8],
}

/// splice_info_section() — §9.6, Table 5.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SpliceInfoSection<'a> {
    /// 2-bit `sap_type` (§9.6.1, Table 6); `0x3` = SAP type not specified.
    pub sap_type: u8,
    /// 8-bit `protocol_version` (0 is the only valid value at present).
    pub protocol_version: u8,
    /// `encrypted_packet` flag (§9.6.1).
    pub encrypted_packet: bool,
    /// 6-bit `encryption_algorithm` (§11.3, Table 29). Undefined when not
    /// encrypted.
    pub encryption_algorithm: u8,
    /// 33-bit `pts_adjustment` (90 kHz ticks), added (wrapping) to every
    /// `pts_time` in the message.
    pub pts_adjustment: u64,
    /// 8-bit `cw_index`. Undefined when not encrypted.
    pub cw_index: u8,
    /// 12-bit `tier` (§9.6.1); 0xFFF shall be ignored downstream.
    pub tier: u16,
    /// The clear command + descriptor loop, present when `encrypted_packet`
    /// is `false`.
    pub clear: Option<ClearPayload<'a>>,
    /// The raw encrypted region (`splice_command_type` through `E_CRC_32`),
    /// present when `encrypted_packet` is `true`. Not decrypted by this crate.
    pub encrypted_payload: Option<&'a [u8]>,
}

impl<'a> SpliceInfoSection<'a> {
    /// Build a clear (unencrypted) section from a command and a raw descriptor
    /// loop, with `pts_adjustment`/`tier` defaulted to zero / `0x3` sap_type.
    #[must_use]
    pub fn new_clear(command: AnyCommand<'a>, descriptor_loop: &'a [u8]) -> Self {
        Self {
            sap_type: 0x3,
            protocol_version: 0,
            encrypted_packet: false,
            encryption_algorithm: 0,
            pts_adjustment: 0,
            cw_index: 0,
            tier: 0,
            clear: Some(ClearPayload {
                command,
                descriptor_loop,
            }),
            encrypted_payload: None,
        }
    }

    /// Walk the splice descriptor loop, yielding typed
    /// [`AnySpliceDescriptor`](crate::descriptors::AnySpliceDescriptor)s. Empty
    /// when the section is encrypted.
    #[must_use]
    pub fn descriptors(&self) -> SpliceDescriptorIter<'a> {
        match &self.clear {
            Some(c) => parse_loop(c.descriptor_loop),
            None => parse_loop(&[]),
        }
    }

    /// The `pts_adjustment` decoded to a [`Duration`](core::time::Duration).
    #[must_use]
    pub fn pts_adjustment_duration(&self) -> core::time::Duration {
        crate::time::ticks_to_duration(self.pts_adjustment)
    }

    /// Set `pts_adjustment` from a [`Duration`](core::time::Duration)
    /// (truncating to 90 kHz ticks). Errors if it exceeds the 33-bit range.
    pub fn set_pts_adjustment_duration(&mut self, d: core::time::Duration) -> Result<()> {
        self.pts_adjustment =
            crate::time::duration_to_ticks(d, PTS_MAX).ok_or(Error::InvalidValue {
                field: "splice_info_section.pts_adjustment",
                reason: "duration exceeds 33-bit 90 kHz range",
            })?;
        Ok(())
    }

    /// True if `tier == 0xFFF`, which downstream equipment shall ignore.
    #[must_use]
    pub fn tier_is_ignored(&self) -> bool {
        self.tier == TIER_IGNORE
    }

    /// Bytes of the section body from `splice_command_type` through (but not
    /// including) the trailing CRC_32 — the splice_command_length region plus
    /// the descriptor loop and its 2-byte length, or the raw encrypted region.
    fn payload_len(&self) -> usize {
        match (&self.clear, self.encrypted_payload) {
            (Some(c), _) => 1 + c.command.body_len() + 2 + c.descriptor_loop.len(),
            (None, Some(enc)) => enc.len(),
            (None, None) => 0,
        }
    }
}

impl<'a> Parse<'a> for SpliceInfoSection<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FIXED_HEADER_LEN + CRC_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_HEADER_LEN + CRC_LEN,
                have: bytes.len(),
                what: "splice_info_section header",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId { table_id: bytes[0] });
        }
        let sap_type = (bytes[1] >> 4) & 0x03;
        let section_length = (u16::from(bytes[1] & 0x0F) << 8) | u16::from(bytes[2]);
        let total = 3 + section_length as usize;
        if bytes.len() < total {
            return Err(Error::LengthOverflow {
                declared: section_length as usize,
                available: bytes.len().saturating_sub(3),
                what: "splice_info_section section_length",
            });
        }
        // Verify CRC over the whole section (table_id..=CRC_32), prior to any
        // decryption (§9.6.1). Guard total >= FIXED_HEADER_LEN + CRC_LEN before
        // computing crc_pos so that section_length == 0 doesn't underflow (#216).
        if total < FIXED_HEADER_LEN + CRC_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_HEADER_LEN + CRC_LEN,
                have: total,
                what: "splice_info_section body + CRC",
            });
        }
        let crc_pos = total - CRC_LEN;
        let (crc_region, crc_bytes) =
            bytes[..total]
                .split_last_chunk::<CRC_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: FIXED_HEADER_LEN + CRC_LEN,
                    have: bytes.len(),
                    what: "splice_info_section header",
                })?;
        let expected = u32::from_be_bytes(*crc_bytes);
        let computed = broadcast_common::crc32_mpeg2::compute(crc_region);
        if computed != expected {
            return Err(Error::CrcMismatch { computed, expected });
        }

        let protocol_version = bytes[3];
        let encrypted_packet = bytes[4] & 0x80 != 0;
        let encryption_algorithm = (bytes[4] >> 1) & 0x3F;
        let pts_adjustment = (u64::from(bytes[4] & 0x01) << 32)
            | (u64::from(bytes[5]) << 24)
            | (u64::from(bytes[6]) << 16)
            | (u64::from(bytes[7]) << 8)
            | u64::from(bytes[8]);
        let cw_index = bytes[9];
        let tier = (u16::from(bytes[10]) << 4) | (u16::from(bytes[11]) >> 4);
        let splice_command_length = (u16::from(bytes[11] & 0x0F) << 8) | u16::from(bytes[12]);
        let splice_command_type = bytes[13];

        let mut section = SpliceInfoSection {
            sap_type,
            protocol_version,
            encrypted_packet,
            encryption_algorithm,
            pts_adjustment,
            cw_index,
            tier,
            clear: None,
            encrypted_payload: None,
        };

        // Region from splice_command_type (byte 13) through E_CRC_32/CRC_32.
        let payload = &bytes[13..crc_pos];
        if encrypted_packet {
            // Keep the encrypted region (command_type + body + descriptors +
            // E_CRC_32) verbatim; do not attempt to interpret it.
            section.encrypted_payload = Some(payload);
            return Ok(section);
        }

        // Clear: splice_command_type (1) is payload[0]; the command body is
        // splice_command_length bytes; then descriptor_loop_length (2) + loop.
        let scl = splice_command_length as usize;
        // splice_command_length may be 0xFFF ("ignore"); when so, derive the
        // command body length from the structure by parsing greedily up to the
        // descriptor_loop_length. We require the actual value here; reject the
        // backwards-compat sentinel as unparseable rather than guess.
        if splice_command_length == 0x0FFF {
            return Err(Error::InvalidValue {
                field: "splice_info_section.splice_command_length",
                reason: "0xFFF backwards-compat sentinel is not supported; provide the actual length",
            });
        }
        // payload = [command_type][command_body(scl)][dll(2)][loop]
        if payload.len() < 1 + scl + 2 {
            return Err(Error::BufferTooShort {
                need: 1 + scl + 2,
                have: payload.len(),
                what: "splice_info_section command + descriptor_loop_length",
            });
        }
        let command_body = &payload[1..1 + scl];
        let command = AnyCommand::dispatch(splice_command_type, command_body)?;

        let dll_pos = 1 + scl;
        let (dll_bytes, _) = payload
            .get(dll_pos..)
            .and_then(|s| s.split_first_chunk::<2>())
            .ok_or(Error::BufferTooShort {
                need: 1 + scl + 2,
                have: payload.len(),
                what: "splice_info_section command + descriptor_loop_length",
            })?;
        let descriptor_loop_length = usize::from(u16::from_be_bytes(*dll_bytes));
        let loop_start = dll_pos + 2;
        if payload.len() < loop_start + descriptor_loop_length {
            return Err(Error::LengthOverflow {
                declared: descriptor_loop_length,
                available: payload.len().saturating_sub(loop_start),
                what: "splice_info_section descriptor_loop_length",
            });
        }
        let descriptor_loop = &payload[loop_start..loop_start + descriptor_loop_length];
        // Any bytes between the descriptor loop and the CRC are alignment
        // stuffing (§9.6.1); for a clear section there should be none, but we
        // tolerate by ignoring them (they are re-derived on serialize).
        section.clear = Some(ClearPayload {
            command,
            descriptor_loop,
        });
        Ok(section)
    }
}

impl Serialize for SpliceInfoSection<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        // table_id(1) + flags/length(2) + protocol_version(1) + 5 + cw_index(1)
        // + tier/scl(3) = 13 bytes before splice_command_type, then the payload
        // (command_type..E_CRC_32 / descriptors), then CRC_32(4).
        13 + self.payload_len() + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if self.pts_adjustment > PTS_MAX {
            return Err(Error::InvalidValue {
                field: "splice_info_section.pts_adjustment",
                reason: "exceeds 33-bit range",
            });
        }
        if self.tier > 0x0FFF {
            return Err(Error::InvalidValue {
                field: "splice_info_section.tier",
                reason: "exceeds 12-bit range",
            });
        }

        // section_length counts the bytes after byte 2 up to and including CRC.
        let section_length = (need - 3) as u16;
        if section_length > 4093 {
            return Err(Error::InvalidValue {
                field: "splice_info_section.section_length",
                reason: "exceeds 4093",
            });
        }

        buf[0] = TABLE_ID;
        // section_syntax_indicator=0, private_indicator=0, sap_type(2),
        // section_length(12).
        buf[1] = ((self.sap_type & 0x03) << 4) | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3] = self.protocol_version;
        buf[4] = (u8::from(self.encrypted_packet) << 7)
            | ((self.encryption_algorithm & 0x3F) << 1)
            | ((self.pts_adjustment >> 32) as u8 & 0x01);
        buf[5] = (self.pts_adjustment >> 24) as u8;
        buf[6] = (self.pts_adjustment >> 16) as u8;
        buf[7] = (self.pts_adjustment >> 8) as u8;
        buf[8] = self.pts_adjustment as u8;
        buf[9] = self.cw_index;
        buf[10] = (self.tier >> 4) as u8;

        // payload region (after splice_command_length byte at index 12).
        match (&self.clear, self.encrypted_payload) {
            (Some(c), _) => {
                let scl = c.command.body_len() as u16;
                buf[11] = (((self.tier & 0x0F) as u8) << 4) | ((scl >> 8) as u8 & 0x0F);
                buf[12] = (scl & 0xFF) as u8;
                buf[13] = c.command.command_type();
                let mut pos = 14;
                pos += c.command.serialize_body_into(&mut buf[pos..])?;
                let dll = c.descriptor_loop.len() as u16;
                buf[pos] = (dll >> 8) as u8;
                buf[pos + 1] = (dll & 0xFF) as u8;
                pos += 2;
                buf[pos..pos + c.descriptor_loop.len()].copy_from_slice(c.descriptor_loop);
                pos += c.descriptor_loop.len();
                debug_assert_eq!(pos, need - CRC_LEN);
            }
            (None, Some(enc)) => {
                // For an encrypted section, splice_command_length is not known
                // to us (it is inside the encrypted region); write the
                // backwards-compat 0xFFF sentinel and emit the region verbatim.
                buf[11] = (((self.tier & 0x0F) as u8) << 4) | 0x0F;
                buf[12] = 0xFF;
                buf[13..13 + enc.len()].copy_from_slice(enc);
            }
            (None, None) => {
                buf[11] = ((self.tier & 0x0F) as u8) << 4;
                buf[12] = 0;
                // No command/payload: splice_command_type would be missing.
                return Err(Error::InvalidValue {
                    field: "splice_info_section",
                    reason: "neither a clear command nor an encrypted payload is present",
                });
            }
        }

        let crc_pos = need - CRC_LEN;
        let crc = broadcast_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..need].copy_from_slice(&crc.to_be_bytes());
        Ok(need)
    }
}
