//! ST 2022-6 HBRMT Payload Header — SMPTE ST 2022-6:2012 §6.4 (Figure 3).
//!
//! See `st2022/docs/st2022-6-framing.md` §3 for the curated spec
//! transcription this module implements field-for-field (including the
//! derived-not-quoted width of [`PayloadHeader::reserve`] — see that doc's
//! "Note on `RESERVE`'s width").
//!
//! Wire layout (network byte order / big-endian, matching the RTP header
//! this payload header always follows):
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |Ext    |F|VSID | FRCount        | R | S | FEC | CF    | RESERVE |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! | MAP |       FRAME      |     FRATE     |SAMPLE | FMT-RESERVE |  <- iff F=1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                    Video timestamp (only if CF>0)              |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                   Header extension (only if Ext>0)             |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```
//!
//! `F` and `Ext` are not stored as independent fields: `F` is derived from
//! [`PayloadHeader::video_source_format`] being `Some`, and the extension
//! byte count is derived from [`PayloadHeader::extension`]'s slice length —
//! the same "derive from the data that is actually present" discipline used
//! throughout this workspace (e.g. `st337::BurstPreamble`'s six-word-preamble
//! detection) so caller state can never disagree with the wire.

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Named constants (no magic numbers) — SMPTE ST 2022-6 §6.4
// ---------------------------------------------------------------------------

/// Byte length of the fixed first row (`Ext`/`F`/`VSID`/`FRCount`/`R`/`S`/
/// `FEC`/`CF`/`RESERVE`) — always present.
pub const FIXED_HEADER_LEN: usize = 4;

/// Byte length of the Video Source Format row (`MAP`/`FRAME`/`FRATE`/
/// `SAMPLE`/`FMT-RESERVE`), present iff `F` = 1.
pub const VIDEO_SOURCE_FORMAT_LEN: usize = 4;

/// Byte length of the Video Timestamp row, present iff `CF` != 0.
pub const VIDEO_TIMESTAMP_LEN: usize = 4;

/// Byte length of one Header Extension unit — `Ext` counts these
/// (§6.4: "this value x 4 octets").
pub const EXTENSION_UNIT_LEN: usize = 4;

/// Maximum `Ext` value: a 4-bit field, so the header extension can be at
/// most `15 * 4` = 60 octets.
pub const MAX_EXTENSION_UNITS: u8 = 0x0F;

// -- byte 0: Ext(4) | F(1) | VSID(3) --
const EXT_SHIFT: u32 = 4;
const F_BIT: u8 = 0x08;
const VSID_MASK: u8 = 0x07;

// -- bytes 2..4, big-endian u16: R(2) | S(2) | FEC(3) | CF(4) | RESERVE(5) --
const R_SHIFT: u32 = 14;
const R_MASK: u16 = 0x0003;
const S_SHIFT: u32 = 12;
const S_MASK: u16 = 0x0003;
const FEC_SHIFT: u32 = 9;
const FEC_MASK: u16 = 0x0007;
const CF_SHIFT: u32 = 5;
const CF_MASK: u16 = 0x000F;
const RESERVE_MASK: u16 = 0x001F;

/// Maximum value of [`PayloadHeader::reserve`] — a 5-bit field.
pub const MAX_RESERVE: u8 = 0x1F;

// -- Video Source Format row, big-endian u32: MAP(4)|FRAME(8)|FRATE(8)|SAMPLE(4)|FMT-RESERVE(8) --
const MAP_SHIFT: u32 = 28;
const MAP_MASK: u32 = 0x0000_000F;
const FRAME_SHIFT: u32 = 20;
const FRAME_MASK: u32 = 0x0000_00FF;
const FRATE_SHIFT: u32 = 12;
const FRATE_MASK: u32 = 0x0000_00FF;
const SAMPLE_SHIFT: u32 = 8;
const SAMPLE_MASK: u32 = 0x0000_000F;
const FMT_RESERVE_MASK: u32 = 0x0000_00FF;

// ---------------------------------------------------------------------------
// VideoSourceId — VSID field, §6.4
// ---------------------------------------------------------------------------

/// `VSID` — Video source ID / protection profile, 3 bits (§6.4).
///
/// Per ST 2022-7 (see `docs/st2022-7-hitless.md`), this field's value is
/// expected to be identical across ST 2022-7 datagram copies of the same
/// content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum VideoSourceId {
    /// `000` — primary stream.
    Primary,
    /// `001` — protect stream.
    Protect,
    /// `010`-`111` — reserved.
    Reserved(u8),
}

impl VideoSourceId {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Primary => "primary stream",
            Self::Protect => "protect stream",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Primary,
            1 => Self::Protect,
            other => Self::Reserved(other),
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Protect => 1,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(VideoSourceId, Reserved);

// ---------------------------------------------------------------------------
// TimestampRef — R field, §6.4
// ---------------------------------------------------------------------------

/// `R` — Timestamp reference, 2 bits (§6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum TimestampRef {
    /// `00` — not locked.
    NotLocked,
    /// `10` — locked to UTC time/frequency reference.
    LockedUtc,
    /// `11` — locked to a private time/frequency reference.
    LockedPrivate,
    /// `01` — reserved.
    Reserved(u8),
}

impl TimestampRef {
    /// The spec token for this value ("reserved" for the reserved code
    /// point) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotLocked => "not locked",
            Self::LockedUtc => "locked to UTC time/frequency reference",
            Self::LockedPrivate => "locked to a private time/frequency reference",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u16) -> Self {
        match bits {
            0 => Self::NotLocked,
            2 => Self::LockedUtc,
            3 => Self::LockedPrivate,
            #[allow(clippy::cast_possible_truncation)]
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u16 {
        match self {
            Self::NotLocked => 0,
            Self::LockedUtc => 2,
            Self::LockedPrivate => 3,
            Self::Reserved(v) => u16::from(v),
        }
    }
}

broadcast_common::impl_spec_display!(TimestampRef, Reserved);

// ---------------------------------------------------------------------------
// Scrambling — S field, §6.4
// ---------------------------------------------------------------------------

/// `S` — Video payload scrambling, 2 bits (§6.4). Any non-`00` value is
/// explicitly out of ST 2022-6's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Scrambling {
    /// `00` — not scrambled.
    NotScrambled,
    /// `01`/`10`/`11` — reserved for future use.
    Reserved(u8),
}

impl Scrambling {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotScrambled => "not scrambled",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u16) -> Self {
        match bits {
            0 => Self::NotScrambled,
            #[allow(clippy::cast_possible_truncation)]
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u16 {
        match self {
            Self::NotScrambled => 0,
            Self::Reserved(v) => u16::from(v),
        }
    }
}

broadcast_common::impl_spec_display!(Scrambling, Reserved);

// ---------------------------------------------------------------------------
// FecUsage — FEC field, §6.4
// ---------------------------------------------------------------------------

/// `FEC` — FEC usage, 3 bits (§6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FecUsage {
    /// `000` — no FEC stream.
    None,
    /// `001` — L (Column) FEC utilized.
    ColumnOnly,
    /// `010` — L & D (Column & Row) FEC utilized.
    ColumnAndRow,
    /// `011`-`111` — reserved.
    Reserved(u8),
}

impl FecUsage {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "no FEC stream",
            Self::ColumnOnly => "L (Column) FEC utilized",
            Self::ColumnAndRow => "L & D (Column & Row) FEC utilized",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u16) -> Self {
        match bits {
            0 => Self::None,
            1 => Self::ColumnOnly,
            2 => Self::ColumnAndRow,
            #[allow(clippy::cast_possible_truncation)]
            other => Self::Reserved(other as u8),
        }
    }

    fn to_bits(self) -> u16 {
        match self {
            Self::None => 0,
            Self::ColumnOnly => 1,
            Self::ColumnAndRow => 2,
            Self::Reserved(v) => u16::from(v),
        }
    }
}

broadcast_common::impl_spec_display!(FecUsage, Reserved);

// ---------------------------------------------------------------------------
// ClockFrequency — CF field, §6.4
// ---------------------------------------------------------------------------

/// `CF` — Clock Frequency, 4 bits (§6.4). Non-[`ClockFrequency::NoTimestamp`]
/// requires the sender to include the 32-bit Video Timestamp row;
/// [`ClockFrequency::NoTimestamp`] requires the sender to omit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ClockFrequency {
    /// `0000` — no timestamp.
    NoTimestamp,
    /// `0001` — 27 MHz.
    Mhz27,
    /// `0010` — 148.5 MHz.
    Mhz148_5,
    /// `0011` — 148.5/1.001 MHz.
    Mhz148_5Div1001,
    /// `0100` — 297 MHz.
    Mhz297,
    /// `0101` — 297/1.001 MHz.
    Mhz297Div1001,
    /// `0110`-`1111` — reserved.
    Reserved(u8),
}

impl ClockFrequency {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NoTimestamp => "no timestamp",
            Self::Mhz27 => "27 MHz",
            Self::Mhz148_5 => "148.5 MHz",
            Self::Mhz148_5Div1001 => "148.5/1.001 MHz",
            Self::Mhz297 => "297 MHz",
            Self::Mhz297Div1001 => "297/1.001 MHz",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::NoTimestamp,
            1 => Self::Mhz27,
            2 => Self::Mhz148_5,
            3 => Self::Mhz148_5Div1001,
            4 => Self::Mhz297,
            5 => Self::Mhz297Div1001,
            other => Self::Reserved(other),
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::NoTimestamp => 0,
            Self::Mhz27 => 1,
            Self::Mhz148_5 => 2,
            Self::Mhz148_5Div1001 => 3,
            Self::Mhz297 => 4,
            Self::Mhz297Div1001 => 5,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(ClockFrequency, Reserved);

// ---------------------------------------------------------------------------
// MapStructure — MAP field, §6.4 (Video Source Format row)
// ---------------------------------------------------------------------------

/// `MAP` — top-level structure of the data stream, 4 bits (§6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MapStructure {
    /// `0x00` — Direct sample structure per SMPTE ST 292-1 / SMPTE ST 425-1
    /// Level A, etc.
    Direct,
    /// `0x01` — SMPTE ST 425-1 Level B-DL mapping of ST 372 Dual-Link.
    LevelBDl,
    /// `0x02` — SMPTE ST 425-1 Level B-DS mapping of two ST 292-1 streams.
    LevelBDs,
    /// `0x03`-`0x0F` — reserved.
    Reserved(u8),
}

impl MapStructure {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Direct => "direct sample structure",
            Self::LevelBDl => "ST 425-1 Level B-DL mapping of ST 372 Dual-Link",
            Self::LevelBDs => "ST 425-1 Level B-DS mapping of two ST 292-1 streams",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Direct,
            1 => Self::LevelBDl,
            2 => Self::LevelBDs,
            other => Self::Reserved(other),
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Direct => 0,
            Self::LevelBDl => 1,
            Self::LevelBDs => 2,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(MapStructure, Reserved);

// ---------------------------------------------------------------------------
// FrameStructure — FRAME field, §6.4 (Video Source Format row)
// ---------------------------------------------------------------------------

/// `FRAME` — luminance active pixel structure, 8 bits (§6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FrameStructure {
    /// `0x00` — unknown/unspecified frame structure.
    Unspecified,
    /// `0x10` — 720x486/525, interlace sampling, interlace transport.
    Sd525i,
    /// `0x11` — 720x576/625, interlace sampling, interlace transport.
    Sd625i,
    /// `0x20` — 1920x1080/1125, interlace sampling, interlace transport.
    Hd1080i,
    /// `0x21` — 1920x1080/1125, progressive sampling, progressive transport.
    Hd1080p,
    /// `0x22` — 1920x1080/1125, progressive sampling, interlace
    /// (segmented-frame) transport.
    Hd1080psf,
    /// `0x23` — 2048x1080/1125, progressive sampling, progressive transport.
    Hd2048x1080p,
    /// `0x24` — 2048x1080/1125, progressive sampling, interlace
    /// (segmented-frame) transport.
    Hd2048x1080psf,
    /// `0x30` — 1280x720/750, progressive sampling, progressive transport.
    Hd720p,
    /// Any other code point (§6.4: `0x01`-`0x0F`, `0x12`-`0x1F`,
    /// `0x25`-`0x2F`, `0x31`-`0xFF` are reserved).
    Reserved(u8),
}

impl FrameStructure {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unspecified => "unknown/unspecified frame structure",
            Self::Sd525i => "720x486/525 interlace",
            Self::Sd625i => "720x576/625 interlace",
            Self::Hd1080i => "1920x1080/1125 interlace",
            Self::Hd1080p => "1920x1080/1125 progressive",
            Self::Hd1080psf => "1920x1080/1125 progressive segmented frame",
            Self::Hd2048x1080p => "2048x1080/1125 progressive",
            Self::Hd2048x1080psf => "2048x1080/1125 progressive segmented frame",
            Self::Hd720p => "1280x720/750 progressive",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits {
            0x00 => Self::Unspecified,
            0x10 => Self::Sd525i,
            0x11 => Self::Sd625i,
            0x20 => Self::Hd1080i,
            0x21 => Self::Hd1080p,
            0x22 => Self::Hd1080psf,
            0x23 => Self::Hd2048x1080p,
            0x24 => Self::Hd2048x1080psf,
            0x30 => Self::Hd720p,
            other => Self::Reserved(other),
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Unspecified => 0x00,
            Self::Sd525i => 0x10,
            Self::Sd625i => 0x11,
            Self::Hd1080i => 0x20,
            Self::Hd1080p => 0x21,
            Self::Hd1080psf => 0x22,
            Self::Hd2048x1080p => 0x23,
            Self::Hd2048x1080psf => 0x24,
            Self::Hd720p => 0x30,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(FrameStructure, Reserved);

// ---------------------------------------------------------------------------
// FrameRate — FRATE field, §6.4 (Video Source Format row)
// ---------------------------------------------------------------------------

/// `FRATE` — frame rate of the payload, 8 bits (§6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FrameRate {
    /// `0x00` — unknown/unspecified frame rate, 2.970 GHz signal.
    UnspecifiedAt2970Ghz,
    /// `0x01` — unknown/unspecified frame rate, 2.970/1.001 GHz signal.
    UnspecifiedAt2970Div1001Ghz,
    /// `0x02` — unknown/unspecified frame rate, 1.485 GHz signal.
    UnspecifiedAt1485Ghz,
    /// `0x03` — unknown/unspecified frame rate, 1.485/1.001 GHz signal.
    UnspecifiedAt1485Div1001Ghz,
    /// `0x04` — unknown/unspecified frame rate, 0.270 GHz signal.
    UnspecifiedAt0270Ghz,
    /// `0x10` — 60 Hz.
    Hz60,
    /// `0x11` — 60/1.001 Hz.
    Hz60Div1001,
    /// `0x12` — 50 Hz.
    Hz50,
    /// `0x14` — 48 Hz.
    Hz48,
    /// `0x15` — 48/1.001 Hz.
    Hz48Div1001,
    /// `0x16` — 30 Hz.
    Hz30,
    /// `0x17` — 30/1.001 Hz.
    Hz30Div1001,
    /// `0x18` — 25 Hz.
    Hz25,
    /// `0x1A` — 24 Hz.
    Hz24,
    /// `0x1B` — 24/1.001 Hz.
    Hz24Div1001,
    /// Any other code point (§6.4: `0x05`-`0x0F`, `0x13`, `0x19`,
    /// `0x1C`-`0xFF` are reserved).
    Reserved(u8),
}

impl FrameRate {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::UnspecifiedAt2970Ghz => "unspecified frame rate, 2.970 GHz signal",
            Self::UnspecifiedAt2970Div1001Ghz => "unspecified frame rate, 2.970/1.001 GHz signal",
            Self::UnspecifiedAt1485Ghz => "unspecified frame rate, 1.485 GHz signal",
            Self::UnspecifiedAt1485Div1001Ghz => "unspecified frame rate, 1.485/1.001 GHz signal",
            Self::UnspecifiedAt0270Ghz => "unspecified frame rate, 0.270 GHz signal",
            Self::Hz60 => "60 Hz",
            Self::Hz60Div1001 => "60/1.001 Hz",
            Self::Hz50 => "50 Hz",
            Self::Hz48 => "48 Hz",
            Self::Hz48Div1001 => "48/1.001 Hz",
            Self::Hz30 => "30 Hz",
            Self::Hz30Div1001 => "30/1.001 Hz",
            Self::Hz25 => "25 Hz",
            Self::Hz24 => "24 Hz",
            Self::Hz24Div1001 => "24/1.001 Hz",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits {
            0x00 => Self::UnspecifiedAt2970Ghz,
            0x01 => Self::UnspecifiedAt2970Div1001Ghz,
            0x02 => Self::UnspecifiedAt1485Ghz,
            0x03 => Self::UnspecifiedAt1485Div1001Ghz,
            0x04 => Self::UnspecifiedAt0270Ghz,
            0x10 => Self::Hz60,
            0x11 => Self::Hz60Div1001,
            0x12 => Self::Hz50,
            0x14 => Self::Hz48,
            0x15 => Self::Hz48Div1001,
            0x16 => Self::Hz30,
            0x17 => Self::Hz30Div1001,
            0x18 => Self::Hz25,
            0x1A => Self::Hz24,
            0x1B => Self::Hz24Div1001,
            other => Self::Reserved(other),
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::UnspecifiedAt2970Ghz => 0x00,
            Self::UnspecifiedAt2970Div1001Ghz => 0x01,
            Self::UnspecifiedAt1485Ghz => 0x02,
            Self::UnspecifiedAt1485Div1001Ghz => 0x03,
            Self::UnspecifiedAt0270Ghz => 0x04,
            Self::Hz60 => 0x10,
            Self::Hz60Div1001 => 0x11,
            Self::Hz50 => 0x12,
            Self::Hz48 => 0x14,
            Self::Hz48Div1001 => 0x15,
            Self::Hz30 => 0x16,
            Self::Hz30Div1001 => 0x17,
            Self::Hz25 => 0x18,
            Self::Hz24 => 0x1A,
            Self::Hz24Div1001 => 0x1B,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(FrameRate, Reserved);

// ---------------------------------------------------------------------------
// SampleStructure — SAMPLE field, §6.4 (Video Source Format row)
// ---------------------------------------------------------------------------

/// `SAMPLE` — component pixel sampling structure and bit depth, 4 bits
/// (§6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SampleStructure {
    /// `0x00` — unknown/unspecified.
    Unspecified,
    /// `0x01` — 4:2:2, 10 bits.
    Yuv422At10Bit,
    /// `0x02` — 4:4:4, 10 bits.
    Yuv444At10Bit,
    /// `0x03` — 4:4:4:4, 10 bits.
    Yuv4444At10Bit,
    /// `0x05` — 4:2:2, 12 bits.
    Yuv422At12Bit,
    /// `0x06` — 4:4:4, 12 bits.
    Yuv444At12Bit,
    /// `0x07` — 4:4:4:4, 12 bits.
    Yuv4444At12Bit,
    /// `0x08` — 4:2:2:4, 12 bits.
    Yuv4224At12Bit,
    /// Any other code point (§6.4: `0x04`, `0x09`-`0x0F` are reserved).
    Reserved(u8),
}

impl SampleStructure {
    /// The spec token for this value ("reserved" for the reserved code
    /// points) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Yuv422At10Bit => "4:2:2, 10 bits",
            Self::Yuv444At10Bit => "4:4:4, 10 bits",
            Self::Yuv4444At10Bit => "4:4:4:4, 10 bits",
            Self::Yuv422At12Bit => "4:2:2, 12 bits",
            Self::Yuv444At12Bit => "4:4:4, 12 bits",
            Self::Yuv4444At12Bit => "4:4:4:4, 12 bits",
            Self::Yuv4224At12Bit => "4:2:2:4, 12 bits",
            Self::Reserved(_) => "reserved",
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits {
            0x00 => Self::Unspecified,
            0x01 => Self::Yuv422At10Bit,
            0x02 => Self::Yuv444At10Bit,
            0x03 => Self::Yuv4444At10Bit,
            0x05 => Self::Yuv422At12Bit,
            0x06 => Self::Yuv444At12Bit,
            0x07 => Self::Yuv4444At12Bit,
            0x08 => Self::Yuv4224At12Bit,
            other => Self::Reserved(other),
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Unspecified => 0x00,
            Self::Yuv422At10Bit => 0x01,
            Self::Yuv444At10Bit => 0x02,
            Self::Yuv4444At10Bit => 0x03,
            Self::Yuv422At12Bit => 0x05,
            Self::Yuv444At12Bit => 0x06,
            Self::Yuv4444At12Bit => 0x07,
            Self::Yuv4224At12Bit => 0x08,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(SampleStructure, Reserved);

// ---------------------------------------------------------------------------
// VideoSourceFormat — MAP/FRAME/FRATE/SAMPLE/FMT-RESERVE row, §6.4
// ---------------------------------------------------------------------------

/// The Video Source Format row (§6.4): `MAP`/`FRAME`/`FRATE`/`SAMPLE` plus
/// the trailing `FMT-RESERVE` byte, present in a [`PayloadHeader`] iff `F` =
/// 1 (this standard requires `F` = 1 — see `docs/st2022-6-framing.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VideoSourceFormat {
    /// `MAP` — top-level stream structure.
    pub map: MapStructure,
    /// `FRAME` — luminance active pixel structure.
    pub frame: FrameStructure,
    /// `FRATE` — frame rate.
    pub frate: FrameRate,
    /// `SAMPLE` — component pixel sampling structure and bit depth.
    pub sample: SampleStructure,
    /// `FMT-RESERVE` — reserved for future use, shall be set to 0 by the
    /// sender. Preserved verbatim on round-trip rather than forced to zero
    /// (the same discipline used for [`PayloadHeader::reserve`] and
    /// `st337::ExtendedPreamble::reserved_pf`).
    pub fmt_reserve: u8,
}

