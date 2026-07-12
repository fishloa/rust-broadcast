//! ST 337 burst-preamble + burst-payload framing — SMPTE ST 337:2015 §7.
//!
//! See `st337/docs/st337.md` for the curated spec transcription this module
//! implements field-for-field, and `st337/docs/st337-PROVENANCE.md` for the
//! real-fixture / `ffmpeg -f spdif` cross-check that verified the constants
//! and bit layout below against real running software.

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Named constants (no magic numbers) — SMPTE ST 337:2015 §7
// ---------------------------------------------------------------------------

/// `Pa` — sync word 1, 16-bit mode (Table 6, §7.2.1). Independently confirmed
/// against a real `ffmpeg -f spdif` burst — see `docs/st337-PROVENANCE.md`.
pub const SYNC_WORD_PA: u16 = 0xF872;

/// `Pb` — sync word 2, 16-bit mode (Table 6, §7.2.1). Independently confirmed
/// against a real `ffmpeg -f spdif` burst — see `docs/st337-PROVENANCE.md`.
pub const SYNC_WORD_PB: u16 = 0x4E1F;

/// `data_type` value reserved to mean "see `extended_data_type` (`Pe`)" —
/// the four-word preamble's `data_type` field is only 5 bits wide, so 31 is
/// the escape code to the six-word preamble's 16-bit `Pe` code space
/// (§7.2.1/§7.2.4.2).
pub const EXTENDED_DATA_TYPE_MARKER: u8 = 31;

/// Maximum `length_code` (`Pd`) value in 16-bit mode: "from 0 to 65,535 bits
/// in the 16-bit mode" (§7.2.5).
pub const MAX_LENGTH_CODE_BITS: u32 = 0xFFFF;

/// Maximum `data_type` / `data_type_dependent` value — both 5-bit fields
/// (Table 7).
pub const MAX_5_BIT_FIELD: u8 = 0x1F;

/// Maximum `data_stream_number` value — a 3-bit field (Table 7, §7.2.4.6).
pub const MAX_DATA_STREAM_NUMBER: u8 = 0x07;

/// Byte width of one 16-bit-mode preamble word (`Pa`..`Pf`) in this crate's
/// byte-stream abstraction — see `docs/st337.md` scope decision 1.
const PREAMBLE_WORD_LEN: usize = 2;

/// Byte length of the four-word preamble (`Pa`,`Pb`,`Pc`,`Pd`).
const FOUR_WORD_PREAMBLE_LEN: usize = 4 * PREAMBLE_WORD_LEN;

/// Byte length of the six-word preamble (`Pa`..`Pf`).
const SIX_WORD_PREAMBLE_LEN: usize = 6 * PREAMBLE_WORD_LEN;

/// Bits contributed to `length_code` by `Pe`+`Pf` when the six-word preamble
/// is used: "Pe and Pf shall be counted as payload bytes" (Table 6, §7.2.5) —
/// so the wire `length_code` is `payload_bits + 32` in that case even though
/// `Pe`/`Pf` are structurally part of the preamble, not the payload.
const EXTENDED_PREAMBLE_LENGTH_CODE_BITS: u32 =
    (SIX_WORD_PREAMBLE_LEN - FOUR_WORD_PREAMBLE_LEN) as u32 * 8;

// `Pc` (burst_info) bit layout, 16-bit mode column of Table 7 (§7.2.4):
// bits 0-4 = data_type, 5-6 = data_mode, 7 = error_flag, 8-12 =
// data_type_dependent, 13-15 = data_stream_number.
const PC_DATA_TYPE_MASK: u16 = 0x001F;
const PC_DATA_MODE_SHIFT: u32 = 5;
const PC_DATA_MODE_MASK: u16 = 0x0003;
const PC_ERROR_FLAG_BIT: u16 = 0x0080;
const PC_DATA_TYPE_DEPENDENT_SHIFT: u32 = 8;
const PC_DATA_TYPE_DEPENDENT_MASK: u16 = 0x001F;
const PC_DATA_STREAM_NUMBER_SHIFT: u32 = 13;
const PC_DATA_STREAM_NUMBER_MASK: u16 = 0x0007;

// ---------------------------------------------------------------------------
// DataMode — SMPTE ST 337 §7.2.4.3 Table 8
// ---------------------------------------------------------------------------

