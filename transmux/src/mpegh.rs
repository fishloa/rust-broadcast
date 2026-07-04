//! MPEG-H 3D Audio in ISOBMFF — `mha1`/`mhm1` + `mhaC` (ISO/IEC 23008-3 §20).
//!
//! Implements the `MHAConfigurationBox` (`mhaC`) and the typed
//! `MHADecoderConfigurationRecord` that it carries.  The `mpegh3daConfig` blob
//! is **opaque** to this crate — the caller supplies it already encoded
//! (identical treatment to the `esds` AudioSpecificConfig and `dac4` body).
//!
//! Sources:
//! - **ISO/IEC 23008-3 §20** — canonical box/record syntax (paid).
//! - **ATSC A/342-3 §5.2.2** — profile-level constraints for broadcast
//!   (`transmux/docs/codec/mpegh-atsc-a342-3.md`).
//! - Record layout: `transmux/docs/codec/mpegh-mhaC.md`.
//!
//! ## Record layout
//!
//! ```text
//! aligned(8) class MHADecoderConfigurationRecord {
//!     unsigned int(8)  configurationVersion;           // must be 1
//!     unsigned int(8)  mpegh3daProfileLevelIndication; // CICP profile-level
//!     unsigned int(8)  referenceChannelLayout;         // CICP ChannelConfig
//!     unsigned int(16) mpegh3daConfigLength;
//!     unsigned int(8)  mpegh3daConfig[mpegh3daConfigLength];
//! }
//! ```
//!
//! ## `mhaC` box
//!
//! `MHAConfigurationBox` extends `Box('mhaC')` with exactly one
//! `MHADecoderConfigurationRecord` as its body.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

// ---------------------------------------------------------------------------
// Four-CC constants
// ---------------------------------------------------------------------------

/// FourCC of the MPEG-H configuration box (`mhaC`).
pub const MHAC_FOURCC: [u8; 4] = *b"mhaC";
/// FourCC of the MPEG-H 3D Audio sample entry — raw (unpackaged) MHAS frames.
pub const MHA1_FOURCC: [u8; 4] = *b"mha1";
/// FourCC of the MPEG-H 3D Audio sample entry (variant 2).
pub const MHA2_FOURCC: [u8; 4] = *b"mha2";
/// FourCC of the MPEG-H 3D Audio sample entry — in-band MHAS (config in stream).
pub const MHM1_FOURCC: [u8; 4] = *b"mhm1";
/// FourCC of the MPEG-H 3D Audio sample entry (variant 2, in-band).
pub const MHM2_FOURCC: [u8; 4] = *b"mhm2";

// ---------------------------------------------------------------------------
// Fixed sizes
// ---------------------------------------------------------------------------

/// Minimum byte size of the `MHADecoderConfigurationRecord` (header fields only,
/// no `mpegh3daConfig` bytes).
///
/// Byte layout: `configurationVersion` `[7:0]` (1) + `mpegh3daProfileLevelIndication`
/// `[7:0]` (1) + `referenceChannelLayout` `[7:0]` (1) + `mpegh3daConfigLength` `[15:0]` (2)
/// = 5 bytes.
pub const MHAC_RECORD_FIXED_LEN: usize = 5;

/// The only valid value for `configurationVersion` (ISO/IEC 23008-3 §20).
pub const MHAC_CONFIGURATION_VERSION: u8 = 1;

// ---------------------------------------------------------------------------
// MHADecoderConfigurationRecord
// ---------------------------------------------------------------------------

/// `MHADecoderConfigurationRecord` — ISO/IEC 23008-3 §20.
///
/// The fixed-header part of an `mhaC` box.  The `mpegh3daConfig` blob is kept
/// opaque; transmux copies it through unchanged.
///
/// ## Wire layout
///
/// | Offset | Size | Field |
/// |--------|------|-------|
/// | 0      | 1    | `configurationVersion` (= 1) |
/// | 1      | 1    | `mpegh3daProfileLevelIndication` `[7:0]` |
/// | 2      | 1    | `referenceChannelLayout` `[7:0]` |
/// | 3      | 2    | `mpegh3daConfigLength` `[15:0]` (big-endian) |
/// | 5      | N    | `mpegh3daConfig[0..mpegh3daConfigLength]` |
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MHADecoderConfigurationRecord {
    /// Must be `1` per ISO/IEC 23008-3 §20.
    pub configuration_version: u8,
    /// CICP profile-level indicator `[7:0]`.  ATSC A/342-3 §5.2.2.1 mandates
    /// `0x0B`/`0x0C`/`0x0D` (LC L1/L2/L3) and `0x10`/`0x11`/`0x12` (BL L1/L2/L3).
    pub mpegh3da_profile_level_indication: u8,
    /// CICP `ChannelConfiguration` `[7:0]` (reference channel layout).
    pub reference_channel_layout: u8,
    /// The opaque `mpegh3daConfig()` bytes (ISO/IEC 23008-3 §20).
    pub mpegh3da_config: Vec<u8>,
}

