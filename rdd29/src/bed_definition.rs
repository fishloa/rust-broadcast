//! `BedDefinition1` element — RDD 29:2019 §2.2/§4.3/§5.3.
//!
//! A Dolby Atmos "bed" is a collection of audio channels played back with a
//! nominal location or function (e.g. "Left", "LFE"); this element lists the
//! bed's channels and, for each, the [`crate::AudioDataDlc`] audio asset it
//! points to.

use alloc::vec::Vec;
use broadcast_common::len::need_out;

/// Minimum bits one channel description occupies: `Plex(4)` ChannelID +
/// `Plex(8)` AudioDataID + 3 reserved bits (SMPTE RDD 29:2019 §4.3).
///
/// Used only as the provable ceiling on `ChannelCount` — a count implying more
/// channels than the remaining bits could describe is rejected, per the
/// unbounded-allocation fix.
const BITS_PER_CHANNEL_DESCRIPTION: u64 = 4 + 8 + 3;

/// Plex(8) container width for `MetaID` and `AudioDataID`.
const PLEX_BASE_WIDTH: u32 = 8;
/// Plex(4) container width for `ChannelCount` and `ChannelID`.
const CHANNEL_PLEX_WIDTH: u32 = 4;
/// Width of the 3-bit per-channel reserved field.
const RESERVED_PER_CHANNEL_BITS: u32 = 3;
/// Width of the 10-bit trailer-1 reserved field.
const RESERVED_TRAILER_1_BITS: u32 = 10;
/// Width of each 8-bit trailer reserved field (`trailer2`/`trailer3`).
const RESERVED_TRAILER_OCTET_BITS: u32 = 8;
/// Bits per byte, for bit<->byte length conversions.
const BITS_PER_BYTE: usize = 8;

use broadcast_common::bits::{BitReader, BitWriter};
use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::plex::{plex_bits, read_plex, write_plex};
use crate::util::{
    expect_fully_consumed, read_align_bits, read_reserved, write_align_bits, write_reserved,
};

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
    /// The raw `AlignBits` byte-alignment padding value (`0` if the fields
    /// above already land on a byte boundary). RDD 29 gives this field no
    /// documented value (unlike this element's other reserved fields), so it
    /// is preserved verbatim rather than forced to zero (RD-W2, #1114) —
    /// [`BedDefinition1::new`] sets it to `0`.
    pub align_bits: u8,
}

impl BedDefinition1 {
    /// Build a new `BedDefinition1` from its channels, with `align_bits`
    /// `0`.
    #[must_use]
    pub fn new(meta_id: u32, channels: Vec<BedChannel>) -> Self {
        Self {
            meta_id,
            channels,
            align_bits: 0,
        }
    }
}

impl<'a> Parse<'a> for BedDefinition1 {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut r = BitReader::new(bytes);
        let meta_id = read_plex(&mut r, PLEX_BASE_WIDTH, "BedDefinition1.MetaID")? as u32;
        read_reserved(
            &mut r,
            1,
            RESERVED_PRE_CHANNEL_COUNT,
            "BedDefinition1.Reserved(pre-ChannelCount)",
        )?;
        let channel_count = read_plex(&mut r, CHANNEL_PLEX_WIDTH, "BedDefinition1.ChannelCount")?;
        // `channel_count` is a Plex field read from the input and is therefore
        // unbounded — see the same hazard in `atmos_frame.rs`. Each channel
        // consumes at least `BITS_PER_CHANNEL_DESCRIPTION` bits, so that
        // divisor gives the
        // provable ceiling on the count, not the raw `remaining_bits`. Given
        // the fewer bits, the reserve is the smaller of that ceiling and the
        // wire claim — and an impossible ceiling is REJECTED rather than a
        // reserve that overcommit would let succeed silently (the identical
        // reasoning as `ATMOSFrame.SubElementCount`).
        let remaining_bits = r.bits_remaining() as u64;
        let provable_ceiling = remaining_bits / BITS_PER_CHANNEL_DESCRIPTION;
        if channel_count > provable_ceiling {
            return Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "BedDefinition1.ChannelCount",
                value: channel_count,
                reason: "exceeds the bits remaining in the element",
            }));
        }
        let mut channels = Vec::with_capacity(channel_count as usize);
        for _ in 0..channel_count {
            let channel_id = ChannelId::from_bits(read_plex(
                &mut r,
                CHANNEL_PLEX_WIDTH,
                "BedDefinition1.ChannelID",
            )?);
            let audio_data_id =
                read_plex(&mut r, PLEX_BASE_WIDTH, "BedDefinition1.AudioDataID")? as u32;
            read_reserved(
                &mut r,
                RESERVED_PER_CHANNEL_BITS,
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
            RESERVED_TRAILER_1_BITS,
            RESERVED_TRAILER_1,
            "BedDefinition1.Reserved(trailer1)",
        )?;
        let align_bits = read_align_bits(&mut r, "BedDefinition1.AlignBits")?;
        read_reserved(
            &mut r,
            RESERVED_TRAILER_OCTET_BITS,
            RESERVED_TRAILER_2,
            "BedDefinition1.Reserved(trailer2)",
        )?;
        read_reserved(
            &mut r,
            RESERVED_TRAILER_OCTET_BITS,
            RESERVED_TRAILER_3,
            "BedDefinition1.Reserved(trailer3)",
        )?;
        expect_fully_consumed(&r, "BedDefinition1")?;
        Ok(Self {
            meta_id,
            channels,
            align_bits,
        })
    }
}

