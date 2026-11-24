//! PES packet header parsing (ISO/IEC 13818-1 §2.4.3.6, Table 2-21).

use crate::PACKET_START_CODE_PREFIX;
use crate::error::{Error, Result};
use crate::stream_id::StreamId;
use crate::timestamp::{self, Dts, Pts};
use broadcast_common::clock33::{Clock27, ClockOutOfRange};
use broadcast_common::len::{need_in, need_out};

const MIN_LEN: usize = 6; // start_code(3) + stream_id(1) + PES_packet_length(2)
const HEADER_FIXED: usize = 3; // 2 flag bytes + PES_header_data_length

// ── PES header flag bits (ISO/IEC 13818-1 §2.4.3.7, Table 2-21) ──────────────
// First flag byte (`'10' | PES_scrambling_control(2) | PES_priority |
// data_alignment_indicator | copyright | original_or_copy`).
const F1_PRIORITY: u8 = 0x08;
const F1_DATA_ALIGNMENT: u8 = 0x04;
const F1_COPYRIGHT: u8 = 0x02;
const F1_ORIGINAL_OR_COPY: u8 = 0x01;
// Second flag byte (`PTS_DTS_flags(2) | ESCR | ES_rate | DSM_trick_mode |
// additional_copy_info | PES_CRC | PES_extension`).
const F2_ESCR: u8 = 0x20;
const F2_ES_RATE: u8 = 0x10;
const F2_DSM_TRICK_MODE: u8 = 0x08;
const F2_ADDITIONAL_COPY_INFO: u8 = 0x04;
const F2_PES_CRC: u8 = 0x02;
const F2_PES_EXTENSION: u8 = 0x01;
// PES_extension flag byte (`PES_private_data | pack_header_field |
// program_packet_sequence_counter | P-STD_buffer | reserved(3) |
// PES_extension_flag_2`).
const EXT_PES_PRIVATE_DATA: u8 = 0x80;
const EXT_PACK_HEADER: u8 = 0x40;
const EXT_PROGRAM_PACKET_SEQUENCE_COUNTER: u8 = 0x20;
const EXT_P_STD_BUFFER: u8 = 0x10;
const EXT_FLAG_2: u8 = 0x01;
/// The 3 `reserved` bits between `P-STD_buffer_flag` and `PES_extension_flag_2`.
const EXT_RESERVED_MASK: u8 = 0x0E;
const EXT_RESERVED_SHIFT: u32 = 1;
/// Spec-convention value of those reserved bits (`'111'`).
const EXT_RESERVED_VALUE: u8 = 0x07;

/// Table 2-21: `marker_bit` / `stream_id_extension_flag` is the MSB of the
/// byte; the remaining 7 bits carry the length / extension value.
const MSB_FLAG: u8 = 0x80;
/// The two fixed `'10'` marker bits at the top of the first PES header flag
/// byte (Table 2-21).
const F1_MARKER_MASK: u8 = 0xC0;
const F1_MARKER_VALUE: u8 = 0x80;
/// `ES_rate` is `marker_bit(1) | ES_rate(22) | marker_bit(1)`; both markers
/// are `1` per Table 2-21.
const ES_RATE_MARKERS_VALUE: u8 = 0b11;
const ES_RATE_LEADING_MARKER: u8 = 0x80;
const ES_RATE_TRAILING_MARKER: u8 = 0x01;
/// Mask of those low 7 bits (`PES_extension_field_length`, `stream_id_extension`).
const LOW7_MASK: u8 = 0x7F;

/// Bit widths of the hand-packed flag/sub-field values (Table 2-21, Table 2-24).
const TRICK_FIELD_ID_BITS: u32 = 2;
const TRICK_FREQUENCY_TRUNCATION_BITS: u32 = 2;
const TRICK_MODE_CONTROL_BITS: u32 = 3;
const TRICK_REP_CNTRL_BITS: u32 = 5;
const TRICK_DATA_BITS: u32 = 5;
const LOW7_BITS: u32 = 7;
const ES_RATE_BITS: u32 = 22;
const ESCR_BASE_BITS: u32 = 33;
const ESCR_EXT_BITS: u32 = 9;
const P_STD_BUFFER_SIZE_BITS: u32 = 13;

/// Masks of the packed fields (Table 2-21, Table 2-24).
const SCRAMBLING_CONTROL_MASK: u8 = 0x03;
const PTS_DTS_FLAGS_MASK: u8 = 0x03;
const TRICK_FIELD_ID_MASK: u8 = 0x03;
const TRICK_FREQUENCY_TRUNCATION_MASK: u8 = 0x03;
const TRICK_MODE_CONTROL_MASK: u8 = 0x07;
const TRICK_RESERVED_MASK: u8 = 0x07;
const TRICK_DATA_MASK: u8 = 0x1F;
const PPSC_MPEG1_MPEG2_IDENTIFIER_MASK: u8 = 0x40;
const P_STD_SCALE_MASK: u8 = 0x20;
const P_STD_SIZE_HI_MASK: u8 = 0x1F;
/// Masks of the ESCR groups as they sit in their field bytes.
const ESCR_BASE_HI_MASK: u8 = 0x07;
const ESCR_BASE_HI_MID_MASK: u8 = 0x03;
const ESCR_BASE_MID_LO_MASK: u8 = 0x1F;
const ESCR_BASE_LO_LO_MASK: u8 = 0x1F;
const ESCR_EXT_HI_MASK: u8 = 0x03;
/// Mask of one whole byte of a multi-byte field.
const BYTE_MASK: u64 = 0xFF;

/// Bit shifts of the single bits each field shares a byte with (Table 2-21,
/// Table 2-24).
const TRICK_FIELD_ID_SHIFT: u32 = 3;
const TRICK_INTRA_SLICE_REFRESH_SHIFT: u32 = 2;
const PPSC_MPEG_ID_SHIFT: u32 = 6;
const P_STD_SCALE_SHIFT: u32 = 5;
const PES_EXTENSION_STREAM_ID_FLAG_SHIFT: u32 = 7;
const F1_SCRAMBLING_SHIFT: u32 = 4;
const F1_PRIORITY_SHIFT: u32 = 3;
const F1_ALIGNMENT_SHIFT: u32 = 2;
const F2_ESCR_SHIFT: u32 = 5;
const F2_ES_RATE_SHIFT: u32 = 4;
const F2_TRICK_MODE_SHIFT: u32 = 3;
const F2_ADDITIONAL_COPY_SHIFT: u32 = 2;
/// Bit shift of the `ES_rate` value's top bit within its first byte.
const ES_RATE_TOP_BIT_SHIFT: u32 = 7;
/// Bit shift of one whole byte within a multi-byte value.
const BYTE_SHIFT: u32 = 8;

/// Byte offsets within the fixed PES packet header (Table 2-21).
const STREAM_ID_OFFSET: usize = crate::PACKET_START_CODE_PREFIX.len();
const PES_PACKET_LENGTH_OFFSET: usize = STREAM_ID_OFFSET + 1;
const FLAGS1_OFFSET: usize = MIN_LEN;
const FLAGS2_OFFSET: usize = FLAGS1_OFFSET + 1;
const HEADER_DATA_LENGTH_OFFSET: usize = FLAGS2_OFFSET + 1;
/// `packet_start_code_prefix` as a 24-bit value.
const PACKET_START_CODE_PREFIX_VALUE: u32 = 0x00_0001;

/// Field widths inside the optional PES header (Table 2-21).
const PTS_FIELD_LEN: usize = timestamp::FIELD_LEN;
const PTS_DTS_FIELD_LEN: usize = 2 * PTS_FIELD_LEN;
/// Width in bytes of the `ESCR` field (ISO/IEC 13818-1 §2.4.3.7, Table 2-21).
pub const ESCR_FIELD_LEN: usize = 6;
const ES_RATE_FIELD_LEN: usize = 3;
const PES_PRIVATE_DATA_LEN: usize = 16;
const ES_RATE_HI_OFFSET: usize = 0;
const ES_RATE_MID_OFFSET: usize = ES_RATE_HI_OFFSET + 1;
const ES_RATE_LO_OFFSET: usize = ES_RATE_MID_OFFSET + 1;

/// `PTS_DTS_flags` values and their bit shift (Table 2-21).
const PTS_DTS_NONE: u8 = 0b00;
const PTS_DTS_PTS_ONLY: u8 = 0b10;
const PTS_DTS_PTS_AND_DTS: u8 = 0b11;
const PTS_DTS_FLAGS_SHIFT: u32 = 6;

/// `trick_mode_control` values and the shift of their field (Table 2-24).
const TRICK_MODE_CONTROL_SHIFT: u32 = 5;
const TRICK_SLOW_MOTION: u8 = 0b001;
const TRICK_FREEZE_FRAME: u8 = 0b010;
const TRICK_FAST_REVERSE: u8 = 0b011;
const TRICK_SLOW_REVERSE: u8 = 0b100;

/// Bit shifts of the `ES_rate` groups.
const ES_RATE_HI_SHIFT: u32 = 15;
const ES_RATE_MID_SHIFT: u32 = 7;
const ES_RATE_LO_SHIFT: u32 = 1;
/// The leading `marker_bit` of `ES_rate`, within `es_rate_marker_bits`.
const ES_RATE_LEADING_MARKER_BIT: u8 = 0b10;

/// Bit shifts of the ESCR groups (Table 2-21).
const ESCR_SHIFT_HI: u32 = 30;
const ESCR_SHIFT_HI_MID: u32 = 28;
const ESCR_SHIFT_MID_MID: u32 = 20;
const ESCR_SHIFT_MID_LO: u32 = 15;
const ESCR_SHIFT_LO_HI: u32 = 13;
const ESCR_SHIFT_LO_MID: u32 = 5;
const ESCR_EXT_SHIFT_HI: u32 = 7;
const ESCR_EXT_SHIFT_LO: u32 = 1;
/// Bit shift of the `base[32:30]`/`base[19:15]`/`base[4:0]` groups within their
/// field bytes.
const ESCR_GROUP_SHIFT: u32 = 3;

/// Read the big-endian `u16` at `off`.
fn read_u16_at(b: &[u8], off: usize) -> u16 {
    let mut raw = [0_u8; core::mem::size_of::<u16>()];
    raw.copy_from_slice(&b[off..off + core::mem::size_of::<u16>()]);
    u16::from_be_bytes(raw)
}

/// Read the big-endian `u32` at `off`.
fn read_u32_at(b: &[u8], off: usize) -> u32 {
    let mut raw = [0_u8; core::mem::size_of::<u32>()];
    raw.copy_from_slice(&b[off..off + core::mem::size_of::<u32>()]);
    u32::from_be_bytes(raw)
}

// ── PES_scrambling_control (ISO/IEC 13818-1 §2.4.3.7, Table 2-21) ──────────

/// 2-bit `PES_scrambling_control` field. Table 2-21 defines only `00` (not
/// scrambled); `01`/`10`/`11` are reserved/user-defined and are carried as a
/// [`ScramblingControl::Reserved`] value so a real stream round-trips.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ScramblingControl {
    /// `00` — not scrambled.
    NotScrambled,
    /// `01`/`10`/`11` — reserved/user-defined; carries the 2-bit value.
    Reserved(u8),
}