impl MHADecoderConfigurationRecord {
    /// Construct a record from its constituent fields.
    pub fn new(
        mpegh3da_profile_level_indication: u8,
        reference_channel_layout: u8,
        mpegh3da_config: Vec<u8>,
    ) -> Self {
        Self {
            configuration_version: MHAC_CONFIGURATION_VERSION,
            mpegh3da_profile_level_indication,
            reference_channel_layout,
            mpegh3da_config,
        }
    }

    /// RFC 6381 codec string for this profile-level — `mhm1.0xNN`
    /// (ATSC A/342-3 §5.2.2; `mhm1` is the in-band MHAS sample-entry code).
    pub fn rfc6381(&self) -> alloc::string::String {
        alloc::format!("mhm1.0x{:02X}", self.mpegh3da_profile_level_indication)
    }
}

impl<'a> Parse<'a> for MHADecoderConfigurationRecord {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MHAC_RECORD_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: MHAC_RECORD_FIXED_LEN,
                have: bytes.len(),
                what: "MHADecoderConfigurationRecord",
            });
        }
        let configuration_version = bytes[0];
        if configuration_version != MHAC_CONFIGURATION_VERSION {
            return Err(Error::InvalidValue {
                field: "configurationVersion",
                value: configuration_version as u64,
                reason: "must be 1 (ISO/IEC 23008-3 §20)",
            });
        }
        let mpegh3da_profile_level_indication = bytes[1];
        let reference_channel_layout = bytes[2];
        let config_len = u16::from_be_bytes([bytes[3], bytes[4]]) as usize;
        let need = MHAC_RECORD_FIXED_LEN + config_len;
        if bytes.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: bytes.len(),
                what: "MHADecoderConfigurationRecord.mpegh3daConfig",
            });
        }
        let mpegh3da_config = bytes[MHAC_RECORD_FIXED_LEN..need].to_vec();
        Ok(Self {
            configuration_version,
            mpegh3da_profile_level_indication,
            reference_channel_layout,
            mpegh3da_config,
        })
    }
}

impl Serialize for MHADecoderConfigurationRecord {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MHAC_RECORD_FIXED_LEN + self.mpegh3da_config.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.configuration_version;
        buf[1] = self.mpegh3da_profile_level_indication;
        buf[2] = self.reference_channel_layout;
        let config_len = self.mpegh3da_config.len() as u16;
        buf[3..5].copy_from_slice(&config_len.to_be_bytes());
        buf[5..need].copy_from_slice(&self.mpegh3da_config);
        Ok(need)
    }
}

// ---------------------------------------------------------------------------
// MhaC box (MHAConfigurationBox body — record only, without the box header)
// ---------------------------------------------------------------------------

/// The body of an `mhaC` box (`MHAConfigurationBox`) — ISO/IEC 23008-3 §20.
///
/// Serialises/parses only the record bytes; the 8-byte box header (`size` +
/// `'mhaC'`) is added by the caller via [`crate::init_segment::OpaqueBox`].
pub type MhaCBox = MHADecoderConfigurationRecord;

