//! MPEG-1/2/2.5 audio (MP1/2/3) frame headers — ISO/IEC 11172-3 §2.4.1.3 and
//! ISO/IEC 13818-3 §2.4.2.3 (`header`).
//!
//! The frame header carries an 11-bit `syncword` (all ones), the MPEG version
//! (`ID`, plus the MPEG-2.5 low-sampling-rate extension), `layer`,
//! `bitrate_index`, `sampling_frequency`, `padding_bit` and `mode`. Only those
//! fields are decoded: they describe the stream and give each frame its length,
//! so a caller can walk frame boundaries; the frame payload stays opaque.
//!
//! # Spec citations
//!
//! - ISO/IEC 11172-3 §2.4.1.3 / §2.4.2.3 (`header`, bitrate and
//!   sampling-frequency tables), §2.4.3.1 (Layer I frame length).
//! - ISO/IEC 13818-3 §2.4.2.3 for the MPEG-2 low-sampling-frequency extension;
//!   the MPEG-2.5 rates are the unofficial quarter-rate extension of the same
//!   table.

use crate::error::{Error, Result};
use broadcast_core::len::need_in;
use broadcast_core::wire::be_u32;

/// Bits in one kilobit: converts a stated kbit/s bitrate to bits per second.
const BITS_PER_KILOBIT: u32 = 1000;

/// The 11-bit `syncword` (all ones) that opens an MPEG audio frame header
/// (ISO/IEC 11172-3 §2.4.2.3).
pub const MPEG_AUDIO_SYNCWORD: u16 = 0x07FF;

/// Audio layer of an MPEG-1/2 audio frame — the `layer` field of the frame
/// header (ISO/IEC 11172-3 §2.4.2.3, coded `11`=I, `10`=II, `01`=III).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MpegAudioLayer {
    /// Layer I (MP1).
    LayerI,
    /// Layer II (MP2).
    LayerII,
    /// Layer III (MP3).
    LayerIII,
}

impl MpegAudioLayer {
    /// The layer number (1, 2, or 3).
    pub fn number(self) -> u8 {
        match self {
            MpegAudioLayer::LayerI => 1,
            MpegAudioLayer::LayerII => 2,
            MpegAudioLayer::LayerIII => 3,
        }
    }

    /// Spec label for the layer.
    pub fn name(&self) -> &'static str {
        match self {
            MpegAudioLayer::LayerI => "Layer I",
            MpegAudioLayer::LayerII => "Layer II",
            MpegAudioLayer::LayerIII => "Layer III",
        }
    }
}

broadcast_core::impl_spec_display!(MpegAudioLayer);

/// MPEG audio version: the `ID` bits of the frame header, with the MPEG-2.5
/// low-sampling-rate extension of ISO/IEC 13818-3 §2.4.2.3 (`ID = 00`).
///
/// It selects the sampling-rate row and the bitrate / samples-per-frame
/// tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MpegAudioVersion {
    /// MPEG-1 (`ID = 1`), ISO/IEC 11172-3.
    Mpeg1,
    /// MPEG-2 low sampling frequency (`ID = 0`), ISO/IEC 13818-3.
    Mpeg2,
    /// MPEG-2.5, the unofficial quarter-rate extension (`ID = 00`).
    Mpeg25,
}

impl MpegAudioVersion {
    /// Spec label for the version.
    pub fn name(&self) -> &'static str {
        match self {
            MpegAudioVersion::Mpeg1 => "MPEG-1",
            MpegAudioVersion::Mpeg2 => "MPEG-2",
            MpegAudioVersion::Mpeg25 => "MPEG-2.5",
        }
    }
}

broadcast_core::impl_spec_display!(MpegAudioVersion);

/// MPEG audio frame header (ISO/IEC 11172-3 §2.4.1.3 / ISO/IEC 13818-3).
///
/// Only the fields needed to describe the stream and walk frame boundaries are
/// decoded; the audio payload is carried opaquely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MpegAudioFrameHeader {
    layer: MpegAudioLayer,
    version: MpegAudioVersion,
    sample_rate: u32,
    channels: u16,
    bitrate_bps: u32,
    frame_length: usize,
    samples_per_frame: u32,
}

impl MpegAudioFrameHeader {
    /// Audio layer (I / II / III).
    pub fn layer(&self) -> MpegAudioLayer {
        self.layer
    }

    /// MPEG version (1, 2, 2.5).
    pub fn version(&self) -> MpegAudioVersion {
        self.version
    }

    /// Sampling frequency in Hz (§2.4.2.3 `sampling_frequency` table).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel count: 1 for `single_channel` mode, else 2.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Bitrate in bits per second from the header's `bitrate_index`
    /// (ISO/IEC 11172-3 §2.4.2.3 bitrate table). Free-format and forbidden
    /// indices are rejected at parse, so this is always non-zero.
    pub fn bitrate_bps(&self) -> u32 {
        self.bitrate_bps
    }