impl ScramblingControl {
    /// Decode from the 2-bit `PES_scrambling_control` value (masked to `[1:0]`).
    pub fn from_bits(bits: u8) -> Self {
        match bits & SCRAMBLING_CONTROL_MASK {
            0b00 => Self::NotScrambled,
            v => Self::Reserved(v),
        }
    }

    /// Encode as the 2-bit field value (`[1:0]`); the caller shifts it into
    /// place within the header flag byte.
    pub fn to_bits(self) -> u8 {
        match self {
            Self::NotScrambled => 0b00,
            Self::Reserved(v) => v & SCRAMBLING_CONTROL_MASK,
        }
    }

    /// Short label for this value, per the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotScrambled => "not_scrambled",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(ScramblingControl, Reserved);

// ── ES_rate (ISO/IEC 13818-1 §2.4.3.7 Table 2-21) ──────────────────────────

/// `ES_rate` — the 22-bit elementary-stream rate (bytes/second × 50), carried
/// between two `marker_bit`s (Table 2-21).
///
/// A typed, bounded field: an out-of-range rate cannot be constructed, so a
/// serializer never masks it to a different value (#2747).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EsRate(u32);

impl EsRate {
    /// Construct from a rate value, rejecting one wider than the 22-bit wire
    /// field.
    ///
    /// # Errors
    /// [`broadcast_common::error::FieldOverflow`] when `rate >= 2^22`.
    pub fn try_new(
        rate: u32,
    ) -> core::result::Result<Self, broadcast_common::error::FieldOverflow> {
        let fitted = broadcast_common::len::fit_bits(u64::from(rate), ES_RATE_BITS, "ES_rate")?;
        Ok(Self(fitted as u32))
    }

    /// The 22-bit rate (bytes/second × 50), always in `[0, 2^22)`.
    #[must_use]
    pub const fn rate(self) -> u32 {
        self.0
    }
}

// ── ESCR (ISO/IEC 13818-1 §2.4.3.7 Table 2-21) ──────────────────────────────

/// Elementary Stream Clock Reference: 33-bit base (90 kHz) + 9-bit extension
/// (27 MHz) — ISO/IEC 13818-1 §2.4.3.7, Table 2-21.
///
/// Wire layout (6 bytes, 48 bits):
/// `2×reserved(1) | ESCR_base[32:30](3) | marker(1) | ESCR_base[29:15](15) |
///  marker(1) | ESCR_base[14:0](15) | marker(1) | ESCR_ext(9) | marker(1)`.
///
/// Constructing is strict ([`Escr::from_clock27`] takes the validated
/// [`Clock27`]). Parsing is lossless: the 9-bit wire extension can be
/// `300..=511`, which is off the 27 MHz clock, so a parsed value keeps the
/// parts exactly as read and re-serializes byte-identically;
/// [`Escr::to_clock27`] is the checked view that reports such a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Escr {
    /// 33-bit base (90 kHz units).
    pub base: u64,
    /// 9-bit extension (27 MHz units); a parsed value may carry `300..=511`
    /// (see [`Escr::to_clock27`]).
    pub extension: u16,
    /// The leading `reserved(2)` of the field (Table 2-21; spec convention
    /// `0b11`), kept so a real stream's value round-trips. Held here, in the
    /// struct's padding, rather than in `PesHeader` so the header does not grow
    /// (the PS demuxer's allocation budget counts its size).
    pub reserved: u8,
}

/// Spec-convention value of the freeze-frame `reserved(3)` bits (all ones,
/// ISO/IEC 13818-1 "reserved" convention; Table 2-24).
pub const FREEZE_FRAME_RESERVED_VALUE: u8 = 0b111;
/// `marker_bit` of the `additional_copy_info` byte (bit 7, Table 2-21).
const ADDITIONAL_COPY_INFO_MARKER: u8 = 0x80;
/// Spec-convention value of the ESCR leading `reserved(2)` (`0b11`).
const ESCR_RESERVED_VALUE: u8 = 0b11;
/// Bit position of the ESCR leading `reserved(2)` in the field's first byte.
const ESCR_RESERVED_SHIFT: u32 = 6;

/// ESCR 9-bit extension mask (`ESCR_extension`).
const ESCR_EXT_MASK: u16 = (1 << 9) - 1;
/// `marker_bit` leading each byte of `program_packet_sequence_counter`.
const PPSC_MARKER: u8 = 0x80;
/// Mask of the 6-bit `original_stuff_length` (ISO/IEC 13818-1 Table 2-21).
const ORIGINAL_STUFF_LENGTH_MASK: u8 = 0x3F;
/// The `'01'` prefix of `P-STD_buffer` and the mask selecting it.
const P_STD_PREFIX: u8 = 0x40;
const P_STD_PREFIX_MASK: u8 = 0xC0;
/// ESCR `marker_bit` that sits in bit 2 of bytes 0, 2 and 4.
const ESCR_MARKER_MID: u8 = 0x04;
/// ESCR trailing `marker_bit` (bit 0 of byte 5).
const ESCR_MARKER_LAST: u8 = 0x01;

impl Escr {
    /// Construct from a validated 27 MHz clock value (spec-convention reserved
    /// bits).
    #[must_use]
    pub fn from_clock27(clock: Clock27) -> Self {
        Self {
            base: clock.base.ticks(),
            extension: clock.ext,
            reserved: ESCR_RESERVED_VALUE,
        }
    }

    /// Construct from an absolute 27 MHz clock value.
    ///
    /// # Errors
    /// [`ClockOutOfRange`] when `ticks >= 2^33 * 300`.
    pub fn from_27mhz(ticks: u64) -> core::result::Result<Self, ClockOutOfRange> {
        Clock27::from_27mhz(ticks).map(Self::from_clock27)
    }

    /// Full ESCR value on the 27 MHz clock: `base * 300 + extension`.
    ///
    /// For a parsed value whose extension is at or above 300 this is not a time
    /// on the clock; check [`to_clock27`](Self::to_clock27) first.
    #[must_use]
    pub fn as_27mhz(self) -> u64 {
        broadcast_common::clock33::pcr_from_parts(self.base, self.extension)
    }

    /// The checked 27 MHz clock value.
    ///
    /// # Errors
    /// [`ClockOutOfRange`] when the extension is at or above 300 (off the
    /// 27 MHz clock), or an over-wide `base` (only constructible by hand, never
    /// by parsing).
    pub fn to_clock27(self) -> core::result::Result<Clock27, ClockOutOfRange> {
        Clock27::from_wire_parts(self.base, self.extension).to_clock27()
    }

    /// Decode from the 6-byte ESCR field.
    ///
    /// Bit layout (ISO/IEC 13818-1 §2.4.3.7, Table 2-21):
    /// `B0[7:6]`=reserved, `B0[5:3]`=`base[32:30]`, `B0[2]`=marker,
    /// `B0[1:0]`=`base[29:28]`, `B1[7:0]`=`base[27:20]`,
    /// `B2[7:3]`=`base[19:15]`, `B2[2]`=marker, `B2[1:0]`=`base[14:13]`,
    /// `B3[7:0]`=`base[12:5]`,
    /// `B4[7:3]`=`base[4:0]`, `B4[2]`=marker, `B4[1:0]`=`ext[8:7]`,
    /// `B5[7:1]`=`ext[6:0]`, `B5[0]`=marker.
    pub fn from_field_bytes(b: &[u8; ESCR_FIELD_LEN]) -> Result<Self> {
        let [b0, b1, b2, b3, b4, b5] = *b;
        if b0 & ESCR_MARKER_MID == 0
            || b2 & ESCR_MARKER_MID == 0
            || b4 & ESCR_MARKER_MID == 0
            || b5 & ESCR_MARKER_LAST == 0
        {
            return Err(Error::BadMarker("ESCR"));
        }
        let base = ((u64::from(b0 >> ESCR_GROUP_SHIFT) & u64::from(ESCR_BASE_HI_MASK)) << ESCR_SHIFT_HI) // base[32:30]
            | ((u64::from(b0 & ESCR_BASE_HI_MID_MASK)) << ESCR_SHIFT_HI_MID) // base[29:28]
            | (u64::from(b1) << ESCR_SHIFT_MID_MID)                 // base[27:20]
            | ((u64::from(b2 >> ESCR_GROUP_SHIFT) & u64::from(ESCR_BASE_MID_LO_MASK)) << ESCR_SHIFT_MID_LO) // base[19:15]
            | ((u64::from(b2 & ESCR_BASE_HI_MID_MASK)) << ESCR_SHIFT_LO_HI)  // base[14:13]
            | (u64::from(b3) << ESCR_SHIFT_LO_MID)                  // base[12:5]
            | (u64::from(b4 >> ESCR_GROUP_SHIFT) & u64::from(ESCR_BASE_MID_LO_MASK)); // base[4:0]
        let extension = ((u16::from(b4 & ESCR_EXT_HI_MASK) << ESCR_EXT_SHIFT_HI)
            | u16::from(b5 >> ESCR_EXT_SHIFT_LO))
            & ESCR_EXT_MASK;
        Ok(Self {
            base,
            extension,
            reserved: b0 >> ESCR_RESERVED_SHIFT,
        })
    }

    /// Encode as the 6-byte ESCR field.
    ///
    /// Reserved bits are set to `1` per the spec convention.
    /// Exact inverse of [`from_field_bytes`](Self::from_field_bytes).
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if `base` is wider than 33 bits, `extension`
    /// wider than 9 bits, or `reserved` wider than 2 bits — never silently
    /// masks (#1129 class, #2747).
    pub fn to_field_bytes(self) -> Result<[u8; ESCR_FIELD_LEN]> {
        let b = broadcast_common::len::fit_bits(self.base, ESCR_BASE_BITS, "ESCR_base")?;
        let e =
            broadcast_common::len::fit_bits(u64::from(self.extension), ESCR_EXT_BITS, "ESCR_ext")?;
        broadcast_common::len::fit_mask(self.reserved, ESCR_RESERVED_VALUE, "ESCR reserved")?;
        Ok([
            // B0: reserved(2)='11' | base[32:30](3) | marker(1)='1' | base[29:28](2)
            (self.reserved << ESCR_RESERVED_SHIFT)
                | (((b >> ESCR_SHIFT_HI) & u64::from(ESCR_BASE_HI_MASK)) as u8) << ESCR_GROUP_SHIFT
                | ESCR_MARKER_MID
                | ((b >> ESCR_SHIFT_HI_MID) & u64::from(ESCR_BASE_HI_MID_MASK)) as u8,
            // B1: base[27:20](8)
            ((b >> ESCR_SHIFT_MID_MID) & BYTE_MASK) as u8,
            // B2: base[19:15](5) | marker(1)='1' | base[14:13](2)
            (((b >> ESCR_SHIFT_MID_LO) & u64::from(ESCR_BASE_MID_LO_MASK)) as u8)
                << ESCR_GROUP_SHIFT
                | ESCR_MARKER_MID
                | ((b >> ESCR_SHIFT_LO_HI) & u64::from(ESCR_BASE_HI_MID_MASK)) as u8,
            // B3: base[12:5](8)
            ((b >> ESCR_SHIFT_LO_MID) & BYTE_MASK) as u8,
            // B4: base[4:0](5) | marker(1)='1' | ext[8:7](2)
            (((b & u64::from(ESCR_BASE_LO_LO_MASK)) as u8) << ESCR_GROUP_SHIFT)
                | ESCR_MARKER_MID
                | ((e >> ESCR_EXT_SHIFT_HI) & u64::from(ESCR_EXT_HI_MASK)) as u8,
            // B5: ext[6:0](7) | marker(1)='1'
            (((e & u64::from(LOW7_MASK)) as u8) << ESCR_EXT_SHIFT_LO) | ESCR_MARKER_LAST,
        ])
    }
}

