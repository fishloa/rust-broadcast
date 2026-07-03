//! Legacy-codec elementary-stream headers: MPEG-2 video (H.262) sequence header
//! and MPEG-1/2 audio (MP1/2/3) frame header.
//!
//! These are the *config-recovery* parsers for the two classic broadcast
//! codecs, feeding the [`CodecConfig::Mpeg2Video`](crate::pipeline::CodecConfig)
//! and [`CodecConfig::MpegAudio`](crate::pipeline::CodecConfig) hub variants.
//! They read only the fixed header fields needed to describe the stream
//! (picture geometry / sampling parameters); the rest of the elementary stream
//! is carried opaquely as coded samples.
//!
//! # Spec
//!
//! - **MPEG-2 video sequence header**: ITU-T H.262 = ISO/IEC 13818-2 §6.2.2.1
//!   (`sequence_header()`): start code `0x000001B3`, then
//!   `horizontal_size_value` `[11:0]` and `vertical_size_value` `[11:0]`.
//! - **MPEG-1/2 audio frame header**: ISO/IEC 11172-3 §2.4.1.3 / ISO/IEC
//!   13818-3 §2.4.2.3 (`header`): 11-bit `syncword` (0x7FF), `ID` (MPEG
//!   version), `layer`, `bitrate_index`, `sampling_frequency`, `padding_bit`,
//!   `mode` (channel mode).
//! - **`esds` object-type indications** for these codecs: ISO/IEC 14496-1
//!   §7.2.6.6 Table 5 — 0x60–0x65 (MPEG-2 Visual profiles, 0x61 = Main),
//!   0x6A (MPEG-1 Visual), 0x69 (MPEG-2 Audio), 0x6B (MPEG-1 Audio).

use crate::error::{Error, Result};

// ── MPEG-2 video sequence header — ISO/IEC 13818-2 §6.2.2.1 ─────────────────

/// The 32-bit `sequence_header_code` start code (`0x000001B3`) that prefixes a
/// MPEG-1/2 video `sequence_header()` — ISO/IEC 13818-2 §6.2.2.1, Table 6-1.
pub const SEQUENCE_HEADER_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xB3];

/// The 24-bit start-code prefix (`0x000001`) common to every MPEG video start
/// code — ISO/IEC 13818-2 §6.2.1.
const START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];

/// Bytes of `horizontal_size_value` + `vertical_size_value` following the
/// 4-byte start code: 12 + 12 = 24 bits = 3 bytes (§6.2.2.1).
const SEQ_SIZE_FIELD_BYTES: usize = 3;

/// The coded picture geometry read from a MPEG-2 video `sequence_header()`.
///
/// ISO/IEC 13818-2 §6.2.2.1: `horizontal_size_value` and `vertical_size_value`
/// are 12-bit fields; the full sizes may be extended by a
/// `sequence_extension()`, but the base 12-bit values carry the standard
/// (≤ 4095) broadcast resolutions this hub targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mpeg2SeqHeader {
    /// `horizontal_size_value` (12 bits) — coded picture width in pixels.
    pub width: u16,
    /// `vertical_size_value` (12 bits) — coded picture height in pixels.
    pub height: u16,
}

impl Mpeg2SeqHeader {
    /// Find the first `sequence_header()` in an MPEG-2 video elementary stream
    /// (or `esds` DecoderSpecificInfo) and decode its picture geometry.
    ///
    /// Scans `es` for the `0x000001B3` start code, then reads the two 12-bit
    /// size fields. Returns [`Error::InvalidInput`] if no sequence header is
    /// present, or [`Error::BufferTooShort`] if it is truncated.
    pub fn find(es: &[u8]) -> Result<Self> {
        let start = find_start_code(es, SEQUENCE_HEADER_CODE[3]).ok_or(Error::InvalidInput(
            "no MPEG-2 sequence_header (start code 0x000001B3) in elementary stream",
        ))?;
        // Fields begin immediately after the 4-byte start code.
        let fields_at = start + SEQUENCE_HEADER_CODE.len();
        if fields_at + SEQ_SIZE_FIELD_BYTES > es.len() {
            return Err(Error::BufferTooShort {
                need: fields_at + SEQ_SIZE_FIELD_BYTES,
                have: es.len(),
                what: "MPEG-2 sequence_header size fields",
            });
        }
        let b = &es[fields_at..fields_at + SEQ_SIZE_FIELD_BYTES];
        // horizontal_size_value = b[0] << 4 | b[1] >> 4 (12 bits)
        // vertical_size_value   = (b[1] & 0x0F) << 8 | b[2] (12 bits)
        let width = ((b[0] as u16) << 4) | ((b[1] as u16) >> 4);
        let height = (((b[1] & 0x0F) as u16) << 8) | b[2] as u16;
        Ok(Self { width, height })
    }
}

