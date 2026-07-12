//! `ObjectDefinition1` element — RDD 29:2019 §2.3/§4.4/§5.4.
//!
//! Updates one panned audio object's position (and related rendering
//! metadata: snap-to-speaker, per-zone gain, spread, decorrelation) at
//! `NumPanSubBlocks` ~5ms sub-block boundaries within a frame, plus an
//! optional text description.

use alloc::vec::Vec;

use broadcast_common::Serialize;
use broadcast_common::bits::{BitReader, BitWriter};

use crate::error::{BitResultExt, Error, Result};
use crate::frame_rate::FrameRate;
use crate::plex::{plex_bits, read_plex, write_plex};
use crate::util::{expect_fully_consumed, read_reserved, write_reserved};

/// Number of known zones (§5.4.5 Table 8 has 9 rows, IDs `0`-`8`).
pub const MAX_KNOWN_ZONES: usize = 9;

/// `Reserved` (11 bits, header) — always `0x7FE` (§4.4).
const RESERVED_HEADER: u64 = 0x7FE;
/// `Reserved` (5 bits, per pan-info) — always `0x1` (§4.4).
const RESERVED_PAN_INFO: u64 = 0x1;
/// `Reserved` (2 bits, when `ObjectSnap == 1`) — always `0` (§4.4).
const RESERVED_SNAP: u64 = 0;
/// `Reserved` (4 bits, before `ObjectDecorCoefPrefix`) — always `0` (§4.4).
const RESERVED_PRE_DECOR: u64 = 0;
/// `Reserved` (8 bits, final field) — always `0` (§4.4).
const RESERVED_FINAL: u64 = 0;

/// Zone identifiers — RDD 29 §5.4.5 Table 8. Informative: this is the
/// *array position* a `ZoneGain` occupies in wire order, not a value that
/// itself appears on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum ZoneId {
    /// `0` — all screen speakers left of center.
    ScreenLeft = 0,
    /// `1` — screen center speakers.
    ScreenCenter = 1,
    /// `2` — all screen speakers right of center.
    ScreenRight = 2,
    /// `3` — all speakers on left wall.
    WallLeft = 3,
    /// `4` — all speakers on right wall.
    WallRight = 4,
    /// `5` — all speakers on left half of rear wall.
    RearLeft = 5,
    /// `6` — all speakers on right half of rear wall.
    RearRight = 6,
    /// `7` — all overhead speakers left of center.
    OverheadLeft = 7,
    /// `8` — all overhead speakers right of center.
    OverheadRight = 8,
}

impl ZoneId {
    /// The spec token for this value.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ScreenLeft => "all screen speakers left of center",
            Self::ScreenCenter => "screen center speakers",
            Self::ScreenRight => "all screen speakers right of center",
            Self::WallLeft => "all speakers on left wall",
            Self::WallRight => "all speakers on right wall",
            Self::RearLeft => "all speakers on left half of rear wall",
            Self::RearRight => "all speakers on right half of rear wall",
            Self::OverheadLeft => "all overhead speakers left of center",
            Self::OverheadRight => "all overhead speakers right of center",
        }
    }

    /// All nine zones, in wire (array) order.
    pub const ALL: [ZoneId; MAX_KNOWN_ZONES] = [
        Self::ScreenLeft,
        Self::ScreenCenter,
        Self::ScreenRight,
        Self::WallLeft,
        Self::WallRight,
        Self::RearLeft,
        Self::RearRight,
        Self::OverheadLeft,
        Self::OverheadRight,
    ];
}

broadcast_common::impl_spec_display!(ZoneId);

/// The 2-bit `ZoneGain` code — RDD 29 §5.4.7 Table 9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ZoneGain {
    /// `0x0` — set gain to `0.0`.
    GainZero,
    /// `0x1` — set gain of `1.0`.
    GainOne,
    /// `0x2`/`0x3` — reserved.
    Reserved(u8),
}

