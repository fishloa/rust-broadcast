//! The derived image bodies ISO/IEC 23008-12:2025 and Amendment 1 add:
//! tone-map derivation (`tmap`, Amd 1 6.6.2.4) and colour format enhancement
//! (`cfen`, Amd 1 6.6.2.5).
//!
//! The gain-map metadata of a `tmap` item is the `GainMapMetadata` structure of
//! ISO 21496-1, which is not held: [`crate::image::GainMapMetadata`] models the
//! layout the open implementations agree on and any other body is carried
//! opaque. The packed channel layout of a `cfen` input is bit-packed exactly as
//! Amd 1 6.6.2.5.2 states.
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025-amd1-2025.md`
//! 6.6.2.4 and 6.6.2.5; `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md`
//! 6.6.2.

use alloc::vec::Vec;

use broadcast_core::bits::{BitBuf, BitReader};
use broadcast_core::len::{fit_mask, need_in, need_out};
use broadcast_core::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::image::GainMapMetadata;

/// Version every body this module types requires (Amd 1 6.6.2.4.3,
/// 6.6.2.5.3).
const DERIVED_VERSION: u8 = 0;
/// Bits in one byte.
const BITS_IN_BYTE: usize = u8::BITS as usize;
/// Bits of the reserved run before `is_packed_flag` (Amd 1 6.6.2.5.2).
const PACKED_RESERVED_BITS: u32 = 7;
/// Bits of `is_packed_flag` (Amd 1 6.6.2.5.2).
const PACKED_IS_PACKED_BITS: u32 = 1;
/// Bits of the `is_packed_flag` byte, reserved prefix included (Amd 1 6.6.2.5.2).
const PACKED_FLAG_BITS: u32 = PACKED_RESERVED_BITS + PACKED_IS_PACKED_BITS;
/// Bits of `num_cols_minus1` (Amd 1 6.6.2.5.2).
const PACKED_COLS_BITS: u32 = 3;
/// Bits of `num_rows_minus1` (Amd 1 6.6.2.5.2).
const PACKED_ROWS_BITS: u32 = 3;
/// Bits of `hor_guard_band_mul2` (Amd 1 6.6.2.5.2).
const PACKED_HOR_BAND_BITS: u32 = 5;
/// Bits of `ver_guard_band_mul2` (Amd 1 6.6.2.5.2).
const PACKED_VER_BAND_BITS: u32 = 5;
/// Bits of one `packed_channel_id` / `channel_id` (Amd 1 6.6.2.5.2).
const CHANNEL_ID_BITS: u32 = 8;
/// Mask of the `is_packed_flag` inside its byte.
const PACKED_FLAG_MASK: u64 = 1;
/// Mask of the reserved run inside the `is_packed_flag` byte.
const PACKED_RESERVED_MASK: u64 = 0x7f;
/// Mask of `num_cols_minus1`.
const PACKED_COLS_MASK: u64 = 0x7;
/// Mask of `num_rows_minus1`.
const PACKED_ROWS_MASK: u64 = 0x7;
/// Mask of `hor_guard_band_mul2`.
const PACKED_HOR_BAND_MASK: u64 = 0x1f;
/// Mask of `ver_guard_band_mul2`.
const PACKED_VER_BAND_MASK: u64 = 0x1f;

/// ToneMapImage body (`tmap`) — ISO/IEC 23008-12:2025 Amd 1 6.6.2.4.2.
///
/// The bytes after the body version byte are the `GainMapMetadata` structure of
/// ISO 21496-1, which is not held. They are carried verbatim, and the typed
/// view [`GainMapMetadata`] is present only when the body is the one layout the
/// open implementations agree on (see the module doc of [`crate::image`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ToneMapImage {
    version: u8,
    gain_map_metadata: Vec<u8>,
    typed_metadata: Option<GainMapMetadata>,
}

impl ToneMapImage {
    /// A version-0 body from raw ISO 21496-1 gain-map metadata bytes.
    #[must_use]
    pub fn new(gain_map_metadata: Vec<u8>) -> Self {
        let typed_metadata = GainMapMetadata::parse(&gain_map_metadata).ok();
        Self {
            version: DERIVED_VERSION,
            gain_map_metadata,
            typed_metadata,
        }
    }

    /// A version-0 body holding `metadata`, with its bytes written from it.
    pub fn with_metadata(metadata: GainMapMetadata) -> Result<Self> {
        let mut bytes = alloc::vec![0u8; metadata.serialized_len()];
        let written = metadata.serialize_into(&mut bytes)?;
        bytes.truncate(written);
        Ok(Self {
            version: DERIVED_VERSION,
            gain_map_metadata: bytes,
            typed_metadata: Some(metadata),
        })
    }

    /// `version`.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// `gain_map_metadata`, the ISO 21496-1 `GainMapMetadata` bytes.
    #[must_use]
    pub fn gain_map_metadata(&self) -> &[u8] {
        &self.gain_map_metadata
    }

    /// The typed gain map metadata, when the body is the layout this crate
    /// models; a body with a `minimum_version` above 0, a reserved flag bit set
    /// or trailing bytes stays opaque and yields `None`.
    #[must_use]
    pub fn gain_map_metadata_typed(&self) -> Option<&GainMapMetadata> {
        self.typed_metadata.as_ref()
    }
}

impl<'a> broadcast_core::Parse<'a> for ToneMapImage {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        need_in(body, 1, "tmap body")?;
        let version = body[0];
        let gain_map_metadata = body[1..].to_vec();
        let typed_metadata = GainMapMetadata::parse(&gain_map_metadata).ok();
        Ok(Self {
            version,
            gain_map_metadata,
            typed_metadata,
        })
    }
}

impl broadcast_core::Serialize for ToneMapImage {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        1 + self.gain_map_metadata.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0] = self.version;
        buf[1..need].copy_from_slice(&self.gain_map_metadata);
        Ok(need)
    }
}

/// One input of a [`ColourFormatEnhancement`] (Amd 1 6.6.2.5.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ColourFormatInput {
    reserved: u8,
    is_packed: bool,
    channel_id: u8,
    num_cols_minus1: u8,
    num_rows_minus1: u8,
    hor_guard_band_mul2: u8,
    ver_guard_band_mul2: u8,
    packed_channel_ids: Vec<u8>,
}

impl ColourFormatInput {
    /// An input whose first channel carries `channel_id`.
    #[must_use]
    pub const fn whole(channel_id: u8) -> Self {
        Self {
            reserved: 0,
            is_packed: false,
            channel_id,
            num_cols_minus1: 0,
            num_rows_minus1: 0,
            hor_guard_band_mul2: 0,
            ver_guard_band_mul2: 0,
            packed_channel_ids: Vec::new(),
        }
    }

    /// A packed input with its partition grid, guard bands and per-partition
    /// channel identifiers in row-major order.
    #[must_use]
    pub fn packed(
        num_cols_minus1: u8,
        num_rows_minus1: u8,
        hor_guard_band_mul2: u8,
        ver_guard_band_mul2: u8,
        packed_channel_ids: Vec<u8>,
    ) -> Self {
        Self {
            reserved: 0,
            is_packed: true,
            channel_id: 0,
            num_cols_minus1,
            num_rows_minus1,
            hor_guard_band_mul2,
            ver_guard_band_mul2,
            packed_channel_ids,
        }
    }

    /// The `reserved` run before `is_packed_flag`, kept verbatim.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// `is_packed_flag`.
    #[must_use]
    pub const fn is_packed(&self) -> bool {
        self.is_packed
    }

    /// `channel_id`, when the input is not packed.
    #[must_use]
    pub const fn channel_id(&self) -> u8 {
        self.channel_id
    }

    /// `num_cols_minus1`, for a packed input.
    #[must_use]
    pub const fn num_cols_minus1(&self) -> u8 {
        self.num_cols_minus1
    }

    /// `num_rows_minus1`, for a packed input.
    #[must_use]
    pub const fn num_rows_minus1(&self) -> u8 {
        self.num_rows_minus1
    }

    /// `hor_guard_band_mul2`, for a packed input.
    #[must_use]
    pub const fn hor_guard_band_mul2(&self) -> u8 {
        self.hor_guard_band_mul2
    }

    /// `ver_guard_band_mul2`, for a packed input.
    #[must_use]
    pub const fn ver_guard_band_mul2(&self) -> u8 {
        self.ver_guard_band_mul2
    }

    /// `packed_channel_id` per partition, in row-major order.
    #[must_use]
    pub fn packed_channel_ids(&self) -> &[u8] {
        &self.packed_channel_ids
    }
}

/// ColourFormatEnhancement body (`cfen`) — ISO/IEC 23008-12:2025 Amd 1
/// 6.6.2.5.2.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ColourFormatEnhancement {
    version: u8,
    inputs: Vec<ColourFormatInput>,
}

impl ColourFormatEnhancement {
    /// A version-0 body over `inputs` (one per `dimg` reference, in order).
    #[must_use]
    pub fn new(inputs: Vec<ColourFormatInput>) -> Self {
        Self {
            version: DERIVED_VERSION,
            inputs,
        }
    }

    /// `version`.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The inputs, in `dimg` order.
    #[must_use]
    pub fn inputs(&self) -> &[ColourFormatInput] {
        &self.inputs
    }

    fn body_bits(input: &ColourFormatInput) -> u64 {
        u64::from(PACKED_FLAG_BITS)
            + if input.is_packed {
                u64::from(
                    PACKED_COLS_BITS
                        + PACKED_ROWS_BITS
                        + PACKED_HOR_BAND_BITS
                        + PACKED_VER_BAND_BITS,
                ) + u64::from(CHANNEL_ID_BITS) * input.packed_channel_ids.len() as u64
            } else {
                u64::from(CHANNEL_ID_BITS)
            }
    }
}

impl<'a> broadcast_core::Parse<'a> for ColourFormatEnhancement {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        let parsed = Self::parse_with_reference_count(body, 0)?;
        if body.len() != 1 {
            return Err(Error::InvalidInput("cfen input count"));
        }
        Ok(parsed)
    }
}

impl ColourFormatEnhancement {
    /// Parse a body with `reference_count` inputs, which the `dimg` reference
    /// states (Amd 1 6.6.2.5.1).
    pub fn parse_with_reference_count(body: &[u8], reference_count: usize) -> Result<Self> {
        need_in(body, 1, "cfen body")?;
        let version = body[0];
        let mut reader = BitReader::starting_at(body, BITS_IN_BYTE)
            .map_err(|_| Error::InvalidInput("cfen body"))?;
        let mut inputs = Vec::with_capacity(reference_count);
        for _ in 0..reference_count {
            let flag = reader
                .read_bits(PACKED_FLAG_BITS)
                .map_err(|_| Error::InvalidInput("cfen packed flag"))?;
            let is_packed = flag & PACKED_FLAG_MASK != 0;
            let reserved = u8::try_from((flag >> PACKED_IS_PACKED_BITS) & PACKED_RESERVED_MASK)
                .map_err(|_| Error::InvalidInput("cfen packed flag"))?;
            if is_packed {
                let cols = reader
                    .read_bits(PACKED_COLS_BITS)
                    .map_err(|_| Error::InvalidInput("cfen num_cols_minus1"))?;
                let rows = reader
                    .read_bits(PACKED_ROWS_BITS)
                    .map_err(|_| Error::InvalidInput("cfen num_rows_minus1"))?;
                let hor = reader
                    .read_bits(PACKED_HOR_BAND_BITS)
                    .map_err(|_| Error::InvalidInput("cfen hor_guard_band_mul2"))?;
                let ver = reader
                    .read_bits(PACKED_VER_BAND_BITS)
                    .map_err(|_| Error::InvalidInput("cfen ver_guard_band_mul2"))?;
                let partitions = (cols as usize + 1)
                    .checked_mul(rows as usize + 1)
                    .ok_or(Error::InvalidInput("cfen partition count"))?;
                let mut ids = Vec::with_capacity(partitions);
                for _ in 0..partitions {
                    ids.push(
                        reader
                            .read_bits(CHANNEL_ID_BITS)
                            .map_err(|_| Error::InvalidInput("cfen packed_channel_id"))?
                            as u8,
                    );
                }
                let mut input =
                    ColourFormatInput::packed(cols as u8, rows as u8, hor as u8, ver as u8, ids);
                input.reserved = reserved;
                inputs.push(input);
            } else {
                let channel = reader
                    .read_bits(CHANNEL_ID_BITS)
                    .map_err(|_| Error::InvalidInput("cfen channel_id"))?;
                let mut input = ColourFormatInput::whole(channel as u8);
                input.reserved = reserved;
                inputs.push(input);
            }
        }
        if !reader.is_byte_aligned() || reader.bits_remaining() != 0 {
            return Err(Error::InvalidInput("cfen trailing bytes"));
        }
        Ok(Self { version, inputs })
    }
}

impl broadcast_core::Serialize for ColourFormatEnhancement {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let bits: u64 = self.inputs.iter().map(Self::body_bits).sum::<u64>();
        1 + usize::try_from(bits / BITS_IN_BYTE as u64).unwrap_or(usize::MAX)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0] = self.version;
        let mut writer = BitBuf::new();
        for input in &self.inputs {
            let reserved = broadcast_core::len::fit_mask(
                u64::from(input.reserved),
                PACKED_RESERVED_MASK,
                "cfen packed flag",
            )?;
            let flag = reserved << PACKED_IS_PACKED_BITS | u64::from(input.is_packed);
            writer
                .write_bits(flag, PACKED_FLAG_BITS)
                .map_err(|_| Error::InvalidInput("cfen packed flag"))?;
            if input.is_packed {
                for (value, bits, mask, what) in [
                    (
                        u64::from(input.num_cols_minus1),
                        PACKED_COLS_BITS,
                        PACKED_COLS_MASK,
                        "cfen num_cols_minus1",
                    ),
                    (
                        u64::from(input.num_rows_minus1),
                        PACKED_ROWS_BITS,
                        PACKED_ROWS_MASK,
                        "cfen num_rows_minus1",
                    ),
                    (
                        u64::from(input.hor_guard_band_mul2),
                        PACKED_HOR_BAND_BITS,
                        PACKED_HOR_BAND_MASK,
                        "cfen hor_guard_band_mul2",
                    ),
                    (
                        u64::from(input.ver_guard_band_mul2),
                        PACKED_VER_BAND_BITS,
                        PACKED_VER_BAND_MASK,
                        "cfen ver_guard_band_mul2",
                    ),
                ] {
                    writer
                        .write_bits(fit_mask(value, mask, what)?, bits)
                        .map_err(|_| Error::InvalidInput(what))?;
                }
                for id in &input.packed_channel_ids {
                    writer
                        .write_bits(u64::from(*id), CHANNEL_ID_BITS)
                        .map_err(|_| Error::InvalidInput("cfen packed_channel_id"))?;
                }
            } else {
                writer
                    .write_bits(u64::from(input.channel_id), CHANNEL_ID_BITS)
                    .map_err(|_| Error::InvalidInput("cfen channel_id"))?;
            }
        }
        let body = writer.as_bytes();
        let end = 1 + body.len();
        buf[1..end].copy_from_slice(body);
        Ok(end)
    }
}

/// The four-CC of the tone-map derived image item type (Amd 1 6.6.2.4).
pub const TMAP: [u8; 4] = *b"tmap";
/// The four-CC of the colour format enhancement derived image item type
/// (Amd 1 6.6.2.5).
pub const CFEN: [u8; 4] = *b"cfen";
