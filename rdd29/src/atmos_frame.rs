//! `ATMOSFrame` element — RDD 29:2019 §2.1/§4.2/§5.2.
//!
//! The top-level container: the entire audio frame is a single `ATMOSFrame`
//! element, whose sub-elements ([`crate::AnyElement`]) carry all of the
//! frame's bed/object metadata and audio essence.

use alloc::vec::Vec;

use broadcast_common::bits::{BitReader, BitWriter};
use broadcast_common::{Parse, Serialize};

use crate::AnyElement;
use crate::element::{ELEMENT_ID_ATMOS_FRAME, element_header_len, write_element_header};
use crate::error::{BitResultExt, Error, Result};
use crate::frame_rate::FrameRate;
use crate::plex::{plex_bits, read_plex, write_plex};

/// The 2-bit `SampleRate` field (§5.2.2 Table 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SampleRate {
    /// `0x0` — 48000 samples per second.
    Hz48000,
    /// `0x1` — 96000 samples per second.
    Hz96000,
    /// `0x2`/`0x3` — reserved.
    Reserved(u8),
}

impl SampleRate {
    /// The spec token for this value ("reserved" for the reserved arm).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hz48000 => "48000 Hz",
            Self::Hz96000 => "96000 Hz",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u64) -> Self {
        match bits {
            0x0 => Self::Hz48000,
            0x1 => Self::Hz96000,
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u64 {
        match self {
            Self::Hz48000 => 0x0,
            Self::Hz96000 => 0x1,
            Self::Reserved(v) => u64::from(v),
        }
    }
}

broadcast_common::impl_spec_display!(SampleRate, Reserved);

/// The 2-bit `BitDepth` field (§5.2.3 Table 3). "Only 24-bits per audio
/// sample are currently supported" (spec's own emphasis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BitDepth {
    /// `0x1` — 24 bits per audio sample.
    Bits24,
    /// `0x0`, `0x2`, `0x3` — reserved.
    Reserved(u8),
}

impl BitDepth {
    /// The spec token for this value ("reserved" for the reserved arm).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bits24 => "24 bits",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u64) -> Self {
        match bits {
            0x1 => Self::Bits24,
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u64 {
        match self {
            Self::Bits24 => 0x1,
            Self::Reserved(v) => u64::from(v),
        }
    }
}

broadcast_common::impl_spec_display!(BitDepth, Reserved);

/// The `ATMOSFrame` element (§2.1/§4.2/§5.2): the entire Dolby Atmos frame.
#[derive(Debug, Clone, PartialEq)]
pub struct AtmosFrame<'a> {
    /// `ATMOSVersion` — this crate implements version `1` (§5.2.1).
    pub version: u8,
    /// `SampleRate` (§5.2.2 Table 2).
    pub sample_rate: SampleRate,
    /// `BitDepth` (§5.2.3 Table 3).
    pub bit_depth: BitDepth,
    /// `FrameRate` (§5.2.4 Table 4). Also determines `NumPanSubBlocks` for
    /// any [`crate::ObjectDefinition1`] sub-elements (§5.4.1 Table 7).
    pub frame_rate: FrameRate,
    /// `MaxRendered` — maximum audio assets rendered during playback
    /// (§5.2.5).
    pub max_rendered: u32,
    /// The frame's sub-elements, in wire order (`SubElementCount`, §5.2.6,
    /// is `elements.len()`, not stored separately).
    pub elements: Vec<AnyElement<'a>>,
}

/// `ATMOSVersion` this crate implements ("This document describes the
/// protocol with ATMOSVersion = 1", §5.2.1).
pub const ATMOS_VERSION: u8 = 1;

impl<'a> AtmosFrame<'a> {
    /// Build a new `ATMOSFrame`.
    #[must_use]
    pub fn new(
        sample_rate: SampleRate,
        bit_depth: BitDepth,
        frame_rate: FrameRate,
        max_rendered: u32,
        elements: Vec<AnyElement<'a>>,
    ) -> Self {
        Self {
            version: ATMOS_VERSION,
            sample_rate,
            bit_depth,
            frame_rate,
            max_rendered,
            elements,
        }
    }

