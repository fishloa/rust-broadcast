//! `FrameRate` — RDD 29:2019 §5.2.4 Table 4, plus the `NumPanSubBlocks`
//! derivation of §5.4.1 Table 7.

use crate::error::{Error, Result};

/// The 4-bit `FrameRate` field (§5.2.4 Table 4): the Dolby Atmos frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FrameRate {
    /// `0x0` — 24 frames per second.
    Fps24,
    /// `0x1` — 25 frames per second.
    Fps25,
    /// `0x2` — 30 frames per second.
    Fps30,
    /// `0x3` — 48 frames per second.
    Fps48,
    /// `0x4` — 50 frames per second.
    Fps50,
    /// `0x5` — 60 frames per second.
    Fps60,
    /// `0x6` — 96 frames per second.
    Fps96,
    /// `0x7` — 100 frames per second.
    Fps100,
    /// `0x8` — 120 frames per second.
    Fps120,
    /// `0x9`-`0xF` — reserved.
    Reserved(u8),
}

impl FrameRate {
    /// The spec token for this value ("reserved" for the reserved arm) —
    /// see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Fps24 => "24 fps",
            Self::Fps25 => "25 fps",
            Self::Fps30 => "30 fps",
            Self::Fps48 => "48 fps",
            Self::Fps50 => "50 fps",
            Self::Fps60 => "60 fps",
            Self::Fps96 => "96 fps",
            Self::Fps100 => "100 fps",
            Self::Fps120 => "120 fps",
            Self::Reserved(_) => "reserved",
        }
    }

    pub(crate) fn from_bits(bits: u64) -> Self {
        match bits {
            0x0 => Self::Fps24,
            0x1 => Self::Fps25,
            0x2 => Self::Fps30,
            0x3 => Self::Fps48,
            0x4 => Self::Fps50,
            0x5 => Self::Fps60,
            0x6 => Self::Fps96,
            0x7 => Self::Fps100,
            0x8 => Self::Fps120,
            other => Self::Reserved(other as u8),
        }
    }

    pub(crate) fn to_bits(self) -> u64 {
        match self {
            Self::Fps24 => 0x0,
            Self::Fps25 => 0x1,
            Self::Fps30 => 0x2,
            Self::Fps48 => 0x3,
            Self::Fps50 => 0x4,
            Self::Fps60 => 0x5,
            Self::Fps96 => 0x6,
            Self::Fps100 => 0x7,
            Self::Fps120 => 0x8,
            Self::Reserved(v) => u64::from(v),
        }
    }

    /// `NumPanSubBlocks` (§5.4.1 Table 7) — the number of ~5ms pan sub-blocks
    /// an `ObjectDefinition1` element's pan-info loop carries per frame at
    /// this frame rate. Table 7 gives the same count for both sample rates
    /// at a given frame rate (see `docs/rdd29.md` scope decision 5), so this
    /// depends only on `FrameRate`.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] if `self` is [`FrameRate::Reserved`] — Table 7
    /// has no row for a reserved frame rate, so `NumPanSubBlocks` cannot be
    /// determined.
    pub fn num_pan_sub_blocks(self) -> Result<u8> {
        match self {
            Self::Fps24 | Self::Fps25 | Self::Fps30 => Ok(8),
            Self::Fps48 | Self::Fps50 | Self::Fps60 => Ok(4),
            Self::Fps96 | Self::Fps100 | Self::Fps120 => Ok(2),
            Self::Reserved(v) => Err(Error::InvalidValue {
                field: "FrameRate",
                value: u64::from(v),
                reason: "reserved FrameRate has no Table 7 NumPanSubBlocks entry",
            }),
        }
    }
}

broadcast_common::impl_spec_display!(FrameRate, Reserved);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_pan_sub_blocks_matches_table_7() {
        for (fr, expected) in [
            (FrameRate::Fps24, 8),
            (FrameRate::Fps25, 8),
            (FrameRate::Fps30, 8),
            (FrameRate::Fps48, 4),
            (FrameRate::Fps50, 4),
            (FrameRate::Fps60, 4),
            (FrameRate::Fps96, 2),
            (FrameRate::Fps100, 2),
            (FrameRate::Fps120, 2),
        ] {
            assert_eq!(fr.num_pan_sub_blocks().unwrap(), expected);
        }
    }

    #[test]
    fn reserved_frame_rate_has_no_pan_sub_block_count() {
        assert!(FrameRate::Reserved(0x9).num_pan_sub_blocks().is_err());
    }

    #[test]
    fn bits_round_trip() {
        for bits in 0u64..=0xF {
            let fr = FrameRate::from_bits(bits);
            assert_eq!(fr.to_bits(), bits);
        }
    }
}
