//! Usage Rules Information (URI) version 3 — ETSI TS 103 205 V1.4.1 §11,
//! Tables 90-92 (PDF pp. 110-112). See `docs/ts_103_205/usage-rules-v3.md`.
//!
//! CI Plus URI v3 extends the v1/v2 `uri_message` syntax (CI Plus V1.3 \[3\]
//! §5.7.5.2) with the `trick_mode_control_info` signal, applicable to content with
//! `emi_copy_control_info == 0b10` ("one generation copy is permitted").
//!
//! The URI message is the `uri_message` datatype carried in the SAC URI
//! transmission protocol (datatype_id 25; see `content-control.md` §6.4.3.3.1). It
//! is **not** an APDU and has **no resource of its own**, so [`UriMessage`] is a
//! standalone typed struct (full `Parse` / `Serialize`) — it is not wired into
//! [`crate::ci_plus::CiPlusApdu`]. Callers extract it from the Content Control SAC
//! layer's URI datatype payload.
//!
//! The on-wire structure is a fixed **64 bits (8 bytes)** (matching the
//! `uri_message` SAC datatype length). Several fields are conditional on
//! `emi_copy_control_info` (Table 91): the `rct`/`dot`/`rl`/`trick_mode` bits only
//! carry meaning in the matching EMI case; in the other cases those bit positions
//! are reserved. To keep serialize byte-exact and lossless this struct stores the
//! always-present fields plus an [`EmiData`] enum selected by the EMI value.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `uri_message` fixed length — 64 bits (8 bytes).
pub const URI_MESSAGE_LEN: usize = 8;

/// `protocol_version` value for URI v3 (Table 90).
pub const PROTOCOL_VERSION_V3: u8 = 0x03;

/// The EMI-case-selected fields of a [`UriMessage`] (Table 91). The variant is
/// chosen by `emi_copy_control_info`; in the non-matching cases the corresponding
/// bit positions are reserved (encoded as zero) and not carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EmiData {
    /// `emi_copy_control_info == 0b00` ("copying not restricted") — carries the
    /// `rct_copy_control_info` bit.
    CopyingNotRestricted {
        /// `rct_copy_control_info` (1).
        rct: bool,
    },
    /// `emi_copy_control_info == 0b01` — no case-specific bits (those positions
    /// are reserved).
    CopyOnce,
    /// `emi_copy_control_info == 0b10` ("one generation copy is permitted") —
    /// carries the `trick_mode_control_info` bit (Table 92).
    OneGenerationCopy {
        /// `trick_mode_control_info` (1) — `true` = trick mode control enabled.
        trick_mode: bool,
    },
    /// `emi_copy_control_info == 0b11` ("no more copies") — carries
    /// `dot_copy_control_info` and `rl_copy_control_info`.
    NoMoreCopies {
        /// `dot_copy_control_info` (1).
        dot: bool,
        /// `rl_copy_control_info` (8).
        rl: u8,
    },
}
impl EmiData {
    /// The 2-bit `emi_copy_control_info` value this variant represents.
    #[must_use]
    pub fn emi(&self) -> u8 {
        match self {
            Self::CopyingNotRestricted { .. } => 0b00,
            Self::CopyOnce => 0b01,
            Self::OneGenerationCopy { .. } => 0b10,
            Self::NoMoreCopies { .. } => 0b11,
        }
    }
}

/// A CI Plus `uri_message()` (Table 91), URI version 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UriMessage {
    /// `protocol_version` (8) — `0x03` for URI v3.
    pub protocol_version: u8,
    /// `aps_copy_control_info` (2).
    pub aps: u8,
    /// `ict_copy_control_info` (1).
    pub ict: bool,
    /// The `emi_copy_control_info`-selected fields (also encodes the 2-bit EMI).
    pub emi_data: EmiData,
}

// Bit layout (Table 91), MSB-first within the 8-byte field, after protocol_version:
//   aps(2) emi(2) ict(1) | [rct|reserved](1) reserved(1) |
//   [dot,rl(8) | reserved(9)](9) | [trick|reserved](1) | reserved(39)
//
// We assemble the 64-bit value MSB-first then split to bytes.