impl VideoSourceFormat {
    fn to_bits(self) -> u32 {
        (u32::from(self.map.to_bits()) << MAP_SHIFT)
            | (u32::from(self.frame.to_bits()) << FRAME_SHIFT)
            | (u32::from(self.frate.to_bits()) << FRATE_SHIFT)
            | (u32::from(self.sample.to_bits()) << SAMPLE_SHIFT)
            | (u32::from(self.fmt_reserve) & FMT_RESERVE_MASK)
    }

    #[allow(clippy::cast_possible_truncation)]
    fn from_bits(word: u32) -> Self {
        Self {
            map: MapStructure::from_bits(((word >> MAP_SHIFT) & MAP_MASK) as u8),
            frame: FrameStructure::from_bits(((word >> FRAME_SHIFT) & FRAME_MASK) as u8),
            frate: FrameRate::from_bits(((word >> FRATE_SHIFT) & FRATE_MASK) as u8),
            sample: SampleStructure::from_bits(((word >> SAMPLE_SHIFT) & SAMPLE_MASK) as u8),
            fmt_reserve: (word & FMT_RESERVE_MASK) as u8,
        }
    }
}

// ---------------------------------------------------------------------------
// PayloadHeader — the full HBRMT payload header, §6.4
// ---------------------------------------------------------------------------