    /// Total frame length in bytes (header + payload), for frame splitting.
    pub fn frame_length(&self) -> usize {
        self.frame_length
    }

    /// Decoded samples per frame (384 for Layer I; 1152 for Layer II; 1152 for
    /// MPEG-1 Layer III, 576 for MPEG-2/2.5 Layer III).
    pub fn samples_per_frame(&self) -> u32 {
        self.samples_per_frame
    }

    /// Parse the 4-byte MPEG audio frame header at the start of `data`.
    ///
    /// Validates the 11-bit syncword and rejects reserved layer / free /
    /// forbidden bitrate / reserved sampling-frequency values with
    /// [`Error::InvalidInput`] (so a false sync in the middle of a payload does
    /// not masquerade as a frame boundary).
    pub fn parse(data: &[u8]) -> Result<Self> {
        need_in(data, FRAME_HEADER_LEN, "MPEG audio frame header")?;
        let h = be_u32(data, 0, "MPEG audio frame header")?;

        let syncword = ((h >> SYNCWORD_SHIFT) & u32::from(MPEG_AUDIO_SYNCWORD)) as u16;
        if syncword != MPEG_AUDIO_SYNCWORD {
            return Err(Error::InvalidInput(
                "MPEG audio frame header: bad syncword (expected 11-bit 0x7FF)",
            ));
        }
        // ID is 2 bits in MPEG-2.5: `00`=2.5, `10`=2, `11`=1 (`01` reserved).
        let id = ((h >> VERSION_ID_SHIFT) & BIT_PAIR_MASK) as u8;
        let version = match id {
            VERSION_ID_MPEG1 => MpegAudioVersion::Mpeg1,
            VERSION_ID_MPEG2 => MpegAudioVersion::Mpeg2,
            VERSION_ID_MPEG25 => MpegAudioVersion::Mpeg25,
            _ => {
                return Err(Error::InvalidInput(
                    "MPEG audio frame header: reserved version ID",
                ));
            }
        };
        let layer = match ((h >> LAYER_SHIFT) & BIT_PAIR_MASK) as u8 {
            LAYER_I => MpegAudioLayer::LayerI,
            LAYER_II => MpegAudioLayer::LayerII,
            LAYER_III => MpegAudioLayer::LayerIII,
            _ => {
                return Err(Error::InvalidInput(
                    "MPEG audio frame header: reserved layer",
                ));
            }
        };
        let bitrate_index = ((h >> BITRATE_INDEX_SHIFT) & BITRATE_INDEX_MASK) as usize;
        let sample_rate_index = ((h >> SAMPLE_RATE_INDEX_SHIFT) & BIT_PAIR_MASK) as usize;
        let padding = ((h >> PADDING_SHIFT) & PADDING_MASK) as usize;
        let mode = ((h >> MODE_SHIFT) & BIT_PAIR_MASK) as u8;

        if sample_rate_index >= SAMPLE_RATE_MPEG1.len() {
            return Err(Error::InvalidInput(
                "MPEG audio frame header: reserved sampling_frequency",
            ));
        }
        let base_rate = SAMPLE_RATE_MPEG1[sample_rate_index];
        let sample_rate = match version {
            MpegAudioVersion::Mpeg1 => base_rate,
            MpegAudioVersion::Mpeg2 => base_rate / MPEG2_RATE_DIVISOR,
            MpegAudioVersion::Mpeg25 => base_rate / MPEG25_RATE_DIVISOR,
        };

        let version_group = matches!(version, MpegAudioVersion::Mpeg1) as usize ^ 1; // 0=MPEG1,1=LSF
        let layer_group = match layer {
            MpegAudioLayer::LayerI => 0,
            MpegAudioLayer::LayerII => 1,
            MpegAudioLayer::LayerIII => 2,
        };
        let bitrate_kbps = BITRATE_KBPS[version_group][layer_group][bitrate_index];
        if bitrate_kbps == 0 {
            // Index 0 (free-format) and 15 (forbidden) — not supported.
            return Err(Error::InvalidInput(
                "MPEG audio frame header: free-format or forbidden bitrate_index",
            ));
        }
        let bitrate = bitrate_kbps as usize * BITS_PER_KILOBIT as usize;

        let channels: u16 = if mode == MODE_SINGLE_CHANNEL {
            1
        } else {
            STEREO_CHANNELS
        };

        // Frame length (bytes) and samples per frame — §2.4.3.1 (Layer I) /
        // §2.4.2.3.  Layer I: (12 * br / sr + pad) * 4; Layers II/III:
        // (coeff * br / sr) + pad, coeff = samples_per_frame / 8.
        let samples_per_frame = match (layer, version) {
            (MpegAudioLayer::LayerI, _) => 384,
            (MpegAudioLayer::LayerII, _) => 1152,
            (MpegAudioLayer::LayerIII, MpegAudioVersion::Mpeg1) => 1152,
            (MpegAudioLayer::LayerIII, _) => 576,
        };
        let frame_length = match layer {
            MpegAudioLayer::LayerI => {
                (LAYER1_SLOTS_PER_FRAME * bitrate / sample_rate as usize + padding)
                    * LAYER1_SLOT_BYTES
            }
            _ => {
                (samples_per_frame as usize / u8::BITS as usize) * bitrate / sample_rate as usize
                    + padding
            }
        };
        if frame_length < FRAME_HEADER_LEN {
            return Err(Error::InvalidInput(
                "MPEG audio frame header: computed frame length shorter than header",
            ));
        }

        Ok(Self {
            layer,
            version,
            sample_rate,
            channels,
            bitrate_bps: bitrate as u32,
            frame_length,
            samples_per_frame,
        })
    }
}