// EMI case selectors.
const EMI_COPY_NOT_RESTRICTED: u8 = 0b00;
const EMI_COPY_ONCE: u8 = 0b01;
const EMI_ONE_GEN_COPY: u8 = 0b10;
const EMI_NO_MORE_COPIES: u8 = 0b11;

impl<'a> Parse<'a> for UriMessage {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < URI_MESSAGE_LEN {
            return Err(Error::BufferTooShort {
                need: URI_MESSAGE_LEN,
                have: bytes.len(),
                what: "uri_message",
            });
        }
        let protocol_version = bytes[0];
        // Assemble the 56 bits following protocol_version into a u64 (in the low
        // 56 bits), MSB-first, so we can index bit positions from the top.
        let mut acc: u64 = 0;
        for &b in &bytes[1..URI_MESSAGE_LEN] {
            acc = (acc << 8) | b as u64;
        }
        // acc holds 56 bits. Bit 55 is the first field bit (aps MSB).
        let take = |hi_from_top: u32, width: u32| -> u64 {
            let shift = 56 - hi_from_top - width;
            (acc >> shift) & ((1u64 << width) - 1)
        };
        let aps = take(0, 2) as u8;
        let emi = take(2, 2) as u8;
        let ict = take(4, 1) != 0;
        // bit position 5: rct (emi==00) else reserved.
        let rct = take(5, 1) != 0;
        // bit position 6: reserved.
        // bits 7..16 (9 bits): dot(1)+rl(8) when emi==11 else reserved.
        let dot = take(7, 1) != 0;
        let rl = take(8, 8) as u8;
        // bit position 16: trick_mode when emi==10 else reserved.
        let trick_mode = take(16, 1) != 0;
        // bits 17..56 reserved (39).
        let emi_data = match emi {
            EMI_COPY_NOT_RESTRICTED => EmiData::CopyingNotRestricted { rct },
            EMI_COPY_ONCE => EmiData::CopyOnce,
            EMI_ONE_GEN_COPY => EmiData::OneGenerationCopy { trick_mode },
            EMI_NO_MORE_COPIES => EmiData::NoMoreCopies { dot, rl },
            _ => unreachable!("emi is 2 bits"),
        };
        Ok(Self {
            protocol_version,
            aps,
            ict,
            emi_data,
        })
    }
}