// ── DSM trick mode (ISO/IEC 13818-1 §2.4.3.7, Table 2-24) ──────────────────

/// Trick-mode control values for the `DSM_trick_mode_flag` field
/// (ISO/IEC 13818-1 §2.4.3.7, Table 2-24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TrickMode {
    /// `000` — fast forward.
    FastForward {
        /// 2-bit `field_id`.
        field_id: u8,
        /// `intra_slice_refresh` flag.
        intra_slice_refresh: bool,
        /// 2-bit `frequency_truncation`.
        frequency_truncation: u8,
    },
    /// `001` — slow motion.
    SlowMotion {
        /// 5-bit `rep_cntrl`.
        rep_cntrl: u8,
    },
    /// `010` — freeze frame.
    FreezeFrame {
        /// 2-bit `field_id`.
        field_id: u8,
        /// The 3 `reserved` bits after `field_id` (Table 2-24; spec
        /// convention [`FREEZE_FRAME_RESERVED_VALUE`] = `0b111`), kept so a
        /// real stream's value round-trips.
        reserved: u8,
    },
    /// `011` — fast reverse.
    FastReverse {
        /// 2-bit `field_id`.
        field_id: u8,
        /// `intra_slice_refresh` flag.
        intra_slice_refresh: bool,
        /// 2-bit `frequency_truncation`.
        frequency_truncation: u8,
    },
    /// `100` — slow reverse.
    SlowReverse {
        /// 5-bit `rep_cntrl`.
        rep_cntrl: u8,
    },
    /// `101`–`111` — reserved.
    Reserved {
        /// Raw 3-bit `trick_mode_control` value.
        trick_mode_control: u8,
        /// Raw 5-bit remainder.
        data: u8,
    },
}

impl TrickMode {
    /// Decode from the 1-byte trick-mode field (ISO/IEC 13818-1 §2.4.3.7).
    pub fn from_byte(b: u8) -> Self {
        let control = (b >> TRICK_MODE_CONTROL_SHIFT) & TRICK_MODE_CONTROL_MASK;
        let data = b & TRICK_DATA_MASK;
        match control {
            0b000 => TrickMode::FastForward {
                field_id: (data >> TRICK_FIELD_ID_SHIFT) & TRICK_FIELD_ID_MASK,
                intra_slice_refresh: (data >> TRICK_INTRA_SLICE_REFRESH_SHIFT) & 1 != 0,
                frequency_truncation: data & TRICK_FREQUENCY_TRUNCATION_MASK,
            },
            0b001 => TrickMode::SlowMotion { rep_cntrl: data },
            0b010 => TrickMode::FreezeFrame {
                field_id: (data >> TRICK_FIELD_ID_SHIFT) & TRICK_FIELD_ID_MASK,
                reserved: data & TRICK_RESERVED_MASK,
            },
            0b011 => TrickMode::FastReverse {
                field_id: (data >> TRICK_FIELD_ID_SHIFT) & TRICK_FIELD_ID_MASK,
                intra_slice_refresh: (data >> TRICK_INTRA_SLICE_REFRESH_SHIFT) & 1 != 0,
                frequency_truncation: data & TRICK_FREQUENCY_TRUNCATION_MASK,
            },
            0b100 => TrickMode::SlowReverse { rep_cntrl: data },
            _ => TrickMode::Reserved {
                trick_mode_control: control,
                data,
            },
        }
    }

    /// Encode as the 1-byte trick-mode field.
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if a sub-field exceeds its wire width
    /// (`field_id` 2 bits, `frequency_truncation` 2 bits, `rep_cntrl` 5 bits,
    /// `reserved` 3 bits) — never silently masks (#2747).
    pub fn to_byte(self) -> Result<u8> {
        use broadcast_common::len::fit_bits;
        use broadcast_common::len::fit_mask;
        match self {
            TrickMode::FastForward {
                field_id,
                intra_slice_refresh,
                frequency_truncation,
            } => {
                let field_id = fit_bits(
                    u64::from(field_id),
                    TRICK_FIELD_ID_BITS,
                    "trick_mode.field_id",
                )?;
                let frequency_truncation = fit_bits(
                    u64::from(frequency_truncation),
                    TRICK_FREQUENCY_TRUNCATION_BITS,
                    "trick_mode.frequency_truncation",
                )?;
                Ok(((field_id as u8) << TRICK_FIELD_ID_SHIFT)
                    | ((intra_slice_refresh as u8) << TRICK_INTRA_SLICE_REFRESH_SHIFT)
                    | (frequency_truncation as u8))
            }
            TrickMode::SlowMotion { rep_cntrl } => {
                let rep_cntrl = fit_bits(
                    u64::from(rep_cntrl),
                    TRICK_REP_CNTRL_BITS,
                    "trick_mode.rep_cntrl",
                )?;
                Ok((TRICK_SLOW_MOTION << TRICK_MODE_CONTROL_SHIFT) | (rep_cntrl as u8))
            }
            TrickMode::FreezeFrame { field_id, reserved } => {
                let field_id = fit_bits(
                    u64::from(field_id),
                    TRICK_FIELD_ID_BITS,
                    "trick_mode.field_id",
                )?;
                let reserved =
                    fit_mask(reserved, FREEZE_FRAME_RESERVED_VALUE, "trick_mode.reserved")?;
                Ok((TRICK_FREEZE_FRAME << TRICK_MODE_CONTROL_SHIFT)
                    | ((field_id as u8) << TRICK_FIELD_ID_SHIFT)
                    | reserved)
            }
            TrickMode::FastReverse {
                field_id,
                intra_slice_refresh,
                frequency_truncation,
            } => {
                let field_id = fit_bits(
                    u64::from(field_id),
                    TRICK_FIELD_ID_BITS,
                    "trick_mode.field_id",
                )?;
                let frequency_truncation = fit_bits(
                    u64::from(frequency_truncation),
                    TRICK_FREQUENCY_TRUNCATION_BITS,
                    "trick_mode.frequency_truncation",
                )?;
                Ok((TRICK_FAST_REVERSE << TRICK_MODE_CONTROL_SHIFT)
                    | ((field_id as u8) << TRICK_FIELD_ID_SHIFT)
                    | ((intra_slice_refresh as u8) << TRICK_INTRA_SLICE_REFRESH_SHIFT)
                    | (frequency_truncation as u8))
            }
            TrickMode::SlowReverse { rep_cntrl } => {
                let rep_cntrl = fit_bits(
                    u64::from(rep_cntrl),
                    TRICK_REP_CNTRL_BITS,
                    "trick_mode.rep_cntrl",
                )?;
                Ok((TRICK_SLOW_REVERSE << TRICK_MODE_CONTROL_SHIFT) | (rep_cntrl as u8))
            }
            TrickMode::Reserved {
                trick_mode_control,
                data,
            } => {
                let trick_mode_control = fit_bits(
                    u64::from(trick_mode_control),
                    TRICK_MODE_CONTROL_BITS,
                    "trick_mode.trick_mode_control",
                )?;
                let data = fit_bits(u64::from(data), TRICK_DATA_BITS, "trick_mode.data")?;
                Ok(((trick_mode_control as u8) << TRICK_MODE_CONTROL_SHIFT) | (data as u8))
            }
        }
    }
}

// ── PES extension (ISO/IEC 13818-1 §2.4.3.7) ──────────────────────────────

/// `program_packet_sequence_counter` sub-field of [`PesExtension`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProgramPacketSequenceCounter {
    /// 7-bit counter.
    pub counter: u8,
    /// `MPEG1_MPEG2_identifier` flag.
    pub mpeg1_mpeg2_identifier: bool,
    /// 6-bit `original_stuff_length`.
    pub original_stuff_length: u8,
}

/// `P-STD_buffer` sub-field of [`PesExtension`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PStdBuffer {
    /// P-STD buffer size scale: `false` = 128 bytes/unit, `true` = 1024 bytes/unit.
    pub scale: bool,
    /// 13-bit buffer size in units of the scale.
    pub size: u16,
}

/// `PES_extension_field()` payload, present when `PES_extension_flag_2` is set
/// (ISO/IEC 13818-1 §2.4.3.7, Table 2-21).
///
/// The field itself is `marker_bit(1) + PES_extension_field_length(7)`
/// (handled by [`PesExtension`]'s parse/serialize) followed by these bytes.
/// The first byte is `stream_id_extension_flag`(1) + 7 more bits that are
/// `stream_id_extension` when the flag is `0`, or `reserved`(6)+
/// `tref_extension_flag`(1) when the flag is `1`; the TREF sub-fields or
/// reserved padding that can follow are not decoded further and stay opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PesExtensionField<'a> {
    /// `stream_id_extension_flag`.
    pub stream_id_extension_flag: bool,
    /// The low 7 bits of the `stream_id_extension` byte (see struct docs for
    /// how to interpret them against `stream_id_extension_flag`).
    pub low_bits: u8,
    /// Bytes following the `stream_id_extension` byte (opaque per spec).
    pub rest: &'a [u8],
}

impl PesExtensionField<'_> {
    /// The 7-bit `stream_id_extension` (Table 2-21), or `None` when
    /// `stream_id_extension_flag` is `1` (the low bits are then `reserved` +
    /// `tref_extension_flag`, not an extension id).
    #[must_use]
    pub const fn stream_id_extension(&self) -> Option<u8> {
        if self.stream_id_extension_flag {
            None
        } else {
            Some(self.low_bits)
        }
    }
}

/// The `additional_copy_info` byte: `marker_bit(1) | additional_copy_info(7)`
/// (ISO/IEC 13818-1 Table 2-21). The marker is spec-fixed to `1` but kept so a
/// real stream's value round-trips; it lives with the value (not as a
/// `PesHeader` field) so the header does not grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AdditionalCopyInfo {
    /// 7-bit `additional_copy_info`.
    pub info: u8,
    /// The `marker_bit` in front of it (spec `1`).
    pub marker: bool,
}

impl AdditionalCopyInfo {
    /// A value with the spec-conformant marker bit.
    #[must_use]
    pub const fn new(info: u8) -> Self {
        Self { info, marker: true }
    }
}

/// Typed PES header extension sub-structure
/// (ISO/IEC 13818-1 §2.4.3.7, Table 2-21, `PES_extension_flag = 1`).
///
/// `#[non_exhaustive]`: build with [`PesExtension::new`] and set fields, so
/// further additions are not a breaking change again.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PesExtension<'a> {
    /// 128-bit PES private data, if `PES_private_data_flag` is set.
    pub pes_private_data: Option<[u8; 16]>,
    /// Pack header field bytes (opaque per spec — `&[u8]` is correct here).
    pub pack_header: Option<&'a [u8]>,
    /// Program packet sequence counter sub-fields.
    pub program_packet_sequence_counter: Option<ProgramPacketSequenceCounter>,
    /// P-STD buffer sub-field.
    pub p_std_buffer: Option<PStdBuffer>,
    /// `PES_extension_field()` payload, if `PES_extension_flag_2` is set.
    pub pes_extension_field: Option<PesExtensionField<'a>>,
    /// The 3 `reserved` bits of the extension flags byte (Table 2-21; spec
    /// convention `0b111`), kept so a real stream's value round-trips.
    pub reserved_bits: u8,
}

