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
use broadcast_core::error::InvalidValue;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

/// ADTS fixed + variable header length without CRC (ISO/IEC 13818-7 §6.2, Tables 8/9: 28 + 28 bits).
pub const ADTS_HEADER_SIZE: usize = 7;

/// Mask selecting the layer bits (bits 1-2 of byte 1) in an ADTS header; layer
/// must be `00` (ISO/IEC 13818-7 §6.2.1).
const ADTS_LAYER_MASK_BITS: u8 = 0x06;

/// Maximum `frame_length` an ADTS fixed header can carry: a 13-bit field
/// (ISO/IEC 13818-7:2003 §A.2.2.3, header size included) — `2^13 - 1`.
const ADTS_FRAME_LENGTH_MAX: u16 = 8191;

/// The `0xFF` first byte of the ADTS syncword (§6.2).
const ADTS_SYNCWORD_FIRST_BYTE: u8 = 0xFF;
/// Byte 1 of a fixed ADTS header: syncword low nibble (1111), `ID` 0,
/// `layer` 00, `protection_absent` 1 (§6.2).
const ADTS_FIXED_HEADER_SECOND_BYTE: u8 = 0xF1;
/// Mask of the syncword's low nibble, shared with the fixed `ID`/`layer` bits.
const ADTS_SYNCWORD_LOW_NIBBLE_MASK: u8 = 0xF0;
/// Shift assembling the 12-bit syncword value for an error report.
const ADTS_SYNCWORD_VALUE_SHIFT: u32 = 8;
/// `protection_absent` is the low bit of byte 1 (§6.2).
const ADTS_PROTECTION_ABSENT_MASK: u8 = 0x01;
/// `profile` is a 2-bit ADTS field at the top of byte 2 (§6.2).
const ADTS_PROFILE_MASK: u8 = 0x03;
/// Shift of `profile` in byte 2.
const ADTS_PROFILE_SHIFT: u32 = 6;
/// Shift of `sampling_frequency_index` in byte 2.
const ADTS_SFI_SHIFT: u32 = 2;
/// High bit of `channel_configuration`, in byte 2.
const ADTS_CHANNEL_CONFIG_HIGH_MASK: u8 = 0x01;
/// Shift of the high `channel_configuration` bit down into place.
const ADTS_CHANNEL_CONFIG_HIGH_SHIFT: u32 = 2;
/// Low bits of `channel_configuration`, at the top of byte 3.
const ADTS_CHANNEL_CONFIG_LOW_MASK: u8 = 0x03;
/// Shift of the low `channel_configuration` bits in byte 3.
const ADTS_CHANNEL_CONFIG_LOW_SHIFT: u32 = 6;
/// Mask of the high `frame_length` bits, in byte 3.
const ADTS_FRAME_LENGTH_HIGH_MASK: u8 = 0x03;
/// Shift of the high `frame_length` bits in byte 3.
const ADTS_FRAME_LENGTH_HIGH_SHIFT: u32 = 11;
/// Shift of the middle `frame_length` bits in byte 4.
const ADTS_FRAME_LENGTH_MID_SHIFT: u32 = 3;
/// Mask of the low `frame_length` bits, in byte 5.
const ADTS_FRAME_LENGTH_LOW_MASK: u8 = 0x07;
/// Shift of the low `frame_length` bits in byte 5.
const ADTS_FRAME_LENGTH_LOW_SHIFT: u32 = 5;
/// `num_raw_data_blocks` is a 2-bit field in byte 6 (§6.2).
const ADTS_NUM_RAW_DATA_BLOCKS_MASK: u8 = 0x03;
/// The `buffer_fullness` sentinel (VBR), all 11 bits set.
const ADTS_BUFFER_FULLNESS_VBR: u16 = 0x7FF;
/// Mask of the high `buffer_fullness` bits, in byte 5.
const ADTS_BUFFER_FULLNESS_HIGH_MASK: u8 = 0x1F;
/// Shift of the high `buffer_fullness` bits in byte 5.
const ADTS_BUFFER_FULLNESS_HIGH_SHIFT: u32 = 6;
/// Mask of the low `buffer_fullness` bits, in byte 6.
const ADTS_BUFFER_FULLNESS_LOW_MASK: u8 = 0x3F;
/// Shift of the low `buffer_fullness` bits in byte 6.
const ADTS_BUFFER_FULLNESS_LOW_SHIFT: u32 = 2;
/// Byte offsets of the ADTS fixed-header fields (§6.2).
const ADTS_PROFILE_OFFSET: usize = 2;
const ADTS_CHANNEL_CONFIG_LOW_OFFSET: usize = 3;
const ADTS_FRAME_LENGTH_MID_OFFSET: usize = 4;
const ADTS_FRAME_LENGTH_LOW_OFFSET: usize = 5;
const ADTS_BUFFER_FULLNESS_LOW_OFFSET: usize = 6;
/// Highest core `audioObjectType` an ADTS 2-bit `profile` can carry (AAC LTP).
const ADTS_CORE_AOT_MAX: u8 = 4;

// --- AudioObjectType §1.5.1.1 ---
/// `audioObjectType` — ISO/IEC 14496-3 §1.5.1.1 Table 1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioObjectType {
    /// No object type assigned.
    Null,
    /// AAC Main.
    AacMain,
    /// AAC LC — the profile ADTS and every MP4 `mp4a.40.2` stream uses.
    AacLc,
    /// AAC SSR.
    AacSsr,
    /// AAC LTP.
    AacLtp,
    /// An object type with no Table 1.1 row (5, 10, 11, 18, 28..=31).
    Reserved(u8),
    /// AAC Scalable.
    AacScalable,
    /// TwinVQ.
    TwinVq,
    /// CELP.
    Celp,
    /// HVXC.
    Hvxc,
    /// TTSI.
    Ttsi,
    /// Main synthetic.
    MainSynthetic,
    /// Wavetable synthesis.
    WavetableSynthesis,
    /// General MIDI.
    GeneralMidi,
    /// Algorithmic Synthesis and Audio FX.
    AlgorithmicSynthesis,
    /// ER AAC LC.
    ErAacLc,
    /// ER AAC LTP.
    ErAacLtp,
    /// ER AAC Scalable.
    ErAacScalable,
    /// ER TwinVQ.
    ErTwinVq,
    /// ER BSAC.
    ErBsac,
    /// ER AAC LD.
    ErAacLd,
    /// ER CELP.
    ErCelp,
    /// ER HVXC.
    ErHvxc,
    /// ER HILN.
    ErHiln,
    /// ER Parametric.
    ErParametric,
}
impl AudioObjectType {
    /// The 5-bit `audioObjectType` value this variant denotes (Table 1.1).
    pub fn raw(self) -> u8 {
        match self {
            Self::Null => 0,
            Self::AacMain => 1,
            Self::AacLc => 2,
            Self::AacSsr => 3,
            Self::AacLtp => 4,
            Self::Reserved(r) => r & AOT_MASK,
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
    /// The Table 1.1 label, or `"reserved"`.
    pub fn name(&self) -> &'static str {
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
        match raw & AOT_MASK {
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
            _ => Self::Reserved(raw & AOT_MASK),
        }
    }
}
broadcast_core::impl_spec_display!(AudioObjectType, Reserved);

// --- SamplingFrequencyIndex §1.6.3.3 ---
/// `samplingFrequencyIndex` — ISO/IEC 14496-3 §1.6.3.3 Table 1.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SamplingFrequencyIndex {
    /// 96000 Hz.
    Fs96000,
    /// 88200 Hz.
    Fs88200,
    /// 64000 Hz.
    Fs64000,
    /// 48000 Hz.
    Fs48000,
    /// 44100 Hz.
    Fs44100,
    /// 32000 Hz.
    Fs32000,
    /// 24000 Hz.
    Fs24000,
    /// 22050 Hz.
    Fs22050,
    /// 16000 Hz.
    Fs16000,
    /// 12000 Hz.
    Fs12000,
    /// 11025 Hz.
    Fs11025,
    /// 8000 Hz.
    Fs8000,
    /// 7350 Hz.
    Fs7350,
    /// A reserved index (13, 14).
    Reserved(u8),
    /// The explicit-rate escape (15): a 24-bit `samplingFrequency` follows.
    Escape,
}
impl SamplingFrequencyIndex {
    /// The sampling rate in Hz this index denotes — ISO/IEC 14496-3 §1.6.3.3
    /// Table 1.10 ("samplingFrequencyIndex").
    ///
    /// This is the crate's **single** copy of Table 1.10, so the value a
    /// frequency index means cannot differ between the AAC
    /// `AudioSpecificConfig` parser, the FLV/RTMP demuxers and the SDP
    /// `config=` decoder. `None` for the reserved indices (13, 14) and for
    /// the explicit-rate escape (15), which carries its own value elsewhere.
    pub fn table_hz(self) -> Option<u32> {
        match self {
            Self::Fs96000 => Some(SAMPLE_RATE_96000_HZ),
            Self::Fs88200 => Some(SAMPLE_RATE_88200_HZ),
            Self::Fs64000 => Some(SAMPLE_RATE_64000_HZ),
            Self::Fs48000 => Some(SAMPLE_RATE_48000_HZ),
            Self::Fs44100 => Some(SAMPLE_RATE_44100_HZ),
            Self::Fs32000 => Some(SAMPLE_RATE_32000_HZ),
            Self::Fs24000 => Some(SAMPLE_RATE_24000_HZ),
            Self::Fs22050 => Some(SAMPLE_RATE_22050_HZ),
            Self::Fs16000 => Some(SAMPLE_RATE_16000_HZ),
            Self::Fs12000 => Some(SAMPLE_RATE_12000_HZ),
            Self::Fs11025 => Some(SAMPLE_RATE_11025_HZ),
            Self::Fs8000 => Some(SAMPLE_RATE_8000_HZ),
            Self::Fs7350 => Some(SAMPLE_RATE_7350_HZ),
            Self::Reserved(_) | Self::Escape => None,
        }
    }