/// Find the byte offset of a `0x000001 <code>` start code in `data`, or `None`.
fn find_start_code(data: &[u8], code: u8) -> Option<usize> {
    let mut i = 0usize;
    while i + 4 <= data.len() {
        if data[i..i + 3] == START_CODE_PREFIX && data[i + 3] == code {
            return Some(i);
        }
        i += 1;
    }
    None
}

// ── MPEG-1/2 audio frame header — ISO/IEC 11172-3 / 13818-3 §2.4 ────────────

/// Audio layer of an MPEG-1/2 audio frame — the `layer` field of the frame
/// header (ISO/IEC 11172-3 §2.4.2.3, coded `11`=I, `10`=II, `01`=III).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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

broadcast_common::impl_spec_display!(MpegAudioLayer);

/// MPEG audio version (the `ID` / `ID` + `MPEG-2.5 extension` bits) — selects
/// the sampling-rate row and the coefficient tables (ISO/IEC 11172-3 §2.4.2.3;
/// ISO/IEC 13818-3 for the LSF/MPEG-2 low-rate extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MpegAudioVersion {
    /// MPEG-1 (`ID = 1`).
    Mpeg1,
    /// MPEG-2 LSF (`ID = 0`).
    Mpeg2,
    /// MPEG-2.5 (unofficial low-sampling-rate extension, `ID` prefix `00`).
    Mpeg25,
}

/// The 11-bit `syncword` (all-ones) that opens an MPEG audio frame header
/// (ISO/IEC 11172-3 §2.4.2.3).
pub const MPEG_AUDIO_SYNCWORD: u16 = 0x07FF;

/// MPEG audio frame header (ISO/IEC 11172-3 §2.4.1.3 / ISO/IEC 13818-3).
///
/// Only the fields needed to describe the stream + walk frame boundaries are
/// decoded; the audio payload is carried opaquely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpegAudioFrameHeader {
    /// Audio layer (I / II / III).
    pub layer: MpegAudioLayer,
    /// Sampling frequency in Hz (§2.4.2.3 `sampling_frequency` table).
    pub sample_rate: u32,
    /// Channel count: 1 for `single_channel` mode, else 2.
    pub channels: u16,
    /// Total frame length in bytes (header + payload), for frame splitting.
    pub frame_length: usize,
    /// Decoded samples per frame (384 for Layer I; 1152 for Layer II; 1152 for
    /// MPEG-1 Layer III, 576 for MPEG-2/2.5 Layer III).
    pub samples_per_frame: u32,
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

/// Channel `mode` value for `single_channel` (mono) — §2.4.2.3.
const MODE_SINGLE_CHANNEL: u8 = 0b11;

