//! `BedDefinition1` element — RDD 29:2019 §2.2/§4.3/§5.3.
//!
//! A Dolby Atmos "bed" is a collection of audio channels played back with a
//! nominal location or function (e.g. "Left", "LFE"); this element lists the
//! bed's channels and, for each, the [`crate::AudioDataDlc`] audio asset it
//! points to.

use alloc::vec::Vec;

use broadcast_common::bits::{BitReader, BitWriter};
use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::plex::{plex_bits, read_plex, write_plex};
use crate::util::{expect_fully_consumed, read_reserved, write_reserved};

/// The 4-bit `ChannelID` field (§5.3.3 Table 6): the nominal loudspeaker a
/// bed channel is assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ChannelId {
    /// `0x0` — Left Screen Speaker.
    LeftScreen,
    /// `0x1` — Left Center Screen Speaker.
    LeftCenterScreen,
    /// `0x2` — Center Screen Speaker.
    CenterScreen,
    /// `0x3` — Right Center Screen Speaker.
    RightCenterScreen,
    /// `0x4` — Right Screen Speaker.
    RightScreen,
    /// `0x5` — Left Side Surround Speaker (7.1).
    LeftSideSurround71,
    /// `0x6` — Left Surround Speaker.
    LeftSurround,
    /// `0x7` — Left Rear Surround Speaker (7.1).
    LeftRearSurround71,
    /// `0x8` — Right Rear Surround Speaker (7.1).
    RightRearSurround71,
    /// `0x9` — Right Side Surround Speaker (7.1).
    RightSideSurround71,
    /// `0xA` — Right Surround Speaker.
    RightSurround,
    /// `0xB` — Left Top Surround Speaker (9.1).
    LeftTopSurround91,
    /// `0xC` — Right Top Surround Speaker (9.1).
    RightTopSurround91,
    /// `0xD` — LFE Speaker.
    Lfe,
    /// Any other 4-bit code — reserved.
    Reserved(u8),
}

impl ChannelId {
    /// The spec token for this value ("reserved" for the reserved arm) —
    /// see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::LeftScreen => "left screen speaker",
            Self::LeftCenterScreen => "left center screen speaker",
            Self::CenterScreen => "center screen speaker",
            Self::RightCenterScreen => "right center screen speaker",
            Self::RightScreen => "right screen speaker",
            Self::LeftSideSurround71 => "left side surround speaker (7.1)",
            Self::LeftSurround => "left surround speaker",
            Self::LeftRearSurround71 => "left rear surround speaker (7.1)",
            Self::RightRearSurround71 => "right rear surround speaker (7.1)",
            Self::RightSideSurround71 => "right side surround speaker (7.1)",
            Self::RightSurround => "right surround speaker",
            Self::LeftTopSurround91 => "left top surround speaker (9.1)",
            Self::RightTopSurround91 => "right top surround speaker (9.1)",
            Self::Lfe => "lfe speaker",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u64) -> Self {
        match bits {
            0x0 => Self::LeftScreen,
            0x1 => Self::LeftCenterScreen,
            0x2 => Self::CenterScreen,
            0x3 => Self::RightCenterScreen,
            0x4 => Self::RightScreen,
            0x5 => Self::LeftSideSurround71,
            0x6 => Self::LeftSurround,
            0x7 => Self::LeftRearSurround71,
            0x8 => Self::RightRearSurround71,
            0x9 => Self::RightSideSurround71,
            0xA => Self::RightSurround,
            0xB => Self::LeftTopSurround91,
            0xC => Self::RightTopSurround91,
            0xD => Self::Lfe,
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u64 {
        match self {
            Self::LeftScreen => 0x0,
            Self::LeftCenterScreen => 0x1,
            Self::CenterScreen => 0x2,
            Self::RightCenterScreen => 0x3,
            Self::RightScreen => 0x4,
            Self::LeftSideSurround71 => 0x5,
            Self::LeftSurround => 0x6,
            Self::LeftRearSurround71 => 0x7,
            Self::RightRearSurround71 => 0x8,
            Self::RightSideSurround71 => 0x9,
            Self::RightSurround => 0xA,
            Self::LeftTopSurround91 => 0xB,
            Self::RightTopSurround91 => 0xC,
            Self::Lfe => 0xD,
            Self::Reserved(v) => u64::from(v),
        }
    }
}

broadcast_common::impl_spec_display!(ChannelId, Reserved);

/// One bed channel: a [`ChannelId`] paired with the [`crate::AudioDataDlc`]
/// `AudioDataID` it draws audio from (§4.3, the `for(n...)` loop body).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BedChannel {
    /// The nominal loudspeaker this channel plays back to.
    pub channel_id: ChannelId,
    /// The `AudioDataDLC` element's `AudioDataID` carrying this channel's
    /// audio essence. `0` means "no audio asset" (§5.3.4).
    pub audio_data_id: u32,
}