    /// The 4-bit `samplingFrequencyIndex` value this variant denotes (Table 1.10).
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
            Self::Reserved(r) => r & SFI_MASK,
            Self::Escape => 0xF,
        }
    }
    /// The Table 1.10 label, or `"reserved"` / `"escape"`.
    pub fn name(&self) -> &'static str {
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
        match raw & SFI_MASK {
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
            0xD | 0xE => Self::Reserved(raw & SFI_MASK),
            0xF => Self::Escape,
            _ => Self::Reserved(raw & SFI_MASK),
        }
    }
}
broadcast_core::impl_spec_display!(SamplingFrequencyIndex, Reserved);

// --- ChannelConfiguration §1.6.3.4 ---
/// `channelConfiguration` — ISO/IEC 14496-3 §1.6.3.4 Table 1.11.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ChannelConfiguration {
    /// The mapping is carried in-band by a `program_config_element` (configuration 0).
    InBand,
    /// Mono (configuration 1).
    Mono,
    /// Stereo (configuration 2).
    Stereo,
    /// Three channels (configuration 3).
    Ch3,
    /// Four channels (configuration 4).
    Ch4,
    /// Five channels (configuration 5).
    Ch5,
    /// 5.1 surround (configuration 6).
    Ch5_1,
    /// 7.1 surround — eight channels (configuration 7).
    Ch7_1,
    /// A reserved configuration (8..=15).
    Reserved(u8),
}
impl ChannelConfiguration {
    /// The 4-bit `channelConfiguration` value this variant denotes (Table 1.11).
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
            Self::Reserved(r) => r & CHANNEL_CONFIG_MASK,
        }
    }
    /// The Table 1.11 label, or `"reserved"`.
    pub fn name(&self) -> &'static str {
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

    /// The number of audio channels this configuration's default channel
    /// mapping carries (ISO/IEC 14496-3 Table 1.19, "channelConfiguration").
    ///
    /// This is **not** the raw field value: configuration `7` is 8 channels
    /// (7.1), and `0` means the mapping is carried in-band by a
    /// `program_config_element` in the raw data stream, so the count is not
    /// known from the ASC — that (and a reserved value, 8..=15) returns
    /// `None` rather than a fabricated `0`. Reporting the raw field as a
    /// channel count made 7.1 look like 7 channels and a PCE-signalled stream
    /// look like 0.
    pub fn channel_count(&self) -> Option<u16> {
        match self {
            Self::InBand | Self::Reserved(_) => None,
            _ => CHANNEL_COUNTS[usize::from(self.raw())],
        }
    }
}
impl From<u8> for ChannelConfiguration {
    fn from(raw: u8) -> Self {
        match raw & CHANNEL_CONFIG_MASK {
            0 => Self::InBand,
            1 => Self::Mono,
            2 => Self::Stereo,
            3 => Self::Ch3,
            4 => Self::Ch4,
            5 => Self::Ch5,
            6 => Self::Ch5_1,
            7 => Self::Ch7_1,
            _ => Self::Reserved(raw & CHANNEL_CONFIG_MASK),
        }
    }
}
broadcast_core::impl_spec_display!(ChannelConfiguration, Reserved);

/// `CodecConfig::Aac.channel_count` for an [`AudioSpecificConfig`] whose
/// `channelConfiguration` does not determine a channel count.
///
/// ISO/IEC 14496-3 Table 1.19 gives a count for configurations 1..=7
/// (configuration 7 is **8** channels, 7.1). Configuration 0 means the mapping
/// is carried in-band by a `program_config_element` in the raw data stream —
/// this crate decodes no PCE — and 8..=15 are reserved. `0` marks "not
/// derived", matching this crate's existing placeholder convention
/// (`ts_demux`'s `MPEGH_CHANNEL_COUNT_UNSPECIFIED`), and is never a channel
/// count a real stream has.
pub const AAC_CHANNEL_COUNT_UNKNOWN: u16 = 0;

/// The channel count a decoded track should carry for `config`.
///
/// [`AAC_CHANNEL_COUNT_UNKNOWN`] when Table 1.19 does not determine one.
pub fn aac_channel_count(config: &AudioSpecificConfig) -> u16 {
    config
        .channel_configuration
        .channel_count()
        .unwrap_or(AAC_CHANNEL_COUNT_UNKNOWN)
}

// --- AudioSpecificConfig §1.6.2.1 ---
///
/// Build it by parsing a decoder-specific config (`Parse::parse`): the parser
/// is the only producer, so a newly decoded field is an additive change for a
/// consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AudioSpecificConfig {
    audio_object_type: AudioObjectType,
    sampling_frequency_index: SamplingFrequencyIndex,
    sampling_frequency: Option<u32>,
    channel_configuration: ChannelConfiguration,
    /// Bytes following the byte that contains trailing data start.
    trailing: Vec<u8>,
    /// Bottom 3 bits of the partial byte shared with fixed fields.
    trailing_top: Option<u8>,
}

impl AudioSpecificConfig {
    /// `audioObjectType` — ISO/IEC 14496-3 §1.5.1.1 Table 1.1.
    pub fn audio_object_type(&self) -> AudioObjectType {
        self.audio_object_type
    }

    /// `samplingFrequencyIndex` — Table 1.10.
    pub fn sampling_frequency_index(&self) -> SamplingFrequencyIndex {
        self.sampling_frequency_index
    }

    /// The explicit 24-bit `samplingFrequency` of the escape form.
    pub fn sampling_frequency(&self) -> Option<u32> {
        self.sampling_frequency
    }

    /// `channelConfiguration` — Table 1.11.
    pub fn channel_configuration(&self) -> ChannelConfiguration {
        self.channel_configuration
    }

    /// Bytes following the byte that contains the trailing data start.
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Bottom 3 bits of the partial byte shared with the fixed fields.
    pub fn trailing_top(&self) -> Option<u8> {
        self.trailing_top
    }
}

/// Explicit-signaling extension state (HE-AAC SBR / HE-AAC v2 PS) recovered from
/// an [`AudioSpecificConfig`] — ISO/IEC 14496-3 §1.6.2 (SBR: Amd 1, PS: Amd 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HeAacSignaling {
    sbr_present: bool,
    ps_present: bool,
    effective_aot: u8,
}

impl HeAacSignaling {
    /// SBR is explicitly signaled (`sbrPresentFlag == 1`, HE-AAC v1).
    pub fn sbr_present(&self) -> bool {
        self.sbr_present
    }

    /// PS is explicitly signaled (`psPresentFlag == 1`, HE-AAC v2).
    pub fn ps_present(&self) -> bool {
        self.ps_present
    }

    /// The effective object type for RFC 6381: `29` (PS), `5` (SBR), or the core AOT.
    pub fn effective_aot(&self) -> u8 {
        self.effective_aot
    }
}

/// AudioObjectType raw value for SBR (HE-AAC v1).
const AOT_SBR: u8 = 5;
/// AudioObjectType raw value for PS (HE-AAC v2).
const AOT_PS: u8 = 29;
/// AAC backward-compatible SBR sync-extension type (11 bits).
const SYNC_EXT_SBR: u16 = 0x2B7;
/// AAC backward-compatible PS sync-extension type (11 bits).
const SYNC_EXT_PS: u16 = 0x548;
/// `frameLengthFlag` within the 3-bit `GASpecificConfig` head held in
/// `trailing_top` (ISO/IEC 14496-3 §4.4.1: frameLengthFlag is its first bit).
const FRAME_LENGTH_FLAG_MASK: u8 = 0b100;
/// Samples per AAC frame when `frameLengthFlag` is 0 (ISO/IEC 14496-3 §4.4.1).
const AAC_FRAME_SAMPLES_1024: u32 = 1024;
/// Samples per AAC frame when `frameLengthFlag` is 1 (ISO/IEC 14496-3 §4.4.1).
const AAC_FRAME_SAMPLES_960: u32 = 960;
/// `samplingFrequencyIndex` escape value (24-bit explicit rate follows).
const SFI_ESCAPE: u8 = 0x0F;
/// Mask of the 5-bit `audioObjectType` field (Table 1.1).
const AOT_MASK: u8 = 0x1F;
/// Mask of the 4-bit `samplingFrequencyIndex` field (Table 1.10).
const SFI_MASK: u8 = 0x0F;
/// Mask of the 4-bit `channelConfiguration` field (Table 1.11).
const CHANNEL_CONFIG_MASK: u8 = 0x0F;
/// `GetAudioObjectType()` escape value 31 (Table 1.1).
const AOT_ESCAPE: u32 = 31;
/// Offset added to the escaped object type (Table 1.1).
const AOT_ESCAPE_OFFSET: u32 = 32;
/// Width of the value following the `GetAudioObjectType()` escape.
const AOT_ESCAPE_BITS: usize = 6;
/// Shift that puts `audioObjectType` at the top of the first ASC byte.
const AOT_SHIFT: u32 = u8::BITS - AOT_BITS;
/// `samplingFrequencyIndex` shares the first ASC byte: its high bits are here.
const SFI_HIGH_MASK: u8 = 0x07;
/// Left shift joining the `samplingFrequencyIndex` high bits to the low bit.
const SFI_HIGH_SHIFT: u32 = 1;
/// Right shift isolating the `samplingFrequencyIndex` low bit in byte 1.
const SFI_LOW_SHIFT: u32 = 7;
/// Right shift of `channelConfiguration` inside an ASC byte.
const CHANNEL_CONFIG_SHIFT: u32 = 3;
/// Low bits of the byte shared with the `GASpecificConfig` head.
const GA_TAIL_MASK: u8 = 0x07;
/// Width of a backward-compatible sync-extension type.
const SYNC_EXT_BITS: usize = 11;
/// Bytes of an ASC without the explicit-rate escape.
const ASC_HEADER_LEN: usize = 2;
/// Bytes of an ASC carrying the 24-bit explicit rate.
const ASC_ESCAPE_LEN: usize = 5;
/// Byte offsets of the ASC bytes after the first.
const ASC_SECOND_BYTE: usize = 1;
const ASC_THIRD_BYTE: usize = 2;
const ASC_FOURTH_BYTE: usize = 3;
const ASC_FIFTH_BYTE: usize = 4;
/// Mask of the 24-bit explicit `samplingFrequency`.
const EXPLICIT_RATE_MASK: u32 = 0xFF_FFFF;
/// Mask of the explicit rate bits held in the second ASC byte.
const EXPLICIT_RATE_SECOND_BYTE_MASK: u32 = 0x7F;
/// Shifts assembling the explicit `samplingFrequency` from bytes 1..=4.
const EXPLICIT_RATE_SECOND_BYTE_SHIFT: u32 = 17;
const EXPLICIT_RATE_THIRD_BYTE_SHIFT: u32 = 9;
const EXPLICIT_RATE_FOURTH_BYTE_SHIFT: u32 = 1;
const EXPLICIT_RATE_FIFTH_BYTE_SHIFT: u32 = 7;
/// Mask of the single explicit-rate bit held in the fifth ASC byte.
const EXPLICIT_RATE_LOW_BIT_MASK: u32 = 1;
/// Table 1.10 sampling rates in Hz.
const SAMPLE_RATE_96000_HZ: u32 = 96_000;
const SAMPLE_RATE_88200_HZ: u32 = 88_200;
const SAMPLE_RATE_64000_HZ: u32 = 64_000;
const SAMPLE_RATE_48000_HZ: u32 = 48_000;
const SAMPLE_RATE_44100_HZ: u32 = 44_100;
const SAMPLE_RATE_32000_HZ: u32 = 32_000;
const SAMPLE_RATE_24000_HZ: u32 = 24_000;
const SAMPLE_RATE_22050_HZ: u32 = 22_050;
const SAMPLE_RATE_16000_HZ: u32 = 16_000;
const SAMPLE_RATE_12000_HZ: u32 = 12_000;
const SAMPLE_RATE_11025_HZ: u32 = 11_025;
const SAMPLE_RATE_8000_HZ: u32 = 8_000;
const SAMPLE_RATE_7350_HZ: u32 = 7_350;
/// Default channel count per `channelConfiguration` (Table 1.19): 0 is
/// in-band, 7 is 7.1 (8 channels), 8..=15 are reserved.
const CHANNEL_COUNTS: [Option<u16>; 8] = [
    None,
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(8),
];