impl ZoneGain {
    /// The spec token for this value ("reserved" for the reserved arm).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::GainZero => "gain 0.0",
            Self::GainOne => "gain 1.0",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u64) -> Self {
        match bits {
            0x0 => Self::GainZero,
            0x1 => Self::GainOne,
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u64 {
        match self {
            Self::GainZero => 0x0,
            Self::GainOne => 0x1,
            Self::Reserved(v) => u64::from(v),
        }
    }
}

broadcast_common::impl_spec_display!(ZoneGain, Reserved);

/// The 2-bit `ObjectSpreadMode` code — RDD 29 §5.4.8 Table 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ObjectSpreadMode {
    /// `0x0` — `OBJECT_SPREAD_LOWREZ`: equal spreading in all dimensions,
    /// 8-bit coding.
    Lowrez,
    /// `0x2` — `OBJECT_SPREAD_1D`: equal spreading in all dimensions, 12-bit
    /// coding.
    OneD,
    /// `0x1`/`0x3` — reserved. Per the pseudocode's `else` branch, no
    /// `ObjectSpread` bits are read/written for these codes (implied spread
    /// `0.0`).
    Reserved(u8),
}

impl ObjectSpreadMode {
    /// The spec token for this value ("reserved" for the reserved arm).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Lowrez => "object spread lowrez",
            Self::OneD => "object spread 1d",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u64) -> Self {
        match bits {
            0x0 => Self::Lowrez,
            0x2 => Self::OneD,
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u64 {
        match self {
            Self::Lowrez => 0x0,
            Self::OneD => 0x2,
            Self::Reserved(v) => u64::from(v),
        }
    }

    /// Wire width (bits) of the following `ObjectSpread` field, or `None`
    /// if this mode reads no `ObjectSpread` bits at all (the reserved
    /// codes — §4.4's `else { ObjectSpread[sb] = 0.0 }` branch).
    fn spread_width(self) -> Option<u32> {
        match self {
            Self::Lowrez => Some(8),
            Self::OneD => Some(12),
            Self::Reserved(_) => None,
        }
    }
}

broadcast_common::impl_spec_display!(ObjectSpreadMode, Reserved);

/// The 2-bit `ObjectDecorCoefPrefix` code — RDD 29 §5.4.10 Table 11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DecorCoefPrefix {
    /// `0x0` — no decorrelation.
    NoDecorrelation,
    /// `0x1` — maximum decorrelation.
    MaxDecorrelation,
    /// `0x2` — decorrelation coefficient follows in the bitstream.
    CoefFollows,
    /// `0x3` — reserved. Per the literal pseudocode condition
    /// (`ObjectDecorCoefPrefix > 1`), `ObjectDecorCoef` is read/written for
    /// this code too, same as `CoefFollows`.
    Reserved,
}

impl DecorCoefPrefix {
    /// The spec token for this value.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NoDecorrelation => "no decorrelation",
            Self::MaxDecorrelation => "maximum decorrelation",
            Self::CoefFollows => "decorrelation coefficient follows",
            Self::Reserved => "reserved",
        }
    }

    fn from_bits(bits: u64) -> Self {
        match bits {
            0x0 => Self::NoDecorrelation,
            0x1 => Self::MaxDecorrelation,
            0x2 => Self::CoefFollows,
            _ => Self::Reserved,
        }
    }

    fn to_bits(self) -> u64 {
        match self {
            Self::NoDecorrelation => 0x0,
            Self::MaxDecorrelation => 0x1,
            Self::CoefFollows => 0x2,
            Self::Reserved => 0x3,
        }
    }

    /// `true` iff `ObjectDecorCoef` follows in the bitstream — the literal
    /// pseudocode condition is `ObjectDecorCoefPrefix > 1`, true for both
    /// `CoefFollows` (`0x2`) and `Reserved` (`0x3`).
    fn reads_coef(self) -> bool {
        matches!(self, Self::CoefFollows | Self::Reserved)
    }
}

broadcast_common::impl_spec_display!(DecorCoefPrefix);

