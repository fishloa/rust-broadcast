//! AVS3 (and the shared AVS2/AVS3 temporal-layer) configuration boxes in
//! ISOBMFF — GY/T 420-2025 Annex A.3.2 and A.3.4 (A.2.2 / A.2.3.4 for AVS2,
//! see [`crate::avs2`]).
//!
//! | Type | FourCC | Spec |
//! |------|--------|------|
//! | [`Avs3ConfigurationBox`] | `av3c` | A.3.2.2 / A.3.2.4 `Avs3DecoderConfigurationRecord` |
//! | [`LAvsConfigurationBox`] | `lavc` (AVS3), `la2c` (AVS2) | A.3.2.3 / A.3.2.5 `LAvs3DecoderConfigurationRecord` |
//! | [`TemporalLayerEntry`] | `telg` sample group | A.3.4.3 |
//! | [`VisualLrapEntry`] | `lrap` sample group | A.3.4.1 |
//! | [`LibrarySampleGroupEntry`] | `a3lg` sample group | A.3.4.2 |
//!
//! The AVS video sample entry that carries these boxes is in this module ([`AvsVideoSampleEntry`]).
//!
//! Sources: GY/T 420-2025 (T/AI 109.6-2022 clause 5 carries the same syntax).
//! The embedded `sequence_header()` (GY/T 368-2023 for AVS3, GY/T 299.1-2016
//! for AVS2) is preserved verbatim, not decoded. Not modelled: the `lidx`
//! LRAP segment index box (A.3.3.6, an extension of `sidx`) and the `avs3`
//! restricted-scheme wrapper (`resv` + `av3l`, A.3.3.3).
//!
//! Reserved bits: the spec fixes them ('1' runs in the configuration records,
//! `0` in the sample groups); they are captured on parse and re-emitted verbatim,
//! hand-built values use the spec value.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::error::{BufferTooShort, InvalidValue, OutputBufferTooSmall};
use broadcast_common::len::{fit_bits, fit_u16, need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// FourCC of the AVS3 configuration box.
pub const AV3C_FOURCC: [u8; 4] = *b"av3c";
/// FourCC of the LAVS3 (temporal layer) configuration box.
pub const LAVC_FOURCC: [u8; 4] = *b"lavc";
/// `grouping_type` of the temporal-layer sample group (T/AI 109.6, `TemporalLayerEntry`).
pub const TELG_FOURCC: [u8; 4] = *b"telg";
/// `grouping_type` of the library random access point sample group (T/AI 109.6,
/// `VisualLRAPEntry`).
pub const LRAP_FOURCC: [u8; 4] = *b"lrap";
/// `grouping_type` of the library-picture sample group (T/AI 109.6,
/// `LibrarySampleGroupEntry`).
pub const A3LG_FOURCC: [u8; 4] = *b"a3lg";
const CONFIGURATION_VERSION: u8 = 1;
const RESERVED6_ONES: u8 = 0x3F;
const RESERVED1_BITS: u8 = 1;
const RESERVED2_BITS: u8 = 0b11;
const LAYER_LEN: usize = 5;

/// `configurationVersion`(1) + `sequence_header_length`(2) (GY/T 420-2025 A.3.2.4).
const CONFIG_HEADER_LEN: usize = 3;
/// `configurationVersion`(1) + `number_of_layers`(1) (A.3.2.5).
const LAYER_RECORD_HEADER_LEN: usize = 2;
/// Offset of the bit-rate tail inside a temporal-layer record.
const LAYER_TAIL_OFFSET: usize = 1;
/// Offset of the `LRAP_type`/`entry_count` byte in a `lrap` entry.
const LRAP_HEADER_LEN: usize = 1;
/// Width of one `LibrarySampleRef` in a `lrap` entry.
const LIBRARY_SAMPLE_LEN: usize = 2;

/// Width of the reserved run in a configuration record's trailing byte.
const RESERVED6_BITS: u32 = 6;
/// Mask and shift of `library_dependency_idc` in that byte.
const LIBRARY_DEPENDENCY_MASK: u8 = 0x03;
const RESERVED_TAIL_SHIFT: u32 = 2;

/// Bit layout of a temporal-layer record.
const TEMPORAL_LAYER_ID_SHIFT: u32 = 5;
const TEMPORAL_LAYER_ID_BITS: u32 = 3;
const FRAME_RATE_CODE_SHIFT: u32 = 1;
const FRAME_RATE_CODE_MASK: u8 = 0x0F;
const FRAME_RATE_CODE_BITS: u32 = 4;
const RESERVED_A_MASK: u8 = 1;
const BIT_RATE_LOWER_SHIFT: u32 = 14;
const BIT_RATE_LOWER_BITS: u32 = 18;
const BIT_RATE_UPPER_SHIFT: u32 = 2;
const BIT_RATE_UPPER_MASK: u32 = 0xFFF;
const BIT_RATE_UPPER_BITS: u32 = 12;
const RESERVED_B_MASK: u32 = 0x03;
const RESERVED_B_BITS: u32 = 2;

/// Bit layout of a `lrap` entry.
const LRAP_TYPE_SHIFT: u32 = 5;
const LRAP_TYPE_BITS: u32 = 3;
const LRAP_ENTRY_COUNT_SHIFT: u32 = 2;
const LRAP_ENTRY_COUNT_MASK: u8 = 0x07;
const LRAP_ENTRY_COUNT_BITS: u32 = 3;
const LRAP_RESERVED_BITS: u32 = 2;
const LIBRARY_SAMPLE_NUMBER_SHIFT: u32 = 7;
const LIBRARY_SAMPLE_NUMBER_BITS: u32 = 9;
const LIBRARY_SAMPLE_RESERVED_MASK: u16 = 0x7F;
const LIBRARY_SAMPLE_RESERVED_BITS: u32 = 7;

/// `library_dependency_idc` (2 bits) — GY/T 420-2025 Table A.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LibraryDependencyIdc {
    /// `00` — main elementary stream that does not reference library pictures.
    MainWithoutLibrary,
    /// `01` — main elementary stream that references library pictures.
    MainWithLibrary,
    /// `10` — library elementary stream.
    Library,
    /// `11` — the track holds both the main and the library stream, and the
    /// main stream references the library pictures.
    MainAndLibrary,
}