impl Serialize for BedDefinition1 {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut bits = plex_bits(u64::from(self.meta_id), PLEX_BASE_WIDTH) + 1;
        bits += plex_bits(self.channels.len() as u64, CHANNEL_PLEX_WIDTH);
        for ch in &self.channels {
            bits += plex_bits(ch.channel_id.to_bits(), CHANNEL_PLEX_WIDTH);
            bits += plex_bits(u64::from(ch.audio_data_id), PLEX_BASE_WIDTH);
            bits += RESERVED_PER_CHANNEL_BITS;
        }
        bits += RESERVED_TRAILER_1_BITS;
        let pre_trailer_bytes = (bits as usize).div_ceil(BITS_PER_BYTE);
        pre_trailer_bytes + 1 + 1
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut w = BitWriter::new(&mut buf[..need]);
        write_plex(
            &mut w,
            u64::from(self.meta_id),
            PLEX_BASE_WIDTH,
            "BedDefinition1.MetaID",
        )?;
        write_reserved(
            &mut w,
            1,
            RESERVED_PRE_CHANNEL_COUNT,
            "BedDefinition1.Reserved(pre-ChannelCount)",
        )?;
        write_plex(
            &mut w,
            self.channels.len() as u64,
            CHANNEL_PLEX_WIDTH,
            "BedDefinition1.ChannelCount",
        )?;
        for ch in &self.channels {
            write_plex(
                &mut w,
                ch.channel_id.to_bits(),
                CHANNEL_PLEX_WIDTH,
                "BedDefinition1.ChannelID",
            )?;
            write_plex(
                &mut w,
                u64::from(ch.audio_data_id),
                PLEX_BASE_WIDTH,
                "BedDefinition1.AudioDataID",
            )?;
            write_reserved(
                &mut w,
                RESERVED_PER_CHANNEL_BITS,
                RESERVED_PER_CHANNEL,
                "BedDefinition1.Reserved(per-channel)",
            )?;
        }
        write_reserved(
            &mut w,
            RESERVED_TRAILER_1_BITS,
            RESERVED_TRAILER_1,
            "BedDefinition1.Reserved(trailer1)",
        )?;
        write_align_bits(&mut w, self.align_bits, "BedDefinition1.AlignBits")?;
        write_reserved(
            &mut w,
            RESERVED_TRAILER_OCTET_BITS,
            RESERVED_TRAILER_2,
            "BedDefinition1.Reserved(trailer2)",
        )?;
        write_reserved(
            &mut w,
            RESERVED_TRAILER_OCTET_BITS,
            RESERVED_TRAILER_3,
            "BedDefinition1.Reserved(trailer3)",
        )?;
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::string::ToString;

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
        let bytes = bed.try_to_bytes().unwrap();
        let parsed = BedDefinition1::parse(&bytes).unwrap();
        assert_eq!(parsed, bed);