impl AudioSpecificConfig {
    /// Build the RFC 6381 `mp4a.40.<AOT>` codec string, honouring explicit HE-AAC
    /// SBR/PS signaling (`mp4a.40.5` / `mp4a.40.29`).
    pub fn rfc6381(&self) -> String {
        crate::rfc6381::rfc6381_mp4a(self.heaac_signaling().effective_aot)
    }

    /// Recover the explicit HE-AAC SBR/PS signaling from the ASC bitstream.
    ///
    /// Handles both encodings from ISO/IEC 14496-3 §1.6.2:
    /// hierarchical (`audioObjectType == 5`/`29`) and the trailing
    /// backward-compatible sync extensions (`0x2B7` → SBR, `0x548` → PS).
    /// Read-only over the parsed fields — never mutates the round-trip bytes.
    pub fn heaac_signaling(&self) -> HeAacSignaling {
        self.with_serialized(detect_heaac_signaling)
            .unwrap_or(HeAacSignaling {
                sbr_present: false,
                ps_present: false,
                effective_aot: self.audio_object_type.raw(),
            })
    }

    /// The effective sampling rate in Hz: the explicit 24-bit rate when present,
    /// else the rate of the `samplingFrequencyIndex` (ISO/IEC 14496-3 Table
    /// 1.10). `None` for the reserved indices. The crate's one place that
    /// combines the two (audit r04-O5/r05-O6).
    pub fn effective_sampling_frequency(&self) -> Option<u32> {
        self.sampling_frequency
            .or_else(|| self.sampling_frequency_index.table_hz())
    }

    /// PCM samples per raw_data_block (one AAC access unit) declared by the
    /// `GASpecificConfig` `frameLengthFlag` — ISO/IEC 14496-3 §4.4.1: 1024
    /// when clear, 960 when set. `None` for object types without a
    /// `GASpecificConfig` this decodes (only AAC main/LC/SSR/LTP are handled),
    /// when the flag byte is absent, or when explicit HE-AAC SBR/PS signalling
    /// makes the access-unit span differ from the core frame.
    pub fn samples_per_frame(&self) -> Option<u32> {
        if !matches!(
            self.audio_object_type,
            AudioObjectType::AacMain
                | AudioObjectType::AacLc
                | AudioObjectType::AacSsr
                | AudioObjectType::AacLtp
        ) {
            return None;
        }
        let signaling = self.heaac_signaling();
        if signaling.sbr_present || signaling.ps_present {
            return None;
        }
        let ga_bits = self.trailing_top?;
        Some(if ga_bits & FRAME_LENGTH_FLAG_MASK != 0 {
            AAC_FRAME_SAMPLES_960
        } else {
            AAC_FRAME_SAMPLES_1024
        })
    }

    /// Run `f` over this ASC's serialized bytes without a heap allocation when
    /// they fit [`STACK_ASC_MAX`] (every real ASC does; the HE-AAC decode below
    /// reads well under 16 bytes), else over a `to_bytes` copy. Byte-identical
    /// input to `f` either way (audit r04-O2).
    fn with_serialized<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let need = self.serialized_len();
        let mut stack = [0u8; STACK_ASC_MAX];
        if need <= STACK_ASC_MAX && self.serialize_into(&mut stack[..need]).is_ok() {
            return f(&stack[..need]);
        }
        // Serialization only fails for a value violating a wire constraint; an
        // empty input then yields the parsers' "no config" result rather than a panic.
        let bytes = self.try_to_bytes().unwrap_or_default();
        f(&bytes)
    }
}

/// Largest serialized ASC (bytes) handled on the stack by
/// [`AudioSpecificConfig::with_serialized`].
const STACK_ASC_MAX: usize = 64;

/// Read `n` MSB-first bits from `bytes` at `*bit`, advancing it. Returns `None`
/// if the buffer is exhausted.
///
/// Delegates to the shared [`crate::bitreader::read_bits_at`] cursor (`n` is
/// always <= 24 for every ASC field this module reads).
fn asc_read_bits(bytes: &[u8], bit: &mut usize, n: usize) -> Option<u32> {
    u32::try_from(crate::bitreader::read_bits_at(bytes, bit, n)?).ok()
}

/// `GetAudioObjectType()` — 5 bits, or `31 + 6` bits (ISO/IEC 14496-3 §1.5.1.1).
fn asc_get_aot(bytes: &[u8], bit: &mut usize) -> Option<u8> {
    let a = asc_read_bits(bytes, bit, AOT_BITS as usize)?;
    if a == AOT_ESCAPE {
        Some((AOT_ESCAPE_OFFSET + asc_read_bits(bytes, bit, AOT_ESCAPE_BITS)?) as u8)
    } else {
        Some(a as u8)
    }
}

/// Bit-decode the ASC to detect explicit SBR/PS signaling.
fn detect_heaac_signaling(bytes: &[u8]) -> Option<HeAacSignaling> {
    let (core_aot, sbr_present, ps_present) = detect_heaac_core(bytes)?;
    let effective_aot = if ps_present {
        AOT_PS
    } else if sbr_present {
        AOT_SBR
    } else {
        core_aot
    };
    Some(HeAacSignaling {
        sbr_present,
        ps_present,
        effective_aot,
    })
}

/// Bit-decode the ASC and return `(core_aot, sbr_present, ps_present)`. The
/// core AOT is what an ADTS `profile` field must be derived from (C2, #1008):
/// with explicit hierarchical signaling (`audioObjectType == 5`/`29`) the
/// *first* AOT read is the extension type, not the codec's actual profile —
/// the real core AOT follows the extension sampling-rate field.
fn detect_heaac_core(bytes: &[u8]) -> Option<(u8, bool, bool)> {
    let mut bit = 0usize;
    let mut aot = asc_get_aot(bytes, &mut bit)?;
    let mut sbr_present = false;
    let mut ps_present = false;

    // samplingFrequencyIndex (+ 24-bit escape rate).
    let sfi = asc_read_bits(bytes, &mut bit, SFI_BITS as usize)? as u8;
    if sfi == SFI_ESCAPE {
        asc_read_bits(bytes, &mut bit, SFI_ESCAPE_BITS)?;
    }
    asc_read_bits(bytes, &mut bit, CHANNEL_CONFIG_BITS)?; // channelConfiguration

    // Hierarchical explicit signaling: AOT 5 (SBR) or 29 (PS).
    if aot == AOT_SBR || aot == AOT_PS {
        if aot == AOT_PS {
            ps_present = true;
        }
        sbr_present = true;
        let esfi = asc_read_bits(bytes, &mut bit, SFI_BITS as usize)? as u8;
        if esfi == SFI_ESCAPE {
            asc_read_bits(bytes, &mut bit, SFI_ESCAPE_BITS)?;
        }
        aot = asc_get_aot(bytes, &mut bit)?; // core AOT
    }

    // Trailing backward-compatible sync extension (best-effort; only for AAC-LC
    // core where GASpecificConfig is a fixed 3-bit tail with no PCE).
    if !sbr_present && aot == AudioObjectType::AacLc.raw() {
        // GASpecificConfig: frameLengthFlag(1) dependsOnCoreCoder(1) extensionFlag(1).
        asc_read_bits(bytes, &mut bit, GA_HEAD_BITS as usize);
        if let Some(sync) = asc_read_bits(bytes, &mut bit, SYNC_EXT_BITS) {
            if sync as u16 == SYNC_EXT_SBR {
                if let Some(ext_aot) = asc_get_aot(bytes, &mut bit)
                    && ext_aot == AOT_SBR
                    && let Some(flag) = asc_read_bits(bytes, &mut bit, 1)
                {
                    sbr_present = flag == 1;
                    if sbr_present {
                        let esfi =
                            asc_read_bits(bytes, &mut bit, SFI_BITS as usize).unwrap_or(0) as u8;
                        if esfi == SFI_ESCAPE {
                            asc_read_bits(bytes, &mut bit, SFI_ESCAPE_BITS);
                        }
                        if let Some(sync2) = asc_read_bits(bytes, &mut bit, SYNC_EXT_BITS)
                            && sync2 as u16 == SYNC_EXT_PS
                        {
                            // The psPresentFlag(1) follows; when the ASC is
                            // byte-aligned it may be truncated, in which case
                            // the presence of the 0x548 sync itself signals PS.
                            ps_present = asc_read_bits(bytes, &mut bit, 1).unwrap_or(1) == 1;
                        }
                    }
                }
            } else if sync as u16 == SYNC_EXT_PS {
                ps_present = asc_read_bits(bytes, &mut bit, 1).unwrap_or(0) == 1;
            }
        }
    }

    Some((aot, sbr_present, ps_present))
}