impl LibraryDependencyIdc {
    /// Decode from the 2-bit wire value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v & LIBRARY_DEPENDENCY_MASK {
            0 => Self::MainWithoutLibrary,
            1 => Self::MainWithLibrary,
            2 => Self::Library,
            _ => Self::MainAndLibrary,
        }
    }

    /// The 2-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::MainWithoutLibrary => 0,
            Self::MainWithLibrary => 1,
            Self::Library => 2,
            Self::MainAndLibrary => 3,
        }
    }

    /// Spec label (Table A.1).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::MainWithoutLibrary => "main stream, no library references",
            Self::MainWithLibrary => "main stream referencing library pictures",
            Self::Library => "library stream",
            Self::MainAndLibrary => "main and library stream in one track",
        }
    }
}
broadcast_common::impl_spec_display!(LibraryDependencyIdc);

/// `frame_rate_code` (4 bits) of the AVS2/AVS3 specifications (GY/T 420-2025
/// Table 7 / Table 4: the frame rate the code stands for).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AvsFrameRateCode {
    /// `0b0000` — forbidden.
    Forbidden,
    /// `0b0001` — 23.976 Hz.
    Fps23_976,
    /// `0b0010` — 24 Hz.
    Fps24,
    /// `0b0011` — 25 Hz.
    Fps25,
    /// `0b0100` — 29.97 Hz.
    Fps29_97,
    /// `0b0101` — 30 Hz.
    Fps30,
    /// `0b0110` — 50 Hz.
    Fps50,
    /// `0b0111` — 59.94 Hz.
    Fps59_94,
    /// `0b1000` — 60 Hz.
    Fps60,
    /// `0b1001` — 100 Hz.
    Fps100,
    /// `0b1010` — 120 Hz.
    Fps120,
    /// `0b1011..=0b1111` — reserved; carries the value.
    Reserved(u8),
}

impl AvsFrameRateCode {
    /// Decode from the 4-bit wire value (lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::Forbidden,
            1 => Self::Fps23_976,
            2 => Self::Fps24,
            3 => Self::Fps25,
            4 => Self::Fps29_97,
            5 => Self::Fps30,
            6 => Self::Fps50,
            7 => Self::Fps59_94,
            8 => Self::Fps60,
            9 => Self::Fps100,
            10 => Self::Fps120,
            other => Self::Reserved(other),
        }
    }

    /// The 4-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Forbidden => 0,
            Self::Fps23_976 => 1,
            Self::Fps24 => 2,
            Self::Fps25 => 3,
            Self::Fps29_97 => 4,
            Self::Fps30 => 5,
            Self::Fps50 => 6,
            Self::Fps59_94 => 7,
            Self::Fps60 => 8,
            Self::Fps100 => 9,
            Self::Fps120 => 10,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (frame rate in Hz).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Fps23_976 => "23.976 Hz",
            Self::Fps24 => "24 Hz",
            Self::Fps25 => "25 Hz",
            Self::Fps29_97 => "29.97 Hz",
            Self::Fps30 => "30 Hz",
            Self::Fps50 => "50 Hz",
            Self::Fps59_94 => "59.94 Hz",
            Self::Fps60 => "60 Hz",
            Self::Fps100 => "100 Hz",
            Self::Fps120 => "120 Hz",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(AvsFrameRateCode, Reserved);

/// `Avs3DecoderConfigurationRecord` — the body of the `av3c` box (A.3.2.2).
///
/// ```text
/// configurationVersion(8)=1 sequence_header_length(16) sequence_header[]
/// reserved(6)='111111' library_dependency_idc(2)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3ConfigurationBox {
    /// `configurationVersion` (8 bits; 1).
    pub configuration_version: u8,
    /// `sequence_header`: the `sequence_header()` of GY/T 368-2023, verbatim
    /// (its length is `sequence_header_length`).
    pub sequence_header: Vec<u8>,
    /// `reserved` (6 bits, right-aligned; '111111').
    pub reserved: u8,
    /// `library_dependency_idc` (2 bits, Table A.1).
    pub library_dependency_idc: LibraryDependencyIdc,
}

impl Avs3ConfigurationBox {
    /// Build a record (version 1, all-ones reserved).
    #[must_use]
    pub fn new(sequence_header: Vec<u8>, library_dependency_idc: LibraryDependencyIdc) -> Self {
        Self {
            configuration_version: CONFIGURATION_VERSION,
            sequence_header,
            reserved: RESERVED6_ONES,
            library_dependency_idc,
        }
    }
}

impl<'a> Parse<'a> for Avs3ConfigurationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, header) = parse_sequence_header_record(bytes, "av3c body", 1)?;
        let tail = bytes[CONFIG_HEADER_LEN + header.len()];
        Ok(Self {
            configuration_version: version,
            sequence_header: header,
            reserved: tail >> RESERVED_TAIL_SHIFT,
            library_dependency_idc: LibraryDependencyIdc::from_u8(tail & LIBRARY_DEPENDENCY_MASK),
        })
    }
}

/// Shared by `av3c` (one trailing byte) and `av2c` (none): version(8),
/// sequence_header_length(16), sequence_header, then `trailing` further bytes
/// must exist. Returns the version and the owned sequence header.
pub(crate) fn parse_sequence_header_record(
    bytes: &[u8],
    what: &'static str,
    trailing: usize,
) -> Result<(u8, Vec<u8>)> {
    if bytes.len() < CONFIG_HEADER_LEN {
        return Err(Error::BufferTooShort(BufferTooShort {
            need: CONFIG_HEADER_LEN,
            have: bytes.len(),
            what,
        }));
    }
    let mut cursor = 1;
    let len = usize::from(broadcast_common::wire::read_u16(bytes, &mut cursor, what)?);
    let need = CONFIG_HEADER_LEN + len + trailing;
    if bytes.len() < need {
        return Err(Error::BufferTooShort(BufferTooShort {
            need,
            have: bytes.len(),
            what,
        }));
    }
    reject_trailing(bytes.len() - need, what)?;
    Ok((
        bytes[0],
        bytes[CONFIG_HEADER_LEN..CONFIG_HEADER_LEN + len].to_vec(),
    ))
}