/// The 2-bit `data_mode` field (§7.2.4.3 Table 8): which of the 16/20/24-bit
/// AES3-3 subframe positions a burst's words occupy.
///
/// This crate's byte-stream abstraction (see `docs/st337.md` scope decision
/// 2) supports only [`DataMode::Mode16`] for parsing/building a [`Burst`] —
/// `Mode20`/`Mode24` are recognized wire values (round-tripped faithfully as
/// data) but imply a wider physical preamble word this crate does not model,
/// so [`Burst::parse`]/[`Burst::new`] return [`Error::UnsupportedDataMode`]
/// for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DataMode {
    /// `0` — 16-bit mode: `burst_payload` in AES3-3 subframe time slots
    /// 27-12 (Table 8).
    Mode16,
    /// `1` — 20-bit mode: time slots 27-8. Not supported for parse/build by
    /// this crate (see the type doc).
    Mode20,
    /// `2` — 24-bit mode: time slots 27-4. Not supported for parse/build by
    /// this crate (see the type doc).
    Mode24,
    /// `3` — reserved (Table 8).
    Reserved,
}

impl DataMode {
    /// The spec token for this value ("reserved" for the reserved code
    /// point) — see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mode16 => "16-bit mode",
            Self::Mode20 => "20-bit mode",
            Self::Mode24 => "24-bit mode",
            Self::Reserved => "reserved",
        }
    }

    fn from_bits(bits: u16) -> Self {
        match bits {
            0 => Self::Mode16,
            1 => Self::Mode20,
            2 => Self::Mode24,
            _ => Self::Reserved,
        }
    }

    fn to_bits(self) -> u16 {
        match self {
            Self::Mode16 => 0,
            Self::Mode20 => 1,
            Self::Mode24 => 2,
            Self::Reserved => 3,
        }
    }
}

broadcast_common::impl_spec_display!(DataMode);

// ---------------------------------------------------------------------------
// ExtendedPreamble — Pe/Pf (six-word preamble form)
// ---------------------------------------------------------------------------

/// The six-word preamble's extra words, `Pe` and `Pf` (Table 6, §7.2.1),
/// present only when [`BurstPreamble::data_type`] ==
/// [`EXTENDED_DATA_TYPE_MARKER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExtendedPreamble {
    /// `Pe` — `extended_data_type`, a 16-bit code registered in SMPTE ST 338
    /// (Table 6). This crate does not interpret the value.
    pub extended_data_type: u16,
    /// `Pf` — reserved (Table 6). Preserved verbatim on round-trip rather
    /// than forced to zero: ST 337 §2's conformance notation defines
    /// "reserved" as "not defined at this time... may be defined in the
    /// future," not "must be zero," so silently zeroing it could corrupt a
    /// real wire value from equipment using a not-yet-defined convention.
    pub reserved_pf: u16,
}

// ---------------------------------------------------------------------------
// BurstPreamble — Pa..Pf
// ---------------------------------------------------------------------------

/// The burst preamble (`Pa`..`Pd`, or `Pa`..`Pf` when [`Self::extended`] is
/// `Some`) — SMPTE ST 337 §7.2.
///
/// `Pa`/`Pb` are not stored as fields: they are always the fixed
/// [`SYNC_WORD_PA`]/[`SYNC_WORD_PB`] constants for the only supported
/// `data_mode` ([`DataMode::Mode16`]), so storing them as independent fields
/// would just be another way for caller state to disagree with the wire (the
/// same "derive from spec-fixed constants" discipline used throughout this
/// project, e.g. `rtp-packet`'s fixed RTP `version`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BurstPreamble {
    /// `data_type` — 5-bit code, `0..=31` (§7.2.4.2). `31`
    /// ([`EXTENDED_DATA_TYPE_MARKER`]) means "see [`Self::extended`]"; this
    /// crate does not define a named enum for the type -> codec mapping
    /// (registered in SMPTE ST 338, not independently verified here — see
    /// `docs/st337.md` scope decision 3).
    pub data_type: u8,
    /// `data_mode` — Table 8 (§7.2.4.3). Only [`DataMode::Mode16`] is
    /// supported by [`Burst::parse`]/[`Burst::new`] — see the type's doc.
    pub data_mode: DataMode,
    /// `error_flag` — `true` means "data burst may contain errors"
    /// (§7.2.4.4).
    pub error_flag: bool,
    /// `data_type_dependent` — 5 raw bits, meaning defined per-`data_type`
    /// in other specs (§7.2.4.5); this crate carries the value
    /// uninterpreted.
    pub data_type_dependent: u8,
    /// `data_stream_number` — 3-bit stream tag, `0..=7` (§7.2.4.6). `0` is
    /// the main audio service; `7` is reserved for the time-stamp data type.
    pub data_stream_number: u8,
    /// `Pd` — `length_code`, the raw wire value: the number of bits in
    /// `burst_payload`, **plus** 32 when [`Self::extended`] is `Some` (`Pe`
    /// and `Pf`'s bits are counted here too — §7.2.5/Table 6). Stored
    /// verbatim (not recomputed) so a parsed burst re-serializes
    /// byte-identically even from an otherwise-unremarkable real capture;
    /// [`Burst::new`] computes it for you from a payload length.
    pub length_code: u16,
    /// `Pe`/`Pf`, present iff the six-word preamble is used. Required to be
    /// `Some` exactly when `data_type ==` [`EXTENDED_DATA_TYPE_MARKER`] —
    /// enforced by both [`Burst::parse`] and [`Serialize`].
    pub extended: Option<ExtendedPreamble>,
}