/// `Reserved` (1 bit, before `ChannelCount`) — always `0` (§4.3).
const RESERVED_PRE_CHANNEL_COUNT: u64 = 0;
/// `Reserved` (3 bits, per channel) — always `0` (§4.3).
const RESERVED_PER_CHANNEL: u64 = 0;
/// `Reserved` (10 bits, after the channel loop) — always `0x180` (§4.3).
const RESERVED_TRAILER_1: u64 = 0x180;
/// `Reserved` (8 bits) — always `0x5` (§4.3).
const RESERVED_TRAILER_2: u64 = 0x5;
/// `Reserved` (8 bits, final) — always `0` (§4.3).
const RESERVED_TRAILER_3: u64 = 0;

/// The `BedDefinition1` element — RDD 29 §2.2/§4.3/§5.3: metadata and
/// pointers to audio essence for one frame of one audio bed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BedDefinition1 {
    /// `MetaID` — unique ID that aids tracking this metadata between frames
    /// (§5.3.1).
    pub meta_id: u32,
    /// The bed's channels, in wire order. `ChannelCount` (§5.3.2) is
    /// `channels.len()`, not stored separately.
    pub channels: Vec<BedChannel>,
}

impl BedDefinition1 {
    /// Build a new `BedDefinition1` from its channels.
    #[must_use]
    pub fn new(meta_id: u32, channels: Vec<BedChannel>) -> Self {
        Self { meta_id, channels }
    }
}

impl<'a> Parse<'a> for BedDefinition1 {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut r = BitReader::new(bytes);
        let meta_id = read_plex(&mut r, 8, "BedDefinition1.MetaID")? as u32;
        read_reserved(
            &mut r,
            1,
            RESERVED_PRE_CHANNEL_COUNT,
            "BedDefinition1.Reserved(pre-ChannelCount)",
        )?;
        let channel_count = read_plex(&mut r, 4, "BedDefinition1.ChannelCount")?;
        let mut channels = Vec::with_capacity(channel_count as usize);
        for _ in 0..channel_count {
            let channel_id =
                ChannelId::from_bits(read_plex(&mut r, 4, "BedDefinition1.ChannelID")?);
            let audio_data_id = read_plex(&mut r, 8, "BedDefinition1.AudioDataID")? as u32;
            read_reserved(
                &mut r,
                3,
                RESERVED_PER_CHANNEL,
                "BedDefinition1.Reserved(per-channel)",
            )?;
            channels.push(BedChannel {
                channel_id,
                audio_data_id,
            });
        }
        read_reserved(
            &mut r,
            10,
            RESERVED_TRAILER_1,
            "BedDefinition1.Reserved(trailer1)",
        )?;
        r.align_to_byte();
        read_reserved(
            &mut r,
            8,
            RESERVED_TRAILER_2,
            "BedDefinition1.Reserved(trailer2)",
        )?;
        read_reserved(
            &mut r,
            8,
            RESERVED_TRAILER_3,
            "BedDefinition1.Reserved(trailer3)",
        )?;
        expect_fully_consumed(&r, "BedDefinition1")?;
        Ok(Self { meta_id, channels })
    }
}

