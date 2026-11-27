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

    /// Offset of the `Xing`/`Info` magic from the start of the frame, when
    /// this header's version and channel mode put it there: after the 4-byte
    /// header and the side-information block (32 bytes for MPEG-1
    /// stereo/joint/dual, 17 for MPEG-1 mono and MPEG-2/2.5 non-mono, 9 for
    /// MPEG-2/2.5 mono).
    #[must_use]
    pub fn xing_offset(&self) -> usize {
        let mpeg1 = matches!(self.version, MpegAudioVersion::Mpeg1);
        let mono = self.channels == 1;
        let side_info = match (mpeg1, mono) {
            (true, false) => MPEG1_STEREO_SIDE_INFO,
            (true, true) => MPEG1_MONO_SIDE_INFO,
            (false, false) => LSF_STEREO_SIDE_INFO,
            (false, true) => LSF_MONO_SIDE_INFO,
        };
        FRAME_HEADER_LEN + side_info
    }
}

/// The LAME/Xing `Info` tag carried in the first MPEG Layer III frame.
///
/// This is a **de-facto format**, not a standard: the layout is the
/// `Mp3 Info Tag revision 1` specification by Gabriel Bouvigne
/// (<http://gabriel.mp3-tech.org/mp3infotag.html>), corroborated by LAME's own
/// reader/writer `libmp3lame/VbrTag.c`
/// (<https://github.com/lameproject/lame>, LGPL-2.0-or-later) and by FFmpeg's
/// reader `mp3_parse_info_tag`
/// (<https://github.com/FFmpeg/FFmpeg>, LGPL-2.1-or-later). The offsets are
/// relative to the `Xing`/`Info` magic, so they are independent of the version
/// and channel mode that locate that magic.
///
/// Only the gapless facts are typed: the 9-byte encoder version string and,
/// when a LAME-family encoder wrote it, the 12-bit encoder delay and 12-bit
/// encoder padding. The rest of the tag stays opaque to this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LameTag {
    encoder: [u8; LAME_VERSION_LEN],
    encoder_delay: Option<u32>,
    encoder_padding: Option<u32>,
}

impl LameTag {
    /// The 9-byte encoder version string (`LAME3.100`, `Lavc62.28`, …).
    #[must_use]
    pub const fn encoder(&self) -> &[u8; LAME_VERSION_LEN] {
        &self.encoder
    }

    /// Encoder delay in samples, when a LAME-family encoder wrote the tag.
    #[must_use]
    pub const fn encoder_delay(&self) -> Option<u32> {
        self.encoder_delay
    }

    /// Encoder padding in samples, when a LAME-family encoder wrote the tag.
    #[must_use]
    pub const fn encoder_padding(&self) -> Option<u32> {
        self.encoder_padding
    }
}

/// Parse the LAME/Xing `Info` tag at the start of an MPEG Layer III frame.
///
/// Returns `Ok(None)` for a non-Layer-III frame, a frame with no `Xing`/`Info`
/// magic at its expected offset, or a tag too short to hold the gapless
/// fields. A frame header that does not parse is the one error.
pub fn parse_lame_tag(frame: &[u8]) -> Result<Option<LameTag>> {
    let header = MpegAudioFrameHeader::parse(frame)?;
    if header.layer() != MpegAudioLayer::LayerIII {
        return Ok(None);
    }
    let end = header.frame_length().min(frame.len());
    let frame = &frame[..end];
    let xing = header.xing_offset();
    let Some(magic) = frame.get(xing..xing.saturating_add(XING_MAGIC_LEN)) else {
        return Ok(None);
    };
    if magic != XING_MAGIC && magic != INFO_MAGIC {
        return Ok(None);
    }
    let Some(flags_bytes) = frame.get(xing + XING_MAGIC_LEN..xing + XING_MAGIC_LEN + XING_FLAGS_LEN)
    else {
        return Ok(None);
    };
    let flags = u32::from_be_bytes([
        flags_bytes[0],
        flags_bytes[1],
        flags_bytes[2],
        flags_bytes[3],
    ]);
    let mut cursor = xing + XING_MAGIC_LEN + XING_FLAGS_LEN;
    for (flag, len) in [
        (XING_FLAG_FRAMES, XING_FRAMES_LEN),
        (XING_FLAG_BYTES, XING_BYTES_LEN),
        (XING_FLAG_TOC, XING_TOC_LEN),
        (XING_FLAG_QUALITY, XING_QUALITY_LEN),
    ] {
        if flags & flag != 0 {
            cursor = cursor.saturating_add(len);
        }
    }
    let Some(version) = frame.get(cursor..cursor.saturating_add(LAME_VERSION_LEN)) else {
        return Ok(None);
    };
    let mut encoder = [0u8; LAME_VERSION_LEN];
    encoder.copy_from_slice(version);
    cursor = cursor.saturating_add(LAME_VERSION_LEN + LAME_FIELDS_BEFORE_DELAY);
    let Some(delay_bytes) = frame.get(cursor..cursor.saturating_add(DELAY_PADDING_LEN)) else {
        return Ok(None);
    };
    let raw = (u32::from(delay_bytes[0]) << WIRE_BYTE_SHIFT_DOUBLED)
        | (u32::from(delay_bytes[1]) << WIRE_BYTE_SHIFT)
        | u32::from(delay_bytes[2]);
    let delay = raw >> DELAY_SHIFT;
    let padding = raw & DELAY_PADDING_MASK;
    let (encoder_delay, encoder_padding) = if is_lame_family(&encoder) {
        (Some(delay), Some(padding))
    } else {
        (None, None)
    };
    Ok(Some(LameTag {
        encoder,
        encoder_delay,
        encoder_padding,
    }))
}