impl Default for PesExtension<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> PesExtension<'a> {
    /// An extension with no optional sub-fields and the spec-convention
    /// reserved bits (`0b111`).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pes_private_data: None,
            pack_header: None,
            program_packet_sequence_counter: None,
            p_std_buffer: None,
            pes_extension_field: None,
            reserved_bits: EXT_RESERVED_VALUE,
        }
    }

    fn parse(data: &'a [u8]) -> Result<Self> {
        need_in(data, 1, "PES_extension flags byte")?;
        let flags = data[0];
        let mut cursor = 1usize;

        let pes_private_data = if flags & EXT_PES_PRIVATE_DATA != 0 {
            let end = cursor + PES_PRIVATE_DATA_LEN;
            let arr: [u8; PES_PRIVATE_DATA_LEN] = data
                .get(cursor..end)
                .and_then(|s| s.try_into().ok())
                .ok_or(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: end,
                        have: data.len(),
                        what: "PES_private_data",
                    },
                ))?;
            cursor = end;
            Some(arr)
        } else {
            None
        };

        let pack_header = if flags & EXT_PACK_HEADER != 0 {
            let pack_len = *data.get(cursor).ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: cursor + 1,
                    have: data.len(),
                    what: "pack_field_length",
                },
            ))? as usize;
            cursor += 1;
            let end = cursor + pack_len;
            let slice = data.get(cursor..end).ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: end,
                    have: data.len(),
                    what: "pack_header",
                },
            ))?;
            cursor = end;
            Some(slice)
        } else {
            None
        };

        let program_packet_sequence_counter = if flags & EXT_PROGRAM_PACKET_SEQUENCE_COUNTER != 0 {
            need_in(
                data,
                cursor + core::mem::size_of::<u16>(),
                "program_packet_sequence_counter",
            )?;
            let b0 = data[cursor];
            let b1 = data[cursor + 1];
            cursor += core::mem::size_of::<u16>();
            if b0 & PPSC_MARKER == 0 || b1 & PPSC_MARKER == 0 {
                return Err(Error::BadMarker("program_packet_sequence_counter"));
            }
            Some(ProgramPacketSequenceCounter {
                counter: b0 & LOW7_MASK,
                mpeg1_mpeg2_identifier: (b1 & PPSC_MPEG1_MPEG2_IDENTIFIER_MASK) != 0,
                original_stuff_length: b1 & ORIGINAL_STUFF_LENGTH_MASK,
            })
        } else {
            None
        };

        let p_std_buffer = if flags & EXT_P_STD_BUFFER != 0 {
            need_in(data, cursor + core::mem::size_of::<u16>(), "P-STD_buffer")?;
            let b0 = data[cursor];
            let b1 = data[cursor + 1];
            cursor += core::mem::size_of::<u16>();
            if b0 & P_STD_PREFIX_MASK != P_STD_PREFIX {
                return Err(Error::BadMarker("P-STD_buffer"));
            }
            Some(PStdBuffer {
                scale: (b0 & P_STD_SCALE_MASK) != 0,
                size: (u16::from(b0 & P_STD_SIZE_HI_MASK) << BYTE_SHIFT) | u16::from(b1),
            })
        } else {
            None
        };

        let pes_extension_field = if flags & EXT_FLAG_2 != 0 {
            let len_byte = *data.get(cursor).ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: cursor + 1,
                    have: data.len(),
                    what: "PES_extension_field_length",
                },
            ))?;
            // Table 2-21: marker_bit(1) + PES_extension_field_length(7), not a
            // plain 8-bit length (issue #1052) — the high bit is a marker,
            // not part of the count.
            let ext_len = (len_byte & LOW7_MASK) as usize;
            cursor += 1;
            let end = cursor + ext_len;
            let field_bytes = data.get(cursor..end).ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: end,
                    have: data.len(),
                    what: "PES_extension_field",
                },
            ))?;
            cursor = end;
            // First byte: stream_id_extension_flag(1) + 7 more bits (Table 2-21).
            let (sie_byte, rest) = field_bytes.split_first().ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "PES_extension_field.stream_id_extension byte",
                },
            ))?;
            Some(PesExtensionField {
                stream_id_extension_flag: (sie_byte & MSB_FLAG) != 0,
                low_bits: sie_byte & LOW7_MASK,
                rest,
            })
        } else {
            None
        };
        let _ = cursor;

        Ok(PesExtension {
            pes_private_data,
            pack_header,
            program_packet_sequence_counter,
            p_std_buffer,
            pes_extension_field,
            reserved_bits: (flags & EXT_RESERVED_MASK) >> EXT_RESERVED_SHIFT,
        })
    }

    /// Number of bytes this PES extension occupies on the wire.
    pub fn serialized_len(&self) -> usize {
        let mut n = 1usize; // flags byte
        if self.pes_private_data.is_some() {
            n += PES_PRIVATE_DATA_LEN;
        }
        if let Some(ph) = self.pack_header {
            n += 1 + ph.len();
        }
        if self.program_packet_sequence_counter.is_some() {
            n += core::mem::size_of::<u16>();
        }
        if self.p_std_buffer.is_some() {
            n += core::mem::size_of::<u16>();
        }
        if let Some(ef) = &self.pes_extension_field {
            // length byte + stream_id_extension byte + trailing bytes.
            n += 1 + 1 + ef.rest.len();
        }
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;

        let mut flags = broadcast_common::len::fit_mask(
            self.reserved_bits,
            EXT_RESERVED_VALUE,
            "PES_extension.reserved",
        )? << EXT_RESERVED_SHIFT;
        if self.pes_private_data.is_some() {
            flags |= EXT_PES_PRIVATE_DATA;
        }
        if self.pack_header.is_some() {
            flags |= EXT_PACK_HEADER;
        }
        if self.program_packet_sequence_counter.is_some() {
            flags |= EXT_PROGRAM_PACKET_SEQUENCE_COUNTER;
        }
        if self.p_std_buffer.is_some() {
            flags |= EXT_P_STD_BUFFER;
        }
        if self.pes_extension_field.is_some() {
            flags |= EXT_FLAG_2;
        }
        buf[0] = flags;
        let mut cursor = 1usize;

        if let Some(pd) = &self.pes_private_data {
            buf[cursor..cursor + PES_PRIVATE_DATA_LEN].copy_from_slice(pd);
            cursor += PES_PRIVATE_DATA_LEN;
        }
        if let Some(ph) = self.pack_header {
            buf[cursor] =
                broadcast_common::len::fit_u8(ph.len(), "PES_extension.pack_field_length")?;
            cursor += 1;
            buf[cursor..cursor + ph.len()].copy_from_slice(ph);
            cursor += ph.len();
        }
        if let Some(ppsc) = self.program_packet_sequence_counter {
            // byte 0: marker(1) | counter(7)
            let counter = broadcast_common::len::fit_mask(
                ppsc.counter,
                LOW7_MASK,
                "program_packet_sequence_counter.counter",
            )?;
            let original_stuff_length = broadcast_common::len::fit_mask(
                ppsc.original_stuff_length,
                ORIGINAL_STUFF_LENGTH_MASK,
                "program_packet_sequence_counter.original_stuff_length",
            )?;
            buf[cursor] = PPSC_MARKER | counter;
            // byte 1: marker(1) | mpeg1_mpeg2_id(1) | original_stuff_length(6)
            buf[cursor + 1] = PPSC_MARKER
                | ((ppsc.mpeg1_mpeg2_identifier as u8) << PPSC_MPEG_ID_SHIFT)
                | original_stuff_length;
            cursor += core::mem::size_of::<u16>();
        }
        if let Some(ps) = self.p_std_buffer {
            // '01' | scale(1) | size(13)
            // size is 13 bits: reject, never mask, an over-range value (#1129).
            let size = broadcast_common::len::fit_bits(
                u64::from(ps.size),
                P_STD_BUFFER_SIZE_BITS,
                "PES_extension.P-STD_buffer_size",
            )? as u16;
            buf[cursor] = P_STD_PREFIX
                | ((ps.scale as u8) << P_STD_SCALE_SHIFT)
                | ((size >> BYTE_SHIFT) as u8 & P_STD_SIZE_HI_MASK);
            buf[cursor + 1] = size.to_be_bytes()[1];
            cursor += core::mem::size_of::<u16>();
        }
        if let Some(ef) = &self.pes_extension_field {
            let field_len = 1 + ef.rest.len(); // stream_id_extension byte + rest
            // Table 2-21: the length is 7 bits, not 8 (issue #1052) — the high
            // bit written below is `marker_bit`, always 1.
            let checked = broadcast_common::len::fit_bits(
                field_len as u64,
                LOW7_BITS,
                "PES_extension.PES_extension_field_length",
            )?;
            buf[cursor] = MSB_FLAG | (checked as u8);
            cursor += 1;
            buf[cursor] = ((ef.stream_id_extension_flag as u8)
                << PES_EXTENSION_STREAM_ID_FLAG_SHIFT)
                | (ef.low_bits & LOW7_MASK);
            cursor += 1;
            buf[cursor..cursor + ef.rest.len()].copy_from_slice(ef.rest);
            cursor += ef.rest.len();
        }
        Ok(cursor)
    }
}

// ── PesHeader ──────────────────────────────────────────────────────────────

/// The optional PES header present for non-special `stream_id`s
/// (ISO/IEC 13818-1 §2.4.3.6, §2.4.3.7). All optional sub-fields are fully
/// typed — the raw `optional_fields` blob has been replaced with the
/// individual decoded fields.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PesHeader<'a> {
    /// PES_scrambling_control (2 bits).
    pub scrambling_control: ScramblingControl,
    /// PES_priority.
    pub pes_priority: bool,
    /// data_alignment_indicator.
    pub data_alignment_indicator: bool,
    /// copyright.
    pub copyright: bool,
    /// original_or_copy.
    pub original_or_copy: bool,
    /// Presentation time stamp, if `PTS_DTS_flags` indicated one.
    pub pts: Option<Pts>,
    /// Decoding time stamp, if `PTS_DTS_flags` was `11`.
    pub dts: Option<Dts>,
    /// Elementary stream clock reference (6 bytes), if `ESCR_flag` is set.
    pub escr: Option<Escr>,
    /// 22-bit ES rate (bytes/second × 50), if `ES_rate_flag` is set.
    pub es_rate: Option<EsRate>,
    /// The two `marker_bit`s around `ES_rate` (Table 2-21), packed as
    /// `0b<leading><trailing>`; the spec fixes both to `1` (`0b11`) but a real
    /// stream (TSDuck `test-002.ts`) carries a `0` leading marker, so they are
    /// kept for a byte-exact round trip. Only meaningful when `es_rate` is
    /// `Some`.
    pub es_rate_marker_bits: u8,
    /// DSM trick-mode control byte (typed), if `DSM_trick_mode_flag` is set.
    pub dsm_trick_mode: Option<TrickMode>,
    /// `marker_bit` + 7-bit `additional_copy_info`, if
    /// `additional_copy_info_flag` is set.
    pub additional_copy_info: Option<AdditionalCopyInfo>,
    /// Previous PES packet CRC (16 bits), if `PES_CRC_flag` is set.
    pub pes_crc: Option<u16>,
    /// PES extension sub-structure, if `PES_extension_flag` is set.
    pub pes_extension: Option<PesExtension<'a>>,
    /// The `stuffing_byte`s inside the `PES_header_data_length` region, after
    /// the typed optional fields (ISO/IEC 13818-1 Table 2-21 —
    /// "stuffing_byte: fixed 8-bit value `0xFF`").
    ///
    /// Encoders pad the optional-header block (often so the elementary stream
    /// starts at a fixed offset). These bytes are part of the wire image and
    /// are kept verbatim (not just counted), so a non-`0xFF` filler from a
    /// non-conforming encoder round-trips byte-for-byte too. Use `&[]` for a
    /// header with no stuffing.
    pub header_stuffing: &'a [u8],
}