/// The record is fully modelled (no reserved tail the typed fields drop), so
/// bytes past it cannot be re-emitted by the serializer: a parse -> serialize
/// would silently shorten the box (the workspace round-trip invariant). Reject
/// them instead of discarding them.
fn reject_trailing(extra: usize, what: &'static str) -> Result<()> {
    if extra == 0 {
        Ok(())
    } else {
        Err(Error::InvalidValue(InvalidValue {
            field: what,
            value: extra as u64,
            reason: "trailing bytes after the configuration record",
        }))
    }
}

impl Serialize for Avs3ConfigurationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        CONFIG_HEADER_LEN + self.sequence_header.len() + 1
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let len = fit_u16(self.sequence_header.len(), "sequence_header_length")?;
        let reserved = fit_bits(u64::from(self.reserved), RESERVED6_BITS, "reserved")? as u8;
        buf[0] = self.configuration_version;
        buf[1..CONFIG_HEADER_LEN].copy_from_slice(&len.to_be_bytes());
        buf[CONFIG_HEADER_LEN..CONFIG_HEADER_LEN + self.sequence_header.len()]
            .copy_from_slice(&self.sequence_header);
        buf[need - 1] = (reserved << RESERVED_TAIL_SHIFT) | self.library_dependency_idc.to_u8();
        Ok(need)
    }
}

/// One temporal layer of a `LAvs3DecoderConfigurationRecord` /
/// `LAvs2DecoderConfigurationRecord` (5 bytes, A.2.2.3 / A.3.2.3).
///
/// ```text
/// temporal_layer_id(3) frame_rate_code(4) reserved(1)='1'
/// temporal_bit_rate_lower(18) temporal_bit_rate_upper(12) reserved(2)='11'
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TemporalLayerInfo {
    /// `temporal_layer_id` (3 bits).
    pub temporal_layer_id: u8,
    /// `frame_rate_code` (4 bits): frame rate accumulating levels up to this one.
    pub frame_rate_code: AvsFrameRateCode,
    /// `reserved` (1 bit; '1') after `frame_rate_code`.
    pub reserved_a: u8,
    /// `temporal_bit_rate_lower` (18 bits): low 18 bits of the bit rate.
    pub temporal_bit_rate_lower: u32,
    /// `temporal_bit_rate_upper` (12 bits): high 12 bits of the bit rate.
    pub temporal_bit_rate_upper: u16,
    /// `reserved` (2 bits; '11') at the end.
    pub reserved_b: u8,
}

impl TemporalLayerInfo {
    /// Build with the spec's reserved values.
    #[must_use]
    pub const fn new(
        temporal_layer_id: u8,
        frame_rate_code: AvsFrameRateCode,
        temporal_bit_rate_lower: u32,
        temporal_bit_rate_upper: u16,
    ) -> Self {
        Self {
            temporal_layer_id,
            frame_rate_code,
            reserved_a: RESERVED1_BITS,
            temporal_bit_rate_lower,
            temporal_bit_rate_upper,
            reserved_b: RESERVED2_BITS,
        }
    }

    /// The 30-bit bit rate: `upper << 18 | lower`.
    #[must_use]
    pub const fn bit_rate(&self) -> u32 {
        ((self.temporal_bit_rate_upper as u32) << 18) | self.temporal_bit_rate_lower
    }
}

/// `LAvs3DecoderConfigurationRecord` / `LAvs2DecoderConfigurationRecord` — the
/// body of a `lavc` / `la2c` box (identical layouts, A.3.2.3 / A.2.2.3):
/// `configurationVersion(8)=1 num_temporal_layers(8)` + one
/// [`TemporalLayerInfo`] per layer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LAvsConfigurationBox {
    /// `configurationVersion` (8 bits; 1).
    pub configuration_version: u8,
    /// The temporal layers; `num_temporal_layers` is their count.
    pub layers: Vec<TemporalLayerInfo>,
}

impl LAvsConfigurationBox {
    /// Build a record (version 1).
    #[must_use]
    pub fn new(layers: Vec<TemporalLayerInfo>) -> Self {
        Self {
            configuration_version: CONFIGURATION_VERSION,
            layers,
        }
    }
}

impl<'a> Parse<'a> for LAvsConfigurationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, LAYER_RECORD_HEADER_LEN, "lavc/la2c body")?;
        let n = usize::from(bytes[1]);
        let need = LAYER_RECORD_HEADER_LEN + n * LAYER_LEN;
        need_in(bytes, need, "lavc/la2c layers")?;
        reject_trailing(bytes.len() - need, "lavc/la2c")?;
        let layers = bytes[LAYER_RECORD_HEADER_LEN..need]
            .chunks_exact(LAYER_LEN)
            .map(|c| {
                let mut cursor = LAYER_TAIL_OFFSET;
                let tail = broadcast_common::wire::read_u32(c, &mut cursor, "temporal layer tail")?;
                Ok(TemporalLayerInfo {
                    temporal_layer_id: c[0] >> TEMPORAL_LAYER_ID_SHIFT,
                    frame_rate_code: AvsFrameRateCode::from_u8(
                        (c[0] >> FRAME_RATE_CODE_SHIFT) & FRAME_RATE_CODE_MASK,
                    ),
                    reserved_a: c[0] & RESERVED_A_MASK,
                    temporal_bit_rate_lower: tail >> BIT_RATE_LOWER_SHIFT,
                    temporal_bit_rate_upper: ((tail >> BIT_RATE_UPPER_SHIFT) & BIT_RATE_UPPER_MASK)
                        as u16,
                    reserved_b: (tail & RESERVED_B_MASK) as u8,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            configuration_version: bytes[0],
            layers,
        })
    }
}

impl Serialize for LAvsConfigurationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        LAYER_RECORD_HEADER_LEN + self.layers.len() * LAYER_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0] = self.configuration_version;
        buf[1] = broadcast_common::len::fit_u8(self.layers.len(), "num_temporal_layers")?;
        let mut i = LAYER_RECORD_HEADER_LEN;
        for l in &self.layers {
            let id = fit_bits(
                u64::from(l.temporal_layer_id),
                TEMPORAL_LAYER_ID_BITS,
                "temporal_layer_id",
            )? as u8;
            let frc = fit_bits(
                u64::from(l.frame_rate_code.to_u8()),
                FRAME_RATE_CODE_BITS,
                "frame_rate_code",
            )? as u8;
            let ra = fit_bits(u64::from(l.reserved_a), 1, "reserved")? as u8;
            let lo = fit_bits(
                u64::from(l.temporal_bit_rate_lower),
                BIT_RATE_LOWER_BITS,
                "temporal_bit_rate_lower",
            )? as u32;
            let hi = fit_bits(
                u64::from(l.temporal_bit_rate_upper),
                BIT_RATE_UPPER_BITS,
                "temporal_bit_rate_upper",
            )? as u32;
            let rb = fit_bits(u64::from(l.reserved_b), RESERVED_B_BITS, "reserved")? as u32;
            buf[i] = (id << TEMPORAL_LAYER_ID_SHIFT) | (frc << FRAME_RATE_CODE_SHIFT) | ra;
            buf[i + LAYER_TAIL_OFFSET..i + LAYER_LEN].copy_from_slice(
                &((lo << BIT_RATE_LOWER_SHIFT) | (hi << BIT_RATE_UPPER_SHIFT) | rb).to_be_bytes(),
            );
            i += LAYER_LEN;
        }
        Ok(need)
    }
}