/// The HBRMT Payload Header (§6.4) that precedes the media payload in every
/// ST 2022-6 RTP datagram: 4, 8, 12, or more bytes depending on whether the
/// Video Source Format row, Video Timestamp row, and/or Header Extension are
/// present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PayloadHeader<'a> {
    /// `VSID` — video source ID / protection profile.
    pub vsid: VideoSourceId,
    /// `FRCount` — video frame counter, rolls over mod 256.
    pub fr_count: u8,
    /// `R` — timestamp reference.
    pub timestamp_ref: TimestampRef,
    /// `S` — video payload scrambling.
    pub scrambling: Scrambling,
    /// `FEC` — FEC usage.
    pub fec_usage: FecUsage,
    /// `CF` — clock frequency. Determines whether [`Self::video_timestamp`]
    /// must be `Some` ([`ClockFrequency::NoTimestamp`] => `None`, anything
    /// else => `Some`).
    pub clock_frequency: ClockFrequency,
    /// `RESERVE` — 5 reserved bits, shall be set to 0 by the sender.
    /// Preserved verbatim on round-trip rather than forced to zero (ST
    /// 2022-6 §2's "reserved" is "not defined at this time," not "must be
    /// zero" — the same discipline `st337` uses for its own reserved
    /// fields).
    pub reserve: u8,
    /// The Video Source Format row, present iff `F` = 1. `F` is not stored
    /// independently — it is exactly `self.video_source_format.is_some()`.
    pub video_source_format: Option<VideoSourceFormat>,
    /// The 32-bit Video Timestamp, present iff `clock_frequency` !=
    /// [`ClockFrequency::NoTimestamp`].
    pub video_timestamp: Option<u32>,
    /// The Header Extension's raw `Ext * 4` octets (§6.4's Tag/Length/Value
    /// sub-structure — see `docs/st2022-6-framing.md` §3.4), present iff
    /// `Ext` != 0. `Ext` is not stored independently — it is exactly
    /// `self.extension.map_or(0, |e| e.len() / 4)`. This crate treats the
    /// extension as opaque bytes; it does not walk the TLV chain.
    pub extension: Option<&'a [u8]>,
}