/// One sub-block's pan/rendering info (present when `PanInfoExists == 1`) —
/// RDD 29 §4.4/§5.4.2-§5.4.11.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanInfo {
    /// `ObjectPosX` — raw `DistanceXY`-coded lateral position (§3.2, §5.4.3).
    /// Use [`crate::distance::distance_xy`] to decode to `[0,1]`.
    pub pos_x: u16,
    /// `ObjectPosY` — raw `DistanceXY`-coded longitude position.
    pub pos_y: u16,
    /// `ObjectPosZ` — raw `DistanceZ`-coded (`n=16`) elevation position.
    /// Use [`crate::distance::distance_z`] to decode to `[0,1]`.
    pub pos_z: u16,
    /// `ObjectSnap` — if `true`, the object should snap to the closest
    /// speaker (§5.4.4).
    pub snap: bool,
    /// Per-zone gain overrides (§5.4.6/§5.4.7), in [`ZoneId::ALL`] order.
    /// `None` if `ObjectZoneControl == 0` (zone control not used).
    pub zone_gains: Option<[ZoneGain; MAX_KNOWN_ZONES]>,
    /// `ObjectSpreadMode` (§5.4.8).
    pub spread_mode: ObjectSpreadMode,
    /// `ObjectSpread` — raw `DistanceZ`-coded spread amount (§5.4.9). Width
    /// depends on `spread_mode` (8 or 12 bits); `0` (with no wire
    /// representation at all) when `spread_mode` is a reserved code, per
    /// the spec's `else { ObjectSpread[sb] = 0.0 }` branch.
    pub spread: u16,
    /// `ObjectDecorCoefPrefix` (§5.4.10).
    pub decor_coef_prefix: DecorCoefPrefix,
    /// `ObjectDecorCoef` (§5.4.11): `0` = no decorrelation, `255` = maximum.
    /// `Some` iff `decor_coef_prefix.reads_coef()`.
    pub decor_coef: Option<u8>,
}

/// One `NumPanSubBlocks` sub-block (§4.4's `for(sb...)` loop body).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanSubBlock {
    /// `Some` iff `PanInfoExists == 1` for this sub-block (always `Some` for
    /// sub-block 0, per §5.4.2). `None` means "repeat the previous
    /// sub-block's pan info" and carries no wire bits of its own beyond the
    /// `PanInfoExists` flag itself.
    pub pan: Option<PanInfo>,
}

/// `AudioDescription` — RDD 29 §4.4 (syntax only; no `5.4.x` prose section
/// documents this field's semantics — see `docs/rdd29.md` scope decision 2).
///
/// `text` is a borrowed `Option<&'a [u8]>`: serde_json's default
/// array-of-numbers representation cannot round-trip back into a borrowed
/// byte slice (it requires the deserializer's "bytes" hint, which a plain
/// JSON array does not satisfy) -- so, like `st337::Burst`, this type
/// derives `Serialize` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AudioDescription<'a> {
    /// The leading byte. Bit `0x80` gates `text`; the meaning of the
    /// remaining bits (when `0x80` is clear) is not documented anywhere in
    /// the disclosure, so this crate preserves the byte verbatim rather
    /// than decomposing it further.
    pub flag_byte: u8,
    /// The NULL-terminated ASCII text (terminator excluded), present iff
    /// `flag_byte & 0x80 != 0`.
    pub text: Option<&'a [u8]>,
}

const AUDIO_DESCRIPTION_TEXT_FOLLOWS: u8 = 0x80;

impl<'a> AudioDescription<'a> {
    /// Build an `AudioDescription` with no text (`flag_byte` with bit
    /// `0x80` clear).
    #[must_use]
    pub fn none() -> Self {
        Self {
            flag_byte: 0,
            text: None,
        }
    }

    /// Build an `AudioDescription` carrying NULL-terminated ASCII `text`.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] if `text` itself contains a `0x00` byte (it
    /// would be indistinguishable from the terminator on the wire).
    pub fn with_text(text: &'a [u8]) -> Result<Self> {
        if text.contains(&0) {
            return Err(Error::InvalidValue {
                field: "AudioDescription.text",
                value: 0,
                reason: "text must not contain an embedded NUL byte",
            });
        }
        Ok(Self {
            flag_byte: AUDIO_DESCRIPTION_TEXT_FOLLOWS,
            text: Some(text),
        })
    }
}