impl<'a> PesHeader<'a> {
    /// A header with `PES_scrambling_control = 0`, every flag clear, no
    /// optional fields, no stuffing and spec-conformant marker bits. Set
    /// fields on the result (the struct is `#[non_exhaustive]`).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scrambling_control: ScramblingControl::NotScrambled,
            pes_priority: false,
            data_alignment_indicator: false,
            copyright: false,
            original_or_copy: false,
            pts: None,
            dts: None,
            escr: None,
            es_rate: None,
            es_rate_marker_bits: ES_RATE_MARKERS_VALUE,
            dsm_trick_mode: None,
            additional_copy_info: None,
            pes_crc: None,
            pes_extension: None,
            header_stuffing: &[],
        }
    }
}

impl Default for PesHeader<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl PesHeader<'_> {
    /// The two header flag bytes (ISO/IEC 13818-1 Table 2-21, §2.4.3.7):
    /// `'10' | PES_scrambling_control | PES_priority | data_alignment_indicator
    /// | copyright | original_or_copy`, then `PTS_DTS_flags` followed by one
    /// flag bit per optional field this header carries.
    #[must_use]
    pub fn flag_bytes(&self) -> [u8; 2] {
        // f1: marker '10' | scrambling(2) | priority(1) | align(1) | copyright(1) | orig(1)
        let f1 = F1_MARKER_VALUE
            | (self.scrambling_control.to_bits() << F1_SCRAMBLING_SHIFT)
            | (u8::from(self.pes_priority) << F1_PRIORITY_SHIFT)
            | (u8::from(self.data_alignment_indicator) << F1_ALIGNMENT_SHIFT)
            | (u8::from(self.copyright) << 1)
            | u8::from(self.original_or_copy);
        // f2: pts_dts_flags(2) | escr_flag(1) | es_rate_flag(1) |
        //     trick_mode(1) | add_copy(1) | crc(1) | ext(1)
        let pts_dts_flags = match (self.pts.is_some(), self.dts.is_some()) {
            (true, true) => PTS_DTS_PTS_AND_DTS,
            (true, false) => PTS_DTS_PTS_ONLY,
            _ => PTS_DTS_NONE,
        };
        let f2 = (pts_dts_flags << PTS_DTS_FLAGS_SHIFT)
            | (u8::from(self.escr.is_some()) << F2_ESCR_SHIFT)
            | (u8::from(self.es_rate.is_some()) << F2_ES_RATE_SHIFT)
            | (u8::from(self.dsm_trick_mode.is_some()) << F2_TRICK_MODE_SHIFT)
            | (u8::from(self.additional_copy_info.is_some()) << F2_ADDITIONAL_COPY_SHIFT)
            | (u8::from(self.pes_crc.is_some()) << 1)
            | u8::from(self.pes_extension.is_some());
        [f1, f2]
    }

    /// Write every optional header field (`PTS`/`DTS`, `ESCR`, `ES_rate`,
    /// `DSM_trick_mode`, `additional_copy_info`, `PES_CRC`,
    /// `PES_extension`) followed by the header stuffing — i.e. everything
    /// after `PES_header_data_length`, whose value is
    /// [`Self::optional_len`].
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if a sub-field exceeds its wire width,
    /// [`Error::OutputBufferTooSmall`] if `buf` is shorter than the fields.
    pub fn serialize_optional_fields(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, self.optional_len())?;
        let mut cursor = 0usize;

        // PTS (and/or DTS).
        if let Some(pts) = self.pts {
            let prefix = if self.dts.is_some() {
                timestamp::PTS_WITH_DTS_PREFIX
            } else {
                timestamp::PTS_PREFIX
            };
            buf[cursor..cursor + PTS_FIELD_LEN]
                .copy_from_slice(&timestamp::write(pts.ticks(), prefix));
            cursor += PTS_FIELD_LEN;
        }
        if let Some(dts) = self.dts {
            buf[cursor..cursor + PTS_FIELD_LEN]
                .copy_from_slice(&timestamp::write(dts.ticks(), timestamp::DTS_PREFIX));
            cursor += PTS_FIELD_LEN;
        }
        // ESCR.
        if let Some(escr) = self.escr {
            buf[cursor..cursor + ESCR_FIELD_LEN].copy_from_slice(&escr.to_field_bytes()?);
            cursor += ESCR_FIELD_LEN;
        }
        // ES_rate: marker(1) | rate(22) | marker(1) = 3 bytes.
        if let Some(rate) = self.es_rate {
            // The two marker bits are 2 low bits of `es_rate_marker_bits`; reject
            // a value wider than that rather than silently dropping its high bits.
            let marker_bits = broadcast_common::len::fit_mask(
                self.es_rate_marker_bits,
                ES_RATE_MARKERS_VALUE,
                "ES_rate marker bits",
            )?;
            let rate = rate.rate();
            let leading = if marker_bits & ES_RATE_LEADING_MARKER_BIT != 0 {
                ES_RATE_LEADING_MARKER
            } else {
                0
            };
            buf[cursor] = leading | ((rate >> ES_RATE_HI_SHIFT) as u8 & LOW7_MASK);
            buf[cursor + 1] = ((rate >> ES_RATE_MID_SHIFT) & u32::from(u8::MAX)) as u8;
            buf[cursor + ES_RATE_LO_OFFSET] = (((rate & u32::from(LOW7_MASK)) as u8)
                << ES_RATE_LO_SHIFT)
                | (marker_bits & ES_RATE_TRAILING_MARKER);
            cursor += ES_RATE_FIELD_LEN;
        }
        // DSM trick mode.
        if let Some(tm) = self.dsm_trick_mode {
            buf[cursor] = tm.to_byte()?;
            cursor += 1;
        }
        // additional_copy_info: marker(1) | info(7).
        if let Some(aci) = self.additional_copy_info {
            let info =
                broadcast_common::len::fit_mask(aci.info, LOW7_MASK, "additional_copy_info")?;
            let marker = if aci.marker {
                ADDITIONAL_COPY_INFO_MARKER
            } else {
                0
            };
            buf[cursor] = marker | info;
            cursor += 1;
        }
        // PES_CRC (2 bytes, big-endian).
        if let Some(crc) = self.pes_crc {
            buf[cursor..cursor + core::mem::size_of::<u16>()].copy_from_slice(&crc.to_be_bytes());
            cursor += core::mem::size_of::<u16>();
        }
        // PES_extension.
        if let Some(ref ext) = self.pes_extension {
            let written = ext.serialize_into(&mut buf[cursor..])?;
            cursor += written;
        }

        // Trailing stuffing inside the header_data_length region
        // (ISO/IEC 13818-1 Table 2-21), reproducing the encoder's pad.
        buf[cursor..cursor + self.header_stuffing.len()].copy_from_slice(self.header_stuffing);
        cursor += self.header_stuffing.len();
        Ok(cursor)
    }

    /// Number of bytes this header occupies in the serialized `PES_header_data_length`
    /// region (not counting the 3 fixed header bytes — the 2 flag bytes + length byte).
    #[must_use]
    pub fn optional_len(&self) -> usize {
        let mut n = 0usize;
        if self.pts.is_some() {
            n += PTS_FIELD_LEN;
        }
        if self.dts.is_some() {
            n += PTS_FIELD_LEN;
        }
        if self.escr.is_some() {
            n += ESCR_FIELD_LEN;
        }
        if self.es_rate.is_some() {
            n += ES_RATE_FIELD_LEN;
        }
        if self.dsm_trick_mode.is_some() {
            n += 1;
        }
        if self.additional_copy_info.is_some() {
            n += 1;
        }
        if self.pes_crc.is_some() {
            n += core::mem::size_of::<u16>();
        }
        if let Some(ref ext) = self.pes_extension {
            n += ext.serialized_len();
        }
        n += self.header_stuffing.len();
        n
    }
}

/// A parsed PES packet.
///
/// `#[non_exhaustive]`: build with [`PesPacket::new`] and set fields, so
/// further additions are not a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PesPacket<'a> {
    /// stream_id (Table 2-22).
    pub stream_id: StreamId,
    /// Optional PES header (absent for the special `stream_id`s).
    pub header: Option<PesHeader<'a>>,
    /// The elementary-stream bytes (`PES_packet_data_byte`s).
    #[cfg_attr(feature = "serde", serde(skip))]
    pub payload: &'a [u8],
}

/// A PES unit that ended before its declared `PES_packet_length`
/// (ISO/IEC 13818-1 Table 2-21), as reported by
/// [`PesPacket::parse_unit`](PesPacket::parse_unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Truncation {
    /// The `PES_packet_length` the unit declared (bytes after that field).
    pub declared: usize,
    /// The bytes that actually followed the length field, fewer than
    /// `declared`.
    pub available: usize,
}

impl<'a> PesPacket<'a> {
    /// A packet with no optional header and an empty payload.
    ///
    /// `PES_packet_length` is not a field: [`serialize_into`](Self::serialize_into)
    /// derives it from the header and payload.
    #[must_use]
    pub const fn new(stream_id: StreamId, payload: &'a [u8]) -> Self {
        Self {
            stream_id,
            header: None,
            payload,
        }
    }

    /// Parse a PES packet from the bytes starting at its `packet_start_code_prefix`.
    ///
    /// `PES_packet_length` bounds everything after it (ISO/IEC 13818-1
    /// Table 2-21): a buffer shorter than the declared length is a truncated
    /// packet, reported as `BufferTooShort` instead of a silent short read.
    /// Use [`parse_unit`](Self::parse_unit) for a unit that may arrive before
    /// its next start indicator, where the trailing bytes are simply absent.
    pub fn parse(b: &'a [u8]) -> Result<Self> {
        Self::parse_select(b, false).map(|(packet, _)| packet)
    }