/// Bitrate index → kbit/s, per version+layer (ISO/IEC 11172-3 Table; 0 =
/// "free", 15 = "forbidden" → both unsupported here). Index 0 and 15 map to 0.
///
/// Columns: `[version_group][layer_group]` where `version_group` is 0 for
/// MPEG-1, 1 for MPEG-2/2.5 (LSF), and `layer_group` is 0=I, 1=II, 2=III.
#[rustfmt::skip]
const BITRATE_KBPS: [[[u16; 16]; 3]; 2] = [
    // MPEG-1
    [
        // Layer I
        [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0],
        // Layer II
        [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0],
        // Layer III
        [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0],
    ],
    // MPEG-2 / 2.5 (LSF)
    [
        // Layer I
        [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0],
        // Layer II & III share the LSF table
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],
    ],
];

/// Base MPEG-1 sampling frequencies (Hz) indexed by `sampling_frequency`
/// (§2.4.2.3). MPEG-2 halves these; MPEG-2.5 quarters them.
const SAMPLE_RATE_MPEG1: [u32; 3] = [44100, 48000, 32000];

/// Layer coded values (`11`=I, `10`=II, `01`=III, `00`=reserved).
const LAYER_III: u8 = 0b01;
const LAYER_II: u8 = 0b10;
const LAYER_I: u8 = 0b11;

/// `ID` coded values (`11`=MPEG-1, `10`=MPEG-2, `00`=MPEG-2.5, `01`=reserved).
const VERSION_ID_MPEG1: u8 = 0b11;
const VERSION_ID_MPEG2: u8 = 0b10;
const VERSION_ID_MPEG25: u8 = 0b00;

/// Channel `mode` value for `single_channel` (mono) — §2.4.2.3.
const MODE_SINGLE_CHANNEL: u8 = 0b11;

/// Bytes of the frame header fixed at the start of every frame (§2.4.1.3).
const FRAME_HEADER_LEN: usize = 4;

/// Bit position of `syncword` in the 32-bit frame header.
const SYNCWORD_SHIFT: u32 = 21;
/// Bit position of the 2-bit `ID` (MPEG version) field.
const VERSION_ID_SHIFT: u32 = 19;
/// Bit position of the 2-bit `layer` field.
const LAYER_SHIFT: u32 = 17;
/// Bit position of the 4-bit `bitrate_index` field.
const BITRATE_INDEX_SHIFT: u32 = 12;
/// Bit position of the 2-bit `sampling_frequency` field.
const SAMPLE_RATE_INDEX_SHIFT: u32 = 10;
/// Bit position of the 1-bit `padding_bit` field.
const PADDING_SHIFT: u32 = 9;
/// Bit position of the 2-bit `mode` field.
const MODE_SHIFT: u32 = 6;
/// Mask for a 2-bit field (`ID`, `layer`, `mode`).
const BIT_PAIR_MASK: u32 = 0b11;
/// Mask for the 4-bit `bitrate_index`.
const BITRATE_INDEX_MASK: u32 = 0xF;
/// Mask for the 1-bit `padding_bit`.
const PADDING_MASK: u32 = 0x1;

/// MPEG-2 sampling frequencies are half the MPEG-1 base rates (§2.4.2.3).
const MPEG2_RATE_DIVISOR: u32 = 2;
/// MPEG-2.5 sampling frequencies are a quarter of them.
const MPEG25_RATE_DIVISOR: u32 = 4;
/// Channels in a non-`single_channel` frame (stereo, or the two-channel
/// joint/dual modes of §2.4.2.3).
const STEREO_CHANNELS: u16 = 2;