/// Whether the 9-byte version string names a writer that stores the gapless
/// counts in the tag (LAME itself, or an FFmpeg `Lavf`/`Lavc` muxer).
fn is_lame_family(encoder: &[u8; LAME_VERSION_LEN]) -> bool {
    encoder.starts_with(b"LAME") || encoder.starts_with(b"Lavf") || encoder.starts_with(b"Lavc")
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

/// The `Xing` magic of a VBR/ABR tag.
const XING_MAGIC: [u8; XING_MAGIC_LEN] = *b"Xing";
/// The `Info` magic of a constant-bitrate tag.
const INFO_MAGIC: [u8; XING_MAGIC_LEN] = *b"Info";
/// Bytes in the `Xing`/`Info` magic.
const XING_MAGIC_LEN: usize = 4;
/// Bytes in the tag's flags word.
const XING_FLAGS_LEN: usize = 4;
/// Flag bit: a frame count follows.
const XING_FLAG_FRAMES: u32 = 0x1;
/// Flag bit: a byte count follows.
const XING_FLAG_BYTES: u32 = 0x2;
/// Flag bit: a 100-entry table of contents follows.
const XING_FLAG_TOC: u32 = 0x4;
/// Flag bit: a VBR quality value follows.
const XING_FLAG_QUALITY: u32 = 0x8;
/// Bytes of the optional frame count.
const XING_FRAMES_LEN: usize = 4;
/// Bytes of the optional byte count.
const XING_BYTES_LEN: usize = 4;
/// Bytes of the optional table of contents.
const XING_TOC_LEN: usize = 100;
/// Bytes of the optional VBR quality value.
const XING_QUALITY_LEN: usize = 4;

/// Side-information bytes in an MPEG-1 stereo/joint/dual frame.
const MPEG1_STEREO_SIDE_INFO: usize = 32;
/// Side-information bytes in an MPEG-1 mono frame.
const MPEG1_MONO_SIDE_INFO: usize = 17;
/// Side-information bytes in an MPEG-2/2.5 non-mono frame.
const LSF_STEREO_SIDE_INFO: usize = 17;
/// Side-information bytes in an MPEG-2/2.5 mono frame.
const LSF_MONO_SIDE_INFO: usize = 9;

/// Bytes in the LAME tag's encoder version string.
const LAME_VERSION_LEN: usize = 9;
/// Bytes between the version string's end and the delay/padding word: revision
/// and VBR method, lowpass, peak, two ReplayGain values, encoding flags, ABR or
/// minimum bitrate.
const LAME_FIELDS_BEFORE_DELAY: usize = 12;
/// Bytes of the packed encoder delay and padding.
const DELAY_PADDING_LEN: usize = 3;
/// Bit position of the 12-bit encoder delay in the packed word.
const DELAY_SHIFT: u32 = 12;
/// Bits in one wire byte, for assembling the 24-bit field.
const WIRE_BYTE_SHIFT: u32 = 8;
/// Bits in two wire bytes.
const WIRE_BYTE_SHIFT_DOUBLED: u32 = 2 * WIRE_BYTE_SHIFT;
/// Mask of the 12-bit encoder padding.
const DELAY_PADDING_MASK: u32 = 0xFFF;

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

    /// Find the first frame in an elementary stream.
    fn first_frame(bytes: &[u8]) -> &[u8] {
        for start in 0..bytes.len().saturating_sub(FRAME_HEADER_LEN) {
            let Ok(header) = MpegAudioFrameHeader::parse(&bytes[start..]) else {
                continue;
            };
            let frames = &bytes[start..];
            if parse_lame_tag(frames).ok().flatten().is_some() {
                return &frames[..header.frame_length().min(frames.len())];
            }
        }
        panic!("no frame with a LAME tag");
    }

    #[test]
    fn a_real_lame_tag_states_the_encoder_delay_and_padding() {
        let bytes = test_fixtures::read("audio/audio.mp3");
        let tag = parse_lame_tag(first_frame(&bytes))
            .expect("parses")
            .expect("a LAME tag");
        assert_eq!(&tag.encoder()[..4], b"Lavc");
        assert_eq!(tag.encoder_delay(), Some(576));
        assert_eq!(tag.encoder_padding(), Some(960));
    }

    #[test]
    fn a_frame_without_a_tag_is_not_a_lame_tag() {
        let bytes = test_fixtures::read("audio/audio.mp3");
        let frame = first_frame(&bytes);
        let silent = alloc::vec![0u8; frame.len()];
        assert_eq!(parse_lame_tag(&silent).ok(), None);
    }

    #[test]
    fn the_xing_offset_follows_the_side_information_size() {
        // MPEG-1 stereo: 4 + 32. MPEG-1 mono: 4 + 17.
        let stereo = MpegAudioFrameHeader::parse(&[0xFF, 0xFB, 0x50, 0x00]).unwrap();
        assert_eq!(stereo.xing_offset(), 36);
        let mono = MpegAudioFrameHeader::parse(&[0xFF, 0xFB, 0x50, 0xC0]).unwrap();
        assert_eq!(mono.xing_offset(), 21);
        assert_eq!(mono.channels(), 1);
        // MPEG-2.5 mono: 4 + 9.
        let lsf = MpegAudioFrameHeader::parse(&[0xFF, 0xE3, 0x90, 0xC4]).unwrap();
        assert_eq!(lsf.xing_offset(), 13);
    }
}