        let bytes2 = parsed.try_to_bytes().unwrap();
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn trailer_constants_are_validated() {
        let bed = sample();
        let mut bytes = bed.try_to_bytes().unwrap();
        let last = bytes.len() - 2;
        bytes[last] = 0xAA; // corrupt "Reserved (set to 0x5)"
        let err = BedDefinition1::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::ReservedBits(..)));
    }

    #[test]
    fn empty_bed_round_trips() {
        let bed = BedDefinition1::new(0, Vec::new());
        let bytes = bed.try_to_bytes().unwrap();
        let parsed = BedDefinition1::parse(&bytes).unwrap();
        assert_eq!(parsed, bed);
    }

    #[test]
    fn mutating_meta_id_changes_only_meta_id_bytes() {
        let mut bed = sample();
        let original = bed.try_to_bytes().unwrap();
        bed.meta_id = 200; // still Plex(8)-direct (<=0xFE)
        let mutated = bed.try_to_bytes().unwrap();
        assert_ne!(original[0], mutated[0]);
        assert_eq!(&original[1..], &mutated[1..]);
    }

    /// RD-W2 (audit issue #1114): RDD 29 gives `AlignBits` no documented
    /// value, so a real producer's non-zero padding must round-trip
    /// byte-exactly rather than being silently zeroed. Observed pre-fix (no
    /// `align_bits` field existed): a real fixture with the corresponding
    /// byte flipped still parsed `Ok` to the identical `BedDefinition1`
    /// (`Reserved`/`ChannelID`/etc. all unaffected — the flipped bits were
    /// dropped, not decoded into anything), but `parsed.to_bytes()` zeroed
    /// them back out, differing from the input bytes.
    #[test]
    fn nonzero_align_bits_round_trip() {
        let mut bed = sample();
        bed.align_bits = 0b1010; // this fixture needs exactly 4 padding bits
        let bytes = bed.try_to_bytes().unwrap();
        let parsed = BedDefinition1::parse(&bytes).unwrap();
        assert_eq!(parsed, bed);
        assert_eq!(
            parsed.try_to_bytes().unwrap(),
            bytes,
            "AlignBits must round-trip byte-exactly"
        );
    }

    #[test]
    fn align_bits_too_wide_for_the_padding_is_rejected() {
        let mut bed = sample();
        // This fixture needs exactly 4 padding bits (0..=0xF); 0x10 doesn't fit.
        bed.align_bits = 0x10;
        let mut buf = alloc::vec![0u8; bed.serialized_len()];
        assert!(matches!(
            bed.serialize_into(&mut buf),
            Err(Error::InvalidValue(..))
        ));
    }

    /// The AlignBits field sits before the last two reserved bytes; with
    /// seven channels everything before it already ends on a byte boundary,
    /// so only `0` fits there.
    #[test]
    fn align_bits_must_be_zero_when_no_padding_bits_are_needed() {
        let mut bed = BedDefinition1::new(
            1,
            alloc::vec![
                BedChannel {
                    channel_id: ChannelId::Lfe,
                    audio_data_id: 1,
                };
                7
            ],
        );
        bed.align_bits = 0;
        assert!(bed.try_to_bytes().is_ok());
        bed.align_bits = 1;
        let mut buf = alloc::vec![0u8; bed.serialized_len()];
        assert!(
            matches!(bed.serialize_into(&mut buf), Err(Error::InvalidValue(..))),
            "nonzero AlignBits where no padding bits exist must be rejected"
        );
    }

    /// The widest value that fits the four padding bits is accepted.
    #[test]
    fn align_bits_at_the_padding_maximum_are_accepted() {
        let mut bed = sample();
        bed.align_bits = 0x0F;
        let bytes = bed.try_to_bytes().unwrap();
        assert_eq!(BedDefinition1::parse(&bytes).unwrap().align_bits, 0x0F);
    }

    #[test]
    fn trailing_bits_in_the_declared_element_are_rejected() {
        let bed = sample();
        let mut bytes = bed.try_to_bytes().unwrap();
        bytes.push(0x00);
        assert!(matches!(
            BedDefinition1::parse(&bytes),
            Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "BedDefinition1",
                ..
            }))
        ));
    }

    /// The bound is "more than the ceiling", so a count equal to it is not
    /// rejected by that check (it still fails later for lack of trailer bits).
    #[test]
    fn channel_count_at_the_provable_ceiling_is_not_rejected_by_the_bound() {
        // MetaID 0x00, reserved 0, Plex(4) ChannelCount = 2: 43 bits remain,
        // whose provable ceiling is 43 / 15 = 2.
        let data = [0x00u8, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00];
        match BedDefinition1::parse(&data) {
            Err(Error::InvalidValue(iv)) => panic!(
                "a count equal to the provable ceiling is not over it, but got {}",
                iv.field
            ),
            Err(_) => {}
            Ok(_) => panic!("7 bytes cannot hold two channels and the trailers"),
        }
    }

    #[test]
    fn serialized_len_matches_the_wire_for_each_channel_count() {
        for (n, want) in [(0usize, 5usize), (1, 7), (5, 15), (6, 17), (7, 18), (8, 20)] {
            let bed = BedDefinition1::new(
                1,
                alloc::vec![
                    BedChannel {
                        channel_id: ChannelId::LeftScreen,
                        audio_data_id: 1,
                    };
                    n
                ],
            );
            assert_eq!(bed.serialized_len(), want, "channels={n}");
            let mut buf = alloc::vec![0u8; want];
            assert_eq!(bed.serialize_into(&mut buf).unwrap(), want);
            assert_eq!(BedDefinition1::parse(&buf).unwrap(), bed);
        }
    }

    #[test]
    fn channel_id_labels_are_pinned() {
        for (id, bits, name, display) in [
            (
                ChannelId::LeftScreen,
                0u64,
                "left screen speaker",
                "left screen speaker",
            ),
            (ChannelId::Lfe, 0xD, "lfe speaker", "lfe speaker"),
            (ChannelId::Reserved(0xE), 0xE, "reserved", "reserved(0x0E)"),
        ] {
            assert_eq!(id.to_bits(), bits);
            assert_eq!(ChannelId::from_bits(bits), id);
            assert_eq!(id.name(), name);
            assert_eq!(id.to_string(), display);
        }
    }
}
