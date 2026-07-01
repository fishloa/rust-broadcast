//! AAC `AudioSpecificConfig` (ASC) — ISO/IEC 14496-3:2001 §1.6.2.1 Table 1.8.
//!
//! Bit-packed (`bslbf`/`uimsbf`, big-endian, MSB-first) audio decoder-config record
//! carried in an MP4 `mp4a` sample entry's `esds` `DecoderSpecificInfo` when
//! `objectTypeIndication == 0x40`, and the source for an ADTS header on TS→MP4
//! transmux.
//!
//! # ADTS↔ASC mapping
//!
//! | ADTS field              | ASC field                  | Relation                  |
//! |-------------------------|----------------------------|---------------------------|
//! | `profile` (2 bits)      | `audioObjectType`          | `ADTS.profile = AOT - 1` |
//! | `sampling_frequency_index` (4) | `samplingFrequencyIndex` | direct copy              |
//! | `channel_configuration` (4)    | `channelConfiguration`   | direct copy              |
//!
//! ## References
//!
//! - ISO/IEC 14496-3:2001, subpart 1 (`audioObjectType` §1.5.1.1 Table 1.1;
//!   `AudioSpecificConfig` §1.6.2.1 Table 1.8; `samplingFrequencyIndex` §1.6.3.3
//!   Table 1.10; `channelConfiguration` §1.6.3.4 Table 1.11).
//! - ADTS: ISO/IEC 13818-7:2003 §A.2.2.3 (Audio Data Transport Stream).

use crate::error::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

const ADTS_HEADER_SIZE: usize = 7;

// --- AudioObjectType §1.5.1.1 ---
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioObjectType {
    Null,
    AacMain,
    AacLc,
    AacSsr,
    AacLtp,
    Reserved(u8),
    AacScalable,
    TwinVq,
    Celp,
    Hvxc,
    Ttsi,
    MainSynthetic,
    WavetableSynthesis,
    GeneralMidi,
    AlgorithmicSynthesis,
    ErAacLc,
    ErAacLtp,
    ErAacScalable,
    ErTwinVq,
    ErBsac,
    ErAacLd,
    ErCelp,
    ErHvxc,
    ErHiln,
    ErParametric,
}
impl AudioObjectType {
    pub fn raw(self) -> u8 {
        match self {
            Self::Null => 0,
            Self::AacMain => 1,
            Self::AacLc => 2,
            Self::AacSsr => 3,
            Self::AacLtp => 4,
            Self::Reserved(r) => r & 0x1F,
            Self::AacScalable => 6,
            Self::TwinVq => 7,
            Self::Celp => 8,
            Self::Hvxc => 9,
            Self::Ttsi => 12,
            Self::MainSynthetic => 13,
            Self::WavetableSynthesis => 14,
            Self::GeneralMidi => 15,
            Self::AlgorithmicSynthesis => 16,
            Self::ErAacLc => 17,
            Self::ErAacLtp => 19,
            Self::ErAacScalable => 20,
            Self::ErTwinVq => 21,
            Self::ErBsac => 22,
            Self::ErAacLd => 23,
            Self::ErCelp => 24,
            Self::ErHvxc => 25,
            Self::ErHiln => 26,
            Self::ErParametric => 27,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Null => "null",
            Self::AacMain => "AAC Main",
            Self::AacLc => "AAC LC",
            Self::AacSsr => "AAC SSR",
            Self::AacLtp => "AAC LTP",
            Self::Reserved(_) => "reserved",
            Self::AacScalable => "AAC Scalable",
            Self::TwinVq => "TwinVQ",
            Self::Celp => "CELP",
            Self::Hvxc => "HVXC",
            Self::Ttsi => "TTSI",
            Self::MainSynthetic => "Main synthetic",
            Self::WavetableSynthesis => "Wavetable synthesis",
            Self::GeneralMidi => "General MIDI",
            Self::AlgorithmicSynthesis => "Algorithmic Synthesis & Audio FX",
            Self::ErAacLc => "ER AAC LC",
            Self::ErAacLtp => "ER AAC LTP",
            Self::ErAacScalable => "ER AAC Scalable",
            Self::ErTwinVq => "ER TwinVQ",
            Self::ErBsac => "ER BSAC",
            Self::ErAacLd => "ER AAC LD",
            Self::ErCelp => "ER CELP",
            Self::ErHvxc => "ER HVXC",
            Self::ErHiln => "ER HILN",
            Self::ErParametric => "ER Parametric",
        }
    }
}
impl From<u8> for AudioObjectType {
    fn from(raw: u8) -> Self {
        match raw & 0x1F {
            0 => Self::Null,
            1 => Self::AacMain,
            2 => Self::AacLc,
            3 => Self::AacSsr,
            4 => Self::AacLtp,
            5 => Self::Reserved(5),
            6 => Self::AacScalable,
            7 => Self::TwinVq,
            8 => Self::Celp,
            9 => Self::Hvxc,
            10 | 11 => Self::Reserved(raw & 0x1F),
            12 => Self::Ttsi,
            13 => Self::MainSynthetic,
            14 => Self::WavetableSynthesis,
            15 => Self::GeneralMidi,
            16 => Self::AlgorithmicSynthesis,
            17 => Self::ErAacLc,
            18 => Self::Reserved(18),
            19 => Self::ErAacLtp,
            20 => Self::ErAacScalable,
            21 => Self::ErTwinVq,
            22 => Self::ErBsac,
            23 => Self::ErAacLd,
            24 => Self::ErCelp,
            25 => Self::ErHvxc,
            26 => Self::ErHiln,
            27 => Self::ErParametric,
            _ => Self::Reserved(raw & 0x1F),
        }
    }
}
broadcast_common::impl_spec_display!(AudioObjectType, Reserved);