    /// Parse a PES unit from the bytes starting at its
    /// `packet_start_code_prefix`, accepting one that ends before its declared
    /// `PES_packet_length`.
    ///
    /// A transport stream hands a PES packet its bytes up to the next
    /// `payload_unit_start_indicator`; a broadcast encoder may declare a
    /// `PES_packet_length` longer than the bytes that arrived. [`parse`](Self::parse)
    /// reports that as `BufferTooShort`; this entry returns the packet's bytes
    /// as received together with a [`Truncation`] naming what was declared and
    /// what arrived (or `None` when the unit is complete). The payload is
    /// exactly the bytes present, never padded.
    pub fn parse_unit(b: &'a [u8]) -> Result<(Self, Option<Truncation>)> {
        Self::parse_select(b, true)
    }

    /// Shared parse body. `lenient` accepts a buffer shorter than the declared
    /// `PES_packet_length` and reports the shortfall as [`Truncation`]; the
    /// strict path returns [`Error::BufferTooShort`] instead.
    fn parse_select(b: &'a [u8], lenient: bool) -> Result<(Self, Option<Truncation>)> {
        need_in(b, MIN_LEN, "PES packet header")?;
        if b[..crate::PACKET_START_CODE_PREFIX.len()] != PACKET_START_CODE_PREFIX {
            return Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                what: "packet_start_code_prefix",
                expected: PACKET_START_CODE_PREFIX_VALUE,
                found: read_u32_at(b, 0) >> BYTE_SHIFT,
            }));
        }
        let stream_id = StreamId::new(b[STREAM_ID_OFFSET]);
        let pes_packet_length = read_u16_at(b, PES_PACKET_LENGTH_OFFSET);
        // Where the payload ends: bounded by PES_packet_length unless 0
        // (unbounded: the packet runs to the end of the buffer).
        let payload_end = if pes_packet_length == 0 {
            b.len()
        } else {
            let end = MIN_LEN + usize::from(pes_packet_length);
            if end > b.len() {
                if lenient {
                    b.len()
                } else {
                    need_in(b, end, "PES_packet_length")?;
                    end
                }
            } else {
                end
            }
        };
        // Only a bounded packet that ran out of bytes is a truncation; the
        // unbounded `0` form is a whole packet by definition.
        let truncation = (pes_packet_length != 0
            && payload_end < MIN_LEN + usize::from(pes_packet_length))
        .then(|| Truncation {
            declared: usize::from(pes_packet_length),
            available: payload_end.saturating_sub(MIN_LEN),
        });

        if !stream_id.has_optional_header() {
            return Ok((
                PesPacket {
                    stream_id,
                    header: None,
                    payload: &b[MIN_LEN..payload_end],
                },
                truncation,
            ));
        }

        need_in(b, MIN_LEN + HEADER_FIXED, "PES optional header")?;
        let f1 = b[FLAGS1_OFFSET];
        let f2 = b[FLAGS2_OFFSET];
        // Table 2-21: the first two bits are the fixed `'10'`. Anything else
        // is not an MPEG-2 PES header (MPEG-1 PES, or damage) and would be
        // misparsed as one.
        if f1 & F1_MARKER_MASK != F1_MARKER_VALUE {
            return Err(Error::BadHeaderMarker);
        }
        let hdl = usize::from(b[HEADER_DATA_LENGTH_OFFSET]);
        let hdr_start = MIN_LEN + HEADER_FIXED; // = 9
        let hdr_end = hdr_start + hdl;
        need_in(b, hdr_end, "PES_header_data_length")?;
        // The header lies inside a bounded packet: a `PES_packet_length` too
        // small to hold `PES_header_data_length` is malformed (and would make
        // serialize write header bytes beyond the declared length).
        if pes_packet_length != 0 && hdr_end > payload_end {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: hdr_end,
                    have: payload_end,
                    what: "PES_header_data_length exceeds PES_packet_length",
                },
            ));
        }
        let opt = &b[hdr_start..hdr_end];
        let mut cursor = 0usize;

        // PTS/DTS (ISO/IEC 13818-1 §2.4.3.7 Table 2-21).
        let pts_dts_flags = (f2 >> PTS_DTS_FLAGS_SHIFT) & PTS_DTS_FLAGS_MASK;

        let (pts, dts) = match pts_dts_flags {
            0b10 => {
                need_in(opt, cursor + PTS_FIELD_LEN, "PTS")?;
                // `need_in` guarantees the field bytes from `cursor` are present.
                let mut field = [0_u8; PTS_FIELD_LEN];
                field.copy_from_slice(&opt[cursor..cursor + PTS_FIELD_LEN]);
                let pts = Pts::from_field_bytes(&field)?;
                cursor += PTS_FIELD_LEN;
                (Some(pts), None)
            }
            0b11 => {
                need_in(opt, cursor + PTS_DTS_FIELD_LEN, "PTS+DTS")?;
                let mut pts_field = [0_u8; PTS_FIELD_LEN];
                pts_field.copy_from_slice(&opt[cursor..cursor + PTS_FIELD_LEN]);
                let pts = Pts::from_field_bytes_with_dts(&pts_field)?;
                cursor += PTS_FIELD_LEN;
                let mut dts_field = [0_u8; PTS_FIELD_LEN];
                dts_field.copy_from_slice(&opt[cursor..cursor + PTS_FIELD_LEN]);
                let dts = Dts::from_field_bytes(&dts_field)?;
                cursor += PTS_FIELD_LEN;
                (Some(pts), Some(dts))
            }
            0b00 => (None, None),
            // 0b01 is a reserved/forbidden combination (Table 2-21 defines
            // only 00/10/11); accepting it silently as "no timestamps"
            // discards a real stream error instead of surfacing it, and the
            // 5 bytes that would have been the timestamp are then misread
            // as whatever comes next (ESCR, ES_rate, ...).
            _ => return Err(Error::ForbiddenPtsDtsFlags),
        };

        // ESCR (6 bytes, ISO/IEC 13818-1 §2.4.3.7).
        let escr = if f2 & F2_ESCR != 0 {
            need_in(opt, cursor + ESCR_FIELD_LEN, "ESCR")?;
            // `need_in` guarantees the field bytes from `cursor` are present.
            let mut arr = [0_u8; ESCR_FIELD_LEN];
            arr.copy_from_slice(&opt[cursor..cursor + ESCR_FIELD_LEN]);
            let e = Escr::from_field_bytes(&arr)?;
            cursor += ESCR_FIELD_LEN;
            Some(e)
        } else {
            None
        };

        // ES_rate (3 bytes: 1 marker + 22-bit rate + 1 marker).
        let mut es_rate_marker_bits = ES_RATE_MARKERS_VALUE;
        let es_rate = if f2 & F2_ES_RATE != 0 {
            need_in(opt, cursor + ES_RATE_FIELD_LEN, "ES_rate")?;
            es_rate_marker_bits = ((opt[cursor + ES_RATE_HI_OFFSET] >> ES_RATE_TOP_BIT_SHIFT)
                << ES_RATE_LO_SHIFT)
                | (opt[cursor + ES_RATE_LO_OFFSET] & 1);
            let rate = (u32::from(opt[cursor + ES_RATE_HI_OFFSET] & LOW7_MASK) << ES_RATE_HI_SHIFT)
                | (u32::from(opt[cursor + ES_RATE_MID_OFFSET]) << ES_RATE_MID_SHIFT)
                | (u32::from(opt[cursor + ES_RATE_LO_OFFSET]) >> ES_RATE_LO_SHIFT);
            cursor += ES_RATE_FIELD_LEN;
            // The 22-bit field is always in range; the check is a reminder that
            // `EsRate` is bounded and reports a (here-unreachable) overflow.
            Some(EsRate::try_new(rate)?)
        } else {
            None
        };

        // DSM trick mode (1 byte).
        let dsm_trick_mode = if f2 & F2_DSM_TRICK_MODE != 0 {
            need_in(opt, cursor + 1, "trick_mode")?;
            let tm = TrickMode::from_byte(opt[cursor]);
            cursor += 1;
            Some(tm)
        } else {
            None
        };

        // additional_copy_info (1 byte: marker + 7-bit info).
        let additional_copy_info = if f2 & F2_ADDITIONAL_COPY_INFO != 0 {
            need_in(opt, cursor + 1, "additional_copy_info")?;
            let v = AdditionalCopyInfo {
                info: opt[cursor] & LOW7_MASK,
                marker: opt[cursor] & ADDITIONAL_COPY_INFO_MARKER != 0,
            };
            cursor += 1;
            Some(v)
        } else {
            None
        };

        // PES_CRC (2 bytes).
        let pes_crc = if f2 & F2_PES_CRC != 0 {
            need_in(opt, cursor + core::mem::size_of::<u16>(), "PES_CRC")?;
            let crc = read_u16_at(opt, cursor);
            cursor += core::mem::size_of::<u16>();
            Some(crc)
        } else {
            None
        };

        // PES_extension.
        let pes_extension = if f2 & F2_PES_EXTENSION != 0 {
            let ext = PesExtension::parse(&opt[cursor..])?;
            cursor += ext.serialized_len();
            Some(ext)
        } else {
            None
        };

        // Bytes remaining in the `PES_header_data_length` region after the typed
        // optional fields are `0xFF` stuffing (ISO/IEC 13818-1:2007 §2.4.3.7).
        // Record the count so serialization reproduces the header byte-for-byte.
        let header_stuffing = opt.get(cursor..).unwrap_or(&[]);

        let header = PesHeader {
            scrambling_control: ScramblingControl::from_bits(f1 >> F1_SCRAMBLING_SHIFT),
            pes_priority: f1 & F1_PRIORITY != 0,
            data_alignment_indicator: f1 & F1_DATA_ALIGNMENT != 0,
            copyright: f1 & F1_COPYRIGHT != 0,
            original_or_copy: f1 & F1_ORIGINAL_OR_COPY != 0,
            pts,
            dts,
            escr,
            es_rate,
            es_rate_marker_bits,
            dsm_trick_mode,
            additional_copy_info,
            pes_crc,
            pes_extension,
            header_stuffing,
        };

        Ok((
            PesPacket {
                stream_id,
                header: Some(header),
                payload: &b[hdr_end..payload_end],
            },
            truncation,
        ))
    }

    /// Serialized length in bytes.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        let hdr = self
            .header
            .as_ref()
            .map_or(0, |h| HEADER_FIXED + h.optional_len());
        MIN_LEN + hdr + self.payload.len()
    }

    /// Serialize back to bytes (byte-identical to a spec-compliant input).
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[..crate::PACKET_START_CODE_PREFIX.len()].copy_from_slice(&PACKET_START_CODE_PREFIX);
        buf[STREAM_ID_OFFSET] = self.stream_id.get();
        // PES_packet_length (ISO/IEC 13818-1 Table 2-21): every byte after the
        // field. `0` is the unbounded form — legal only where the MPEG-2
        // systems layer allows it (video in a transport stream), so a packet
        // with a payload that fills the field range must fail, never wrap.
        let body_len = len - MIN_LEN;
        let packet_length = match u16::try_from(body_len) {
            Ok(n) if n != 0 => n,
            _ if self.stream_id.unbounded_length_allowed() => 0,
            _ => {
                return Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "PES_packet_length",
                    value: body_len as u64,
                    max: u64::from(u16::MAX),
                }));
            }
        };
        buf[PES_PACKET_LENGTH_OFFSET..MIN_LEN].copy_from_slice(&packet_length.to_be_bytes());
        let payload_at = match &self.header {
            None => MIN_LEN,
            Some(h) => {
                if h.dts.is_some() && h.pts.is_none() {
                    // ISO/IEC 13818-1 Table 2-21 has no `PTS_DTS_flags` code
                    // for "DTS but no PTS": without this guard, the flags
                    // written below fall through to `_ => 0b00` (no
                    // timestamps present) while the DTS bytes are written
                    // into the optional region anyway, producing a header
                    // whose declared PES_header_data_length/flags disagree
                    // with its actual bytes.
                    return Err(Error::ForbiddenPtsDtsFlags);
                }
                let opt_len = h.optional_len();
                if opt_len > usize::from(u8::MAX) {
                    return Err(Error::OptionalFieldsTooLarge(opt_len));
                }

                let [f1, f2] = h.flag_bytes();
                buf[FLAGS1_OFFSET] = f1;
                buf[FLAGS2_OFFSET] = f2;
                buf[HEADER_DATA_LENGTH_OFFSET] = opt_len as u8;
                let cursor = MIN_LEN + HEADER_FIXED; // = 9
                h.serialize_optional_fields(&mut buf[cursor..])?;

                cursor + opt_len
            }
        };
        buf[payload_at..len].copy_from_slice(self.payload);
        Ok(len)
    }
}