/// The `ObjectDefinition1` element — RDD 29 §2.3/§4.4/§5.4: pan/rendering
/// metadata (position, snap, zone gain, spread, decorrelation) and a
/// pointer to audio essence, for one frame of one panned audio object.
///
/// Derives `Serialize` only (not `Deserialize`) — it embeds
/// [`AudioDescription`], which has the same borrowed-`&[u8]` limitation.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ObjectDefinition1<'a> {
    /// `MetaID` — unique ID identifying this object across frames (§2.3).
    pub meta_id: u32,
    /// `AudioDataID` — the `AudioDataDLC` element carrying this object's
    /// audio essence.
    pub audio_data_id: u32,
    /// One entry per `NumPanSubBlocks` sub-block (§5.4.1 Table 7; derived
    /// from the enclosing `ATMOSFrame`'s `FrameRate`, not stored here).
    pub pan_sub_blocks: Vec<PanSubBlock>,
    /// `AudioDescription` (§4.4).
    pub audio_description: AudioDescription<'a>,
}

impl<'a> ObjectDefinition1<'a> {
    /// Build a new `ObjectDefinition1`.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] if `pan_sub_blocks` is non-empty and its
    /// first entry's `pan` is `None` — sub-block 0 always carries pan info
    /// (`PanInfoExists` is hard-coded `1` for `sb == 0`, §5.4.2), so a
    /// `None` there cannot be represented on the wire.
    pub fn new(
        meta_id: u32,
        audio_data_id: u32,
        pan_sub_blocks: Vec<PanSubBlock>,
        audio_description: AudioDescription<'a>,
    ) -> Result<Self> {
        if let Some(first) = pan_sub_blocks.first() {
            if first.pan.is_none() {
                return Err(Error::InvalidValue {
                    field: "ObjectDefinition1.pan_sub_blocks[0]",
                    value: 0,
                    reason: "sub-block 0 always carries pan info (PanInfoExists is implied 1)",
                });
            }
        }
        Ok(Self {
            meta_id,
            audio_data_id,
            pan_sub_blocks,
            audio_description,
        })
    }

    /// Parse an `ObjectDefinition1` element body.
    ///
    /// Unlike every other element type in this crate, `ObjectDefinition1`
    /// cannot implement [`broadcast_common::Parse`]: its pan-info loop
    /// length (`NumPanSubBlocks`, §5.4.1 Table 7) is derived from the
    /// enclosing `ATMOSFrame`'s `FrameRate`, which is not available from
    /// `bytes` alone — RDD 29's own syntax makes this element's body
    /// genuinely ambiguous out of context. See `docs/rdd29.md` scope
    /// decision 5.
    ///
    /// # Errors
    /// Returns [`Error`] on any protocol violation, buffer underrun, or if
    /// `frame_rate` is [`FrameRate::Reserved`] (no `NumPanSubBlocks` entry).
    pub fn parse_with_frame_rate(bytes: &'a [u8], frame_rate: FrameRate) -> Result<Self> {
        let num_pan_sub_blocks = frame_rate.num_pan_sub_blocks()?;
        let mut r = BitReader::new(bytes);
        let meta_id = read_plex(&mut r, 8, "ObjectDefinition1.MetaID")? as u32;
        let audio_data_id = read_plex(&mut r, 8, "ObjectDefinition1.AudioDataID")? as u32;
        read_reserved(
            &mut r,
            11,
            RESERVED_HEADER,
            "ObjectDefinition1.Reserved(header)",
        )?;

        let mut pan_sub_blocks = Vec::with_capacity(usize::from(num_pan_sub_blocks));
        for sb in 0..num_pan_sub_blocks {
            let pan_info_exists = if sb == 0 {
                true
            } else {
                r.read_bool().ctx("ObjectDefinition1.PanInfoExists")?
            };
            let pan = if pan_info_exists {
                Some(Self::parse_pan_info(&mut r)?)
            } else {
                None
            };
            pan_sub_blocks.push(PanSubBlock { pan });
        }

        r.align_to_byte();
        let audio_description = Self::parse_audio_description(bytes, &mut r)?;

        read_reserved(
            &mut r,
            8,
            RESERVED_FINAL,
            "ObjectDefinition1.Reserved(final)",
        )?;
        expect_fully_consumed(&r, "ObjectDefinition1")?;

        Ok(Self {
            meta_id,
            audio_data_id,
            pan_sub_blocks,
            audio_description,
        })
    }