// --- SamplingFrequencyIndex §1.6.3.3 ---
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SamplingFrequencyIndex {
    Fs96000,
    Fs88200,
    Fs64000,
    Fs48000,
    Fs44100,
    Fs32000,
    Fs24000,
    Fs22050,
    Fs16000,
    Fs12000,
    Fs11025,
    Fs8000,
    Fs7350,
    Reserved(u8),
    Escape,
}
impl SamplingFrequencyIndex {
    pub fn raw(self) -> u8 {
        match self {
            Self::Fs96000 => 0x0,
            Self::Fs88200 => 0x1,
            Self::Fs64000 => 0x2,
            Self::Fs48000 => 0x3,
            Self::Fs44100 => 0x4,
            Self::Fs32000 => 0x5,
            Self::Fs24000 => 0x6,
            Self::Fs22050 => 0x7,
            Self::Fs16000 => 0x8,
            Self::Fs12000 => 0x9,
            Self::Fs11025 => 0xA,
            Self::Fs8000 => 0xB,
            Self::Fs7350 => 0xC,
            Self::Reserved(r) => r & 0x0F,
            Self::Escape => 0xF,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Fs96000 => "96000 Hz",
            Self::Fs88200 => "88200 Hz",
            Self::Fs64000 => "64000 Hz",
            Self::Fs48000 => "48000 Hz",
            Self::Fs44100 => "44100 Hz",
            Self::Fs32000 => "32000 Hz",
            Self::Fs24000 => "24000 Hz",
            Self::Fs22050 => "22050 Hz",
            Self::Fs16000 => "16000 Hz",
            Self::Fs12000 => "12000 Hz",
            Self::Fs11025 => "11025 Hz",
            Self::Fs8000 => "8000 Hz",
            Self::Fs7350 => "7350 Hz",
            Self::Reserved(_) => "reserved",
            Self::Escape => "escape",
        }
    }
}
impl From<u8> for SamplingFrequencyIndex {
    fn from(raw: u8) -> Self {
        match raw & 0x0F {
            0x0 => Self::Fs96000,
            0x1 => Self::Fs88200,
            0x2 => Self::Fs64000,
            0x3 => Self::Fs48000,
            0x4 => Self::Fs44100,
            0x5 => Self::Fs32000,
            0x6 => Self::Fs24000,
            0x7 => Self::Fs22050,
            0x8 => Self::Fs16000,
            0x9 => Self::Fs12000,
            0xA => Self::Fs11025,
            0xB => Self::Fs8000,
            0xC => Self::Fs7350,
            0xD | 0xE => Self::Reserved(raw & 0x0F),
            0xF => Self::Escape,
            _ => Self::Reserved(raw & 0x0F),
        }
    }
}
broadcast_common::impl_spec_display!(SamplingFrequencyIndex, Reserved);