    fn body_len(&self) -> usize {
        // ATMOSVersion(8) + SampleRate(2) + BitDepth(2) + FrameRate(4) = 16
        // bits, already byte-aligned -- ByteAlign() is a genuine no-op here.
        let mut bits: u32 = 8 + 2 + 2 + 4;
        bits += plex_bits(u64::from(self.max_rendered), 8);
        bits += plex_bits(self.elements.len() as u64, 8);
        let mut bytes = (bits as usize) / 8;
        for e in &self.elements {
            bytes += e.serialized_len();
        }
        bytes
    }
}

impl<'a> Parse<'a> for AtmosFrame<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut r = BitReader::new(bytes);
        let element_id = read_plex(&mut r, 8, "ElementID")? as u32;
        if element_id != ELEMENT_ID_ATMOS_FRAME {
            return Err(Error::UnexpectedElementId {
                expected: ELEMENT_ID_ATMOS_FRAME,
                found: element_id,
            });
        }
        let element_size = read_plex(&mut r, 8, "ElementSize")?;
        debug_assert!(r.is_byte_aligned());
        let header_len = r.bits_read() / 8;
        let element_size = usize::try_from(element_size).map_err(|_| Error::InvalidValue {
            field: "ElementSize",
            value: element_size,
            reason: "does not fit in this platform's usize",
        })?;
        let body_end = header_len
            .checked_add(element_size)
            .ok_or(Error::InvalidValue {
                field: "ElementSize",
                value: element_size as u64,
                reason: "overflowed usize",
            })?;
        if body_end > bytes.len() {
            return Err(Error::BufferTooShort {
                need: body_end,
                have: bytes.len(),
                what: "ATMOSFrame body",
            });
        }
        let body = &bytes[header_len..body_end];

        let mut br = BitReader::new(body);
        let version = br.read_bits(8).ctx("ATMOSFrame.ATMOSVersion")? as u8;
        let sample_rate = SampleRate::from_bits(br.read_bits(2).ctx("ATMOSFrame.SampleRate")?);
        let bit_depth = BitDepth::from_bits(br.read_bits(2).ctx("ATMOSFrame.BitDepth")?);
        let frame_rate = FrameRate::from_bits(br.read_bits(4).ctx("ATMOSFrame.FrameRate")?);
        let max_rendered = read_plex(&mut br, 8, "ATMOSFrame.MaxRendered")? as u32;
        br.align_to_byte(); // ByteAlign() -- a no-op given the fixed 16-bit
        // header above, kept for parser symmetry with
        // the other elements' genuine AlignBits.
        let sub_element_count = read_plex(&mut br, 8, "ATMOSFrame.SubElementCount")?;
        debug_assert!(br.is_byte_aligned());

        let mut offset = br.bits_read() / 8;
        let mut elements = Vec::with_capacity(sub_element_count as usize);
        for _ in 0..sub_element_count {
            let (element, consumed) =
                AnyElement::parse_with_frame_rate(&body[offset..], Some(frame_rate))?;
            offset += consumed;
            elements.push(element);
        }
        if offset != body.len() {
            return Err(Error::InvalidValue {
                field: "ATMOSFrame",
                value: (body.len() - offset) as u64,
                reason: "trailing bytes remain after SubElementCount elements",
            });
        }

        Ok(Self {
            version,
            sample_rate,
            bit_depth,
            frame_rate,
            max_rendered,
            elements,
        })
    }
}