impl Parse<'_> for AudioSpecificConfig {
    type Error = Error;
    fn parse(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, ASC_HEADER_LEN, "ASC header")?;
        let b0 = bytes[0];
        let b1 = bytes[ASC_SECOND_BYTE];
        let aot = AudioObjectType::from(b0 >> AOT_SHIFT);
        let sfi_raw = ((b0 & SFI_HIGH_MASK) << SFI_HIGH_SHIFT) | (b1 >> SFI_LOW_SHIFT);
        let sfi = SamplingFrequencyIndex::from(sfi_raw);
        let sf = if sfi_raw == SFI_ESCAPE {
            need_in(bytes, ASC_ESCAPE_LEN, "ASC escape")?;
            Some(
                ((b1 as u32 & EXPLICIT_RATE_SECOND_BYTE_MASK) << EXPLICIT_RATE_SECOND_BYTE_SHIFT)
                    | (bytes[ASC_THIRD_BYTE] as u32) << EXPLICIT_RATE_THIRD_BYTE_SHIFT
                    | (bytes[ASC_FOURTH_BYTE] as u32) << EXPLICIT_RATE_FOURTH_BYTE_SHIFT
                    | (bytes[ASC_FIFTH_BYTE] as u32 >> EXPLICIT_RATE_FIFTH_BYTE_SHIFT)
                        & EXPLICIT_RATE_LOW_BIT_MASK,
            )
        } else {
            None
        };
        let cc = if sfi_raw == SFI_ESCAPE {
            need_in(bytes, ASC_ESCAPE_LEN, "ASC cc esc")?;
            ChannelConfiguration::from(
                (bytes[ASC_FIFTH_BYTE] >> CHANNEL_CONFIG_SHIFT) & CHANNEL_CONFIG_MASK,
            )
        } else {
            ChannelConfiguration::from((b1 >> CHANNEL_CONFIG_SHIFT) & CHANNEL_CONFIG_MASK)
        };
        // trailing stores bytes after the partial byte. trailing_top stores the GASpecific
        // bits from the partial byte (bottom 3 bits of byte 1 or byte 4).
        let (partial_idx, trailing_start) = if sfi_raw == SFI_ESCAPE {
            (ASC_FIFTH_BYTE, ASC_ESCAPE_LEN)
        } else {
            (ASC_SECOND_BYTE, ASC_THIRD_BYTE)
        };
        let trailing_top = if partial_idx < bytes.len() {
            Some(bytes[partial_idx] & GA_TAIL_MASK)
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

impl AudioSpecificConfig {
    #[cfg(test)]
    fn with_fields(
        audio_object_type: AudioObjectType,
        sampling_frequency_index: SamplingFrequencyIndex,
        sampling_frequency: Option<u32>,
        channel_configuration: ChannelConfiguration,
        trailing: Vec<u8>,
        trailing_top: Option<u8>,
    ) -> Self {
        Self {
            audio_object_type,
            sampling_frequency_index,
            sampling_frequency,
            channel_configuration,
            trailing,
            trailing_top,
        }
    }
}

impl Serialize for AudioSpecificConfig {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let base = if self.sampling_frequency_index == SamplingFrequencyIndex::Escape {
            ASC_ESCAPE_LEN
        } else {
            ASC_HEADER_LEN
        };
        base + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        // Only the region this config occupies: the `Serialize` contract lets a
        // caller serialize into a larger shared buffer, and a whole-buffer
        // `fill(0)` would zero their bytes past `need` (r04-W9).
        buf[..need].fill(0);
        let aot = self.audio_object_type.raw() & AOT_MASK;
        let sv = self.sampling_frequency_index.raw() & SFI_MASK;
        let cv = self.channel_configuration.raw() & CHANNEL_CONFIG_MASK;
        let esc = self.sampling_frequency_index == SamplingFrequencyIndex::Escape;
        buf[0] = (aot << AOT_SHIFT) | ((sv >> SFI_HIGH_SHIFT) & SFI_HIGH_MASK);
        if esc {
            let freq = self.sampling_frequency.unwrap_or(0) & EXPLICIT_RATE_MASK;
            buf[ASC_SECOND_BYTE] = ((sv & 1) << SFI_LOW_SHIFT)
                | ((freq >> EXPLICIT_RATE_SECOND_BYTE_SHIFT) as u8
                    & EXPLICIT_RATE_SECOND_BYTE_MASK as u8);
            buf[ASC_THIRD_BYTE] = (freq >> EXPLICIT_RATE_THIRD_BYTE_SHIFT) as u8;
            buf[ASC_FOURTH_BYTE] = (freq >> EXPLICIT_RATE_FOURTH_BYTE_SHIFT) as u8;
            let sf0 = (freq & EXPLICIT_RATE_LOW_BIT_MASK) as u8;
            let tt = self.trailing_top.unwrap_or(0) & GA_TAIL_MASK;
            buf[ASC_FIFTH_BYTE] = (sf0 << SFI_LOW_SHIFT) | (cv << CHANNEL_CONFIG_SHIFT) | tt;
            // `need` already covers `trailing` in full, and `buf.len() >= need`
            // was checked above, so the copy can never overrun.
            buf[ASC_ESCAPE_LEN..need].copy_from_slice(&self.trailing);
        } else {
            let tt = self.trailing_top.unwrap_or(0) & GA_TAIL_MASK;
            buf[ASC_SECOND_BYTE] = ((sv & 1) << SFI_LOW_SHIFT) | (cv << CHANNEL_CONFIG_SHIFT) | tt;
            // As above: `need == 2 + trailing.len()`, already bounds-checked.
            buf[ASC_THIRD_BYTE..need].copy_from_slice(&self.trailing);
        }
        Ok(need)
    }
}

// --- ADTS header §A.2.2.3 ---
/// Fields of an ADTS fixed + variable header (ISO/IEC 13818-7 §6.2, Tables 8/9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AdtsHeader {
    /// `protection_absent` (1 bit): `true` = no `adts_error_check()` CRC
    /// follows the fixed+variable header; `false` = a 16-bit `crc_check`
    /// (2 bytes) is present before the raw data block(s) — ISO/IEC 13818-7
    /// §6.2, `adts_frame()`. C11 (#1012): this was not read at all, so a
    /// CRC-protected ADTS frame's header was always treated as 7 bytes wide
    /// instead of 9, leaving the 2 CRC bytes inside the "sample" data.
    protection_absent: bool,
    profile: u8,
    sampling_frequency_index: u8,
    channel_configuration: u8,
    frame_length: u16,
    buffer_fullness: u16,
    num_raw_data_blocks: u8,
}

impl AdtsHeader {
    /// `protection_absent`: the frame carries no `adts_error_check()` CRC.
    pub fn protection_absent(&self) -> bool {
        self.protection_absent
    }

    /// `profile` (2 bits): the core `audioObjectType` minus one.
    pub fn profile(&self) -> u8 {
        self.profile
    }

    /// Raw `sampling_frequency_index` (4 bits) — ISO/IEC 14496-3 Table 1.10.
    pub fn sampling_frequency_index(&self) -> u8 {
        self.sampling_frequency_index
    }

    /// Raw `channel_configuration` (3 bits, split across bytes 2-3).
    pub fn channel_configuration(&self) -> u8 {
        self.channel_configuration
    }

    /// `frame_length` (13 bits), the whole frame including this header.
    pub fn frame_length(&self) -> u16 {
        self.frame_length
    }

    /// `adts_buffer_fullness` (11 bits); `0x7FF` means VBR.
    pub fn buffer_fullness(&self) -> u16 {
        self.buffer_fullness
    }

    /// `num_raw_data_blocks` (2 bits): raw data blocks in the frame, minus one.
    pub fn num_raw_data_blocks(&self) -> u8 {
        self.num_raw_data_blocks
    }
}

/// Build the seven fixed+variable ADTS header bytes for a raw data block of
/// `frame_len` bytes (ISO/IEC 13818-7 §6.2). Returns the header bytes because
/// that is the whole of what it produces.
pub fn build_adts_header(profile: u8, sfi: u8, cc: u8, frame_len: u16) -> [u8; ADTS_HEADER_SIZE] {
    let mut h = [0u8; ADTS_HEADER_SIZE];
    h[0] = ADTS_SYNCWORD_FIRST_BYTE;
    h[1] = ADTS_FIXED_HEADER_SECOND_BYTE;
    // channel_configuration is a 3-bit field split across the byte boundary
    // (ISO/IEC 13818-7 §6.2 ADTS fixed header): its MSB is byte2 bit0, and its
    // low 2 bits are byte3 bits `[7:6]`.
    h[ADTS_PROFILE_OFFSET] = (profile & ADTS_PROFILE_MASK) << ADTS_PROFILE_SHIFT
        | (sfi & SFI_MASK) << ADTS_SFI_SHIFT
        | (cc >> ADTS_CHANNEL_CONFIG_HIGH_SHIFT) & ADTS_CHANNEL_CONFIG_HIGH_MASK;
    h[ADTS_CHANNEL_CONFIG_LOW_OFFSET] = (cc & ADTS_CHANNEL_CONFIG_LOW_MASK)
        << ADTS_CHANNEL_CONFIG_LOW_SHIFT
        | ((frame_len >> ADTS_FRAME_LENGTH_HIGH_SHIFT) & u16::from(ADTS_FRAME_LENGTH_HIGH_MASK))
            as u8;
    h[ADTS_FRAME_LENGTH_MID_OFFSET] = (frame_len >> ADTS_FRAME_LENGTH_MID_SHIFT) as u8;
    let bf: u16 = ADTS_BUFFER_FULLNESS_VBR;
    h[ADTS_FRAME_LENGTH_LOW_OFFSET] = ((frame_len as u8 & ADTS_FRAME_LENGTH_LOW_MASK)
        << ADTS_FRAME_LENGTH_LOW_SHIFT)
        | ((bf >> ADTS_BUFFER_FULLNESS_HIGH_SHIFT) as u8 & ADTS_BUFFER_FULLNESS_HIGH_MASK);
    h[ADTS_BUFFER_FULLNESS_LOW_OFFSET] =
        (bf as u8 & ADTS_BUFFER_FULLNESS_LOW_MASK) << ADTS_BUFFER_FULLNESS_LOW_SHIFT;
    h
}