// --- ChannelConfiguration §1.6.3.4 ---
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ChannelConfiguration {
    InBand,
    Mono,
    Stereo,
    Ch3,
    Ch4,
    Ch5,
    Ch5_1,
    Ch7_1,
    Reserved(u8),
}
impl ChannelConfiguration {
    pub fn raw(self) -> u8 {
        match self {
            Self::InBand => 0,
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Ch3 => 3,
            Self::Ch4 => 4,
            Self::Ch5 => 5,
            Self::Ch5_1 => 6,
            Self::Ch7_1 => 7,
            Self::Reserved(r) => r & 0x0F,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::InBand => "in-band",
            Self::Mono => "mono",
            Self::Stereo => "stereo",
            Self::Ch3 => "3 channels",
            Self::Ch4 => "4 channels",
            Self::Ch5 => "5 channels",
            Self::Ch5_1 => "5.1 surround",
            Self::Ch7_1 => "7.1 surround",
            Self::Reserved(_) => "reserved",
        }
    }
}
impl From<u8> for ChannelConfiguration {
    fn from(raw: u8) -> Self {
        match raw & 0x0F {
            0 => Self::InBand,
            1 => Self::Mono,
            2 => Self::Stereo,
            3 => Self::Ch3,
            4 => Self::Ch4,
            5 => Self::Ch5,
            6 => Self::Ch5_1,
            7 => Self::Ch7_1,
            _ => Self::Reserved(raw & 0x0F),
        }
    }
}
broadcast_common::impl_spec_display!(ChannelConfiguration, Reserved);

// --- AudioSpecificConfig §1.6.2.1 ---
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AudioSpecificConfig {
    pub audio_object_type: AudioObjectType,
    pub sampling_frequency_index: SamplingFrequencyIndex,
    pub sampling_frequency: Option<u32>,
    pub channel_configuration: ChannelConfiguration,
    /// Bytes following the byte that contains trailing data start.
    pub trailing: Vec<u8>,
    /// Bottom 3 bits of the partial byte shared with fixed fields.
    pub trailing_top: Option<u8>,
}

/// Explicit-signaling extension state (HE-AAC SBR / HE-AAC v2 PS) recovered from
/// an [`AudioSpecificConfig`] — ISO/IEC 14496-3 §1.6.2 (SBR: Amd 1, PS: Amd 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HeAacSignaling {
    /// SBR is explicitly signaled (`sbrPresentFlag == 1`, HE-AAC v1).
    pub sbr_present: bool,
    /// PS is explicitly signaled (`psPresentFlag == 1`, HE-AAC v2).
    pub ps_present: bool,
    /// The effective object type for RFC 6381: `29` (PS), `5` (SBR), or the core AOT.
    pub effective_aot: u8,
}

/// AudioObjectType raw value for SBR (HE-AAC v1).
const AOT_SBR: u8 = 5;
/// AudioObjectType raw value for PS (HE-AAC v2).
const AOT_PS: u8 = 29;
/// AAC backward-compatible SBR sync-extension type (11 bits).
const SYNC_EXT_SBR: u16 = 0x2B7;
/// AAC backward-compatible PS sync-extension type (11 bits).
const SYNC_EXT_PS: u16 = 0x548;
/// `samplingFrequencyIndex` escape value (24-bit explicit rate follows).
const SFI_ESCAPE: u8 = 0x0F;