impl Serialize for AtmosFrame<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let body_len = self.body_len();
        element_header_len(ELEMENT_ID_ATMOS_FRAME, body_len) + body_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        let need = element_header_len(ELEMENT_ID_ATMOS_FRAME, body_len) + body_len;
        if buf.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: buf.len(),
                what: "ATMOSFrame",
            });
        }
        write_element_header(&mut buf[..need], ELEMENT_ID_ATMOS_FRAME, body_len)?;
        let header_len = need - body_len;

        let fixed_part_bytes;
        {
            let mut w = BitWriter::new(&mut buf[header_len..need]);
            w.write_bits(u64::from(self.version), 8)
                .ctx("ATMOSFrame.ATMOSVersion")?;
            w.write_bits(self.sample_rate.to_bits(), 2)
                .ctx("ATMOSFrame.SampleRate")?;
            w.write_bits(self.bit_depth.to_bits(), 2)
                .ctx("ATMOSFrame.BitDepth")?;
            w.write_bits(self.frame_rate.to_bits(), 4)
                .ctx("ATMOSFrame.FrameRate")?;
            write_plex(
                &mut w,
                u64::from(self.max_rendered),
                8,
                "ATMOSFrame.MaxRendered",
            )?;
            w.align_to_byte().ctx("ATMOSFrame.ByteAlign")?;
            write_plex(
                &mut w,
                self.elements.len() as u64,
                8,
                "ATMOSFrame.SubElementCount",
            )?;
            debug_assert!(w.is_byte_aligned());
            fixed_part_bytes = w.bits_written() / 8;
        }

        let mut offset = header_len + fixed_part_bytes;
        for e in &self.elements {
            let n = e.serialize_into(&mut buf[offset..need])?;
            offset += n;
        }
        debug_assert_eq!(offset, need);
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AudioDataDlc, AudioDescription, BedChannel, BedDefinition1, ChannelId, PanSubBlock,
    };

    fn sample_frame() -> AtmosFrame<'static> {
        let bed = BedDefinition1::new(
            1,
            alloc::vec![BedChannel {
                channel_id: ChannelId::LeftScreen,
                audio_data_id: 10,
            }],
        );
        let mut pan_sub_blocks = alloc::vec![PanSubBlock {
            pan: Some(crate::PanInfo {
                pos_x: 0x8000,
                pos_y: 0x8000,
                pos_z: 0x8000,
                snap: false,
                zone_gains: None,
                spread_mode: crate::ObjectSpreadMode::Lowrez,
                spread: 0,
                decor_coef_prefix: crate::DecorCoefPrefix::NoDecorrelation,
                decor_coef: None,
            }),
        }];
        // FrameRate::Fps24 => 8 pan sub-blocks (Table 7); the remaining 7
        // repeat sub-block 0's pan info (PanInfoExists == 0).
        pan_sub_blocks.resize(8, PanSubBlock { pan: None });
        let obj =
            crate::ObjectDefinition1::new(2, 11, pan_sub_blocks, AudioDescription::none()).unwrap();
        let dlc = AudioDataDlc::new(10, &[0x11, 0x22, 0x33]).unwrap();
        let dlc2 = AudioDataDlc::new(11, &[0x44, 0x55]).unwrap();

        AtmosFrame::new(
            SampleRate::Hz48000,
            BitDepth::Bits24,
            FrameRate::Fps24, // 8 pan sub-blocks, matches obj above
            128,
            alloc::vec![
                AnyElement::BedDefinition1(bed),
                AnyElement::ObjectDefinition1(obj),
                AnyElement::AudioDataDlc(dlc),
                AnyElement::AudioDataDlc(dlc2),
            ],
        )
    }

    #[test]
    fn round_trips() {
        let frame = sample_frame();
        let bytes = frame.to_bytes();
        let parsed = AtmosFrame::parse(&bytes).unwrap();
        assert_eq!(parsed, frame);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn empty_frame_round_trips() {
        let frame = AtmosFrame::new(
            SampleRate::Hz48000,
            BitDepth::Bits24,
            FrameRate::Fps24,
            0,
            Vec::new(),
        );
        let bytes = frame.to_bytes();
        let parsed = AtmosFrame::parse(&bytes).unwrap();
        assert_eq!(parsed, frame);
    }

    #[test]
    fn wrong_top_level_element_id_is_rejected() {
        // A BedDefinition1's own header, standing in for a stream that
        // doesn't start with an ATMOS_FRAME element.
        let bed = BedDefinition1::new(0, Vec::new());
        let element = AnyElement::BedDefinition1(bed);
        let mut bytes = alloc::vec![0u8; element.serialized_len()];
        element.serialize_into(&mut bytes).unwrap();

        let err = AtmosFrame::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::UnexpectedElementId { .. }));
    }

    #[test]
    fn mutating_max_rendered_changes_only_that_field_and_after() {
        let mut frame = sample_frame();
        let original = frame.to_bytes();
        frame.max_rendered = 1; // still Plex(8)-direct, same width as 128
        let mutated = frame.to_bytes();
        assert_ne!(original, mutated);
        // The fixed 16-bit prefix (ATMOSVersion/SampleRate/BitDepth/FrameRate)
        // is unaffected by changing MaxRendered.
        assert_eq!(&original[..3], &mutated[..3]); // ElementID+ElementSize+ATMOSVersion byte-ish prefix
    }
}