impl BurstPreamble {
    /// `Pc` (`burst_info`) bit-packed per the 16-bit-mode column of Table 7.
    fn pack_pc(&self) -> u16 {
        (u16::from(self.data_type) & PC_DATA_TYPE_MASK)
            | ((self.data_mode.to_bits() & PC_DATA_MODE_MASK) << PC_DATA_MODE_SHIFT)
            | (if self.error_flag {
                PC_ERROR_FLAG_BIT
            } else {
                0
            })
            | ((u16::from(self.data_type_dependent) & PC_DATA_TYPE_DEPENDENT_MASK)
                << PC_DATA_TYPE_DEPENDENT_SHIFT)
            | ((u16::from(self.data_stream_number) & PC_DATA_STREAM_NUMBER_MASK)
                << PC_DATA_STREAM_NUMBER_SHIFT)
    }

    fn unpack_pc(pc: u16) -> (u8, DataMode, bool, u8, u8) {
        let data_type = (pc & PC_DATA_TYPE_MASK) as u8;
        let data_mode = DataMode::from_bits((pc >> PC_DATA_MODE_SHIFT) & PC_DATA_MODE_MASK);
        let error_flag = pc & PC_ERROR_FLAG_BIT != 0;
        let data_type_dependent =
            ((pc >> PC_DATA_TYPE_DEPENDENT_SHIFT) & PC_DATA_TYPE_DEPENDENT_MASK) as u8;
        let data_stream_number =
            ((pc >> PC_DATA_STREAM_NUMBER_SHIFT) & PC_DATA_STREAM_NUMBER_MASK) as u8;
        (
            data_type,
            data_mode,
            error_flag,
            data_type_dependent,
            data_stream_number,
        )
    }

    /// Wire byte length of this preamble alone (8 bytes for the four-word
    /// form, 12 for the six-word form).
    #[must_use]
    pub fn wire_len(&self) -> usize {
        if self.extended.is_some() {
            SIX_WORD_PREAMBLE_LEN
        } else {
            FOUR_WORD_PREAMBLE_LEN
        }
    }