impl AudioSpecificConfig {
    /// Build the RFC 6381 `mp4a.40.<AOT>` codec string, honouring explicit HE-AAC
    /// SBR/PS signaling (`mp4a.40.5` / `mp4a.40.29`).
    pub fn rfc6381(&self) -> String {
        crate::sps::rfc6381_mp4a(self.heaac_signaling().effective_aot)
    }

    /// Recover the explicit HE-AAC SBR/PS signaling from the ASC bitstream.
    ///
    /// Handles both encodings from ISO/IEC 14496-3 §1.6.2:
    /// hierarchical (`audioObjectType == 5`/`29`) and the trailing
    /// backward-compatible sync extensions (`0x2B7` → SBR, `0x548` → PS).
    /// Read-only over the parsed fields — never mutates the round-trip bytes.
    pub fn heaac_signaling(&self) -> HeAacSignaling {
        let bytes = self.to_bytes();
        detect_heaac_signaling(&bytes).unwrap_or(HeAacSignaling {
            sbr_present: false,
            ps_present: false,
            effective_aot: self.audio_object_type.raw(),
        })
    }
}

/// Read `n` MSB-first bits from `bytes` at `*bit`, advancing it. Returns `None`
/// if the buffer is exhausted.
fn asc_read_bits(bytes: &[u8], bit: &mut usize, n: usize) -> Option<u32> {
    if *bit + n > bytes.len() * 8 {
        return None;
    }
    let mut v = 0u32;
    for _ in 0..n {
        let byte = bytes[*bit / 8];
        let b = (byte >> (7 - (*bit % 8))) & 1;
        v = (v << 1) | b as u32;
        *bit += 1;
    }
    Some(v)
}

/// `GetAudioObjectType()` — 5 bits, or `31 + 6` bits (ISO/IEC 14496-3 §1.5.1.1).
fn asc_get_aot(bytes: &[u8], bit: &mut usize) -> Option<u8> {
    let a = asc_read_bits(bytes, bit, 5)?;
    if a == 31 {
        Some((32 + asc_read_bits(bytes, bit, 6)?) as u8)
    } else {
        Some(a as u8)
    }
}

