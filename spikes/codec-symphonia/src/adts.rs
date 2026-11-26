//! ADTS frame framing for the spike.
//!
//! Symphonia also ships an ADTS reader, but the seam's `Decoder` takes one
//! access unit per `send`, so the spike splits the file itself: a 7- or 9-byte
//! header followed by the raw AAC access unit, repeated.

use broadcast_codec::ChannelLayout;
use core::num::NonZeroU16;

/// Bytes in an ADTS header with no CRC.
const HEADER_LEN: usize = 7;
/// Bytes in an ADTS header with a 16-bit CRC.
const HEADER_LEN_CRC: usize = 9;
/// The 12-bit syncword, as stored in the first two bytes.
const SYNCWORD: u16 = 0x0fff;
/// Mask of the two low bits of byte 3 that begin `frame_length`.
const FRAME_LENGTH_HIGH_MASK: u8 = 0x03;
/// Shift that puts the two high `frame_length` bits above byte 4.
const FRAME_LENGTH_HIGH_SHIFT: usize = 11;
/// Shift of byte 4 into `frame_length`.
const FRAME_LENGTH_MIDDLE_SHIFT: usize = 3;
/// Shift that puts byte 5's high bits into `frame_length`.
const FRAME_LENGTH_LOW_SHIFT: usize = 5;
/// Mask of the four `sampling_frequency_index` bits.
const SAMPLE_RATE_INDEX_MASK: u8 = 0x0f;
/// Mask of the low `channel_configuration` bit in byte 2.
const CHANNEL_CONFIG_LOW_MASK: u8 = 0x01;
/// Mask of the two remaining `channel_configuration` bits in byte 3.
const CHANNEL_CONFIG_HIGH_MASK: u8 = 0x03;
/// Shift of the two `channel_configuration` bits from the top of byte 3.
const CHANNEL_CONFIG_HIGH_SHIFT: u8 = 6;
/// Shift that puts byte 0's syncword bits above byte 1's high nibble.
const SYNCWORD_SHIFT: usize = 4;
/// Sample rates by `sampling_frequency_index` (ISO/IEC 14496-3).
const SAMPLE_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];
/// The 5.1 channel configuration.
const CONFIG_5_1: u16 = 6;
/// The 7.1 channel configuration.
const CONFIG_7_1: u16 = 7;
/// The configuration that names a program-configuration element instead.
const CONFIG_PCE: u16 = 0;

/// One ADTS frame, header included.
#[derive(Debug, Clone, Copy)]
pub struct AdtsFrame<'a> {
    bytes: &'a [u8],
    header_len: usize,
    sample_rate: u32,
    channels: Option<ChannelLayout>,
}

impl<'a> AdtsFrame<'a> {
    /// Parse one frame from the start of `bytes`.
    #[must_use]
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let syncword = (u16::from(bytes[0]) << SYNCWORD_SHIFT) | u16::from(bytes[1] >> 4);
        if syncword != SYNCWORD {
            return None;
        }
        let protection_absent = bytes[1] & 0x01 == 0x01;
        let header_len = if protection_absent {
            HEADER_LEN
        } else {
            HEADER_LEN_CRC
        };
        let sample_rate_index = (bytes[2] >> 2) & SAMPLE_RATE_INDEX_MASK;
        let sample_rate = *SAMPLE_RATES.get(usize::from(sample_rate_index))?;
        let channel_configuration = (u16::from(bytes[2] & CHANNEL_CONFIG_LOW_MASK) << 2)
            | u16::from((bytes[3] >> CHANNEL_CONFIG_HIGH_SHIFT) & CHANNEL_CONFIG_HIGH_MASK);
        let frame_length = (usize::from(bytes[3] & FRAME_LENGTH_HIGH_MASK)
            << FRAME_LENGTH_HIGH_SHIFT)
            | (usize::from(bytes[4]) << FRAME_LENGTH_MIDDLE_SHIFT)
            | (usize::from(bytes[5]) >> FRAME_LENGTH_LOW_SHIFT);
        if frame_length < header_len || frame_length > bytes.len() {
            return None;
        }
        let channels = match channel_configuration {
            CONFIG_PCE => None,
            1 => Some(ChannelLayout::Mono),
            2 => Some(ChannelLayout::Stereo),
            CONFIG_5_1 => Some(ChannelLayout::Surround5_1),
            CONFIG_7_1 => Some(ChannelLayout::Surround7_1),
            3..=5 => NonZeroU16::new(channel_configuration).map(ChannelLayout::Discrete),
            _ => None,
        };
        Some(Self {
            bytes: &bytes[..frame_length],
            header_len,
            sample_rate,
            channels,
        })
    }

    /// The whole frame, header included.
    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The raw AAC access unit after the header.
    #[must_use]
    pub fn payload(&self) -> &'a [u8] {
        &self.bytes[self.header_len..]
    }

    /// Sample rate named by the header.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel layout named by the header, or `None` for a program-configuration
    /// element (channel configuration 0).
    #[must_use]
    pub fn channels(&self) -> Option<ChannelLayout> {
        self.channels
    }
}

/// Split an ADTS byte stream into frames.
pub struct AdtsFrames<'a> {
    data: &'a [u8],
}

impl<'a> AdtsFrames<'a> {
    /// Frames in `data`.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }
}

impl<'a> Iterator for AdtsFrames<'a> {
    type Item = AdtsFrame<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let frame = AdtsFrame::parse(self.data)?;
        self.data = &self.data[frame.bytes.len()..];
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_fixture_frames() {
        let data = test_fixtures::read("audio/aac.adts");
        let frames: Vec<_> = AdtsFrames::new(&data).collect();
        assert!(frames.len() > 40, "{}", frames.len());
        assert_eq!(frames[0].sample_rate(), 48_000);
        assert!(matches!(frames[0].channels(), Some(ChannelLayout::Mono)));
        assert!(frames[0].bytes().len() > HEADER_LEN);
        assert_eq!(
            frames[0].payload().len(),
            frames[0].bytes().len() - HEADER_LEN
        );
        let total: usize = frames.iter().map(|frame| frame.bytes().len()).sum();
        assert_eq!(total, data.len());
    }
}