/// Size of the `raw_data_block_position` field (16 bits) in a CRC-protected
/// multi-block ADTS header (ISO/IEC 13818-7 §6.2, Table 6).
pub const ADTS_BLOCK_POSITION_SIZE: usize = 2;

/// Size of the 16-bit `crc_check` word following an ADTS header when
/// `protection_absent == 0` (ISO/IEC 13818-7 §6.2, `adts_error_check()`).
pub const ADTS_CRC_SIZE: usize = 2;

/// Parse an ADTS fixed + variable header (ISO/IEC 13818-7 §6.2).
///
/// Rejects a bad syncword, a non-zero `layer`, and a `frame_length` smaller
/// than the header itself.
pub fn parse_adts_header(bytes: &[u8]) -> Result<AdtsHeader> {
    need_in(bytes, ADTS_HEADER_SIZE, "ADTS")?;
    if bytes[0] != ADTS_SYNCWORD_FIRST_BYTE
        || (bytes[1] & ADTS_SYNCWORD_LOW_NIBBLE_MASK) != ADTS_SYNCWORD_LOW_NIBBLE_MASK
    {
        return Err(Error::InvalidValue(InvalidValue {
            field: "adts_syncword",
            value: ((bytes[0] as u64) << ADTS_SYNCWORD_VALUE_SHIFT) | (bytes[1] as u64),
            reason: "must be 0xFFF",
        }));
    }
    if (bytes[1] & ADTS_LAYER_MASK_BITS) != 0 {
        return Err(Error::InvalidValue(InvalidValue {
            field: "adts_layer",
            value: (bytes[1] & ADTS_LAYER_MASK_BITS) as u64,
            reason: "must be 00",
        }));
    }
    let (b2, b3, b4, b5, b6) = (
        bytes[ADTS_PROFILE_OFFSET],
        bytes[ADTS_CHANNEL_CONFIG_LOW_OFFSET],
        bytes[ADTS_FRAME_LENGTH_MID_OFFSET],
        bytes[ADTS_FRAME_LENGTH_LOW_OFFSET],
        bytes[ADTS_BUFFER_FULLNESS_LOW_OFFSET],
    );
    let protection_absent = (bytes[1] & ADTS_PROTECTION_ABSENT_MASK) != 0;
    let sampling_frequency_index_raw = (b2 >> ADTS_SFI_SHIFT) & SFI_MASK;
    let num_raw_data_blocks = b6 & ADTS_NUM_RAW_DATA_BLOCKS_MASK;
    let frame_length = (u16::from(b3 & ADTS_FRAME_LENGTH_HIGH_MASK)
        << ADTS_FRAME_LENGTH_HIGH_SHIFT)
        | (u16::from(b4) << ADTS_FRAME_LENGTH_MID_SHIFT)
        | (u16::from(b5) >> ADTS_FRAME_LENGTH_LOW_SHIFT);

    let hdr = AdtsHeader {
        protection_absent,
        profile: (b2 >> ADTS_PROFILE_SHIFT) & ADTS_PROFILE_MASK,
        sampling_frequency_index: sampling_frequency_index_raw,
        channel_configuration: ((b2 & ADTS_CHANNEL_CONFIG_HIGH_MASK)
            << ADTS_CHANNEL_CONFIG_HIGH_SHIFT)
            | ((b3 >> ADTS_CHANNEL_CONFIG_LOW_SHIFT) & ADTS_CHANNEL_CONFIG_LOW_MASK),
        frame_length,
        buffer_fullness: ((u16::from(b5 & ADTS_BUFFER_FULLNESS_HIGH_MASK)
            << ADTS_BUFFER_FULLNESS_HIGH_SHIFT)
            | (u16::from(b6) >> ADTS_BUFFER_FULLNESS_LOW_SHIFT)),
        num_raw_data_blocks,
    };

    if (frame_length as usize) < adts_header_len(&hdr) {
        return Err(Error::InvalidValue(InvalidValue {
            field: "adts_frame_length",
            value: frame_length as u64,
            reason: "smaller than header length",
        }));
    }
    Ok(hdr)
}

/// The ADTS header length (fixed + variable + CRC/position words) for a parsed
/// header (ISO/IEC 13818-7 §6.2, Tables 5/6/10).
pub fn adts_header_len(hdr: &AdtsHeader) -> usize {
    if hdr.protection_absent {
        ADTS_HEADER_SIZE
    } else {
        ADTS_HEADER_SIZE
            + ADTS_BLOCK_POSITION_SIZE * hdr.num_raw_data_blocks as usize
            + ADTS_CRC_SIZE
    }
}

impl AudioSpecificConfig {
    /// Build the seven fixed+variable ADTS header bytes this config implies for
    /// a raw data block of `frame_len` bytes. Returns the header bytes because
    /// that is the whole of what it produces.
    pub fn to_adts_header(&self, frame_len: u16) -> Result<[u8; ADTS_HEADER_SIZE]> {
        if frame_len < ADTS_HEADER_SIZE as u16 {
            return Err(Error::InvalidValue(InvalidValue {
                field: "frame_len",
                value: frame_len as u64,
                reason: "too small",
            }));
        }
        if frame_len > ADTS_FRAME_LENGTH_MAX {
            return Err(Error::InvalidValue(InvalidValue {
                field: "frame_len",
                value: frame_len as u64,
                reason: "exceeds ADTS 13-bit frame_length maximum (8191)",
            }));
        }
        if self.sampling_frequency_index == SamplingFrequencyIndex::Escape {
            return Err(Error::InvalidValue(InvalidValue {
                field: "sampling_frequency_index",
                value: self.sampling_frequency_index.raw() as u64,
                reason: "escape (explicit rate) is not representable in ADTS's 4-bit field",
            }));
        }
        // C2 (#1008): with explicit hierarchical HE-AAC/HE-AACv2 signaling
        // (`audioObjectType == 5`/`29`), the ASC's *first* AOT is the SBR/PS
        // extension type, not the codec's actual core profile — the real
        // core AOT follows the extension sfi. `self.audio_object_type` is
        // that first AOT, so it must not be used directly as the ADTS
        // `profile` source; decode the core AOT the same way `heaac_signaling`
        // does. ADTS `profile` (2 bits, `= AOT - 1`) can only represent core
        // AOT 1..=4 (Main/LC/SSR/LTP); anything else is rejected rather than
        // silently wrapped into an unrelated profile.
        let core_aot = self
            .with_serialized(detect_heaac_core)
            .map(|(aot, _, _)| aot)
            .unwrap_or(self.audio_object_type.raw());
        if !(1..=ADTS_CORE_AOT_MAX).contains(&core_aot) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "audio_object_type",
                value: core_aot as u64,
                reason: "core AOT is not representable in ADTS's 2-bit profile field (must be Main/LC/SSR/LTP, 1..=4)",
            }));
        }
        Ok(build_adts_header(
            core_aot - 1,
            self.sampling_frequency_index.raw(),
            self.channel_configuration.raw(),
            frame_len,
        ))
    }
    /// The config an ADTS header states: object type `profile + 1`, its
    /// sampling-frequency index and channel configuration, no trailing data.
    pub fn from_adts_header(adts: &AdtsHeader) -> Self {
        Self {
            audio_object_type: AudioObjectType::from((adts.profile + 1) & AOT_MASK),
            sampling_frequency_index: SamplingFrequencyIndex::from(adts.sampling_frequency_index),
            sampling_frequency: None,
            channel_configuration: ChannelConfiguration::from(adts.channel_configuration),
            trailing: Vec::new(),
            trailing_top: None,
        }
    }
}

/// HE-AAC flavour to signal explicitly in an `AudioSpecificConfig`: the
/// hierarchical SBR (`audioObjectType` 5, HE-AAC v1) or PS (`audioObjectType`
/// 29, HE-AAC v2) form of ISO/IEC 14496-3 §1.6.2, which Apple's HLS authoring
/// rules require for HE-AAC in fMP4 instead of an implicit ADTS-derived config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HeAacProfile {
    /// HE-AAC v1: AAC-LC core + SBR.
    V1,
    /// HE-AAC v2: AAC-LC core + SBR + Parametric Stereo.
    V2,
}

impl HeAacProfile {
    /// The `audioObjectType` that opens the hierarchical ASC for this profile.
    pub fn audio_object_type(self) -> u8 {
        match self {
            Self::V1 => AOT_SBR,
            Self::V2 => AOT_PS,
        }
    }

    /// Spec-token label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::V1 => "HE-AAC v1 (SBR)",
            Self::V2 => "HE-AAC v2 (SBR+PS)",
        }
    }
}

broadcast_core::impl_spec_display!(HeAacProfile);

/// Extension rate is the core rate times this (SBR doubles the output rate,
/// ISO/IEC 14496-3 §4.6.18).
const SBR_RATE_FACTOR: u32 = 2;
/// Highest raw `samplingFrequencyIndex` that names a table rate (Table 1.10).
const SFI_TABLE_LAST: u8 = 0x0C;
/// Core `audioObjectType` of an AAC-LC stream (Table 1.1).
const AOT_AAC_LC: u8 = 2;
/// Width of `audioObjectType` in the ASC.
const AOT_BITS: u32 = 5;
/// Width of the `GASpecificConfig` head (frameLengthFlag, dependsOnCoreCoder,
/// extensionFlag) that follows the core object type.
const GA_HEAD_BITS: u32 = 3;
/// Width of `samplingFrequencyIndex`.
const SFI_BITS: u32 = 4;
/// Width of `channelConfiguration`.
const CHANNEL_CONFIG_BITS: usize = 4;
/// Width of an explicit `samplingFrequency` escape value.
const SFI_ESCAPE_BITS: usize = 24;
/// Bits of the hierarchical tail that share the byte holding the fixed
/// `channelConfiguration` field.
const TAIL_BITS_IN_SHARED_BYTE: u32 = 3;
/// Width of the hierarchical tail after `channelConfiguration`.
const TAIL_BITS: u32 = SFI_BITS + AOT_BITS + GA_HEAD_BITS;
/// Bits of the tail that follow the shared byte.
const TAIL_SPILL_BITS: u32 = TAIL_BITS - TAIL_BITS_IN_SHARED_BYTE;
/// Bits in the two bytes the spill is left-aligned into.
const SPILL_FIELD_BITS: u32 = 16;