    fn validate(&self) -> Result<()> {
        if self.data_mode != DataMode::Mode16 {
            return Err(Error::UnsupportedDataMode(self.data_mode));
        }
        if self.data_type > MAX_5_BIT_FIELD {
            return Err(Error::InvalidValue {
                field: "data_type",
                value: u64::from(self.data_type),
                reason: "must be a 5-bit value (0..=31)",
            });
        }
        if self.data_type_dependent > MAX_5_BIT_FIELD {
            return Err(Error::InvalidValue {
                field: "data_type_dependent",
                value: u64::from(self.data_type_dependent),
                reason: "must be a 5-bit value (0..=31)",
            });
        }
        if self.data_stream_number > MAX_DATA_STREAM_NUMBER {
            return Err(Error::InvalidValue {
                field: "data_stream_number",
                value: u64::from(self.data_stream_number),
                reason: "must be a 3-bit value (0..=7)",
            });
        }
        let requires_extended = self.data_type == EXTENDED_DATA_TYPE_MARKER;
        if requires_extended != self.extended.is_some() {
            return Err(Error::ExtendedPreambleMismatch {
                data_type: self.data_type,
                expected_extended: requires_extended,
                found_extended: self.extended.is_some(),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Burst — the full burst_preamble + burst_payload (§7.1)
// ---------------------------------------------------------------------------

/// One complete non-PCM data burst: [`BurstPreamble`] + `burst_payload`
/// (SMPTE ST 337 §7.1).
///
/// `burst_payload` is carried as an **opaque borrowed byte slice** — this
/// crate never interprets or transforms its content (§7.5 defers all
/// payload-content formatting to per-`data_type` specs; the same "parse the
/// container, not the codec" discipline `transmux` uses for sample data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Burst<'a> {
    /// The burst preamble (`Pa`..`Pd`/`Pf`).
    pub preamble: BurstPreamble,
    /// The burst payload — opaque bytes, `ceil(payload_bits / 8)` long,
    /// where `payload_bits` is [`BurstPreamble::length_code`] minus 32 when
    /// [`BurstPreamble::extended`] is `Some` (§7.2.5).
    pub payload: &'a [u8],
}

impl<'a> Burst<'a> {
    /// Build a new burst from typed preamble fields and a payload,
    /// computing `length_code` for you (`payload.len() * 8`, plus 32 when
    /// `extended` is `Some` — §7.2.5/Table 6).
    ///
    /// # Errors
    /// [`Error::UnsupportedDataMode`] if `data_mode != DataMode::Mode16`;
    /// [`Error::InvalidValue`] if `data_type`/`data_type_dependent`/
    /// `data_stream_number` exceed their bit widths;
    /// [`Error::ExtendedPreambleMismatch`] if `extended.is_some()` disagrees
    /// with `data_type == `[`EXTENDED_DATA_TYPE_MARKER`]; [`Error::PayloadTooLarge`]
    /// if the resulting bit count would exceed [`MAX_LENGTH_CODE_BITS`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data_type: u8,
        data_mode: DataMode,
        error_flag: bool,
        data_type_dependent: u8,
        data_stream_number: u8,
        extended: Option<ExtendedPreamble>,
        payload: &'a [u8],
    ) -> Result<Self> {
        let extended_offset_bits = if extended.is_some() {
            EXTENDED_PREAMBLE_LENGTH_CODE_BITS
        } else {
            0
        };
        let payload_bits = (payload.len() as u64) * 8 + u64::from(extended_offset_bits);
        if payload_bits > u64::from(MAX_LENGTH_CODE_BITS) {
            return Err(Error::PayloadTooLarge {
                payload_bits: payload_bits as u32,
                extended_offset_bits,
            });
        }
        #[allow(clippy::cast_possible_truncation)]
        let length_code = payload_bits as u16;
        let preamble = BurstPreamble {
            data_type,
            data_mode,
            error_flag,
            data_type_dependent,
            data_stream_number,
            length_code,
            extended,
        };
        preamble.validate()?;
        Ok(Self { preamble, payload })
    }
}

impl<'a> Parse<'a> for Burst<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FOUR_WORD_PREAMBLE_LEN {
            return Err(Error::BufferTooShort {
                need: FOUR_WORD_PREAMBLE_LEN,
                have: bytes.len(),
                what: "burst_preamble (Pa-Pd)",
            });
        }
        let pa = u16::from_le_bytes([bytes[0], bytes[1]]);
        if pa != SYNC_WORD_PA {
            return Err(Error::InvalidSync {
                word: "Pa",
                expected: SYNC_WORD_PA,
                found: pa,
            });
        }
        let pb = u16::from_le_bytes([bytes[2], bytes[3]]);
        if pb != SYNC_WORD_PB {
            return Err(Error::InvalidSync {
                word: "Pb",
                expected: SYNC_WORD_PB,
                found: pb,
            });
        }
        let pc = u16::from_le_bytes([bytes[4], bytes[5]]);
        let (data_type, data_mode, error_flag, data_type_dependent, data_stream_number) =
            BurstPreamble::unpack_pc(pc);
        if data_mode != DataMode::Mode16 {
            return Err(Error::UnsupportedDataMode(data_mode));
        }
        let length_code = u16::from_le_bytes([bytes[6], bytes[7]]);

        let is_extended = data_type == EXTENDED_DATA_TYPE_MARKER;
        let (extended, preamble_len) = if is_extended {
            if bytes.len() < SIX_WORD_PREAMBLE_LEN {
                return Err(Error::BufferTooShort {
                    need: SIX_WORD_PREAMBLE_LEN,
                    have: bytes.len(),
                    what: "burst_preamble (Pa-Pf, six-word form)",
                });
            }
            let pe = u16::from_le_bytes([bytes[8], bytes[9]]);
            let pf = u16::from_le_bytes([bytes[10], bytes[11]]);
            (
                Some(ExtendedPreamble {
                    extended_data_type: pe,
                    reserved_pf: pf,
                }),
                SIX_WORD_PREAMBLE_LEN,
            )
        } else {
            (None, FOUR_WORD_PREAMBLE_LEN)
        };

        let extended_offset_bits = if is_extended {
            EXTENDED_PREAMBLE_LENGTH_CODE_BITS
        } else {
            0
        };
        let payload_bits = u32::from(length_code)
            .checked_sub(extended_offset_bits)
            .ok_or(Error::InvalidValue {
                field: "length_code",
                value: u64::from(length_code),
                reason: "shorter than the six-word preamble's own 32 extended-preamble bits",
            })?;
        let payload_len = payload_bits.div_ceil(8) as usize;

        let total_len = preamble_len + payload_len;
        if bytes.len() < total_len {
            return Err(Error::BufferTooShort {
                need: total_len,
                have: bytes.len(),
                what: "burst_payload",
            });
        }

        Ok(Self {
            preamble: BurstPreamble {
                data_type,
                data_mode,
                error_flag,
                data_type_dependent,
                data_stream_number,
                length_code,
                extended,
            },
            payload: &bytes[preamble_len..total_len],
        })
    }
}

impl Serialize for Burst<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.preamble.wire_len() + self.payload.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "burst serialize output",
            });
        }
        self.preamble.validate()?;

        let extended_offset_bits = if self.preamble.extended.is_some() {
            EXTENDED_PREAMBLE_LENGTH_CODE_BITS
        } else {
            0
        };
        let payload_bits = u32::from(self.preamble.length_code)
            .checked_sub(extended_offset_bits)
            .ok_or(Error::InvalidValue {
                field: "length_code",
                value: u64::from(self.preamble.length_code),
                reason: "shorter than the six-word preamble's own 32 extended-preamble bits",
            })?;
        let expected_payload_len = payload_bits.div_ceil(8) as usize;
        if self.payload.len() != expected_payload_len {
            return Err(Error::InvalidValue {
                field: "length_code",
                value: u64::from(self.preamble.length_code),
                reason: "does not match payload.len() (see BurstPreamble::length_code docs)",
            });
        }

        buf[0..2].copy_from_slice(&SYNC_WORD_PA.to_le_bytes());
        buf[2..4].copy_from_slice(&SYNC_WORD_PB.to_le_bytes());
        buf[4..6].copy_from_slice(&self.preamble.pack_pc().to_le_bytes());
        buf[6..8].copy_from_slice(&self.preamble.length_code.to_le_bytes());
        let preamble_len = if let Some(ext) = self.preamble.extended {
            buf[8..10].copy_from_slice(&ext.extended_data_type.to_le_bytes());
            buf[10..12].copy_from_slice(&ext.reserved_pf.to_le_bytes());
            SIX_WORD_PREAMBLE_LEN
        } else {
            FOUR_WORD_PREAMBLE_LEN
        };
        buf[preamble_len..preamble_len + self.payload.len()].copy_from_slice(self.payload);

        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_minimal_four_word_burst() {
        let payload = [0xAAu8, 0xBB, 0xCC];
        let burst = Burst::new(1, DataMode::Mode16, false, 0, 0, None, &payload).unwrap();
        assert_eq!(burst.preamble.length_code, 24);
        let bytes = burst.to_bytes();
        assert_eq!(bytes.len(), 8 + 3);
        let reparsed = Burst::parse(&bytes).unwrap();
        assert_eq!(reparsed, burst);
    }

    #[test]
    fn round_trips_a_six_word_extended_burst() {
        let payload = [0x11u8, 0x22, 0x33, 0x44];
        let ext = ExtendedPreamble {
            extended_data_type: 0x1234,
            reserved_pf: 0x0000,
        };
        let burst = Burst::new(
            EXTENDED_DATA_TYPE_MARKER,
            DataMode::Mode16,
            true,
            5,
            7,
            Some(ext),
            &payload,
        )
        .unwrap();
        // 4 payload bytes = 32 bits, + 32 extended-preamble bits = 64.
        assert_eq!(burst.preamble.length_code, 64);
        let bytes = burst.to_bytes();
        assert_eq!(bytes.len(), 12 + 4);
        let reparsed = Burst::parse(&bytes).unwrap();
        assert_eq!(reparsed, burst);
        assert_eq!(reparsed.preamble.extended, Some(ext));
    }

    #[test]
    fn pc_bit_layout_matches_the_real_ffmpeg_oracle() {
        // docs/st337-PROVENANCE.md: ffmpeg -f spdif's real E-AC-3 burst has
        // Pc = 0x0015 (LE bytes 15 00): data_type=21, data_mode=Mode16,
        // error_flag=false, data_type_dependent=0, data_stream_number=0.
        let preamble = BurstPreamble {
            data_type: 21,
            data_mode: DataMode::Mode16,
            error_flag: false,
            data_type_dependent: 0,
            data_stream_number: 0,
            length_code: 0,
            extended: None,
        };
        assert_eq!(preamble.pack_pc(), 0x0015);
        let (dt, dm, ef, dtd, dsn) = BurstPreamble::unpack_pc(0x0015);
        assert_eq!(dt, 21);
        assert_eq!(dm, DataMode::Mode16);
        assert!(!ef);
        assert_eq!(dtd, 0);
        assert_eq!(dsn, 0);
    }

    #[test]
    fn rejects_bad_pa_sync() {
        let mut bytes = [0u8; 8];
        bytes[0..2].copy_from_slice(&0x0000u16.to_le_bytes());
        let err = Burst::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidSync { word: "Pa", .. }));
    }

    #[test]
    fn rejects_bad_pb_sync() {
        let mut bytes = [0u8; 8];
        bytes[0..2].copy_from_slice(&SYNC_WORD_PA.to_le_bytes());
        bytes[2..4].copy_from_slice(&0x0000u16.to_le_bytes());
        let err = Burst::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidSync { word: "Pb", .. }));
    }

    #[test]
    fn rejects_unsupported_data_mode() {
        let payload = [0u8; 1];
        let err = Burst::new(0, DataMode::Mode20, false, 0, 0, None, &payload).unwrap_err();
        assert!(matches!(err, Error::UnsupportedDataMode(DataMode::Mode20)));
    }

    #[test]
    fn data_type_31_requires_extended_preamble() {
        let payload = [0u8; 1];
        let err = Burst::new(
            EXTENDED_DATA_TYPE_MARKER,
            DataMode::Mode16,
            false,
            0,
            0,
            None,
            &payload,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Error::ExtendedPreambleMismatch {
                data_type: EXTENDED_DATA_TYPE_MARKER,
                expected_extended: true,
                found_extended: false,
            }
        ));
    }

    #[test]
    fn non_31_data_type_rejects_extended_preamble() {
        let payload = [0u8; 1];
        let ext = ExtendedPreamble {
            extended_data_type: 0,
            reserved_pf: 0,
        };
        let err = Burst::new(1, DataMode::Mode16, false, 0, 0, Some(ext), &payload).unwrap_err();
        assert!(matches!(
            err,
            Error::ExtendedPreambleMismatch {
                data_type: 1,
                expected_extended: false,
                found_extended: true,
            }
        ));
    }

    #[test]
    fn max_16_bit_mode_payload_is_accepted() {
        // 65535 bits = 8191 bytes + 7 spare bits (last byte only 7 bits "real").
        let payload = vec![0xFFu8; 8191];
        let burst = Burst::new(0, DataMode::Mode16, false, 0, 0, None, &payload).unwrap();
        assert_eq!(burst.preamble.length_code, 65528); // 8191*8, still <= 65535
        let bytes = burst.to_bytes();
        let reparsed = Burst::parse(&bytes).unwrap();
        assert_eq!(reparsed, burst);
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let payload = vec![0u8; 8192]; // 65536 bits > 65535 max
        let err = Burst::new(0, DataMode::Mode16, false, 0, 0, None, &payload).unwrap_err();
        assert!(matches!(err, Error::PayloadTooLarge { .. }));
    }

    #[test]
    fn buffer_too_short_for_preamble() {
        let err = Burst::parse(&[0u8; 4]).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                need: 8,
                have: 4,
                ..
            }
        ));
    }

    #[test]
    fn buffer_too_short_for_payload() {
        let burst = Burst::new(0, DataMode::Mode16, false, 0, 0, None, &[1, 2, 3]).unwrap();
        let bytes = burst.to_bytes();
        let err = Burst::parse(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                what: "burst_payload",
                ..
            }
        ));
    }

    #[test]
    fn data_mode_name_and_display() {
        assert_eq!(DataMode::Mode16.name(), "16-bit mode");
        assert_eq!(DataMode::Reserved.to_string(), "reserved");
    }
}