    fn parse_pan_info(r: &mut BitReader<'_>) -> Result<PanInfo> {
        read_reserved(
            r,
            5,
            RESERVED_PAN_INFO,
            "ObjectDefinition1.Reserved(pan-info)",
        )?;
        let pos_x = r.read_bits(16).ctx("ObjectDefinition1.ObjectPosX")? as u16;
        let pos_y = r.read_bits(16).ctx("ObjectDefinition1.ObjectPosY")? as u16;
        let pos_z = r.read_bits(16).ctx("ObjectDefinition1.ObjectPosZ")? as u16;
        let snap = r.read_bool().ctx("ObjectDefinition1.ObjectSnap")?;
        if snap {
            read_reserved(r, 2, RESERVED_SNAP, "ObjectDefinition1.Reserved(snap)")?;
        }
        let zone_control = r.read_bool().ctx("ObjectDefinition1.ObjectZoneControl")?;
        let zone_gains = if zone_control {
            let mut gains = [ZoneGain::GainZero; MAX_KNOWN_ZONES];
            for g in &mut gains {
                *g = ZoneGain::from_bits(r.read_bits(2).ctx("ObjectDefinition1.ZoneGain")?);
            }
            Some(gains)
        } else {
            None
        };
        let spread_mode =
            ObjectSpreadMode::from_bits(r.read_bits(2).ctx("ObjectDefinition1.ObjectSpreadMode")?);
        let spread = match spread_mode.spread_width() {
            Some(width) => r.read_bits(width).ctx("ObjectDefinition1.ObjectSpread")? as u16,
            None => 0,
        };
        read_reserved(
            r,
            4,
            RESERVED_PRE_DECOR,
            "ObjectDefinition1.Reserved(pre-decor)",
        )?;
        let decor_coef_prefix = DecorCoefPrefix::from_bits(
            r.read_bits(2)
                .ctx("ObjectDefinition1.ObjectDecorCoefPrefix")?,
        );
        let decor_coef = if decor_coef_prefix.reads_coef() {
            Some(r.read_bits(8).ctx("ObjectDefinition1.ObjectDecorCoef")? as u8)
        } else {
            None
        };
        Ok(PanInfo {
            pos_x,
            pos_y,
            pos_z,
            snap,
            zone_gains,
            spread_mode,
            spread,
            decor_coef_prefix,
            decor_coef,
        })
    }

    fn parse_audio_description(
        bytes: &'a [u8],
        r: &mut BitReader<'a>,
    ) -> Result<AudioDescription<'a>> {
        debug_assert!(r.is_byte_aligned());
        let flag_start = r.bits_read() / 8;
        let flag_byte = bytes[flag_start];
        r.skip_bits(8).ctx("ObjectDefinition1.AudioDescription")?;
        let text =
            if flag_byte & AUDIO_DESCRIPTION_TEXT_FOLLOWS != 0 {
                let text_start = flag_start + 1;
                let nul_offset = bytes[text_start..].iter().position(|&b| b == 0x00).ok_or(
                    Error::InvalidValue {
                        field: "ObjectDefinition1.AudioDescription",
                        value: 0,
                        reason: "NULL-terminated text ran past the end of the element body",
                    },
                )?;
                let text_end = text_start + nul_offset;
                let consumed_bytes = (text_end + 1) - text_start; // text + terminator
                r.skip_bits(consumed_bytes * 8)
                    .ctx("ObjectDefinition1.AudioDescription(text)")?;
                Some(&bytes[text_start..text_end])
            } else {
                None
            };
        Ok(AudioDescription { flag_byte, text })
    }

    fn pan_info_bits(pan: Option<&PanInfo>, is_first_sub_block: bool) -> u32 {
        let mut bits = u32::from(!is_first_sub_block); // PanInfoExists, absent for sb==0
        if let Some(info) = pan {
            bits += 5; // Reserved(pan-info)
            bits += 16 * 3; // ObjectPosX/Y/Z
            bits += 1; // ObjectSnap
            if info.snap {
                bits += 2; // Reserved(snap)
            }
            bits += 1; // ObjectZoneControl
            if info.zone_gains.is_some() {
                bits += 2 * MAX_KNOWN_ZONES as u32; // ZoneGain * 9
            }
            bits += 2; // ObjectSpreadMode
            bits += info.spread_mode.spread_width().unwrap_or(0);
            bits += 4; // Reserved(pre-decor)
            bits += 2; // ObjectDecorCoefPrefix
            if info.decor_coef.is_some() {
                bits += 8; // ObjectDecorCoef
            }
        }
        bits
    }
}