// ---------------------------------------------------------------------------
// MHAS packet framing (MPEG-2 TS carriage — issue #579)
// ---------------------------------------------------------------------------
//
// MPEG-H Audio elementary streams in MPEG-2 TS are formatted as the MPEG-H
// Audio Stream (MHAS), ISO/IEC 23008-3 Clause 14 — ETSI TS 101 154 §6.8.3
// ("MHAS elementary stream formatting") and ATSC A/342-3 §5.2.1 both cite it
// but neither vendored spec transcribes the packet's byte-level framing
// (Clause 14 itself is ISO/IEC 23008-3, paid, not vendored). This crate never
// decodes the MPEG-H audio bitstream, so the framing below is used for
// exactly one purpose: **locating** the `PACTYP_MPEGH3DACFG` packet whose
// payload is the opaque `mpegh3daConfig()` blob (already copied through
// unchanged by [`MHADecoderConfigurationRecord`]), and flagging an access
// unit that carries one as a random-access point.
//
// The packet-type numbering and the three-tier "escaped value" header coding
// are **empirically verified**, not printed in either vendored spec: every
// access unit of the vendored Fraunhofer MPEG-H-in-TS fixture
// (`private/fixtures/ts/mpegh-cicp01-baseline.ts`) parses end-to-end under
// this scheme with no truncation/overrun, and the two random-access access
// units decode to exactly the packet order ETSI TS 101 154 §6.8.4.1
// mandates for a RAP (`PACTYP_MPEGH3DACFG`, then — directly following, per
// §6.8.4.1 — `PACTYP_AUDIOSCENEINFO`, `PACTYP_BUFFERINFO`,
// `PACTYP_MPEGH3DAFRAME`). The recovered `mpegh3daConfig()`'s leading byte
// also independently agrees with the PMT `MPEG-H_3dAudio_descriptor`'s
// `mpegh3daProfileLevelIndication` byte in the same fixture (both `0x10` —
// BL Profile Level 1), cross-confirming the packet boundaries are correct.
// See `transmux/docs/codec/mpegh-ts-101154.md` for the full derivation.

/// `MHASPacketType` for the config packet carrying `mpegh3daConfig()`
/// (empirically verified — see the module section docs above).
const MHAS_PACTYP_MPEGH3DACFG: u8 = 1;

/// Bit widths `(n1, n2, n3)` for the three-tier "escaped value" header
/// coding: read `n1` bits; if all-ones, read `n2` more and add; if *those*
/// are all-ones too, read `n3` more and add. Chosen (empirically confirmed)
/// so every representable `MHASPacketType`/`Label`/`Length` combination
/// leaves the following payload byte-aligned, matching ATSC A/342-3 §4.2.3's
/// description that "any MHAS packet payload always is byte-aligned".
const MHAS_TYPE_BITS: (u32, u32, u32) = (3, 8, 8);
const MHAS_LABEL_BITS: (u32, u32, u32) = (2, 8, 32);
const MHAS_LENGTH_BITS: (u32, u32, u32) = (11, 24, 24);

/// A big-endian, MSB-first bit cursor over a byte slice.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    /// Read `n` bits (`n <= 64`), MSB first. `None` if fewer than `n` bits remain.
    fn read(&mut self, n: u32) -> Option<u64> {
        if (n as usize) > self.bits_left() {
            return None;
        }
        let mut v = 0u64;
        for _ in 0..n {
            let byte = self.data[self.pos / 8];
            let bit = (byte >> (7 - (self.pos % 8))) & 1;
            v = (v << 1) | u64::from(bit);
            self.pos += 1;
        }
        Some(v)
    }

    /// The current position as a byte offset, if bit-aligned to a byte.
    fn byte_pos(&self) -> Option<usize> {
        (self.pos % 8 == 0).then_some(self.pos / 8)
    }
}

/// Decode one three-tier "escaped value" field (see [`MHAS_TYPE_BITS`] etc.).
fn escaped_value(br: &mut BitReader<'_>, bits: (u32, u32, u32)) -> Option<u64> {
    let (n1, n2, n3) = bits;
    let mut value = br.read(n1)?;
    if value == (1u64 << n1) - 1 {
        let v2 = br.read(n2)?;
        value += v2;
        if v2 == (1u64 << n2) - 1 {
            value += br.read(n3)?;
        }
    }
    Some(value)
}

/// One MHAS packet's type and payload, walked from an MHAS elementary-stream
/// access unit (`MHASPacketLabel` is discarded — not needed for config/RAP
/// recovery).
pub(crate) struct MhasPacket<'a> {
    pub(crate) packet_type: u8,
    pub(crate) payload: &'a [u8],
}