impl MpegAudioFrameHeader {
    /// Parse the 4-byte MPEG audio frame header at the start of `data`.
    ///
    /// Validates the 11-bit syncword and rejects reserved layer / free /
    /// forbidden bitrate / reserved sampling-frequency values with
    /// [`Error::InvalidInput`] (so a false sync in the middle of a payload does
    /// not masquerade as a frame boundary).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(Error::BufferTooShort {
                need: 4,
                have: data.len(),
                what: "MPEG audio frame header",
            });
        }
        let h = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);

        let syncword = ((h >> 21) & 0x07FF) as u16;
        if syncword != MPEG_AUDIO_SYNCWORD {
            return Err(Error::InvalidInput(
                "MPEG audio frame header: bad syncword (expected 11-bit 0x7FF)",
            ));
        }
        // ID is 2 bits in MPEG-2.5: `00`=2.5, `10`=2, `11`=1 (`01` reserved).
        let id = ((h >> 19) & 0x3) as u8;
        let version = match id {
            0b11 => MpegAudioVersion::Mpeg1,
            0b10 => MpegAudioVersion::Mpeg2,
            0b00 => MpegAudioVersion::Mpeg25,
            _ => {
                return Err(Error::InvalidInput(
                    "MPEG audio frame header: reserved version ID",
                ));
            }
        };
        let layer = match ((h >> 17) & 0x3) as u8 {
            LAYER_I => MpegAudioLayer::LayerI,
            LAYER_II => MpegAudioLayer::LayerII,
            LAYER_III => MpegAudioLayer::LayerIII,
            _ => {
                return Err(Error::InvalidInput(
                    "MPEG audio frame header: reserved layer",
                ));
            }
        };
        let bitrate_index = ((h >> 12) & 0xF) as usize;
        let sample_rate_index = ((h >> 10) & 0x3) as usize;
        let padding = ((h >> 9) & 0x1) as usize;
        let mode = ((h >> 6) & 0x3) as u8;

        if sample_rate_index >= SAMPLE_RATE_MPEG1.len() {
            return Err(Error::InvalidInput(
                "MPEG audio frame header: reserved sampling_frequency",
            ));
        }
        let base_rate = SAMPLE_RATE_MPEG1[sample_rate_index];
        let sample_rate = match version {
            MpegAudioVersion::Mpeg1 => base_rate,
            MpegAudioVersion::Mpeg2 => base_rate / 2,
            MpegAudioVersion::Mpeg25 => base_rate / 4,
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
        let bitrate = bitrate_kbps as usize * 1000;

        let channels: u16 = if mode == MODE_SINGLE_CHANNEL { 1 } else { 2 };

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
            MpegAudioLayer::LayerI => (12 * bitrate / sample_rate as usize + padding) * 4,
            _ => (samples_per_frame as usize / 8) * bitrate / sample_rate as usize + padding,
        };
        if frame_length < 4 {
            return Err(Error::InvalidInput(
                "MPEG audio frame header: computed frame length shorter than header",
            ));
        }

        Ok(Self {
            layer,
            sample_rate,
            channels,
            frame_length,
            samples_per_frame,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_header_320x240() {
        // start code 00 00 01 B3, then 320 (0x140) / 240 (0x0F0):
        // 0x140 << 12 | 0x0F0 = 0x1400F0 → bytes 14 00 F0
        let es = [0x00, 0x00, 0x01, 0xB3, 0x14, 0x00, 0xF0, 0x23, 0xFF];
        let sh = Mpeg2SeqHeader::find(&es).unwrap();
        assert_eq!(sh.width, 320);
        assert_eq!(sh.height, 240);
    }

    #[test]
    fn seq_header_scans_past_leading_bytes() {
        let mut es = alloc::vec![0xDE, 0xAD, 0xBE, 0xEF];
        es.extend_from_slice(&[0x00, 0x00, 0x01, 0xB3, 0x28, 0x01, 0xE0]);
        let sh = Mpeg2SeqHeader::find(&es).unwrap();
        assert_eq!(sh.width, 640);
        assert_eq!(sh.height, 480);
    }

    #[test]
    fn seq_header_missing() {
        assert!(Mpeg2SeqHeader::find(&[0x00, 0x00, 0x01, 0xB8]).is_err());
    }

    #[test]
    fn mp2_frame_header_44100_mono() {
        // FF FD E0 C4 — MPEG-1 Layer II, 44100 Hz, mono, 384 kbps.
        let h = MpegAudioFrameHeader::parse(&[0xFF, 0xFD, 0xE0, 0xC4]).unwrap();
        assert_eq!(h.layer, MpegAudioLayer::LayerII);
        assert_eq!(h.layer.number(), 2);
        assert_eq!(h.sample_rate, 44100);
        assert_eq!(h.channels, 1);
        assert_eq!(h.samples_per_frame, 1152);
        assert_eq!(h.frame_length, 1253);
    }

    #[test]
    fn mp3_frame_header_44100() {
        // FF FB 50 C4 — MPEG-1 Layer III, 44100 Hz.
        let h = MpegAudioFrameHeader::parse(&[0xFF, 0xFB, 0x50, 0xC4]).unwrap();
        assert_eq!(h.layer, MpegAudioLayer::LayerIII);
        assert_eq!(h.sample_rate, 44100);
        assert_eq!(h.samples_per_frame, 1152);
    }

    #[test]
    fn bad_syncword_rejected() {
        assert!(MpegAudioFrameHeader::parse(&[0x00, 0x00, 0x00, 0x00]).is_err());
    }

    #[test]
    fn display_labels() {
        assert_eq!(MpegAudioLayer::LayerII.to_string(), "Layer II");
    }
}