/// `telg` temporal-level sample group entry (A.3.4.3 / A.2.3.4): one byte,
/// `unsigned int(8) temporal_layer_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TemporalLayerEntry {
    /// `temporal_layer_id` (8 bits).
    pub temporal_layer_id: u8,
}

impl<'a> Parse<'a> for TemporalLayerEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let b = bytes.first().ok_or(Error::BufferTooShort(BufferTooShort {
            need: 1,
            have: 0,
            what: "telg entry",
        }))?;
        Ok(Self {
            temporal_layer_id: *b,
        })
    }
}

impl Serialize for TemporalLayerEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let slot = buf
            .first_mut()
            .ok_or(Error::OutputBufferTooSmall(OutputBufferTooSmall {
                need: 1,
                have: 0,
            }))?;
        *slot = self.temporal_layer_id;
        Ok(1)
    }
}

/// One referenced library-picture sample of a [`VisualLrapEntry`] (2 bytes:
/// `library_sample_number(9) reserved(7)=0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LibrarySampleRef {
    /// `library_sample_number` (9 bits): sample number of the library picture.
    pub library_sample_number: u16,
    /// `reserved` (7 bits; 0).
    pub reserved: u8,
}

impl LibrarySampleRef {
    /// Build with the spec's zero reserved bits.
    #[must_use]
    pub const fn new(library_sample_number: u16) -> Self {
        Self {
            library_sample_number,
            reserved: 0,
        }
    }
}

/// `lrap` sample group entry `VisualLRAPEntry` (A.3.4.1):
/// `LRAP_type(3) entry_count(3) reserved(2)=0` then `entry_count` x
/// [`LibrarySampleRef`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct VisualLrapEntry {
    /// `LRAP_type` (3 bits): the SAP type the sample would have without the
    /// library dependency (1..=3; others reserved).
    pub lrap_type: u8,
    /// `reserved` (2 bits; 0) after `entry_count`.
    pub reserved: u8,
    /// Referenced library-picture samples; `entry_count` is their count (the
    /// spec allows `001` only; other counts are reserved but round-trip).
    pub library_samples: Vec<LibrarySampleRef>,
}

impl VisualLrapEntry {
    /// Build with the spec's zero reserved bits.
    #[must_use]
    pub fn new(lrap_type: u8, library_samples: Vec<LibrarySampleRef>) -> Self {
        Self {
            lrap_type,
            reserved: 0,
            library_samples,
        }
    }
}

impl<'a> Parse<'a> for VisualLrapEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let b0 = *bytes.first().ok_or(Error::BufferTooShort(BufferTooShort {
            need: 1,
            have: 0,
            what: "lrap entry",
        }))?;
        let n = usize::from((b0 >> LRAP_ENTRY_COUNT_SHIFT) & LRAP_ENTRY_COUNT_MASK);
        let need = LRAP_HEADER_LEN + n * LIBRARY_SAMPLE_LEN;
        need_in(bytes, need, "lrap library samples")?;
        let library_samples = bytes[LRAP_HEADER_LEN..need]
            .chunks_exact(LIBRARY_SAMPLE_LEN)
            .map(|c| {
                let mut cursor = 0;
                let v = broadcast_common::wire::read_u16(c, &mut cursor, "lrap library sample")?;
                Ok(LibrarySampleRef {
                    library_sample_number: v >> LIBRARY_SAMPLE_NUMBER_SHIFT,
                    reserved: (v & LIBRARY_SAMPLE_RESERVED_MASK) as u8,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            lrap_type: b0 >> LRAP_TYPE_SHIFT,
            reserved: b0 & LIBRARY_DEPENDENCY_MASK,
            library_samples,
        })
    }
}

impl Serialize for VisualLrapEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        LRAP_HEADER_LEN + self.library_samples.len() * LIBRARY_SAMPLE_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let t = fit_bits(u64::from(self.lrap_type), LRAP_TYPE_BITS, "LRAP_type")? as u8;
        let n = fit_bits(
            self.library_samples.len() as u64,
            LRAP_ENTRY_COUNT_BITS,
            "entry_count",
        )? as u8;
        let r = fit_bits(u64::from(self.reserved), LRAP_RESERVED_BITS, "reserved")? as u8;
        buf[0] = (t << LRAP_TYPE_SHIFT) | (n << LRAP_ENTRY_COUNT_SHIFT) | r;
        let mut i = 1;
        for s in &self.library_samples {
            let num = fit_bits(
                u64::from(s.library_sample_number),
                LIBRARY_SAMPLE_NUMBER_BITS,
                "library_sample_number",
            )? as u16;
            let rs = fit_bits(
                u64::from(s.reserved),
                LIBRARY_SAMPLE_RESERVED_BITS,
                "reserved",
            )? as u16;
            buf[i..i + LIBRARY_SAMPLE_LEN]
                .copy_from_slice(&((num << LIBRARY_SAMPLE_NUMBER_SHIFT) | rs).to_be_bytes());
            i += LIBRARY_SAMPLE_LEN;
        }
        Ok(need)
    }
}

/// `a3lg` library picture sample group entry (A.3.4.2): an empty entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LibrarySampleGroupEntry;

impl<'a> Parse<'a> for LibrarySampleGroupEntry {
    type Error = Error;
    fn parse(_bytes: &'a [u8]) -> Result<Self> {
        Ok(Self)
    }
}