/// Walk the MHAS packets in one access unit. Stops (returning whatever was
/// found so far) at the first malformed/truncated header instead of
/// erroring: this framing is used only to *locate* the
/// `PACTYP_MPEGH3DACFG` payload, never to reject a stream — an
/// unparseable/unknown-future packet layout simply yields no more packets,
/// and the access unit's bytes are still carried through opaquely as the
/// `Sample`.
pub(crate) fn walk_mhas_packets(data: &[u8]) -> Vec<MhasPacket<'_>> {
    let mut br = BitReader::new(data);
    let mut out = Vec::new();
    // A packet's minimal header (unescaped type + label + length) is 16 bits.
    const MIN_HEADER_BITS: usize = 16;
    while br.bits_left() >= MIN_HEADER_BITS {
        let Some(packet_type) = escaped_value(&mut br, MHAS_TYPE_BITS) else {
            break;
        };
        let Some(_label) = escaped_value(&mut br, MHAS_LABEL_BITS) else {
            break;
        };
        let Some(length) = escaped_value(&mut br, MHAS_LENGTH_BITS) else {
            break;
        };
        let Some(start) = br.byte_pos() else {
            break; // never byte-aligned in a well-formed MHAS stream
        };
        if packet_type > u64::from(u8::MAX) {
            break;
        }
        let Some(end) = start.checked_add(length as usize) else {
            break;
        };
        if end > data.len() {
            break;
        }
        out.push(MhasPacket {
            packet_type: packet_type as u8,
            payload: &data[start..end],
        });
        br.pos = end * 8;
    }
    out
}

/// Find the `mpegh3daConfig()` blob — the `PACTYP_MPEGH3DACFG` packet's
/// payload — in one MHAS access unit, if present. An access unit carrying
/// one is a random-access point (ETSI TS 101 154 §6.8.4.1).
pub(crate) fn find_mpegh3da_config(data: &[u8]) -> Option<&[u8]> {
    walk_mhas_packets(data)
        .into_iter()
        .find(|p| p.packet_type == MHAS_PACTYP_MPEGH3DACFG)
        .map(|p| p.payload)
}

#[cfg(test)]
mod mhas_tests {
    use super::*;

    /// Hand-built two-packet MHAS buffer using only unescaped (base) header
    /// widths: a 1-byte `PACTYP_SYNC`-style packet (type=6, label=0, len=1,
    /// payload `0xA5`) followed by a `PACTYP_MPEGH3DACFG`-style packet
    /// (type=1, label=1, len=3, payload `0x10 0x11 0x12`).
    fn build_packet(packet_type: u8, label: u8, payload: &[u8]) -> Vec<u8> {
        // header: 3 bits type + 2 bits label + 11 bits length = 16 bits (2 bytes).
        let len = payload.len() as u16;
        assert!(packet_type < 7 && label < 3 && len < 0x7FF);
        let word: u32 = ((packet_type as u32) << 13) | ((label as u32) << 11) | len as u32;
        let mut out = alloc::vec![(word >> 8) as u8, (word & 0xFF) as u8];
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn walks_unescaped_packets() {
        let mut data = build_packet(6, 0, &[0xA5]);
        data.extend(build_packet(
            MHAS_PACTYP_MPEGH3DACFG,
            1,
            &[0x10, 0x11, 0x12],
        ));
        let packets = walk_mhas_packets(&data);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].packet_type, 6);
        assert_eq!(packets[0].payload, &[0xA5]);
        assert_eq!(packets[1].packet_type, MHAS_PACTYP_MPEGH3DACFG);
        assert_eq!(packets[1].payload, &[0x10, 0x11, 0x12]);
    }

    #[test]
    fn finds_config_packet() {
        let mut data = build_packet(6, 0, &[0xA5]);
        data.extend(build_packet(MHAS_PACTYP_MPEGH3DACFG, 1, &[0xAA, 0xBB]));
        assert_eq!(find_mpegh3da_config(&data), Some([0xAA, 0xBB].as_slice()));
    }

    #[test]
    fn no_config_packet_returns_none() {
        let data = build_packet(6, 0, &[0xA5]);
        assert_eq!(find_mpegh3da_config(&data), None);
    }

    #[test]
    fn truncated_header_stops_cleanly() {
        // A single stray byte: not enough bits for even the minimal header.
        let data = [0xFFu8];
        assert!(walk_mhas_packets(&data).is_empty());
    }

    #[test]
    fn declared_length_past_end_stops_cleanly() {
        // type=1(MPEGH3DACFG) label=0 len=5, but only 1 payload byte follows.
        let word: u32 = (1u32 << 13) | 5;
        let data = [(word >> 8) as u8, (word & 0xFF) as u8, 0xAA];
        assert!(walk_mhas_packets(&data).is_empty());
    }
}