impl AudioSpecificConfig {
    /// Re-express an AAC-LC config as explicit hierarchical HE-AAC signalling
    /// (ISO/IEC 14496-3 §1.6.2): `audioObjectType` 5 (v1) or 29 (v2), the
    /// core rate in `samplingFrequencyIndex`, twice that rate in
    /// `extensionSamplingFrequencyIndex`, then the underlying AAC-LC object
    /// type and its `GASpecificConfig`.
    ///
    /// A config already in exactly this hierarchical form is re-expressed for
    /// `profile` (so an explicit v1 can become v2 and back). Fails for any
    /// other config: not AAC-LC, explicit-rate escape, a non-default
    /// `GASpecificConfig`, trailing extension data, or a doubled core rate
    /// with no Table 1.10 index.
    pub fn with_explicit_he_aac(&self, profile: HeAacProfile) -> Result<Self> {
        let raw_aot = self.audio_object_type.raw();
        let current = match raw_aot {
            AOT_SBR => Some(HeAacProfile::V1),
            AOT_PS => Some(HeAacProfile::V2),
            _ => None,
        };
        if let Some(current) = current {
            let derived = self.with_plain_core().build_explicit_he_aac(current)?;
            if derived.trailing_top != self.trailing_top || derived.trailing != self.trailing {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "trailing",
                    value: self.trailing.len() as u64,
                    reason: "hierarchical config is not the default AAC-LC form",
                }));
            }
        } else if self.audio_object_type != AudioObjectType::AacLc {
            return Err(Error::InvalidValue(InvalidValue {
                field: "audio_object_type",
                value: u64::from(raw_aot),
                reason: "explicit HE-AAC signalling needs an AAC-LC core",
            }));
        } else if !self.trailing.is_empty() || self.trailing_top.is_some_and(|t| t != 0) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "trailing",
                value: self.trailing.len() as u64,
                reason: "config already carries GASpecificConfig/extension data",
            }));
        }
        let base = if current.is_some() {
            self.with_plain_core()
        } else {
            self.clone()
        };
        base.build_explicit_he_aac(profile)
    }

    fn with_plain_core(&self) -> Self {
        Self {
            audio_object_type: AudioObjectType::AacLc,
            sampling_frequency_index: self.sampling_frequency_index,
            sampling_frequency: None,
            channel_configuration: self.channel_configuration,
            trailing: Vec::new(),
            trailing_top: None,
        }
    }

    fn build_explicit_he_aac(&self, profile: HeAacProfile) -> Result<Self> {
        let core_hz = self
            .sampling_frequency_index
            .table_hz()
            .ok_or(Error::InvalidValue(InvalidValue {
                field: "sampling_frequency_index",
                value: u64::from(self.sampling_frequency_index.raw()),
                reason: "core rate is not a Table 1.10 rate",
            }))?;
        let ext_hz = core_hz.saturating_mul(SBR_RATE_FACTOR);
        let ext_index = extension_index_for(ext_hz).ok_or(Error::InvalidValue(InvalidValue {
            field: "extension_sampling_frequency_index",
            value: u64::from(ext_hz),
            reason: "doubled core rate has no Table 1.10 index",
        }))?;
        let tail = (u16::from(ext_index.raw()) << (AOT_BITS + GA_HEAD_BITS))
            | (u16::from(AOT_AAC_LC) << GA_HEAD_BITS);
        let top = (tail >> TAIL_SPILL_BITS) as u8;
        let spill_mask = (1u16 << TAIL_SPILL_BITS) - 1;
        let spill = ((tail & spill_mask) << (SPILL_FIELD_BITS - TAIL_SPILL_BITS)).to_be_bytes();
        Ok(Self {
            audio_object_type: AudioObjectType::from(profile.audio_object_type()),
            sampling_frequency_index: self.sampling_frequency_index,
            sampling_frequency: None,
            channel_configuration: self.channel_configuration,
            trailing: spill.to_vec(),
            trailing_top: Some(top),
        })
    }

    /// The underlying codec's object type: for hierarchical HE-AAC signalling
    /// (`audioObjectType` 5/29 first) the one that follows the extension rate
    /// (normally AAC-LC), otherwise [`Self::audio_object_type`].
    pub fn core_audio_object_type(&self) -> AudioObjectType {
        self.with_serialized(detect_heaac_core)
            .map_or(self.audio_object_type, |(aot, _, _)| {
                AudioObjectType::from(aot)
            })
    }

    /// The explicit SBR output rate in Hz declared by a hierarchical
    /// (`audioObjectType` 5/29) config; `None` otherwise.
    pub fn extension_sampling_frequency(&self) -> Option<u32> {
        self.with_serialized(|bytes| {
            let mut bit = 0usize;
            let aot = asc_get_aot(bytes, &mut bit)?;
            if aot != AOT_SBR && aot != AOT_PS {
                return None;
            }
            let sfi = asc_read_bits(bytes, &mut bit, SFI_BITS as usize)? as u8;
            if sfi == SFI_ESCAPE {
                asc_read_bits(bytes, &mut bit, SFI_ESCAPE_BITS)?;
            }
            asc_read_bits(bytes, &mut bit, CHANNEL_CONFIG_BITS)?;
            let esfi = asc_read_bits(bytes, &mut bit, SFI_BITS as usize)? as u8;
            if esfi == SFI_ESCAPE {
                return asc_read_bits(bytes, &mut bit, SFI_ESCAPE_BITS);
            }
            SamplingFrequencyIndex::from(esfi).table_hz()
        })
    }
}