/// Bit-decode the ASC to detect explicit SBR/PS signaling.
fn detect_heaac_signaling(bytes: &[u8]) -> Option<HeAacSignaling> {
    let mut bit = 0usize;
    let mut aot = asc_get_aot(bytes, &mut bit)?;
    let mut sbr_present = false;
    let mut ps_present = false;

    // samplingFrequencyIndex (+ 24-bit escape rate).
    let sfi = asc_read_bits(bytes, &mut bit, 4)? as u8;
    if sfi == SFI_ESCAPE {
        asc_read_bits(bytes, &mut bit, 24)?;
    }
    asc_read_bits(bytes, &mut bit, 4)?; // channelConfiguration

    // Hierarchical explicit signaling: AOT 5 (SBR) or 29 (PS).
    if aot == AOT_SBR || aot == AOT_PS {
        if aot == AOT_PS {
            ps_present = true;
        }
        sbr_present = true;
        let esfi = asc_read_bits(bytes, &mut bit, 4)? as u8;
        if esfi == SFI_ESCAPE {
            asc_read_bits(bytes, &mut bit, 24)?;
        }
        aot = asc_get_aot(bytes, &mut bit)?; // core AOT
    }

    // Trailing backward-compatible sync extension (best-effort; only for AAC-LC
    // core where GASpecificConfig is a fixed 3-bit tail with no PCE).
    if !sbr_present && aot == AudioObjectType::AacLc.raw() {
        // GASpecificConfig: frameLengthFlag(1) dependsOnCoreCoder(1) extensionFlag(1).
        asc_read_bits(bytes, &mut bit, 3);
        if let Some(sync) = asc_read_bits(bytes, &mut bit, 11) {
            if sync as u16 == SYNC_EXT_SBR {
                if let Some(ext_aot) = asc_get_aot(bytes, &mut bit) {
                    if ext_aot == AOT_SBR {
                        if let Some(flag) = asc_read_bits(bytes, &mut bit, 1) {
                            sbr_present = flag == 1;
                            if sbr_present {
                                let esfi = asc_read_bits(bytes, &mut bit, 4).unwrap_or(0) as u8;
                                if esfi == SFI_ESCAPE {
                                    asc_read_bits(bytes, &mut bit, 24);
                                }
                                if let Some(sync2) = asc_read_bits(bytes, &mut bit, 11) {
                                    if sync2 as u16 == SYNC_EXT_PS {
                                        // The psPresentFlag(1) follows; when the ASC is
                                        // byte-aligned it may be truncated, in which case
                                        // the presence of the 0x548 sync itself signals PS.
                                        ps_present =
                                            asc_read_bits(bytes, &mut bit, 1).unwrap_or(1) == 1;
                                    }
                                }
                            }
                        }
                    }
                }
            } else if sync as u16 == SYNC_EXT_PS {
                ps_present = asc_read_bits(bytes, &mut bit, 1).unwrap_or(0) == 1;
            }
        }
    }

    let effective_aot = if ps_present {
        AOT_PS
    } else if sbr_present {
        AOT_SBR
    } else {
        aot
    };
    Some(HeAacSignaling {
        sbr_present,
        ps_present,
        effective_aot,
    })
}

impl Parse<'_> for AudioSpecificConfig {
    type Error = Error;
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: bytes.len(),
                what: "ASC header",
            });
        }
        let b0 = bytes[0];
        let b1 = bytes[1];
        let aot = AudioObjectType::from(b0 >> 3);
        let sfi_raw = ((b0 & 7) << 1) | (b1 >> 7);
        let sfi = SamplingFrequencyIndex::from(sfi_raw);
        let sf = if sfi_raw == 0x0F {
            if bytes.len() < 5 {
                return Err(Error::BufferTooShort {
                    need: 5,
                    have: bytes.len(),
                    what: "ASC escape",
                });
            }
            Some(
                ((b1 as u32 & 0x7F) << 17)
                    | (bytes[2] as u32) << 9
                    | (bytes[3] as u32) << 1
                    | (bytes[4] as u32 & 1),
            )
        } else {
            None
        };
        let cc = if sfi_raw == 0x0F {
            if bytes.len() < 5 {
                return Err(Error::BufferTooShort {
                    need: 5,
                    have: bytes.len(),
                    what: "ASC cc esc",
                });
            }
            ChannelConfiguration::from((bytes[4] >> 3) & 0x0F)
        } else {
            ChannelConfiguration::from((b1 >> 3) & 0x0F)
        };
        // trailing stores bytes after the partial byte. trailing_top stores the GASpecific
        // bits from the partial byte (bottom 3 bits of byte 1 or byte 4).
        let (partial_idx, trailing_start) = if sfi_raw == 0x0F {
            (4, 5usize)
        } else {
            (1, 2usize)
        };
        let trailing_top = if partial_idx < bytes.len() {
            Some(bytes[partial_idx] & 7)
        } else {
            None
        };
        let trailing = if trailing_start < bytes.len() {
            bytes[trailing_start..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            audio_object_type: aot,
            sampling_frequency_index: sfi,
            sampling_frequency: sf,
            channel_configuration: cc,
            trailing,
            trailing_top,
        })
    }
}