impl Serialize for ObjectDefinition1<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut bits = plex_bits(u64::from(self.meta_id), 8)
            + plex_bits(u64::from(self.audio_data_id), 8)
            + 11;
        for (i, sb) in self.pan_sub_blocks.iter().enumerate() {
            bits += Self::pan_info_bits(sb.pan.as_ref(), i == 0);
        }
        let mut total_bytes = (bits as usize).div_ceil(8); // AlignBits boundary
        total_bytes += 1; // AudioDescription flag byte
        if let Some(text) = self.audio_description.text {
            total_bytes += text.len() + 1; // + NUL terminator
        }
        total_bytes += 1; // Reserved(final)
        total_bytes
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: buf.len(),
                what: "ObjectDefinition1",
            });
        }
        let mut w = BitWriter::new(&mut buf[..need]);
        write_plex(
            &mut w,
            u64::from(self.meta_id),
            8,
            "ObjectDefinition1.MetaID",
        )?;
        write_plex(
            &mut w,
            u64::from(self.audio_data_id),
            8,
            "ObjectDefinition1.AudioDataID",
        )?;
        write_reserved(
            &mut w,
            11,
            RESERVED_HEADER,
            "ObjectDefinition1.Reserved(header)",
        )?;

        for (i, sb) in self.pan_sub_blocks.iter().enumerate() {
            if i > 0 {
                w.write_bool(sb.pan.is_some())
                    .ctx("ObjectDefinition1.PanInfoExists")?;
            }
            if let Some(info) = &sb.pan {
                write_pan_info(&mut w, info)?;
            }
        }

        w.align_to_byte().map_err(|source| Error::Bits {
            what: "ObjectDefinition1.AlignBits",
            source,
        })?;
        w.write_bits(u64::from(self.audio_description.flag_byte), 8)
            .ctx("ObjectDefinition1.AudioDescription")?;
        if let Some(text) = self.audio_description.text {
            for &b in text {
                w.write_bits(u64::from(b), 8)
                    .ctx("ObjectDefinition1.AudioDescription(text)")?;
            }
            w.write_bits(0, 8)
                .ctx("ObjectDefinition1.AudioDescription(terminator)")?;
        }
        write_reserved(
            &mut w,
            8,
            RESERVED_FINAL,
            "ObjectDefinition1.Reserved(final)",
        )?;
        Ok(need)
    }
}