fn extension_index_for(hz: u32) -> Option<SamplingFrequencyIndex> {
    (0..=SFI_TABLE_LAST)
        .map(SamplingFrequencyIndex::from)
        .find(|i| i.table_hz() == Some(hz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::Serialize;
    use test_fixtures::bits::BitWriter;
    const KNOWN: [u8; 5] = [0x12, 0x08, 0x56, 0xe5, 0x00];

    fn asc_bits(f: impl FnOnce(&mut BitWriter)) -> Vec<u8> {
        let mut writer = BitWriter::new();
        f(&mut writer);
        writer.finish()
    }

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
        let out = AudioSpecificConfig::parse(&KNOWN)
            .unwrap()
            .try_to_bytes()
            .unwrap();
        assert_eq!(out.as_slice(), KNOWN.as_slice());
    }

    /// r04-W9: serializing into a larger shared buffer must leave the caller's
    /// bytes past `serialized_len()` untouched. Unfixed, `buf.fill(0)` zeroed
    /// the entire slice, so a caller batching several configs into one buffer
    /// lost everything it had already written after this point.
    #[test]
    fn serialize_does_not_zero_bytes_past_need() {
        let asc = AudioSpecificConfig::parse(&KNOWN).unwrap();
        let need = asc.serialized_len();
        let mut buf = alloc::vec![0xAAu8; need + 32];
        let n = asc.serialize_into(&mut buf).unwrap();
        assert_eq!(n, need);
        assert_eq!(&buf[..need], KNOWN.as_slice());
        assert!(
            buf[need..].iter().all(|&b| b == 0xAA),
            "bytes past `need` must be untouched, got {:02x?}",
            &buf[need..]
        );
    }

    /// The same for the Escape (explicit sampling frequency) form, whose
    /// trailing region starts at a different offset.
    #[test]
    fn serialize_escape_form_keeps_the_tail_intact() {
        let known = AudioSpecificConfig::parse(&KNOWN).unwrap();
        let asc = AudioSpecificConfig::with_fields(
            known.audio_object_type,
            SamplingFrequencyIndex::Escape,
            Some(44100),
            known.channel_configuration,
            alloc::vec![0x01, 0x02],
            known.trailing_top,
        );
        let need = asc.serialized_len();
        assert_eq!(need, 7);
        let mut buf = alloc::vec![0x55u8; need + 16];
        asc.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[5..7], &[0x01, 0x02]);
        assert!(buf[need..].iter().all(|&b| b == 0x55));
        // And the written region round-trips byte-exactly.
        let back = AudioSpecificConfig::parse(&buf[..need]).unwrap();
        assert_eq!(
            back.serialize_into(&mut alloc::vec![0u8; need]).unwrap(),
            need
        );
        assert_eq!(back.try_to_bytes().unwrap(), buf[..need]);
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
        let bytes = a.try_to_bytes().unwrap();
        assert!(bytes.len() >= 5);
        let a2 = AudioSpecificConfig::parse(&bytes).unwrap();
        assert_eq!(a2.sampling_frequency_index, SamplingFrequencyIndex::Escape);
        assert_eq!(a2.sampling_frequency, Some(48000));
        assert_eq!(a2.channel_configuration, ChannelConfiguration::Stereo);
    }

    fn escape_config(rate: u32, trailing_top: u8) -> AudioSpecificConfig {
        AudioSpecificConfig::with_fields(
            AudioObjectType::AacLc,
            SamplingFrequencyIndex::Escape,
            Some(rate),
            ChannelConfiguration::Stereo,
            Vec::new(),
            Some(trailing_top),
        )
    }

    /// The 24-bit explicit rate's last bit is bit 7 of byte 4 (ISO/IEC 14496-3
    /// §1.6.2.1), not bit 0: bit 0 there is the `GASpecificConfig`
    /// `extensionFlag` held in `trailing_top`. Reading bit 0 made every odd
    /// rate parse as even (and a set extension flag fabricate an odd rate).
    #[test]
    fn escape_rate_lsb_is_byte4_bit7() {
        for &rate in &[44_101u32, 0x01_FFFF, 0xFF_FFFF, 48_000, 1] {
            for &trailing_top in &[0u8, 7] {
                let asc = escape_config(rate, trailing_top);
                let bytes = asc.try_to_bytes().unwrap();
                let back = AudioSpecificConfig::parse(&bytes).unwrap();
                assert_eq!(
                    back.sampling_frequency,
                    Some(rate),
                    "rate {rate:#x}, trailing_top {trailing_top}"
                );
                assert_eq!(
                    back.trailing_top,
                    Some(trailing_top),
                    "rate {rate:#x}, trailing_top {trailing_top}"
                );
                assert_eq!(
                    back.sampling_frequency_index,
                    SamplingFrequencyIndex::Escape
                );
                assert_eq!(back.channel_configuration, ChannelConfiguration::Stereo);
                assert_eq!(
                    back.try_to_bytes().unwrap(),
                    bytes,
                    "rate {rate:#x}, trailing_top {trailing_top}"
                );
            }
        }
    }

    /// Hand-written escape config: AOT 2 (LC), sfi 15, rate 1, cc 2, and the GA
    /// `extensionFlag` set. Only reading byte 4 bit 7 recovers rate 1.
    #[test]
    fn escape_rate_with_ga_extension_flag_set() {
        let bytes = [0x17u8, 0x80, 0x00, 0x00, 0x91];
        let asc = AudioSpecificConfig::parse(&bytes).unwrap();
        assert_eq!(asc.sampling_frequency, Some(1));
        assert_eq!(asc.trailing_top, Some(1));
        assert_eq!(asc.channel_configuration, ChannelConfiguration::Stereo);
        assert_eq!(asc.try_to_bytes().unwrap(), bytes);
    }

    /// The top 7 bits of the explicit rate (above 131071) are the last field
    /// packed into byte 1 (ISO/IEC 14496-3 §1.6.2.1): 192000 and the 24-bit
    /// maximum must survive both directions, and the bytes are pinned to the
    /// spec layout so parse and serializer are checked against an independent
    /// value.
    #[test]
    fn escape_rate_above_131071_bits() {
        let asc = escape_config(192_000, 0);
        assert_eq!(asc.try_to_bytes().unwrap(), [0x17, 0x81, 0x77, 0x00, 0x10]);
        for &rate in &[192_000u32, 0x7F_FFFF] {
            let asc = escape_config(rate, 0);
            let bytes = asc.try_to_bytes().unwrap();
            assert_eq!(
                AudioSpecificConfig::parse(&bytes)
                    .unwrap()
                    .sampling_frequency,
                Some(rate),
                "rate {rate:#x}"
            );
        }
    }
    #[test]
    fn mutate_config_changes_bytes() {
        let o = AudioSpecificConfig::parse(&KNOWN)
            .unwrap()
            .try_to_bytes()
            .unwrap();
        let mut b = AudioSpecificConfig::parse(&KNOWN).unwrap();
        b.channel_configuration = ChannelConfiguration::Stereo;
        assert_ne!(o, b.try_to_bytes().unwrap());
    }

    /// Every public spec label (ISO/IEC 14496-3 Tables 1.1, 1.10, 1.11) is
    /// pinned; `Display` is `name()`, with a reserved value rendered as
    /// `reserved(0x..)`.
    #[test]
    fn audio_object_type_labels() {
        let cases: &[(AudioObjectType, &str, &str)] = &[
            (AudioObjectType::Null, "null", "null"),
            (AudioObjectType::AacMain, "AAC Main", "AAC Main"),
            (AudioObjectType::AacLc, "AAC LC", "AAC LC"),
            (AudioObjectType::AacSsr, "AAC SSR", "AAC SSR"),
            (AudioObjectType::AacLtp, "AAC LTP", "AAC LTP"),
            (AudioObjectType::Reserved(5), "reserved", "reserved(0x05)"),
            (AudioObjectType::AacScalable, "AAC Scalable", "AAC Scalable"),
            (AudioObjectType::TwinVq, "TwinVQ", "TwinVQ"),
            (AudioObjectType::Celp, "CELP", "CELP"),
            (AudioObjectType::Hvxc, "HVXC", "HVXC"),
            (AudioObjectType::Ttsi, "TTSI", "TTSI"),
            (
                AudioObjectType::MainSynthetic,
                "Main synthetic",
                "Main synthetic",
            ),
            (
                AudioObjectType::WavetableSynthesis,
                "Wavetable synthesis",
                "Wavetable synthesis",
            ),
            (AudioObjectType::GeneralMidi, "General MIDI", "General MIDI"),
            (
                AudioObjectType::AlgorithmicSynthesis,
                "Algorithmic Synthesis & Audio FX",
                "Algorithmic Synthesis & Audio FX",
            ),
            (AudioObjectType::ErAacLc, "ER AAC LC", "ER AAC LC"),
            (AudioObjectType::ErAacLtp, "ER AAC LTP", "ER AAC LTP"),
            (
                AudioObjectType::ErAacScalable,
                "ER AAC Scalable",
                "ER AAC Scalable",
            ),
            (AudioObjectType::ErTwinVq, "ER TwinVQ", "ER TwinVQ"),
            (AudioObjectType::ErBsac, "ER BSAC", "ER BSAC"),
            (AudioObjectType::ErAacLd, "ER AAC LD", "ER AAC LD"),
            (AudioObjectType::ErCelp, "ER CELP", "ER CELP"),
            (AudioObjectType::ErHvxc, "ER HVXC", "ER HVXC"),
            (AudioObjectType::ErHiln, "ER HILN", "ER HILN"),
            (
                AudioObjectType::ErParametric,
                "ER Parametric",
                "ER Parametric",
            ),
        ];
        for &(value, name, display) in cases {
            assert_eq!(value.name(), name, "{value:?}");
            assert_eq!(alloc::format!("{value}"), display, "{value:?}");
        }
    }

    #[test]
    fn sampling_frequency_index_labels() {
        let cases: &[(SamplingFrequencyIndex, &str, &str)] = &[
            (SamplingFrequencyIndex::Fs96000, "96000 Hz", "96000 Hz"),
            (SamplingFrequencyIndex::Fs88200, "88200 Hz", "88200 Hz"),
            (SamplingFrequencyIndex::Fs64000, "64000 Hz", "64000 Hz"),
            (SamplingFrequencyIndex::Fs48000, "48000 Hz", "48000 Hz"),
            (SamplingFrequencyIndex::Fs44100, "44100 Hz", "44100 Hz"),
            (SamplingFrequencyIndex::Fs32000, "32000 Hz", "32000 Hz"),
            (SamplingFrequencyIndex::Fs24000, "24000 Hz", "24000 Hz"),
            (SamplingFrequencyIndex::Fs22050, "22050 Hz", "22050 Hz"),
            (SamplingFrequencyIndex::Fs16000, "16000 Hz", "16000 Hz"),
            (SamplingFrequencyIndex::Fs12000, "12000 Hz", "12000 Hz"),
            (SamplingFrequencyIndex::Fs11025, "11025 Hz", "11025 Hz"),
            (SamplingFrequencyIndex::Fs8000, "8000 Hz", "8000 Hz"),
            (SamplingFrequencyIndex::Fs7350, "7350 Hz", "7350 Hz"),
            (
                SamplingFrequencyIndex::Reserved(13),
                "reserved",
                "reserved(0x0D)",
            ),
            (SamplingFrequencyIndex::Escape, "escape", "escape"),
        ];
        for &(value, name, display) in cases {
            assert_eq!(value.name(), name, "{value:?}");
            assert_eq!(alloc::format!("{value}"), display, "{value:?}");
        }
    }

    #[test]
    fn channel_configuration_labels() {
        let cases: &[(ChannelConfiguration, &str, &str)] = &[
            (ChannelConfiguration::InBand, "in-band", "in-band"),
            (ChannelConfiguration::Mono, "mono", "mono"),
            (ChannelConfiguration::Stereo, "stereo", "stereo"),
            (ChannelConfiguration::Ch3, "3 channels", "3 channels"),
            (ChannelConfiguration::Ch4, "4 channels", "4 channels"),
            (ChannelConfiguration::Ch5, "5 channels", "5 channels"),
            (ChannelConfiguration::Ch5_1, "5.1 surround", "5.1 surround"),
            (ChannelConfiguration::Ch7_1, "7.1 surround", "7.1 surround"),
            (
                ChannelConfiguration::Reserved(9),
                "reserved",
                "reserved(0x09)",
            ),
        ];
        for &(value, name, display) in cases {
            assert_eq!(value.name(), name, "{value:?}");
            assert_eq!(alloc::format!("{value}"), display, "{value:?}");
        }
    }

    /// Every 5-bit object type maps to its Table 1.1 variant and back.
    #[test]
    fn audio_object_type_from_and_raw_cover_every_value() {
        let expected: [AudioObjectType; 32] = [
            AudioObjectType::Null,
            AudioObjectType::AacMain,
            AudioObjectType::AacLc,
            AudioObjectType::AacSsr,
            AudioObjectType::AacLtp,
            AudioObjectType::Reserved(5),
            AudioObjectType::AacScalable,
            AudioObjectType::TwinVq,
            AudioObjectType::Celp,
            AudioObjectType::Hvxc,
            AudioObjectType::Reserved(10),
            AudioObjectType::Reserved(11),
            AudioObjectType::Ttsi,
            AudioObjectType::MainSynthetic,
            AudioObjectType::WavetableSynthesis,
            AudioObjectType::GeneralMidi,
            AudioObjectType::AlgorithmicSynthesis,
            AudioObjectType::ErAacLc,
            AudioObjectType::Reserved(18),
            AudioObjectType::ErAacLtp,
            AudioObjectType::ErAacScalable,
            AudioObjectType::ErTwinVq,
            AudioObjectType::ErBsac,
            AudioObjectType::ErAacLd,
            AudioObjectType::ErCelp,
            AudioObjectType::ErHvxc,
            AudioObjectType::ErHiln,
            AudioObjectType::ErParametric,
            AudioObjectType::Reserved(28),
            AudioObjectType::Reserved(29),
            AudioObjectType::Reserved(30),
            AudioObjectType::Reserved(31),
        ];
        for (n, &want) in expected.iter().enumerate() {
            let n = n as u8;
            assert_eq!(AudioObjectType::from(n), want, "AOT {n}");
            assert_eq!(AudioObjectType::from(n).raw(), n, "AOT {n}");
            assert_eq!(AudioObjectType::from(n + 32), want, "AOT {:#04x}", n + 32);
        }
        assert_eq!(AudioObjectType::Reserved(13).raw(), 13);
        assert_eq!(AudioObjectType::Reserved(0x3D).raw(), 0x1D);
        assert_eq!(AudioObjectType::from(0x2A), AudioObjectType::Reserved(0x0A));
    }

    /// Every 4-bit sampling-frequency index maps to its Table 1.10 variant and
    /// back, including the reserved values and the explicit-rate escape.
    #[test]
    fn sampling_frequency_index_from_and_raw_cover_every_value() {
        let expected: [SamplingFrequencyIndex; 16] = [
            SamplingFrequencyIndex::Fs96000,
            SamplingFrequencyIndex::Fs88200,
            SamplingFrequencyIndex::Fs64000,
            SamplingFrequencyIndex::Fs48000,
            SamplingFrequencyIndex::Fs44100,
            SamplingFrequencyIndex::Fs32000,
            SamplingFrequencyIndex::Fs24000,
            SamplingFrequencyIndex::Fs22050,
            SamplingFrequencyIndex::Fs16000,
            SamplingFrequencyIndex::Fs12000,
            SamplingFrequencyIndex::Fs11025,
            SamplingFrequencyIndex::Fs8000,
            SamplingFrequencyIndex::Fs7350,
            SamplingFrequencyIndex::Reserved(13),
            SamplingFrequencyIndex::Reserved(14),
            SamplingFrequencyIndex::Escape,
        ];
        for (n, &want) in expected.iter().enumerate() {
            let n = n as u8;
            assert_eq!(SamplingFrequencyIndex::from(n), want, "SFI {n}");
            assert_eq!(SamplingFrequencyIndex::from(n).raw(), n, "SFI {n}");
            assert_eq!(
                SamplingFrequencyIndex::from(n + 16),
                want,
                "SFI {:#04x}",
                n + 16
            );
        }
        assert_eq!(SamplingFrequencyIndex::Reserved(0x1D).raw(), 0x0D);
    }

    /// Every 4-bit channel configuration maps to its Table 1.11 variant and
    /// back; 8..=15 are reserved.
    #[test]
    fn channel_configuration_from_and_raw_cover_every_value() {
        let expected: [ChannelConfiguration; 8] = [
            ChannelConfiguration::InBand,
            ChannelConfiguration::Mono,
            ChannelConfiguration::Stereo,
            ChannelConfiguration::Ch3,
            ChannelConfiguration::Ch4,
            ChannelConfiguration::Ch5,
            ChannelConfiguration::Ch5_1,
            ChannelConfiguration::Ch7_1,
        ];
        for (n, &want) in expected.iter().enumerate() {
            let n = n as u8;
            assert_eq!(ChannelConfiguration::from(n), want, "CC {n}");
            assert_eq!(ChannelConfiguration::from(n).raw(), n, "CC {n}");
            assert_eq!(
                ChannelConfiguration::from(n + 16),
                want,
                "CC {:#04x}",
                n + 16
            );
        }
        for n in 8u8..=15 {
            assert_eq!(
                ChannelConfiguration::from(n),
                ChannelConfiguration::Reserved(n),
                "CC {n}"
            );
            assert_eq!(ChannelConfiguration::from(n).raw(), n, "CC {n}");
            assert_eq!(
                ChannelConfiguration::from(n + 16),
                ChannelConfiguration::Reserved(n),
                "CC {:#04x}",
                n + 16
            );
        }
        assert_eq!(ChannelConfiguration::Reserved(0x1D).raw(), 0x0D);
    }

    /// AAC-LC 44100 stereo followed by the GA bits and a 0x2B7 SBR sync
    /// extension with `sbrPresentFlag == 1` (implicit HE-AAC v1) has an
    /// explicit SBR signal, so the core 1024-sample frame is not the
    /// access-unit span.
    #[test]
    fn samples_per_frame_none_with_sync_extension_sbr() {
        let bytes = asc_bits(|w| {
            w.u(5, 2).u(4, 4).u(4, 2).u(3, 0);
            w.u(11, 0x2B7).u(5, 5).u(1, 1);
        });
        assert_eq!(bytes.len(), 5);
        let asc = AudioSpecificConfig::parse(&bytes).unwrap();
        let signaling = asc.heaac_signaling();
        assert!(signaling.sbr_present);
        assert!(!signaling.ps_present);
        assert_eq!(asc.samples_per_frame(), None);
    }

    /// Without an extension the core frame length applies: 1024 by default,
    /// 960 when `frameLengthFlag` (bit 2 of the shared byte) is set.
    #[test]
    fn samples_per_frame_reads_the_frame_length_flag() {
        let plain = asc_bits(|w| {
            w.u(5, 2).u(4, 4).u(4, 2).u(3, 0);
        });
        assert_eq!(plain.len(), 2);
        assert_eq!(
            AudioSpecificConfig::parse(&plain)
                .unwrap()
                .samples_per_frame(),
            Some(1024)
        );
        let long = asc_bits(|w| {
            w.u(5, 2).u(4, 4).u(4, 2).u(3, 4);
        });
        let asc = AudioSpecificConfig::parse(&long).unwrap();
        assert_eq!(asc.trailing_top, Some(4));
        assert_eq!(asc.samples_per_frame(), Some(960));
    }

    /// `GetAudioObjectType()`: an object-type field of 31 is followed by six
    /// more bits and the value is `32 + ext` (ISO/IEC 14496-3 §1.5.1.1).
    /// `[0xF9, 0x50]` is field 31, ext `001010` = 10, so AOT 42 (USAC).
    #[test]
    fn get_audio_object_type_escape_is_32_plus_six_bits() {
        let mut bit = 0usize;
        assert_eq!(asc_get_aot(&[0xF9, 0x50, 0x00], &mut bit), Some(42));
        assert_eq!(bit, 11);
        let asc = AudioSpecificConfig::parse(&[0xF9, 0x50, 0x00]).unwrap();
        assert_eq!(asc.heaac_signaling().effective_aot, 42);
        let err = asc.to_adts_header(200).unwrap_err();
        assert!(
            matches!(err, crate::error::Error::InvalidValue(v) if v.field == "audio_object_type")
        );
    }

    /// The backward-compatible sync extension is only read for an AAC-LC
    /// core: AAC Main followed by bytes that spell 0x2B7 is not SBR.
    #[test]
    fn sync_extension_ignored_for_non_lc_core() {
        let bytes = asc_bits(|w| {
            w.u(5, 1).u(4, 4).u(4, 2).u(3, 0);
            w.u(11, 0x2B7).u(5, 5).u(1, 1);
        });
        let asc = AudioSpecificConfig::parse(&bytes).unwrap();
        assert_eq!(asc.audio_object_type, AudioObjectType::AacMain);
        let signaling = asc.heaac_signaling();
        assert!(!signaling.sbr_present);
        assert!(!signaling.ps_present);
        assert_eq!(signaling.effective_aot, 1);
    }

    /// The SBR sync extension's own sampling-frequency index may be the escape
    /// form; the 24-bit explicit extension rate must be skipped so the
    /// following 0x548 PS sync is aligned and decoded.
    #[test]
    fn sync_extension_escape_rate_keeps_ps_aligned() {
        let bytes = asc_bits(|w| {
            w.u(5, 2).u(4, 4).u(4, 2).u(3, 0);
            w.u(11, 0x2B7).u(5, 5).u(1, 1);
            w.u(4, 0xF).u(24, 44_101);
            w.u(11, 0x548).u(1, 1);
        });
        let asc = AudioSpecificConfig::parse(&bytes).unwrap();
        let signaling = asc.heaac_signaling();
        assert!(signaling.sbr_present);
        assert!(signaling.ps_present);
        assert_eq!(signaling.effective_aot, 29);
    }

    /// A bare 0x548 sync after the GA bits signals PS without SBR, and its
    /// one-bit `psPresentFlag` is honoured.
    #[test]
    fn ps_only_sync_extension_flag() {
        for &(flag, want) in &[(1u64, true), (0, false)] {
            let bytes = asc_bits(|w| {
                w.u(5, 2).u(4, 4).u(4, 2).u(3, 0);
                w.u(11, 0x548).u(1, flag);
            });
            let signaling = AudioSpecificConfig::parse(&bytes)
                .unwrap()
                .heaac_signaling();
            assert_eq!(signaling.ps_present, want, "flag {flag}");
            assert!(!signaling.sbr_present, "flag {flag}");
        }
    }

    /// `frame_length` is a 13-bit field: frames above 2047 must survive both
    /// the builder and the parser, and the 8191-byte maximum is pinned.
    #[test]
    fn adts_frame_length_top_bits() {
        for &len in &[2047u16, 2048, 4095, 8191] {
            let parsed = parse_adts_header(&build_adts_header(1, 4, 2, len)).unwrap();
            assert_eq!(parsed.frame_length, len, "frame_length {len}");
        }
        assert_eq!(
            build_adts_header(1, 4, 2, 8191),
            [0xFF, 0xF1, 0x50, 0x83, 0xFF, 0xFF, 0xFC]
        );
    }

    /// `adts_buffer_fullness` occupies the low 5 bits of byte 5 and the top
    /// 6 bits of byte 6; 0x7FF means VBR.
    #[test]
    fn adts_buffer_fullness_bits() {
        let hdr = [0xFFu8, 0xF1, 0x50, 0x80, 0x19, 0x0A, 0x96];
        let parsed = parse_adts_header(&hdr).unwrap();
        assert_eq!(parsed.frame_length, 200);
        assert_eq!(parsed.buffer_fullness, 0x2A5);
        assert_eq!(parsed.num_raw_data_blocks, 2);
        let built = parse_adts_header(&build_adts_header(1, 4, 2, 200)).unwrap();
        assert_eq!(built.buffer_fullness, 0x7FF);
    }

    /// `to_adts_header` accepts exactly the 13-bit maximum and refuses one past
    /// it, so the field cannot wrap.
    #[test]
    fn to_adts_header_frame_len_upper_bound() {
        let asc = AudioSpecificConfig::parse(&KNOWN).unwrap();
        assert_eq!(
            parse_adts_header(&asc.to_adts_header(8191).unwrap())
                .unwrap()
                .frame_length,
            8191
        );
        for len in [8192u16, u16::MAX] {
            let err = asc.to_adts_header(len).unwrap_err();
            assert!(
                matches!(err, crate::error::Error::InvalidValue(v) if v.field == "frame_len"),
                "frame_len {len}"
            );
        }
    }
}