impl Serialize for AudioSpecificConfig {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let base = if self.sampling_frequency_index == SamplingFrequencyIndex::Escape {
            5
        } else {
            2
        };
        base + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf.fill(0);
        let aot = self.audio_object_type.raw() & 0x1F;
        let sv = self.sampling_frequency_index.raw() & 0x0F;
        let cv = self.channel_configuration.raw() & 0x0F;
        let esc = self.sampling_frequency_index == SamplingFrequencyIndex::Escape;
        buf[0] = (aot << 3) | ((sv >> 1) & 7);
        if esc {
            let freq = self.sampling_frequency.unwrap_or(0) & 0xFF_FFFF;
            buf[1] = ((sv & 1) << 7) | ((freq >> 17) as u8 & 0x7F);
            buf[2] = (freq >> 9) as u8;
            buf[3] = (freq >> 1) as u8;
            let sf0 = (freq & 1) as u8;
            let tt = self.trailing_top.unwrap_or(0) & 7;
            buf[4] = (sf0 << 7) | (cv << 3) | tt;
            if !self.trailing.is_empty() {
                let n = self.trailing.len().min(buf.len().saturating_sub(5));
                if n > 0 {
                    buf[5..5 + n].copy_from_slice(&self.trailing[..n]);
                }
            }
        } else {
            let tt = self.trailing_top.unwrap_or(0) & 7;
            buf[1] = ((sv & 1) << 7) | (cv << 3) | tt;
            if !self.trailing.is_empty() {
                let n = self.trailing.len().min(buf.len().saturating_sub(2));
                if n > 0 {
                    buf[2..2 + n].copy_from_slice(&self.trailing[..n]);
                }
            }
        }
        Ok(need)
    }
}

// --- ADTS header §A.2.2.3 ---
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AdtsHeader {
    pub profile: u8,
    pub sampling_frequency_index: u8,
    pub channel_configuration: u8,
    pub frame_length: u16,
    pub buffer_fullness: u16,
    pub num_raw_data_blocks: u8,
}

pub fn build_adts_header(profile: u8, sfi: u8, cc: u8, frame_len: u16) -> [u8; ADTS_HEADER_SIZE] {
    let mut h = [0u8; ADTS_HEADER_SIZE];
    h[0] = 0xFF;
    h[1] = 0xF1;
    h[2] = (profile & 3) << 6 | (sfi & 0x0F) << 2 | (cc >> 3) & 1;
    h[3] = (cc & 7) << 5 | ((frame_len >> 11) & 3) as u8;
    h[4] = (frame_len >> 3) as u8;
    let bf: u16 = 0x7FF;
    h[5] = ((frame_len as u8 & 7) << 5) | ((bf >> 6) as u8 & 0x1F);
    h[6] = (bf as u8 & 0x3F) << 2;
    h
}

pub fn parse_adts_header(bytes: &[u8]) -> Result<AdtsHeader> {
    if bytes.len() < ADTS_HEADER_SIZE {
        return Err(Error::BufferTooShort {
            need: ADTS_HEADER_SIZE,
            have: bytes.len(),
            what: "ADTS",
        });
    }
    if bytes[0] != 0xFF || (bytes[1] & 0xF0) != 0xF0 {
        return Err(Error::InvalidValue {
            field: "adts_syncword",
            value: ((bytes[0] as u64) << 8) | (bytes[1] as u64),
            reason: "must be 0xFFF",
        });
    }
    let (b2, b3, b4, b5, b6) = (bytes[2], bytes[3], bytes[4], bytes[5], bytes[6]);
    Ok(AdtsHeader {
        profile: (b2 >> 6) & 3,
        sampling_frequency_index: (b2 >> 2) & 0x0F,
        channel_configuration: ((b2 & 1) << 3) | ((b3 >> 5) & 7),
        frame_length: (((b3 & 3) as u16) << 11) | ((b4 as u16) << 3) | ((b5 >> 5) as u16),
        buffer_fullness: (((b5 & 0x1F) as u16) << 6) | ((b6 >> 2) as u16),
        num_raw_data_blocks: b6 & 3,
    })
}