fn write_pan_info(w: &mut BitWriter<'_>, info: &PanInfo) -> Result<()> {
    write_reserved(
        w,
        5,
        RESERVED_PAN_INFO,
        "ObjectDefinition1.Reserved(pan-info)",
    )?;
    w.write_bits(u64::from(info.pos_x), 16)
        .ctx("ObjectDefinition1.ObjectPosX")?;
    w.write_bits(u64::from(info.pos_y), 16)
        .ctx("ObjectDefinition1.ObjectPosY")?;
    w.write_bits(u64::from(info.pos_z), 16)
        .ctx("ObjectDefinition1.ObjectPosZ")?;
    w.write_bool(info.snap)
        .ctx("ObjectDefinition1.ObjectSnap")?;
    if info.snap {
        write_reserved(w, 2, RESERVED_SNAP, "ObjectDefinition1.Reserved(snap)")?;
    }
    w.write_bool(info.zone_gains.is_some())
        .ctx("ObjectDefinition1.ObjectZoneControl")?;
    if let Some(gains) = info.zone_gains {
        for gain in gains {
            w.write_bits(gain.to_bits(), 2)
                .ctx("ObjectDefinition1.ZoneGain")?;
        }
    }
    w.write_bits(info.spread_mode.to_bits(), 2)
        .ctx("ObjectDefinition1.ObjectSpreadMode")?;
    if let Some(width) = info.spread_mode.spread_width() {
        w.write_bits(u64::from(info.spread), width)
            .ctx("ObjectDefinition1.ObjectSpread")?;
    }
    write_reserved(
        w,
        4,
        RESERVED_PRE_DECOR,
        "ObjectDefinition1.Reserved(pre-decor)",
    )?;
    w.write_bits(info.decor_coef_prefix.to_bits(), 2)
        .ctx("ObjectDefinition1.ObjectDecorCoefPrefix")?;
    if let Some(coef) = info.decor_coef {
        w.write_bits(u64::from(coef), 8)
            .ctx("ObjectDefinition1.ObjectDecorCoef")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_pan_info() -> PanInfo {
        PanInfo {
            pos_x: 0x8000,
            pos_y: 0x8000,
            pos_z: 0x8000,
            snap: false,
            zone_gains: None,
            spread_mode: ObjectSpreadMode::Reserved(1),
            spread: 0,
            decor_coef_prefix: DecorCoefPrefix::NoDecorrelation,
            decor_coef: None,
        }
    }

    fn full_pan_info() -> PanInfo {
        PanInfo {
            pos_x: 0x1234,
            pos_y: 0x5678,
            pos_z: 0x9ABC,
            snap: true,
            zone_gains: Some([ZoneGain::GainOne; MAX_KNOWN_ZONES]),
            spread_mode: ObjectSpreadMode::OneD,
            spread: 0x0AB,
            decor_coef_prefix: DecorCoefPrefix::CoefFollows,
            decor_coef: Some(128),
        }
    }

    fn sample(frame_rate: FrameRate) -> ObjectDefinition1<'static> {
        let n = usize::from(frame_rate.num_pan_sub_blocks().unwrap());
        let mut pan_sub_blocks = alloc::vec![PanSubBlock {
            pan: Some(full_pan_info())
        }];
        for i in 1..n {
            pan_sub_blocks.push(PanSubBlock {
                pan: if i % 2 == 0 {
                    Some(minimal_pan_info())
                } else {
                    None
                },
            });
        }
        ObjectDefinition1::new(
            42,
            7,
            pan_sub_blocks,
            AudioDescription::with_text(b"dialogue").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn round_trips_at_every_frame_rate() {
        for fr in [
            FrameRate::Fps24,
            FrameRate::Fps48,
            FrameRate::Fps96,
            FrameRate::Fps120,
        ] {
            let obj = sample(fr);
            let bytes = obj.to_bytes();
            let parsed = ObjectDefinition1::parse_with_frame_rate(&bytes, fr).unwrap();
            assert_eq!(parsed, obj, "frame rate {fr:?}");
            assert_eq!(parsed.to_bytes(), bytes);
        }
    }

    #[test]
    fn no_description_text_round_trips() {
        // FrameRate::Fps120 requires exactly 2 pan sub-blocks (Table 7).
        let obj = ObjectDefinition1::new(
            1,
            2,
            alloc::vec![
                PanSubBlock {
                    pan: Some(minimal_pan_info())
                },
                PanSubBlock { pan: None },
            ],
            AudioDescription::none(),
        )
        .unwrap();
        let bytes = obj.to_bytes();
        let parsed = ObjectDefinition1::parse_with_frame_rate(&bytes, FrameRate::Fps120).unwrap();
        assert_eq!(parsed, obj);
    }

    #[test]
    fn leading_sub_block_without_pan_info_is_rejected() {
        let err = ObjectDefinition1::new(
            0,
            0,
            alloc::vec![PanSubBlock { pan: None }],
            AudioDescription::none(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }));
    }

    #[test]
    fn embedded_nul_in_text_is_rejected() {
        assert!(AudioDescription::with_text(b"bad\0text").is_err());
    }

    #[test]
    fn reserved_frame_rate_is_rejected() {
        let obj = sample(FrameRate::Fps24);
        let bytes = obj.to_bytes();
        let err =
            ObjectDefinition1::parse_with_frame_rate(&bytes, FrameRate::Reserved(0x9)).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }));
    }

    #[test]
    fn mutating_pos_x_changes_only_that_sub_blocks_bytes() {
        let mut obj = sample(FrameRate::Fps120);
        let original = obj.to_bytes();
        obj.pan_sub_blocks[0].pan.as_mut().unwrap().pos_x ^= 0xFFFF;
        let mutated = obj.to_bytes();
        assert_ne!(original, mutated, "flipping pos_x must change wire bytes");
    }
}