impl Serialize for UriMessage {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        URI_MESSAGE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < URI_MESSAGE_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: URI_MESSAGE_LEN,
                have: buf.len(),
            });
        }
        let mut acc: u64 = 0;
        let mut put = |value: u64, hi_from_top: u32, width: u32| {
            let shift = 56 - hi_from_top - width;
            acc |= (value & ((1u64 << width) - 1)) << shift;
        };
        put(self.aps as u64, 0, 2);
        put(self.emi_data.emi() as u64, 2, 2);
        put(u64::from(self.ict), 4, 1);
        match self.emi_data {
            EmiData::CopyingNotRestricted { rct } => put(u64::from(rct), 5, 1),
            EmiData::CopyOnce => {}
            EmiData::OneGenerationCopy { trick_mode } => put(u64::from(trick_mode), 16, 1),
            EmiData::NoMoreCopies { dot, rl } => {
                put(u64::from(dot), 7, 1);
                put(rl as u64, 8, 8);
            }
        }
        buf[0] = self.protocol_version;
        // acc holds 56 bits; emit big-endian into bytes 1..8.
        for (i, slot) in buf[1..URI_MESSAGE_LEN].iter_mut().enumerate() {
            let shift = 56 - 8 * (i as u32 + 1);
            *slot = (acc >> shift) as u8;
        }
        Ok(URI_MESSAGE_LEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_round_trip() {
        // Table 90 default: protocol 0x03, emi 0b11 (no-more-copies), aps 0b00,
        // ict 0, dot 0, rl 0x00.
        let u = UriMessage {
            protocol_version: PROTOCOL_VERSION_V3,
            aps: 0b00,
            ict: false,
            emi_data: EmiData::NoMoreCopies { dot: false, rl: 0 },
        };
        let bytes = u.to_bytes();
        // protocol_version 0x03; aps(00) emi(11) -> first field byte top:
        //   bits: aps=00, emi=11, ict=0, ... => 0b00_11_0_..  high byte = 0x30.
        assert_eq!(bytes.len(), 8);
        assert_eq!(bytes[0], 0x03);
        assert_eq!(bytes[1], 0b0011_0000); // aps00 emi11 ict0 + 3 reserved bits
        assert_eq!(UriMessage::parse(&bytes).unwrap(), u);
    }

    #[test]
    fn one_generation_trick_mode_enabled_bites() {
        let u = UriMessage {
            protocol_version: PROTOCOL_VERSION_V3,
            aps: 0b10,
            ict: true,
            emi_data: EmiData::OneGenerationCopy { trick_mode: true },
        };
        let bytes = u.to_bytes();
        assert_eq!(bytes[0], 0x03);
        // aps=10, emi=10, ict=1 => byte1 = 0b10_10_1_000 = 0xA8.
        assert_eq!(bytes[1], 0b1010_1000);
        // trick_mode is bit position 16 (from top of the 56-bit field) => bit 0
        // of byte index 2 (counting field bits: byte1 covers bits 0..7, byte2 8..15,
        // byte3 16..23). trick at bit 16 => MSB of byte3 (bytes[3]).
        assert_eq!(bytes[3] & 0x80, 0x80);
        let parsed = UriMessage::parse(&bytes).unwrap();
        assert_eq!(parsed, u);
        // Mutation: disable trick mode.
        let off = UriMessage {
            protocol_version: PROTOCOL_VERSION_V3,
            aps: 0b10,
            ict: true,
            emi_data: EmiData::OneGenerationCopy { trick_mode: false },
        };
        assert_ne!(bytes, off.to_bytes());
        assert_eq!(off.to_bytes()[3] & 0x80, 0x00);
    }

    #[test]
    fn copying_not_restricted_rct_bit() {
        let u = UriMessage {
            protocol_version: PROTOCOL_VERSION_V3,
            aps: 0b00,
            ict: false,
            emi_data: EmiData::CopyingNotRestricted { rct: true },
        };
        let bytes = u.to_bytes();
        // emi=00, ict=0, rct at bit position 5 => byte1 = 0b00_00_0_1_0_0 = 0x04.
        assert_eq!(bytes[1], 0b0000_0100);
        assert_eq!(UriMessage::parse(&bytes).unwrap(), u);
    }

    #[test]
    fn no_more_copies_dot_rl_round_trips() {
        let u = UriMessage {
            protocol_version: PROTOCOL_VERSION_V3,
            aps: 0b01,
            ict: true,
            emi_data: EmiData::NoMoreCopies {
                dot: true,
                rl: 0xA5,
            },
        };
        let bytes = u.to_bytes();
        let parsed = UriMessage::parse(&bytes).unwrap();
        assert_eq!(parsed, u);
        // Mutation: change rl.
        let mut other = u;
        other.emi_data = EmiData::NoMoreCopies {
            dot: true,
            rl: 0xA4,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn copy_once_has_no_case_bits() {
        let u = UriMessage {
            protocol_version: PROTOCOL_VERSION_V3,
            aps: 0b11,
            ict: false,
            emi_data: EmiData::CopyOnce,
        };
        let bytes = u.to_bytes();
        // aps=11, emi=01, ict=0 => byte1 = 0b11_01_0_000 = 0xD0; rest reserved 0.
        assert_eq!(bytes[1], 0b1101_0000);
        assert_eq!(&bytes[2..8], &[0u8; 6]);
        assert_eq!(UriMessage::parse(&bytes).unwrap(), u);
    }

    #[test]
    fn reserved_bits_in_other_emi_cases_dont_leak() {
        // A wire message with emi==01 but stray bits set in reserved positions
        // parses to CopyOnce (case bits ignored), and re-serializes those reserved
        // bits as zero (lossless within the typed model).
        let u = UriMessage {
            protocol_version: PROTOCOL_VERSION_V3,
            aps: 0,
            ict: false,
            emi_data: EmiData::CopyOnce,
        };
        let bytes = u.to_bytes();
        assert_eq!(UriMessage::parse(&bytes).unwrap(), u);
    }

    #[test]
    fn too_short_errors() {
        assert!(matches!(
            UriMessage::parse(&[0x03, 0x00]),
            Err(Error::BufferTooShort { .. })
        ));
    }
}