impl<'a> PayloadHeader<'a> {
    /// Build a new payload header, validating field widths and the
    /// `clock_frequency` <-> `video_timestamp` / extension-length
    /// invariants.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] if `reserve` exceeds [`MAX_RESERVE`], or if
    /// `extension` is `Some` with a length that is zero, not a multiple of
    /// [`EXTENSION_UNIT_LEN`], or exceeds [`MAX_EXTENSION_UNITS`] units;
    /// [`Error::VideoTimestampMismatch`] if `video_timestamp.is_some()`
    /// disagrees with `clock_frequency != ClockFrequency::NoTimestamp`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        vsid: VideoSourceId,
        fr_count: u8,
        timestamp_ref: TimestampRef,
        scrambling: Scrambling,
        fec_usage: FecUsage,
        clock_frequency: ClockFrequency,
        reserve: u8,
        video_source_format: Option<VideoSourceFormat>,
        video_timestamp: Option<u32>,
        extension: Option<&'a [u8]>,
    ) -> Result<Self> {
        let header = Self {
            vsid,
            fr_count,
            timestamp_ref,
            scrambling,
            fec_usage,
            clock_frequency,
            reserve,
            video_source_format,
            video_timestamp,
            extension,
        };
        header.validate()?;
        Ok(header)
    }

    /// `Ext` — the wire nibble, derived from [`Self::extension`]'s length.
    #[must_use]
    pub fn ext(&self) -> u8 {
        #[allow(clippy::cast_possible_truncation)]
        self.extension
            .map_or(0, |e| (e.len() / EXTENSION_UNIT_LEN) as u8)
    }

    fn validate(&self) -> Result<()> {
        if self.reserve > MAX_RESERVE {
            return Err(Error::InvalidValue {
                field: "reserve",
                value: u64::from(self.reserve),
                reason: "must be a 5-bit value (0..=31)",
            });
        }
        let requires_timestamp = self.clock_frequency != ClockFrequency::NoTimestamp;
        if requires_timestamp != self.video_timestamp.is_some() {
            return Err(Error::VideoTimestampMismatch {
                clock_frequency: self.clock_frequency,
                expected_present: requires_timestamp,
                found_present: self.video_timestamp.is_some(),
            });
        }
        if let Some(ext) = self.extension {
            if ext.is_empty() || ext.len() % EXTENSION_UNIT_LEN != 0 {
                return Err(Error::InvalidValue {
                    field: "extension",
                    value: ext.len() as u64,
                    reason: "must be a non-zero multiple of 4 octets",
                });
            }
            let units = ext.len() / EXTENSION_UNIT_LEN;
            if units > MAX_EXTENSION_UNITS as usize {
                return Err(Error::InvalidValue {
                    field: "extension",
                    value: units as u64,
                    reason: "Ext is a 4-bit field: extension may be at most 15 * 4 = 60 octets",
                });
            }
        }
        Ok(())
    }
}