/// Slot count term of the Layer I frame length: `12 * bitrate / sample_rate`
/// (§2.4.3.1).
const LAYER1_SLOTS_PER_FRAME: usize = 12;
/// Bytes per Layer I slot.
const LAYER1_SLOT_BYTES: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn mp2_frame_header_44100_mono() {
        // FF FD E0 C4 — MPEG-1 Layer II, 44100 Hz, mono, 384 kbps.
        let h = MpegAudioFrameHeader::parse(&[0xFF, 0xFD, 0xE0, 0xC4]).unwrap();
        assert_eq!(h.layer(), MpegAudioLayer::LayerII);
        assert_eq!(h.layer().number(), 2);
        assert_eq!(h.version(), MpegAudioVersion::Mpeg1);
        assert_eq!(h.sample_rate(), 44100);
        assert_eq!(h.channels(), 1);
        assert_eq!(h.samples_per_frame(), 1152);
        assert_eq!(h.frame_length(), 1253);
    }

    #[test]
    fn mp3_frame_header_44100() {
        // FF FB 50 C4 — MPEG-1 Layer III, 44100 Hz.
        let h = MpegAudioFrameHeader::parse(&[0xFF, 0xFB, 0x50, 0xC4]).unwrap();
        assert_eq!(h.layer(), MpegAudioLayer::LayerIII);
        assert_eq!(h.version(), MpegAudioVersion::Mpeg1);
        assert_eq!(h.sample_rate(), 44100);
        assert_eq!(h.samples_per_frame(), 1152);
    }

    #[test]
    fn bad_syncword_rejected() {
        assert!(matches!(
            MpegAudioFrameHeader::parse(&[0x00, 0x00, 0x00, 0x00]),
            Err(crate::error::Error::InvalidInput(..))
        ));
    }

    #[test]
    fn display_labels() {
        assert_eq!(MpegAudioLayer::LayerII.to_string(), "Layer II");
        assert_eq!(MpegAudioVersion::Mpeg1.to_string(), "MPEG-1");
        assert_eq!(MpegAudioVersion::Mpeg2.to_string(), "MPEG-2");
        assert_eq!(MpegAudioVersion::Mpeg25.to_string(), "MPEG-2.5");
    }

    /// #3798: `ID = 00` is MPEG-2.5, not a reserved version.
    #[test]
    fn mpeg25_layer3_frame_header() {
        let h = MpegAudioFrameHeader::parse(&[0xFF, 0xE3, 0x90, 0xC4]).unwrap();
        assert_eq!(h.layer(), MpegAudioLayer::LayerIII);
        assert_eq!(h.version(), MpegAudioVersion::Mpeg25);
        assert_eq!(h.sample_rate(), 11025, "MPEG-2.5 quarters the base rate");
        assert_eq!(h.samples_per_frame(), 576);
        assert_eq!(h.channels(), 1);
    }

    /// #3798: Layer I has its own frame-length granule, `(12 * br / sr + pad) * 4`.
    #[test]
    fn layer1_frame_length_uses_the_4_byte_granule() {
        let h = MpegAudioFrameHeader::parse(&[0xFF, 0xFF, 0x10, 0xC4]).unwrap();
        assert_eq!(h.layer(), MpegAudioLayer::LayerI);
        assert_eq!(h.bitrate_bps(), 32_000);
        assert_eq!(h.sample_rate(), 44100);
        assert_eq!(h.samples_per_frame(), 384);
        assert_eq!(h.frame_length(), 32, "(12 * 32000 / 44100 + 0) * 4");

        let padded = MpegAudioFrameHeader::parse(&[0xFF, 0xFF, 0x12, 0xC4]).unwrap();
        assert_eq!(padded.frame_length(), 36, "one pad slot adds 4 bytes");
    }

    /// The 2-byte `ID` value `01` is reserved (§2.4.2.3), never a version.
    #[test]
    fn reserved_version_id_is_refused() {
        // FF ED: MPEG version bits `01`.
        assert!(matches!(
            MpegAudioFrameHeader::parse(&[0xFF, 0xED, 0x50, 0xC4]),
            Err(crate::error::Error::InvalidInput(..))
        ));
    }

    /// A header shorter than the fixed 4 bytes is `BufferTooShort`, never a
    /// slice panic.
    #[test]
    fn short_header_reports_the_need() {
        use broadcast_core::error::BufferTooShort;
        match MpegAudioFrameHeader::parse(&[0xFF, 0xFB]) {
            Err(crate::error::Error::BufferTooShort(BufferTooShort { need, have, .. })) => {
                assert_eq!((need, have), (FRAME_HEADER_LEN, 2));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// Free-format (`bitrate_index` 0) and forbidden (15) bitrates are refused.
    #[test]
    fn free_and_forbidden_bitrates_are_refused() {
        assert!(MpegAudioFrameHeader::parse(&[0xFF, 0xFB, 0x00, 0xC4]).is_err());
        assert!(MpegAudioFrameHeader::parse(&[0xFF, 0xFB, 0xF0, 0xC4]).is_err());
    }
}