impl<'a> broadcast_common::Parse<'a> for PesPacket<'a> {
    type Error = crate::Error;

    fn parse(bytes: &'a [u8]) -> core::result::Result<Self, Self::Error> {
        PesPacket::parse(bytes)
    }
}

impl broadcast_common::Serialize for PesPacket<'_> {
    type Error = crate::Error;

    fn serialized_len(&self) -> usize {
        PesPacket::serialized_len(self)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        PesPacket::serialize_into(self, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec;

    fn round_trip(b: &[u8]) {
        let pkt = PesPacket::parse(b).unwrap();
        let mut out = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], b, "round-trip mismatch");
        let re = PesPacket::parse(&out).unwrap();
        // Compare without the borrowed lifetime complexity — compare serialized form.
        let mut re_out = vec![0u8; re.serialized_len()];
        re.serialize_into(&mut re_out).unwrap();
        assert_eq!(out, re_out, "re-parse mismatch");
    }

    #[test]
    fn serialize_into_short_output_is_output_buffer_too_small() {
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0xAA, 0xBB,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        let mut out = vec![0u8; b.len() - 1];
        assert_eq!(
            pkt.serialize_into(&mut out),
            Err(Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall {
                    need: b.len(),
                    have: b.len() - 1
                }
            ))
        );
    }

    #[test]
    fn video_pts_only() {
        // stream_id 0xE0, len=0x0A, flags 0x80/0x80, hdl=5, PTS=0, payload AA BB.
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0xAA, 0xBB,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        assert_eq!(pkt.stream_id, StreamId::new(0xE0));
        let h = pkt.header.as_ref().unwrap();
        assert_eq!(h.pts, Some(Pts::try_new(0).unwrap()));
        assert!(h.dts.is_none());
        assert_eq!(pkt.payload, &[0xAA, 0xBB]);
        round_trip(&b);
    }

    #[test]
    fn pts_and_dts() {
        // PTS_DTS_flags=11, hdl=10. PTS prefix 0011, DTS prefix 0001.
        // PES_packet_length covers 13 bytes (3 header + 10 optional region).
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0E, 0x80, 0xC0, 0x0A, 0x31, 0x00, 0x03, 0x00, 0x01,
            0x11, 0x00, 0x05, 0x00, 0x01, 0xCC,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        let h = pkt.header.as_ref().unwrap();
        assert!(h.pts.is_some());
        assert!(h.dts.is_some());
        round_trip(&b);
    }

    // Regression (audit run-09 W4): `PTS_DTS_flags = 01` is a reserved/
    // forbidden combination (ISO/IEC 13818-1 Table 2-21 defines only
    // 00/10/11) that used to be silently accepted as "no timestamps",
    // discarding a real stream error and misreading the 5 bytes that would
    // have been the timestamp as whatever field comes next.
    #[test]
    fn forbidden_pts_dts_flags_01_is_rejected() {
        let mut b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0E, 0x80, 0xC0, 0x0A, 0x31, 0x00, 0x03, 0x00, 0x01,
            0x11, 0x20, 0x05, 0x20, 0x01, 0xCC,
        ];
        b[7] = 0x40; // pts_dts_flags = 01 (top two bits of f2)
        assert!(matches!(
            PesPacket::parse(&b),
            Err(Error::ForbiddenPtsDtsFlags)
        ));
    }

    // Regression (audit run-09 W3): a `PesHeader` with `dts.is_some()` but
    // `pts.is_none()` must be rejected on serialize, not silently written
    // with `PTS_DTS_flags = 00` while the DTS bytes still occupy the
    // optional region — a header whose declared flags and actual bytes
    // disagree.
    #[test]
    fn serialize_rejects_dts_without_pts() {
        let header = PesHeader {
            scrambling_control: ScramblingControl::NotScrambled,
            pes_priority: false,
            data_alignment_indicator: false,
            copyright: false,
            original_or_copy: false,
            pts: None,
            dts: Some(Dts::try_new(0).unwrap()),
            escr: None,
            es_rate: None,
            es_rate_marker_bits: 0b11,
            dsm_trick_mode: None,
            additional_copy_info: None,
            pes_crc: None,
            pes_extension: None,
            header_stuffing: &[],
        };
        let mut pkt = PesPacket::new(StreamId::new(0xE0), &[0xAA]);
        pkt.header = Some(header);
        let mut buf = vec![0u8; pkt.serialized_len()];
        assert!(matches!(
            pkt.serialize_into(&mut buf),
            Err(Error::ForbiddenPtsDtsFlags)
        ));
    }

    #[test]
    fn pes_header_stuffing_round_trip() {
        // PTS-only (flags 0x80), PES_header_data_length = 8: 5 PTS bytes + 3
        // 0xFF stuffing bytes (ISO/IEC 13818-1:2007 §2.4.3.7).
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x0C, 0x80, 0x80, 0x08, // hdl = 8
            0x21, 0x00, 0x01, 0x00, 0x01, // PTS = 0
            0xFF, 0xFF, 0xFF, // 3 stuffing bytes
            0xAA, // payload
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        let h = pkt.header.as_ref().unwrap();
        assert!(h.pts.is_some());
        assert_eq!(
            h.header_stuffing,
            &[0xFF, 0xFF, 0xFF],
            "3 stuffing bytes after the 5-byte PTS"
        );
        // serialized_len must account for the stuffing.
        assert_eq!(pkt.serialized_len(), b.len());
        round_trip(&b); // byte-identical, incl. the 0xFF stuffing
    }

    #[test]
    fn special_stream_no_header() {
        // padding_stream 0xBE: bytes after length are payload directly.
        let b = [0x00, 0x00, 0x01, 0xBE, 0x00, 0x03, 0xFF, 0xFF, 0xFF];
        let pkt = PesPacket::parse(&b).unwrap();
        assert!(pkt.header.is_none());
        assert_eq!(pkt.payload, &[0xFF, 0xFF, 0xFF]);
        round_trip(&b);
    }

    #[test]
    fn unbounded_length_zero() {
        // PES_packet_length=0 (video): payload runs to end of buffer.
        let b = [
            0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01,
            0x01, 0x02, 0x03,
        ];
        let pkt = PesPacket::parse(&b).unwrap();
        assert_eq!(&b[4..6], &[0x00, 0x00], "unbounded length on the wire");
        assert_eq!(pkt.payload, &[0x01, 0x02, 0x03]);
        // A video packet whose content fits is written with its real length;
        // the unbounded form is used only past the length ceiling.
        let mut out = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut out).unwrap();
        assert_eq!(u16::from_be_bytes([out[4], out[5]]), 11);
        assert_eq!(&out[6..], &b[6..]);
        assert_eq!(PesPacket::parse(&out).unwrap().payload, pkt.payload);
    }

    #[test]
    fn rejects_bad_start_code() {
        let err = PesPacket::parse(&[0x00, 0x00, 0x02, 0xE0, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidSync(e) if e.found == 0x000002));
    }

    #[test]
    fn rejects_short() {
        let err = PesPacket::parse(&[0x00, 0x00, 0x01]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort(..)));
    }

    #[test]
    fn serialize_rejects_oversized_optional_fields() {
        let pack_header = [0u8; 255];
        let h = PesHeader {
            pes_extension: Some(PesExtension {
                pack_header: Some(&pack_header),
                ..PesExtension::new()
            }),
            ..empty_header()
        };
        let mut pkt = PesPacket::new(StreamId::new(0xE0), &[]);
        pkt.header = Some(h);
        let mut out = vec![0u8; pkt.serialized_len()];
        let err = pkt.serialize_into(&mut out).unwrap_err();
        assert!(matches!(err, Error::OptionalFieldsTooLarge(n) if n > 255));
    }

    // ── Typed optional fields round-trips ──────────────────────────────────

    fn build_pes<'a>(h: PesHeader<'a>, payload: &'a [u8]) -> alloc::vec::Vec<u8> {
        let mut pkt = PesPacket::new(StreamId::new(0xE0), payload);
        pkt.header = Some(h);
        let mut out = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut out).unwrap();
        out
    }

    fn empty_header<'a>() -> PesHeader<'a> {
        PesHeader {
            scrambling_control: ScramblingControl::NotScrambled,
            pes_priority: false,
            data_alignment_indicator: false,
            copyright: false,
            original_or_copy: false,
            pts: None,
            dts: None,
            escr: None,
            es_rate: None,
            es_rate_marker_bits: 0b11,
            dsm_trick_mode: None,
            additional_copy_info: None,
            pes_crc: None,
            pes_extension: None,
            header_stuffing: &[],
        }
    }

    /// Construct PES with ESCR set, serialize, parse, assert field preserved.
    #[test]
    fn pes_header_escr_round_trip() {
        let escr = Escr::from_clock27(
            broadcast_common::clock33::Clock27::from_parts(90_000, 150).unwrap(),
        );
        let h = PesHeader {
            escr: Some(escr),
            ..empty_header()
        };
        let bytes = build_pes(h, &[0xAA]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        assert_eq!(pkt.header.unwrap().escr, Some(escr));
    }

    /// Escr::from_27mhz / as_27mhz round-trip.
    #[test]
    fn escr_27mhz_round_trip() {
        for ticks in [0u64, 1, 300, 27_000_000, 8_589_934_591] {
            let e = Escr::from_27mhz(ticks).unwrap();
            assert_eq!(e.as_27mhz(), ticks, "ticks={ticks}");
        }
    }

    /// Escr::to_field_bytes / from_field_bytes round-trip.
    #[test]
    fn escr_field_bytes_round_trip() {
        for (base, ext) in [
            (0u64, 0u16),
            (10_000, 0),
            (0x1_FFFF_FFFF, 0x1FF),
            (1234, 56),
        ] {
            let e = Escr {
                base,
                extension: ext,
                reserved: 0b11,
            };
            let bytes = e.to_field_bytes().unwrap();
            let decoded = Escr::from_field_bytes(&bytes).unwrap();
            assert_eq!(decoded, e, "base={base} ext={ext}");
        }
    }

    /// ES_rate round-trip.
    #[test]
    fn pes_header_es_rate_round_trip() {
        let rate = EsRate::try_new(0x3FFFFF).unwrap();
        let h = PesHeader {
            es_rate: Some(rate),
            es_rate_marker_bits: 0b11,
            ..empty_header()
        };
        let bytes = build_pes(h, &[]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        assert_eq!(pkt.header.unwrap().es_rate, Some(rate));
    }

    /// TrickMode round-trip (all variants).
    #[test]
    fn trick_mode_all_variants_round_trip() {
        let cases = [
            TrickMode::FastForward {
                field_id: 0x2,
                intra_slice_refresh: true,
                frequency_truncation: 0x3,
            },
            TrickMode::SlowMotion { rep_cntrl: 0x1F },
            TrickMode::FreezeFrame {
                field_id: 0x1,
                reserved: 0b101,
            },
            TrickMode::FastReverse {
                field_id: 0x0,
                intra_slice_refresh: false,
                frequency_truncation: 0x1,
            },
            TrickMode::SlowReverse { rep_cntrl: 0 },
            TrickMode::Reserved {
                trick_mode_control: 0b101,
                data: 0x1A,
            },
        ];
        for tm in cases {
            let b = tm.to_byte().unwrap();
            let decoded = TrickMode::from_byte(b);
            assert_eq!(decoded, tm, "tm={tm:?}");
        }
    }

    /// TrickMode in PES header round-trip.
    #[test]
    fn pes_header_trick_mode_round_trip() {
        let tm = TrickMode::FastForward {
            field_id: 1,
            intra_slice_refresh: false,
            frequency_truncation: 2,
        };
        let h = PesHeader {
            dsm_trick_mode: Some(tm),
            ..empty_header()
        };
        let bytes = build_pes(h, &[]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        assert_eq!(pkt.header.unwrap().dsm_trick_mode, Some(tm));
    }

    /// additional_copy_info round-trip.
    #[test]
    fn pes_header_additional_copy_info_round_trip() {
        let h = PesHeader {
            additional_copy_info: Some(AdditionalCopyInfo::new(0x7F)),
            ..empty_header()
        };
        let bytes = build_pes(h, &[]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        assert_eq!(
            pkt.header.unwrap().additional_copy_info,
            Some(AdditionalCopyInfo::new(0x7F))
        );
    }

    /// PES_CRC round-trip.
    #[test]
    fn pes_header_pes_crc_round_trip() {
        let h = PesHeader {
            pes_crc: Some(0xDEAD),
            ..empty_header()
        };
        let bytes = build_pes(h, &[]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        assert_eq!(pkt.header.unwrap().pes_crc, Some(0xDEAD));
    }

    /// PesExtension with program_packet_sequence_counter.
    #[test]
    fn pes_extension_ppsc_round_trip() {
        let ppsc = ProgramPacketSequenceCounter {
            counter: 42,
            mpeg1_mpeg2_identifier: true,
            original_stuff_length: 7,
        };
        let ext = PesExtension {
            pes_private_data: None,
            pack_header: None,
            program_packet_sequence_counter: Some(ppsc),
            p_std_buffer: None,
            pes_extension_field: None,
            ..PesExtension::new()
        };
        let h = PesHeader {
            pes_extension: Some(ext),
            ..empty_header()
        };
        let bytes = build_pes(h, &[]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        let decoded_ext = pkt.header.unwrap().pes_extension.unwrap();
        assert_eq!(decoded_ext.program_packet_sequence_counter, Some(ppsc));
    }

    /// PesExtension with P-STD buffer.
    #[test]
    fn pes_extension_p_std_buffer_round_trip() {
        let pstd = PStdBuffer {
            scale: true,
            size: 0x1FFF,
        };
        let ext = PesExtension {
            pes_private_data: None,
            pack_header: None,
            program_packet_sequence_counter: None,
            p_std_buffer: Some(pstd),
            pes_extension_field: None,
            ..PesExtension::new()
        };
        let h = PesHeader {
            pes_extension: Some(ext),
            ..empty_header()
        };
        let bytes = build_pes(h, &[]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        let decoded_ext = pkt.header.unwrap().pes_extension.unwrap();
        assert_eq!(decoded_ext.p_std_buffer, Some(pstd));
    }

    /// W10 (#1129): `pack_field_length` is an 8-bit field but used to be
    /// written with `ph.len() as u8`, wrapping to a short length while the
    /// full data was still copied after it. Calls the private
    /// `PesExtension::serialize_into` directly (this test module is a
    /// descendant of its defining module) so the assertion isolates this
    /// specific cast: reaching it via the public `PesPacket` API is also
    /// blocked by `PesHeader::optional_len`'s pre-existing `opt_len > 255`
    /// check, since a `pack_header` this size necessarily makes the whole
    /// optional-fields region exceed 255 bytes too.
    #[test]
    fn pack_header_over_255_bytes_rejected_not_wrapped() {
        let data = alloc::vec![0u8; 256];
        let ext = PesExtension {
            pes_private_data: None,
            pack_header: Some(&data),
            program_packet_sequence_counter: None,
            p_std_buffer: None,
            pes_extension_field: None,
            ..PesExtension::new()
        };
        let mut buf = vec![0u8; ext.serialized_len()];
        assert!(matches!(
            &(ext.serialize_into(&mut buf)),
            Err(crate::error::Error::FieldOverflow(_))
        ));
    }

    /// AVS3-P3 audio (T/AI 109.7 8.1.2 / TS 101 154 V2.10.1 4.1.8.35) is a
    /// `stream_id = 0xFD` PES whose `stream_id_extension` is `100 1111`; AVS3
    /// video uses `0x41`. The extension survives a serialize -> parse trip.
    #[test]
    fn avs3_stream_id_extension_round_trips() {
        for want in [
            crate::STREAM_ID_EXTENSION_AVS3_AUDIO,
            crate::STREAM_ID_EXTENSION_AVS3_VIDEO,
        ] {
            let ext = PesExtension {
                pes_extension_field: Some(PesExtensionField {
                    stream_id_extension_flag: false,
                    low_bits: want,
                    rest: &[],
                }),
                ..PesExtension::new()
            };
            let mut buf = vec![0u8; ext.serialized_len()];
            ext.serialize_into(&mut buf).unwrap();
            let back = PesExtension::parse(&buf).unwrap();
            let f = back.pes_extension_field.unwrap();
            assert_eq!(f.stream_id_extension(), Some(want));
        }
        let tref = PesExtensionField {
            stream_id_extension_flag: true,
            low_bits: 1,
            rest: &[],
        };
        assert_eq!(tref.stream_id_extension(), None);
    }

    /// W10 (#1129): `PES_extension_field_length` is an 8-bit field but used
    /// to be written with `ef.len() as u8`, wrapping the same way. See the
    /// note on `pack_header_over_255_bytes_rejected_not_wrapped` above about
    /// calling the private method directly. Post-#1052 the field is 7 bits
    /// (max 127), so this now also covers the narrower ceiling.
    #[test]
    fn pes_extension_field_over_127_bytes_rejected_not_wrapped() {
        let data = alloc::vec![0u8; 256];
        let ext = PesExtension {
            pes_private_data: None,
            pack_header: None,
            program_packet_sequence_counter: None,
            p_std_buffer: None,
            pes_extension_field: Some(PesExtensionField {
                stream_id_extension_flag: false,
                low_bits: 0,
                rest: &data,
            }),
            ..PesExtension::new()
        };
        let mut buf = vec![0u8; ext.serialized_len()];
        assert!(matches!(
            ext.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
    }

    /// #1129: a P-STD buffer size over 13 bits is an error, not masked.
    #[test]
    fn p_std_buffer_size_over_13_bits_rejected_not_masked() {
        let mut ext = PesExtension::new();
        ext.p_std_buffer = Some(PStdBuffer {
            scale: false,
            size: 0x2000,
        });
        let mut buf = vec![0u8; ext.serialized_len()];
        assert!(matches!(
            ext.serialize_into(&mut buf).unwrap_err(),
            Error::FieldOverflow(_)
        ));
        ext.p_std_buffer = Some(PStdBuffer {
            scale: true,
            size: 0x1FFF,
        });
        let mut buf = vec![0u8; ext.serialized_len()];
        assert!(ext.serialize_into(&mut buf).is_ok());
    }

    /// PesExtension with private data (16 bytes).
    #[test]
    fn pes_extension_private_data_round_trip() {
        let pd: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ];
        let ext = PesExtension {
            pes_private_data: Some(pd),
            pack_header: None,
            program_packet_sequence_counter: None,
            p_std_buffer: None,
            pes_extension_field: None,
            ..PesExtension::new()
        };
        let h = PesHeader {
            pes_extension: Some(ext),
            ..empty_header()
        };
        let bytes = build_pes(h, &[]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        let decoded_ext = pkt.header.unwrap().pes_extension.unwrap();
        assert_eq!(decoded_ext.pes_private_data, Some(pd));
    }

    /// All optional PES header fields set at once — serialize and parse round-trip.
    #[test]
    fn pes_header_all_fields_round_trip() {
        let ppsc = ProgramPacketSequenceCounter {
            counter: 1,
            mpeg1_mpeg2_identifier: false,
            original_stuff_length: 0,
        };
        let ext = PesExtension {
            pes_private_data: None,
            pack_header: None,
            program_packet_sequence_counter: Some(ppsc),
            p_std_buffer: Some(PStdBuffer {
                scale: false,
                size: 100,
            }),
            pes_extension_field: None,
            ..PesExtension::new()
        };
        let h = PesHeader {
            scrambling_control: ScramblingControl::NotScrambled,
            pes_priority: true,
            data_alignment_indicator: false,
            copyright: false,
            original_or_copy: true,
            pts: Some(Pts::try_new(90_000).unwrap()),
            dts: Some(Dts::try_new(85_000).unwrap()),
            escr: Some(Escr::from_clock27(
                broadcast_common::clock33::Clock27::from_parts(1000, 0).unwrap(),
            )),
            es_rate: Some(EsRate::try_new(50_000).unwrap()),
            es_rate_marker_bits: 0b11,
            dsm_trick_mode: Some(TrickMode::SlowMotion { rep_cntrl: 3 }),
            additional_copy_info: Some(AdditionalCopyInfo::new(5)),
            pes_crc: Some(0xCAFE),
            pes_extension: Some(ext),
            header_stuffing: &[],
        };
        let bytes = build_pes(h, &[0xFF]);
        let pkt = PesPacket::parse(&bytes).unwrap();
        let dh = pkt.header.unwrap();
        assert_eq!(dh.pts, Some(Pts::try_new(90_000).unwrap()));
        assert_eq!(dh.dts, Some(Dts::try_new(85_000).unwrap()));
        assert!(dh.escr.is_some());
        assert_eq!(dh.es_rate, Some(EsRate::try_new(50_000).unwrap()));
        assert_eq!(
            dh.dsm_trick_mode,
            Some(TrickMode::SlowMotion { rep_cntrl: 3 })
        );
        assert_eq!(dh.additional_copy_info, Some(AdditionalCopyInfo::new(5)));
        assert_eq!(dh.pes_crc, Some(0xCAFE));
        assert!(dh.pes_extension.is_some());
    }
}