impl<'a> Parse<'a> for PayloadHeader<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FIXED_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_HEADER_LEN,
                have: bytes.len(),
                what: "HBRMT payload header fixed row",
            });
        }

        let byte0 = bytes[0];
        let ext = byte0 >> EXT_SHIFT;
        let f = byte0 & F_BIT != 0;
        let vsid = VideoSourceId::from_bits(byte0 & VSID_MASK);
        let fr_count = bytes[1];

        let word = u16::from_be_bytes([bytes[2], bytes[3]]);
        let timestamp_ref = TimestampRef::from_bits((word >> R_SHIFT) & R_MASK);
        let scrambling = Scrambling::from_bits((word >> S_SHIFT) & S_MASK);
        let fec_usage = FecUsage::from_bits((word >> FEC_SHIFT) & FEC_MASK);
        #[allow(clippy::cast_possible_truncation)]
        let clock_frequency = ClockFrequency::from_bits(((word >> CF_SHIFT) & CF_MASK) as u8);
        #[allow(clippy::cast_possible_truncation)]
        let reserve = (word & RESERVE_MASK) as u8;

        let mut pos = FIXED_HEADER_LEN;

        let video_source_format = if f {
            let end = pos + VIDEO_SOURCE_FORMAT_LEN;
            if bytes.len() < end {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: bytes.len(),
                    what: "video source format row",
                });
            }
            let vsf_word =
                u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
            pos = end;
            Some(VideoSourceFormat::from_bits(vsf_word))
        } else {
            None
        };

        let video_timestamp = if clock_frequency != ClockFrequency::NoTimestamp {
            let end = pos + VIDEO_TIMESTAMP_LEN;
            if bytes.len() < end {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: bytes.len(),
                    what: "video timestamp",
                });
            }
            let ts =
                u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
            pos = end;
            Some(ts)
        } else {
            None
        };

        let extension = if ext != 0 {
            let ext_len = usize::from(ext) * EXTENSION_UNIT_LEN;
            let end = pos + ext_len;
            if bytes.len() < end {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: bytes.len(),
                    what: "header extension",
                });
            }
            let data = &bytes[pos..end];
            pos = end;
            Some(data)
        } else {
            None
        };
        let _ = pos;

        Ok(Self {
            vsid,
            fr_count,
            timestamp_ref,
            scrambling,
            fec_usage,
            clock_frequency,
            reserve,
            video_source_format,
            video_timestamp,
            extension,
        })
    }
}