impl AudioSpecificConfig {
    pub fn to_adts_header(&self, frame_len: u16) -> Result<[u8; ADTS_HEADER_SIZE]> {
        if frame_len < ADTS_HEADER_SIZE as u16 {
            return Err(Error::InvalidValue {
                field: "frame_len",
                value: frame_len as u64,
                reason: "too small",
            });
        }
        Ok(build_adts_header(
            self.audio_object_type.raw().saturating_sub(1),
            self.sampling_frequency_index.raw(),
            self.channel_configuration.raw(),
            frame_len,
        ))
    }
    pub fn from_adts_header(adts: &AdtsHeader) -> Self {
        Self {
            audio_object_type: AudioObjectType::from((adts.profile + 1) & 0x1F),
            sampling_frequency_index: SamplingFrequencyIndex::from(adts.sampling_frequency_index),
            sampling_frequency: None,
            channel_configuration: ChannelConfiguration::from(adts.channel_configuration),
            trailing: Vec::new(),
            trailing_top: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;
    const KNOWN: [u8; 5] = [0x12, 0x08, 0x56, 0xe5, 0x00];

    #[test]
    fn parse_real_fixture() {
        let a = AudioSpecificConfig::parse(&KNOWN).unwrap();
        assert_eq!(a.audio_object_type, AudioObjectType::AacLc);
        assert_eq!(a.sampling_frequency_index, SamplingFrequencyIndex::Fs44100);
        assert_eq!(a.channel_configuration, ChannelConfiguration::Mono);
        assert!(a.sampling_frequency.is_none());
    }
    #[test]
    fn serialize_byte_identical() {
        let out = AudioSpecificConfig::parse(&KNOWN).unwrap().to_bytes();
        assert_eq!(out.as_slice(), KNOWN.as_slice());
    }
    #[test]
    fn adts_round_trip() {
        let a = AudioSpecificConfig::parse(&KNOWN).unwrap();
        let h = a.to_adts_header(200).unwrap();
        let p = parse_adts_header(&h).unwrap();
        assert_eq!(p.profile, 1);
        assert_eq!(p.sampling_frequency_index, 4);
        assert_eq!(p.channel_configuration, 1);
        assert_eq!(p.frame_length, 200);
        let a2 = AudioSpecificConfig::from_adts_header(&p);
        assert_eq!(a.audio_object_type, a2.audio_object_type);
        assert_eq!(a.sampling_frequency_index, a2.sampling_frequency_index);
        assert_eq!(a.channel_configuration, a2.channel_configuration);
    }
    #[test]
    fn adts_build_parse() {
        let p = parse_adts_header(&build_adts_header(1, 4, 2, 300)).unwrap();
        assert_eq!(p.profile, 1);
        assert_eq!(p.sampling_frequency_index, 4);
        assert_eq!(p.channel_configuration, 2);
        assert_eq!(p.frame_length, 300);
    }
    #[test]
    fn escape_round_trip() {
        let mut a = AudioSpecificConfig::parse(&KNOWN).unwrap();
        a.sampling_frequency_index = SamplingFrequencyIndex::Escape;
        a.sampling_frequency = Some(48000);
        a.channel_configuration = ChannelConfiguration::Stereo;
        let bytes = a.to_bytes();
        assert!(bytes.len() >= 5);
        let a2 = AudioSpecificConfig::parse(&bytes).unwrap();
        assert_eq!(a2.sampling_frequency_index, SamplingFrequencyIndex::Escape);
        assert_eq!(a2.sampling_frequency, Some(48000));
        assert_eq!(a2.channel_configuration, ChannelConfiguration::Stereo);
    }
    #[test]
    fn mutate_config_changes_bytes() {
        let o = AudioSpecificConfig::parse(&KNOWN).unwrap().to_bytes();
        let mut b = AudioSpecificConfig::parse(&KNOWN).unwrap();
        b.channel_configuration = ChannelConfiguration::Stereo;
        assert_ne!(o, b.to_bytes());
    }
}