impl Serialize for LibrarySampleGroupEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        0
    }
    fn serialize_into(&self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn round_trip<T>(bytes: &[u8]) -> T
    where
        T: for<'a> Parse<'a, Error = Error> + Serialize<Error = Error>,
    {
        let v = T::parse(bytes).unwrap();
        let mut out = vec![0u8; v.serialized_len()];
        let n = v.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        v
    }

    /// GY/T 420-2025 A.3.2.2: configurationVersion 1, a 4-byte sequence header
    /// (opaque here), reserved '111111', library_dependency_idc 01.
    #[test]
    fn av3c_record_vector() {
        let bytes = [1, 0, 4, 0x00, 0x00, 0x01, 0xB0, 0xFD];
        let c: Avs3ConfigurationBox = round_trip(&bytes);
        assert_eq!(c.sequence_header, [0x00, 0x00, 0x01, 0xB0]);
        assert_eq!(c.reserved, 0x3F);
        assert_eq!(
            c.library_dependency_idc,
            LibraryDependencyIdc::MainWithLibrary
        );
        for v in 0..4 {
            assert_eq!(LibraryDependencyIdc::from_u8(v).to_u8(), v);
        }
    }

    #[test]
    fn av3c_rejects_truncation_and_overlong_header() {
        assert!(matches!(
            Avs3ConfigurationBox::parse(&[1, 0]),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        assert!(matches!(
            Avs3ConfigurationBox::parse(&[1, 0, 4, 0, 0, 1]),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        let mut c = Avs3ConfigurationBox::new(vec![0; 70_000], LibraryDependencyIdc::Library);
        let mut out = vec![0u8; c.serialized_len()];
        assert!(
            matches!(
                c.serialize_into(&mut out),
                Err(crate::error::Error::FieldOverflow(..))
            ),
            "length overflows u16"
        );
        c.sequence_header.clear();
        c.reserved = 0x40;
        assert!(
            matches!(
                c.serialize_into(&mut out),
                Err(crate::error::Error::FieldOverflow(..))
            ),
            "reserved wider than 6 bits"
        );
    }

    /// A.3.2.3: two temporal layers.
    /// layer 0: id 0, 30 Hz, lower 0x12345 (18-bit), upper 0x0AB (12-bit);
    /// layer 1: id 1, 60 Hz, lower 0x3FFFF, upper 0xFFF.
    #[test]
    fn lavc_record_vector() {
        // layer0 byte0: id(3)=000 frc(4)=0101 rsv(1)=1 -> 0b000_0101_1 = 0x0B
        // tail = lower<<14 | upper<<2 | 0b11 = 0x12345<<14 | 0x0AB<<2 | 3
        let t0 = (0x12345u32 << 14) | (0x0AB << 2) | 3;
        let t1 = (0x3FFFFu32 << 14) | (0xFFF << 2) | 3;
        let mut bytes = vec![1u8, 2, 0x0B];
        bytes.extend_from_slice(&t0.to_be_bytes());
        bytes.push((1 << 5) | (8 << 1) | 1);
        bytes.extend_from_slice(&t1.to_be_bytes());
        let c: LAvsConfigurationBox = round_trip(&bytes);
        assert_eq!(c.layers.len(), 2);
        let l0 = c.layers[0];
        assert_eq!(l0.frame_rate_code, AvsFrameRateCode::Fps30);
        assert_eq!(
            (l0.temporal_bit_rate_lower, l0.temporal_bit_rate_upper),
            (0x12345, 0x0AB)
        );
        assert_eq!(l0.bit_rate(), (0x0AB << 18) | 0x12345);
        assert_eq!((l0.reserved_a, l0.reserved_b), (1, 3));
        assert_eq!(c.layers[1].temporal_layer_id, 1);
        assert_eq!(c.layers[1].frame_rate_code, AvsFrameRateCode::Fps60);
        assert_eq!(c.layers[1].bit_rate(), 0x3FFF_FFFF);
    }

    #[test]
    fn lavc_rejects_truncation_and_overwide() {
        assert!(matches!(
            LAvsConfigurationBox::parse(&[1]),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        assert!(matches!(
            LAvsConfigurationBox::parse(&[1, 2, 0, 0, 0, 0, 0]),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        let mut l = LAvsConfigurationBox::new(vec![TemporalLayerInfo::new(
            8,
            AvsFrameRateCode::Fps25,
            0,
            0,
        )]);
        let mut out = vec![0u8; l.serialized_len()];
        assert!(
            matches!(
                l.serialize_into(&mut out),
                Err(crate::error::Error::FieldOverflow(..))
            ),
            "temporal_layer_id > 3 bits"
        );
        l.layers[0].temporal_layer_id = 0;
        l.layers[0].temporal_bit_rate_lower = 1 << 18;
        assert!(
            matches!(
                l.serialize_into(&mut out),
                Err(crate::error::Error::FieldOverflow(..))
            ),
            "lower > 18 bits"
        );
    }

    #[test]
    fn frame_rate_code_is_lossless() {
        for v in 0u8..16 {
            assert_eq!(AvsFrameRateCode::from_u8(v).to_u8(), v);
        }
        assert_eq!(AvsFrameRateCode::Fps29_97.name(), "29.97 Hz");
    }

    /// A.3.4: telg (1 byte), lrap (LRAP_type 2, one library sample 0x0A5), a3lg (empty).
    #[test]
    fn sample_group_entry_vectors() {
        let t: TemporalLayerEntry = round_trip(&[3]);
        assert_eq!(t.temporal_layer_id, 3);
        // LRAP_type=2 (010) entry_count=1 (001) reserved 00 -> 0b010_001_00 = 0x44
        // library_sample_number 0x0A5<<7 | reserved 0 = 0x5280
        let l: VisualLrapEntry = round_trip(&[0x44, 0x52, 0x80]);
        assert_eq!(l.lrap_type, 2);
        assert_eq!(l.library_samples, vec![LibrarySampleRef::new(0x0A5)]);
        let _: LibrarySampleGroupEntry = round_trip(&[]);
        assert!(matches!(
            VisualLrapEntry::parse(&[0x44]),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        let bad = VisualLrapEntry::new(8, vec![]);
        assert!(matches!(
            bad.serialize_into(&mut [0u8; 4]),
            Err(crate::error::Error::FieldOverflow(..))
        ));
    }

    /// A.3.2.2: version(8) + sequence_header_length(16) with a zero-length
    /// sequence header (no `av2c` trailing byte) is exactly three bytes.
    #[test]
    fn sequence_header_record_accepts_an_exactly_three_byte_av2c() {
        assert_eq!(
            parse_sequence_header_record(&[1, 0, 0], "av2c", 0).unwrap(),
            (1u8, Vec::<u8>::new())
        );
        match parse_sequence_header_record(&[1, 0], "av2c", 0) {
            Err(Error::BufferTooShort(short)) => {
                assert_eq!((short.need, short.have), (3, 2));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// A.3.4.1: every `entry_count` library sample is written once at its own
    /// offset, and the empty `a3lg` entry is zero bytes.
    #[test]
    fn visual_lrap_entry_writes_all_library_samples_and_a3lg_is_empty() {
        let entry = VisualLrapEntry::new(
            2,
            vec![
                LibrarySampleRef::new(1),
                LibrarySampleRef::new(300),
                LibrarySampleRef::new(7),
            ],
        );
        let mut buf = vec![0u8; entry.serialized_len()];
        let n = entry.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 7);
        assert_eq!(buf, [0x4C, 0x00, 0x80, 0x96, 0x00, 0x03, 0x80]);
        assert_eq!(VisualLrapEntry::parse(&buf).unwrap(), entry);
        assert_eq!(LibrarySampleGroupEntry.serialized_len(), 0);
    }

    #[test]
    fn library_dependency_labels_are_pinned() {
        for (v, label) in [
            (
                LibraryDependencyIdc::MainWithoutLibrary,
                "main stream, no library references",
            ),
            (
                LibraryDependencyIdc::MainWithLibrary,
                "main stream referencing library pictures",
            ),
            (LibraryDependencyIdc::Library, "library stream"),
            (
                LibraryDependencyIdc::MainAndLibrary,
                "main and library stream in one track",
            ),
        ] {
            assert_eq!(v.name(), label);
            assert_eq!(v.to_string(), label);
        }
    }

    #[test]
    fn sample_group_grouping_types_are_the_t_ai_109_6_four_ccs() {
        assert_eq!(&TELG_FOURCC, b"telg");
        assert_eq!(&LRAP_FOURCC, b"lrap");
        assert_eq!(&A3LG_FOURCC, b"a3lg");
    }
}

use crate::avs2::LA2C_FOURCC;
use crate::box_types::BOX_HEADER_MIN_SIZE;
use crate::sample_entries::{SampleEntryChild, VisualSampleEntryFields};

/// FourCC of the AVS3 sample entry.
pub const AVS3_FOURCC: [u8; 4] = *b"avs3";
/// FourCC of the LAVS3 sample entry.
pub const LAV3_FOURCC: [u8; 4] = *b"lav3";

/// Which AVS video sample entry / primary configuration box a track uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AvsVideoEntryKind {
    /// `avs3` — AVS3 track, primary box `av3c`.
    Avs3,
    /// `lav3` — AVS3 temporal-layer track without the lowest level, primary box `lavc`.
    LAvs3,
    /// `avst` — AVS2 track, primary box `av2c`.
    Avs2,
    /// `lav2` — AVS2 temporal-layer track without the lowest level, primary box `la2c`.
    LAvs2,
}

impl AvsVideoEntryKind {
    /// FourCC of the sample entry.
    #[must_use]
    pub const fn entry_fourcc(self) -> [u8; 4] {
        match self {
            Self::Avs3 => AVS3_FOURCC,
            Self::LAvs3 => LAV3_FOURCC,
            Self::Avs2 => crate::avs2::AVST_FOURCC,
            Self::LAvs2 => crate::avs2::LAV2_FOURCC,
        }
    }

    /// FourCC of the primary configuration box of this entry kind.
    #[must_use]
    pub const fn config_fourcc(self) -> [u8; 4] {
        match self {
            Self::Avs3 => AV3C_FOURCC,
            Self::LAvs3 => LAVC_FOURCC,
            Self::Avs2 => crate::avs2::AV2C_FOURCC,
            Self::LAvs2 => crate::avs2::LA2C_FOURCC,
        }
    }

    /// Decode from a sample-entry FourCC.
    #[must_use]
    pub fn from_entry_fourcc(f: [u8; 4]) -> Option<Self> {
        [Self::Avs3, Self::LAvs3, Self::Avs2, Self::LAvs2]
            .into_iter()
            .find(|k| k.entry_fourcc() == f)
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Avs3 => "AVS3 sample entry (avs3)",
            Self::LAvs3 => "LAVS3 sample entry (lav3)",
            Self::Avs2 => "AVS2 sample entry (avst)",
            Self::LAvs2 => "LAVS2 sample entry (lav2)",
        }
    }
}
broadcast_common::impl_spec_display!(AvsVideoEntryKind);

/// The typed primary configuration box of an [`AvsVideoSampleEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AvsVideoConfig {
    /// `av3c`.
    Avs3(Avs3ConfigurationBox),
    /// `av2c`.
    Avs2(crate::avs2::Avs2ConfigurationBox),
    /// `lavc` or `la2c`.
    Layers(LAvsConfigurationBox),
}

impl AvsVideoConfig {
    /// FourCC-style label of the configuration box kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Avs3(_) => "av3c",
            Self::Avs2(_) => "av2c",
            Self::Layers(_) => "lavc/la2c",
        }
    }
}
broadcast_common::impl_spec_display!(AvsVideoConfig);

/// An AVS2/AVS3 video sample entry (`avs3`, `lav3`, `avst`, `lav2`): a
/// `VisualSampleEntry` carrying the primary configuration box
/// ([`AvsVideoEntryKind::config_fourcc`]). A second configuration box (`lavc` /
/// `la2c` alongside `av3c` / `av2c`, A.3.3.1 b) and any `pasp`/`btrt`/... children
/// stay in `children` verbatim at their wire positions;
/// [`AvsVideoSampleEntry::secondary_layer_config`] decodes the layer box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AvsVideoSampleEntry {
    /// Entry kind (fixes the sample-entry and primary-config FourCCs).
    pub kind: AvsVideoEntryKind,
    /// Fixed VisualSampleEntry fields.
    pub visual: VisualSampleEntryFields,
    /// The primary configuration box.
    pub config: AvsVideoConfig,
    /// The entry's children in wire order (the primary config box at its position).
    pub children: Vec<SampleEntryChild>,
}

impl AvsVideoSampleEntry {
    /// Parse a full sample-entry box (including the 8-byte header).
    pub fn parse_entry(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "AVS sample entry")?;
        let mut cursor = crate::box_types::BOX_TYPE_SIZE;
        let fourcc = broadcast_common::wire::read_u32(bytes, &mut cursor, "AVS sample entry type")?
            .to_be_bytes();
        let kind = AvsVideoEntryKind::from_entry_fourcc(fourcc).ok_or(Error::UnexpectedBox {
            expected: "avs3/lav3/avst/lav2",
        })?;
        let visual = VisualSampleEntryFields::parse_body(bytes, "AVS sample entry")?;
        let region = VisualSampleEntryFields::children_region(bytes, "AVS sample entry")?;
        let cfg_fourcc = kind.config_fourcc();
        let cfg = crate::sample_entries::find_config_box(region, &cfg_fourcc).ok_or(
            Error::UnexpectedBox {
                expected: "AVS configuration box",
            },
        )?;
        let body = crate::box_types::parse_box(cfg)?.0.body;
        let config = match kind {
            AvsVideoEntryKind::Avs3 => AvsVideoConfig::Avs3(Avs3ConfigurationBox::parse(body)?),
            AvsVideoEntryKind::Avs2 => {
                AvsVideoConfig::Avs2(crate::avs2::Avs2ConfigurationBox::parse(body)?)
            }
            AvsVideoEntryKind::LAvs3 | AvsVideoEntryKind::LAvs2 => {
                AvsVideoConfig::Layers(LAvsConfigurationBox::parse(body)?)
            }
        };
        let children = crate::sample_entries::entry_children(region, &cfg_fourcc)?;
        Ok(Self {
            kind,
            visual,
            config,
            children,
        })
    }

    /// Decode the second configuration box (`lavc` in an `avs3` entry, `la2c` in
    /// an `avst` entry) if the entry carries one.
    #[must_use]
    pub fn secondary_layer_config(&self) -> Option<Result<LAvsConfigurationBox>> {
        let want = match self.kind {
            AvsVideoEntryKind::Avs3 => LAVC_FOURCC,
            AvsVideoEntryKind::Avs2 => LA2C_FOURCC,
            _ => return None,
        };
        self.children.iter().find_map(|c| match c {
            SampleEntryChild::Other(b) if b.get(4..8) == Some(&want[..]) => {
                let body = crate::box_types::parse_box(b).ok()?.0.body;
                Some(LAvsConfigurationBox::parse(body))
            }
            _ => None,
        })
    }

    fn config_body_len(&self) -> usize {
        match &self.config {
            AvsVideoConfig::Avs3(c) => c.serialized_len(),
            AvsVideoConfig::Avs2(c) => c.serialized_len(),
            AvsVideoConfig::Layers(c) => c.serialized_len(),
        }
    }

    fn write_config_body(&self, buf: &mut [u8]) -> Result<usize> {
        match &self.config {
            AvsVideoConfig::Avs3(c) => c.serialize_into(buf),
            AvsVideoConfig::Avs2(c) => c.serialize_into(buf),
            AvsVideoConfig::Layers(c) => c.serialize_into(buf),
        }
    }
}

impl Serialize for AvsVideoSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let config_len = BOX_HEADER_MIN_SIZE + self.config_body_len();
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE
                + VisualSampleEntryFields::serialized_len()
                + crate::sample_entries::entry_children_len(&self.children, config_len),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_box_header(buf, &self.kind.entry_fourcc(), need)?;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        let cfg_fourcc = self.kind.config_fourcc();
        let write_config = |buf: &mut [u8], c: &mut usize| -> Result<()> {
            let len = BOX_HEADER_MIN_SIZE + self.config_body_len();
            let len = broadcast_common::len::fit_u32(len, "config box size")?;
            buf[*c..*c + crate::box_types::BOX_TYPE_SIZE].copy_from_slice(&len.to_be_bytes());
            *c += crate::box_types::BOX_TYPE_SIZE;
            buf[*c..*c + crate::box_types::BOX_TYPE_SIZE].copy_from_slice(&cfg_fourcc);
            *c += crate::box_types::BOX_TYPE_SIZE;
            *c += self.write_config_body(&mut buf[*c..])?;
            Ok(())
        };
        crate::sample_entries::serialize_entry_children(&self.children, write_config, buf, &mut c)?;
        if c != need {
            return Err(Error::InvalidInput(
                "sample entry child layout does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;
    use crate::avs2::{AV2C_FOURCC, AVST_FOURCC, Avs2ConfigurationBox, LAV2_FOURCC};
    use crate::avs3::{AvsFrameRateCode, LibraryDependencyIdc, TemporalLayerInfo};
    use alloc::vec;

    fn visual() -> VisualSampleEntryFields {
        VisualSampleEntryFields {
            width: 3840,
            height: 2160,
            ..VisualSampleEntryFields::default()
        }
    }

    fn child_box(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 8 + body.len()];
        b[..4].copy_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        b[4..8].copy_from_slice(&fourcc);
        b[8..].copy_from_slice(body);
        b
    }

    fn layers_body(layers: &LAvsConfigurationBox) -> Vec<u8> {
        let mut body = vec![0u8; layers.serialized_len()];
        layers.serialize_into(&mut body).unwrap();
        body
    }

    /// `avs3` entry's children, and an unrelated child (`btrt`) before it is
    /// not mistaken for one.
    #[test]
    fn secondary_layer_config_selects_the_layers_fourcc_and_ignores_others() {
        let lavc = LAvsConfigurationBox::new(vec![TemporalLayerInfo::new(
            0,
            AvsFrameRateCode::Fps25,
            100,
            0,
        )]);
        let btrt = child_box(*b"btrt", &[0u8; 12]);
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs3,
            visual: visual(),
            config: AvsVideoConfig::Avs3(Avs3ConfigurationBox::new(
                vec![0, 0, 1, 0xB0],
                LibraryDependencyIdc::MainWithoutLibrary,
            )),
            children: vec![
                SampleEntryChild::Config,
                SampleEntryChild::Other(btrt.clone()),
                SampleEntryChild::Other(child_box(LAVC_FOURCC, &layers_body(&lavc))),
            ],
        };
        assert_eq!(entry.secondary_layer_config().unwrap().unwrap(), lavc);

        let only_other = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs3,
            visual: visual(),
            config: AvsVideoConfig::Avs3(Avs3ConfigurationBox::new(
                vec![],
                LibraryDependencyIdc::Library,
            )),
            children: vec![SampleEntryChild::Config, SampleEntryChild::Other(btrt)],
        };
        assert!(only_other.secondary_layer_config().is_none());
    }

    /// A.2.3.1 b): an `avst` entry's second configuration box is `la2c`.
    #[test]
    fn avst_entry_decodes_its_la2c_secondary_layer_config() {
        let layers = LAvsConfigurationBox::new(vec![TemporalLayerInfo::new(
            1,
            AvsFrameRateCode::Fps50,
            5,
            0,
        )]);
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs2,
            visual: visual(),
            config: AvsVideoConfig::Avs2(crate::avs2::Avs2ConfigurationBox::new(vec![
                0, 0, 1, 0xB0,
            ])),
            children: vec![
                SampleEntryChild::Config,
                SampleEntryChild::Other(child_box(crate::avs2::LA2C_FOURCC, &layers_body(&layers))),
            ],
        };
        assert_eq!(entry.secondary_layer_config().unwrap().unwrap(), layers);
    }

    #[test]
    fn entry_kind_and_config_labels_are_pinned() {
        for (v, label) in [
            (AvsVideoEntryKind::Avs3, "AVS3 sample entry (avs3)"),
            (AvsVideoEntryKind::LAvs3, "LAVS3 sample entry (lav3)"),
            (AvsVideoEntryKind::Avs2, "AVS2 sample entry (avst)"),
            (AvsVideoEntryKind::LAvs2, "LAVS2 sample entry (lav2)"),
        ] {
            assert_eq!(v.name(), label);
            assert_eq!(v.to_string(), label);
        }
        for (v, label) in [
            (
                AvsVideoConfig::Avs3(Avs3ConfigurationBox::new(
                    vec![],
                    LibraryDependencyIdc::Library,
                )),
                "av3c",
            ),
            (
                AvsVideoConfig::Avs2(Avs2ConfigurationBox::new(vec![])),
                "av2c",
            ),
            (
                AvsVideoConfig::Layers(LAvsConfigurationBox::new(vec![])),
                "lavc/la2c",
            ),
        ] {
            assert_eq!(v.name(), label);
            assert_eq!(v.to_string(), label);
        }
    }

    /// An `avs3` sample entry with `av3c`, a `lavc` second config box and a
    /// `pasp` box in wire order, round-trips byte-exact; the secondary layer
    /// configuration is decodable.
    #[test]
    fn avs3_sample_entry_with_second_config_round_trips() {
        let av3c = Avs3ConfigurationBox::new(
            vec![0, 0, 1, 0xB0],
            LibraryDependencyIdc::MainWithoutLibrary,
        );
        let lavc = LAvsConfigurationBox::new(vec![TemporalLayerInfo::new(
            0,
            AvsFrameRateCode::Fps25,
            100,
            0,
        )]);
        let mut lavc_box = vec![];
        let mut body = vec![0u8; lavc.serialized_len()];
        lavc.serialize_into(&mut body).unwrap();
        lavc_box.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        lavc_box.extend_from_slice(&LAVC_FOURCC);
        lavc_box.extend_from_slice(&body);
        let pasp = [0, 0, 0, 16, b'p', b'a', b's', b'p', 0, 0, 0, 1, 0, 0, 0, 1];
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs3,
            visual: visual(),
            config: AvsVideoConfig::Avs3(av3c.clone()),
            children: vec![
                SampleEntryChild::Config,
                SampleEntryChild::Other(lavc_box),
                SampleEntryChild::Other(pasp.to_vec()),
            ],
        };
        let mut buf = vec![0u8; entry.serialized_len()];
        let n = entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"avs3");
        let back = AvsVideoSampleEntry::parse_entry(&buf[..n]).unwrap();
        assert_eq!(back, entry);
        assert_eq!(back.visual.width, 3840);
        assert_eq!(back.secondary_layer_config().unwrap().unwrap(), lavc);
        let mut again = vec![0u8; back.serialized_len()];
        back.serialize_into(&mut again).unwrap();
        assert_eq!(again, buf, "byte-exact round trip");
    }

    /// A `lav3` entry carries only `lavc` as its primary configuration.
    #[test]
    fn lav3_sample_entry_round_trips() {
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::LAvs3,
            visual: visual(),
            config: AvsVideoConfig::Layers(LAvsConfigurationBox::new(vec![
                TemporalLayerInfo::new(1, AvsFrameRateCode::Fps50, 5, 0),
            ])),
            children: vec![SampleEntryChild::Config],
        };
        let mut buf = vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"lav3");
        let back = AvsVideoSampleEntry::parse_entry(&buf).unwrap();
        assert_eq!(back, entry);
        assert!(back.secondary_layer_config().is_none());
        assert!(matches!(
            AvsVideoSampleEntry::parse_entry(&[0, 0, 0, 8, b'a', b'v', b'c', b'1']),
            Err(crate::error::Error::UnexpectedBox { .. })
        ));
    }

    /// An `avst` entry with `av2c` round-trips; the fourcc constants agree
    /// with the entry kinds.
    #[test]
    fn avst_sample_entry_round_trips() {
        assert_eq!(AvsVideoEntryKind::Avs2.entry_fourcc(), AVST_FOURCC);
        assert_eq!(AvsVideoEntryKind::Avs2.config_fourcc(), AV2C_FOURCC);
        assert_eq!(AvsVideoEntryKind::LAvs2.entry_fourcc(), LAV2_FOURCC);
        assert_eq!(AvsVideoEntryKind::LAvs2.config_fourcc(), LA2C_FOURCC);
        let entry = AvsVideoSampleEntry {
            kind: AvsVideoEntryKind::Avs2,
            visual: VisualSampleEntryFields {
                width: 1920,
                height: 1080,
                ..VisualSampleEntryFields::default()
            },
            config: AvsVideoConfig::Avs2(Avs2ConfigurationBox::new(vec![0, 0, 1, 0xB0, 0x20])),
            children: vec![SampleEntryChild::Config],
        };
        let mut buf = vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[4..8], b"avst");
        let back = AvsVideoSampleEntry::parse_entry(&buf).unwrap();
        assert_eq!(back, entry);
    }
}