impl Serialize for PayloadHeader<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        FIXED_HEADER_LEN
            + self
                .video_source_format
                .map_or(0, |_| VIDEO_SOURCE_FORMAT_LEN)
            + self.video_timestamp.map_or(0, |_| VIDEO_TIMESTAMP_LEN)
            + self.extension.map_or(0, <[u8]>::len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.validate()?;
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "HBRMT payload header serialize output",
            });
        }

        let f_bit = if self.video_source_format.is_some() {
            F_BIT
        } else {
            0
        };
        buf[0] = (self.ext() << EXT_SHIFT) | f_bit | (self.vsid.to_bits() & VSID_MASK);
        buf[1] = self.fr_count;
        let word = (self.timestamp_ref.to_bits() << R_SHIFT)
            | (self.scrambling.to_bits() << S_SHIFT)
            | (self.fec_usage.to_bits() << FEC_SHIFT)
            | (u16::from(self.clock_frequency.to_bits()) << CF_SHIFT)
            | (u16::from(self.reserve) & RESERVE_MASK);
        buf[2..4].copy_from_slice(&word.to_be_bytes());

        let mut pos = FIXED_HEADER_LEN;
        if let Some(vsf) = self.video_source_format {
            buf[pos..pos + VIDEO_SOURCE_FORMAT_LEN].copy_from_slice(&vsf.to_bits().to_be_bytes());
            pos += VIDEO_SOURCE_FORMAT_LEN;
        }
        if let Some(ts) = self.video_timestamp {
            buf[pos..pos + VIDEO_TIMESTAMP_LEN].copy_from_slice(&ts.to_be_bytes());
            pos += VIDEO_TIMESTAMP_LEN;
        }
        if let Some(ext) = self.extension {
            buf[pos..pos + ext.len()].copy_from_slice(ext);
            pos += ext.len();
        }
        let _ = pos;

        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn minimal_header() -> PayloadHeader<'static> {
        PayloadHeader::new(
            VideoSourceId::Primary,
            0,
            TimestampRef::NotLocked,
            Scrambling::NotScrambled,
            FecUsage::None,
            ClockFrequency::NoTimestamp,
            0,
            None,
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn round_trips_the_minimal_4_byte_header() {
        let header = minimal_header();
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), FIXED_HEADER_LEN);
        let reparsed = PayloadHeader::parse(&bytes).unwrap();
        assert_eq!(reparsed, header);
    }

    #[test]
    fn round_trips_an_8_byte_header_with_video_source_format() {
        let vsf = VideoSourceFormat {
            map: MapStructure::Direct,
            frame: FrameStructure::Hd1080i,
            frate: FrameRate::Hz25,
            sample: SampleStructure::Yuv422At10Bit,
            fmt_reserve: 0,
        };
        let header = PayloadHeader::new(
            VideoSourceId::Protect,
            42,
            TimestampRef::LockedUtc,
            Scrambling::NotScrambled,
            FecUsage::ColumnOnly,
            ClockFrequency::NoTimestamp,
            0,
            Some(vsf),
            None,
            None,
        )
        .unwrap();
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 8);
        let reparsed = PayloadHeader::parse(&bytes).unwrap();
        assert_eq!(reparsed, header);
    }

    #[test]
    fn round_trips_a_12_byte_header_with_timestamp() {
        let vsf = VideoSourceFormat {
            map: MapStructure::LevelBDl,
            frame: FrameStructure::Hd1080p,
            frate: FrameRate::Hz60,
            sample: SampleStructure::Yuv444At12Bit,
            fmt_reserve: 0,
        };
        let header = PayloadHeader::new(
            VideoSourceId::Primary,
            255,
            TimestampRef::LockedPrivate,
            Scrambling::NotScrambled,
            FecUsage::ColumnAndRow,
            ClockFrequency::Mhz148_5,
            0,
            Some(vsf),
            Some(0x1234_5678),
            None,
        )
        .unwrap();
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 12);
        let reparsed = PayloadHeader::parse(&bytes).unwrap();
        assert_eq!(reparsed, header);
        assert_eq!(reparsed.video_timestamp, Some(0x1234_5678));
    }

    #[test]
    fn round_trips_a_header_with_extension() {
        let ext_bytes = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x11, 0x22];
        let header = PayloadHeader::new(
            VideoSourceId::Primary,
            1,
            TimestampRef::NotLocked,
            Scrambling::NotScrambled,
            FecUsage::None,
            ClockFrequency::Mhz27,
            0,
            None,
            Some(0xAAAA_BBBB),
            Some(&ext_bytes),
        )
        .unwrap();
        assert_eq!(header.ext(), 2);
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 4 + 4 + 8);
        let reparsed = PayloadHeader::parse(&bytes).unwrap();
        assert_eq!(reparsed, header);
        assert_eq!(reparsed.extension, Some(&ext_bytes[..]));
    }

    #[test]
    fn round_trips_reserved_bits_verbatim() {
        let header = PayloadHeader::new(
            VideoSourceId::Reserved(0x05),
            7,
            TimestampRef::Reserved(1),
            Scrambling::Reserved(3),
            FecUsage::Reserved(6),
            ClockFrequency::Reserved(0x0A),
            0x1F,
            None,
            Some(0x0BAD_F00D),
            None,
        )
        .unwrap();
        let bytes = header.to_bytes();
        let reparsed = PayloadHeader::parse(&bytes).unwrap();
        assert_eq!(reparsed, header);
        assert_eq!(reparsed.reserve, 0x1F);
    }

    #[test]
    fn all_video_source_format_enum_variants_round_trip() {
        let maps = [
            MapStructure::Direct,
            MapStructure::LevelBDl,
            MapStructure::LevelBDs,
            MapStructure::Reserved(0x0F),
        ];
        let frames = [
            FrameStructure::Unspecified,
            FrameStructure::Sd525i,
            FrameStructure::Sd625i,
            FrameStructure::Hd1080i,
            FrameStructure::Hd1080p,
            FrameStructure::Hd1080psf,
            FrameStructure::Hd2048x1080p,
            FrameStructure::Hd2048x1080psf,
            FrameStructure::Hd720p,
            FrameStructure::Reserved(0xFF),
        ];
        let frates = [
            FrameRate::UnspecifiedAt2970Ghz,
            FrameRate::UnspecifiedAt2970Div1001Ghz,
            FrameRate::UnspecifiedAt1485Ghz,
            FrameRate::UnspecifiedAt1485Div1001Ghz,
            FrameRate::UnspecifiedAt0270Ghz,
            FrameRate::Hz60,
            FrameRate::Hz60Div1001,
            FrameRate::Hz50,
            FrameRate::Hz48,
            FrameRate::Hz48Div1001,
            FrameRate::Hz30,
            FrameRate::Hz30Div1001,
            FrameRate::Hz25,
            FrameRate::Hz24,
            FrameRate::Hz24Div1001,
            FrameRate::Reserved(0x1C),
        ];
        let samples = [
            SampleStructure::Unspecified,
            SampleStructure::Yuv422At10Bit,
            SampleStructure::Yuv444At10Bit,
            SampleStructure::Yuv4444At10Bit,
            SampleStructure::Yuv422At12Bit,
            SampleStructure::Yuv444At12Bit,
            SampleStructure::Yuv4444At12Bit,
            SampleStructure::Yuv4224At12Bit,
            SampleStructure::Reserved(0x0F),
        ];
        for &map in &maps {
            for &frame in &frames {
                for &frate in &frates {
                    for &sample in &samples {
                        let vsf = VideoSourceFormat {
                            map,
                            frame,
                            frate,
                            sample,
                            fmt_reserve: 0xAB,
                        };
                        let word = vsf.to_bits();
                        let back = VideoSourceFormat::from_bits(word);
                        assert_eq!(back, vsf);
                    }
                }
            }
        }
    }

    #[test]
    fn all_top_level_enum_variants_round_trip() {
        let vsids = [
            VideoSourceId::Primary,
            VideoSourceId::Protect,
            VideoSourceId::Reserved(0x07),
        ];
        let refs = [
            TimestampRef::NotLocked,
            TimestampRef::LockedUtc,
            TimestampRef::LockedPrivate,
            TimestampRef::Reserved(1),
        ];
        let scrs = [Scrambling::NotScrambled, Scrambling::Reserved(3)];
        let fecs = [
            FecUsage::None,
            FecUsage::ColumnOnly,
            FecUsage::ColumnAndRow,
            FecUsage::Reserved(7),
        ];
        let cfs = [
            ClockFrequency::NoTimestamp,
            ClockFrequency::Mhz27,
            ClockFrequency::Mhz148_5,
            ClockFrequency::Mhz148_5Div1001,
            ClockFrequency::Mhz297,
            ClockFrequency::Mhz297Div1001,
            ClockFrequency::Reserved(0x0F),
        ];
        for &vsid in &vsids {
            for &r in &refs {
                for &s in &scrs {
                    for &fec in &fecs {
                        for &cf in &cfs {
                            let ts = if cf == ClockFrequency::NoTimestamp {
                                None
                            } else {
                                Some(0x1122_3344)
                            };
                            let header =
                                PayloadHeader::new(vsid, 0, r, s, fec, cf, 0, None, ts, None)
                                    .unwrap();
                            let bytes = header.to_bytes();
                            let reparsed = PayloadHeader::parse(&bytes).unwrap();
                            assert_eq!(reparsed, header);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_reserve_over_5_bits() {
        let err = PayloadHeader::new(
            VideoSourceId::Primary,
            0,
            TimestampRef::NotLocked,
            Scrambling::NotScrambled,
            FecUsage::None,
            ClockFrequency::NoTimestamp,
            0x20,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidValue {
                field: "reserve",
                ..
            }
        ));
    }

    #[test]
    fn rejects_timestamp_without_clock_frequency() {
        let err = PayloadHeader::new(
            VideoSourceId::Primary,
            0,
            TimestampRef::NotLocked,
            Scrambling::NotScrambled,
            FecUsage::None,
            ClockFrequency::NoTimestamp,
            0,
            None,
            Some(0),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::VideoTimestampMismatch {
                clock_frequency: ClockFrequency::NoTimestamp,
                expected_present: false,
                found_present: true,
            }
        ));
    }

    #[test]
    fn rejects_clock_frequency_without_timestamp() {
        let err = PayloadHeader::new(
            VideoSourceId::Primary,
            0,
            TimestampRef::NotLocked,
            Scrambling::NotScrambled,
            FecUsage::None,
            ClockFrequency::Mhz27,
            0,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::VideoTimestampMismatch {
                clock_frequency: ClockFrequency::Mhz27,
                expected_present: true,
                found_present: false,
            }
        ));
    }

    #[test]
    fn rejects_extension_not_a_multiple_of_4() {
        let bytes = [0u8; 5];
        let err = PayloadHeader::new(
            VideoSourceId::Primary,
            0,
            TimestampRef::NotLocked,
            Scrambling::NotScrambled,
            FecUsage::None,
            ClockFrequency::NoTimestamp,
            0,
            None,
            None,
            Some(&bytes),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidValue {
                field: "extension",
                ..
            }
        ));
    }

    #[test]
    fn rejects_extension_over_15_units() {
        let bytes = [0u8; 64]; // 16 units > MAX_EXTENSION_UNITS (15)
        let err = PayloadHeader::new(
            VideoSourceId::Primary,
            0,
            TimestampRef::NotLocked,
            Scrambling::NotScrambled,
            FecUsage::None,
            ClockFrequency::NoTimestamp,
            0,
            None,
            None,
            Some(&bytes),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidValue {
                field: "extension",
                ..
            }
        ));
    }

    #[test]
    fn buffer_too_short_for_fixed_row() {
        let err = PayloadHeader::parse(&[0u8; 3]).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                need: FIXED_HEADER_LEN,
                have: 3,
                ..
            }
        ));
    }

    #[test]
    fn buffer_too_short_for_video_source_format() {
        // F=1 (byte0 bit3 set), but only 4 bytes total.
        let bytes = [F_BIT, 0, 0, 0];
        let err = PayloadHeader::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                what: "video source format row",
                ..
            }
        ));
    }

    #[test]
    fn buffer_too_short_for_video_timestamp() {
        // CF=1 (27 MHz) needs a timestamp row; only 4 bytes present.
        let bytes = [0, 0, 0x00, 0x20]; // CF bits at word bits 8-5 -> 0b0010_0000 = 0x20 low byte
        let err = PayloadHeader::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                what: "video timestamp",
                ..
            }
        ));
    }

    #[test]
    fn buffer_too_short_for_extension() {
        // Ext=1 (byte0 top nibble = 1) but no extension bytes follow.
        let bytes = [0x10, 0, 0, 0];
        let err = PayloadHeader::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                what: "header extension",
                ..
            }
        ));
    }

    #[test]
    fn enum_name_and_display_spot_checks() {
        assert_eq!(VideoSourceId::Primary.name(), "primary stream");
        assert_eq!(VideoSourceId::Reserved(5).to_string(), "reserved(0x05)");
        assert_eq!(ClockFrequency::Mhz27.to_string(), "27 MHz");
        assert_eq!(
            FecUsage::ColumnAndRow.name(),
            "L & D (Column & Row) FEC utilized"
        );
        assert_eq!(Scrambling::NotScrambled.to_string(), "not scrambled");
        assert_eq!(
            TimestampRef::LockedUtc.name(),
            "locked to UTC time/frequency reference"
        );
        assert_eq!(
            MapStructure::LevelBDs.name(),
            "ST 425-1 Level B-DS mapping of two ST 292-1 streams"
        );
        assert_eq!(
            FrameStructure::Hd720p.to_string(),
            "1280x720/750 progressive"
        );
        assert_eq!(FrameRate::Hz24Div1001.name(), "24/1.001 Hz");
        assert_eq!(SampleStructure::Yuv4224At12Bit.name(), "4:2:2:4, 12 bits");
    }
}