impl Serialize for BedDefinition1 {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut bits = plex_bits(u64::from(self.meta_id), 8) + 1;
        bits += plex_bits(self.channels.len() as u64, 4);
        for ch in &self.channels {
            bits += plex_bits(ch.channel_id.to_bits(), 4);
            bits += plex_bits(u64::from(ch.audio_data_id), 8);
            bits += 3;
        }
        bits += 10;
        let pre_trailer_bytes = (bits as usize).div_ceil(8);
        pre_trailer_bytes + 1 + 1
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: buf.len(),
                what: "BedDefinition1",
            });
        }
        let mut w = BitWriter::new(&mut buf[..need]);
        write_plex(&mut w, u64::from(self.meta_id), 8, "BedDefinition1.MetaID")?;
        write_reserved(
            &mut w,
            1,
            RESERVED_PRE_CHANNEL_COUNT,
            "BedDefinition1.Reserved(pre-ChannelCount)",
        )?;
        write_plex(
            &mut w,
            self.channels.len() as u64,
            4,
            "BedDefinition1.ChannelCount",
        )?;
        for ch in &self.channels {
            write_plex(
                &mut w,
                ch.channel_id.to_bits(),
                4,
                "BedDefinition1.ChannelID",
            )?;
            write_plex(
                &mut w,
                u64::from(ch.audio_data_id),
                8,
                "BedDefinition1.AudioDataID",
            )?;
            write_reserved(
                &mut w,
                3,
                RESERVED_PER_CHANNEL,
                "BedDefinition1.Reserved(per-channel)",
            )?;
        }
        write_reserved(
            &mut w,
            10,
            RESERVED_TRAILER_1,
            "BedDefinition1.Reserved(trailer1)",
        )?;
        w.align_to_byte().map_err(|source| Error::Bits {
            what: "BedDefinition1.AlignBits",
            source,
        })?;
        write_reserved(
            &mut w,
            8,
            RESERVED_TRAILER_2,
            "BedDefinition1.Reserved(trailer2)",
        )?;
        write_reserved(
            &mut w,
            8,
            RESERVED_TRAILER_3,
            "BedDefinition1.Reserved(trailer3)",
        )?;
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BedDefinition1 {
        BedDefinition1::new(
            7,
            alloc::vec![
                BedChannel {
                    channel_id: ChannelId::LeftScreen,
                    audio_data_id: 1,
                },
                BedChannel {
                    channel_id: ChannelId::RightScreen,
                    audio_data_id: 2,
                },
                BedChannel {
                    channel_id: ChannelId::Lfe,
                    audio_data_id: 3,
                },
            ],
        )
    }

    #[test]
    fn round_trips() {
        let bed = sample();
        let bytes = bed.to_bytes();
        let parsed = BedDefinition1::parse(&bytes).unwrap();
        assert_eq!(parsed, bed);

        let bytes2 = parsed.to_bytes();
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn trailer_constants_are_validated() {
        let bed = sample();
        let mut bytes = bed.to_bytes();
        let last = bytes.len() - 2;
        bytes[last] = 0xAA; // corrupt "Reserved (set to 0x5)"
        let err = BedDefinition1::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidReserved { .. }));
    }

    #[test]
    fn empty_bed_round_trips() {
        let bed = BedDefinition1::new(0, Vec::new());
        let bytes = bed.to_bytes();
        let parsed = BedDefinition1::parse(&bytes).unwrap();
        assert_eq!(parsed, bed);
    }

    #[test]
    fn mutating_meta_id_changes_only_meta_id_bytes() {
        let mut bed = sample();
        let original = bed.to_bytes();
        bed.meta_id = 200; // still Plex(8)-direct (<=0xFE)
        let mutated = bed.to_bytes();
        assert_ne!(original[0], mutated[0]);
        assert_eq!(&original[1..], &mutated[1..]);
    }
}
