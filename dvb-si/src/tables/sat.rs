//! Satellite Access Table (SAT) — ETSI EN 300 468 §5.2.11.
//!
//! Long-form private section on PID 0x001B with table_id 0x4D. The SAT is a
//! *family*: a common `satellite_access_section()` header carries a 6-bit
//! `satellite_table_id` discriminant ([`SatTableId`]) that selects one of five
//! body structures (position v2, cell fragment, time association, beamhopping
//! time plan, position v3).
//!
//! The body is typed as [`SatBody`] — an enum with one variant per defined
//! layout plus a [`SatBody::Raw`] fallthrough for reserved
//! `satellite_table_id` values 5–63. All five layouts use bit-packed fields; a
//! private bit-level reader/writer handles the extraction and emission.

use crate::error::{Error, Result};
use alloc::vec;
use alloc::vec::Vec;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::reserved::ReservedZero;
use broadcast_common::wire_enum;
use broadcast_common::{Parse, Serialize};

/// table_id for the Satellite Access Table.
pub const TABLE_ID: u8 = 0x4D;
/// Well-known PID on which the SAT is carried (EN 300 468 Table 1, §5.1.3).
pub const PID: mpeg_ts::pid::Pid = mpeg_ts::pid::Pid::from_const(0x001B);

const HEADER_LEN: usize = 9;
const SECTION_LENGTH_PREFIX: usize = 3;
/// Trial-encode buffer for [`sat_body_serialized_len`]: one byte over the
/// largest legal SAT body (`MAX_SECTION_LENGTH` less the 5 extension-header
/// bytes and 4 CRC bytes is < 4096).
const SAT_BODY_STACK_BUF: usize = 4096;
/// Largest `section_length` (12 bits; EN 300 468 §5.2.11 / Table 11a).
const MAX_SECTION_LENGTH: usize = 0x0FFF;
/// `satellite_table_id(6)` sits in the top six bits of byte 3 (§5.2.11).
const SATELLITE_TABLE_ID_SHIFT: u8 = 2;
/// Low two bits of byte 3: the high bits of the 10-bit `table_count`.
const TABLE_COUNT_HI_MASK: u8 = 0x03;
/// Six-bit `satellite_table_id` mask in byte 3.
const SATELLITE_TABLE_ID_MASK: u8 = 0x3F;

/// Byte 3 carries `satellite_table_id` and the high `table_count` bits.
const SATELLITE_TABLE_ID_OFFSET: usize = 3;
/// Low `table_count` bits (byte 4).
const TABLE_COUNT_LO_OFFSET: usize = 4;
/// `version_number` / `current_next_indicator` byte.
/// Trailing `reserved_zero_future_use(8)` header byte.
const RESERVED_TRAILER_OFFSET: usize = 8;
/// Low byte of `section_length`.
const SECTION_LENGTH_LO_OFFSET: usize = 2;

/// Wire masks for the SAT code enums (Tables 11f, 11g, 11i).
const ASSOCIATION_TYPE_MASK: u8 = 0x0F;
const TIME_PLAN_MODE_MASK: u8 = 0x03;
const INTERPOLATION_TYPE_MASK: u8 = 0x07;

/// Highest `satellite_table_id` with a typed body (Tables 11c–11h).
const MAX_TYPED_SAT_TABLE_ID: u8 = SatTableId::PositionV3 as u8;

/// Bit-field widths of the SAT layouts, named for the spec field each carries
/// (ETSI EN 300 468 §5.2.11).
pub const HEADER_RESERVED_BITS: u8 = 2;
const TIME_PLAN_MODE_BITS: u8 = 2;
const POSITION_V3_FLAGS_BITS: u8 = 5;
/// Width of the `reserved` field in the position-v3 layout.
pub const POSITION_V3_RESERVED_BITS: u8 = 3;
const INTERPOLATION_BITS: u8 = 3;
/// Width of a `reserved` field in the SAT layouts.
pub const RESERVED_FIELD_BITS: u8 = 4;
const ASSOCIATION_TYPE_BITS: u8 = 4;
const OEM_VERSION_BITS: u8 = 4;
/// Width of the `reserved` field in the cell-centre layout.
pub const CELL_CENTRE_RESERVED_BITS: u8 = 5;
/// Width of a `reserved_future_use` field in the SAT layouts.
pub const RESERVED_FUTURE_USE_BITS: u8 = 6;
/// Width of a trailing `reserved` field in the SAT layouts.
pub const TRAILING_RESERVED_BITS: u8 = 7;
/// Width of the reserved section-header byte in a SAT section.
pub const HEADER_RESERVED_BYTE_BITS: u8 = 8;
const YEAR_BITS: u8 = 8;
const EXTENSION_BITS: u8 = 9;
const DAY_BITS: u8 = 9;
const LOOP_COUNT_BITS: u8 = 10;
const PLAN_LENGTH_BITS: u8 = 12;
const BIT_MAP_BITS: u8 = 15;
const ORBITAL_POSITION_BITS: u8 = 16;
const DAY_OF_YEAR_BITS: u8 = 16;
const EPHEMERIS_COUNT_BITS: u8 = 16;
const LATITUDE_BITS: u8 = 18;
const LONGITUDE_BITS: u8 = 19;
const SATELLITE_ID_BITS: u8 = 24;
const MAX_DISTANCE_BITS: u8 = 24;
const WORD_BITS: u8 = 32;
const BASE_BITS: u8 = 33;
const TIMESTAMP_SECONDS_BITS: u8 = 64;

/// Fixed widths of the derived entry layouts, in bits.
const ORBITAL_BITS: usize = ORBITAL_POSITION_BITS as usize + 1 + TRAILING_RESERVED_BITS as usize;
const SGP4_VECTOR_FIELDS: usize = 10;
const SGP4_BITS: usize =
    YEAR_BITS as usize + DAY_OF_YEAR_BITS as usize + WORD_BITS as usize * SGP4_VECTOR_FIELDS;
const CELL_CENTRE_BITS: usize = RESERVED_FIELD_BITS as usize
    + LATITUDE_BITS as usize
    + CELL_CENTRE_RESERVED_BITS as usize
    + LONGITUDE_BITS as usize
    + MAX_DISTANCE_BITS as usize
    + RESERVED_FUTURE_USE_BITS as usize;
const DELIVERY_SYSTEM_ENTRY_BITS: usize = WORD_BITS as usize
    + BASE_BITS as usize
    + RESERVED_FUTURE_USE_BITS as usize
    + EXTENSION_BITS as usize;
const TIME_ASSOC_MIN_BITS: usize = ASSOCIATION_TYPE_BITS as usize
    + RESERVED_FIELD_BITS as usize
    + BASE_BITS as usize
    + RESERVED_FUTURE_USE_BITS as usize
    + EXTENSION_BITS as usize
    + TIMESTAMP_SECONDS_BITS as usize
    + WORD_BITS as usize;
const MODE0_BITS: usize = BASE_BITS as usize
    + RESERVED_FUTURE_USE_BITS as usize
    + EXTENSION_BITS as usize
    + BASE_BITS as usize
    + RESERVED_FUTURE_USE_BITS as usize
    + EXTENSION_BITS as usize;
const MODE1_HEADER_BITS: usize = 1 + BIT_MAP_BITS as usize + 1 + BIT_MAP_BITS as usize;
const MODE2_BITS: usize = MODE0_BITS * 2;
const POS_V3_HEADER_BITS: usize = OEM_VERSION_BITS as usize
    + OEM_VERSION_BITS as usize
    + YEAR_BITS as usize
    + TRAILING_RESERVED_BITS as usize
    + DAY_BITS as usize
    + WORD_BITS as usize;
const USABLE_TIME_BITS: usize =
    YEAR_BITS as usize + TRAILING_RESERVED_BITS as usize + DAY_BITS as usize + WORD_BITS as usize;
const METADATA_FIXED_BITS: usize =
    USABLE_TIME_BITS * 2 + 1 + 1 + INTERPOLATION_BITS as usize + INTERPOLATION_BITS as usize;
const EPHEMERIS_VECTOR_FIELDS: usize = 6;
const EPHEMERIS_ACCEL_FIELDS: usize = 3;
const COV_HEADER_BITS: usize =
    YEAR_BITS as usize + TRAILING_RESERVED_BITS as usize + DAY_BITS as usize + WORD_BITS as usize;
/// Number of covariance-matrix elements in the ephemeris block.
pub const COVARIANCE_ELEMENTS: usize = 21;
const COV_ELEMENTS_BITS: usize = COVARIANCE_ELEMENTS * WORD_BITS as usize;
const COV_BITS: usize = COV_HEADER_BITS + COV_ELEMENTS_BITS;
const PLAN_FIELDS_BITS: usize = RESERVED_FUTURE_USE_BITS as usize
    + TIME_PLAN_MODE_BITS as usize
    + BASE_BITS as usize
    + RESERVED_FUTURE_USE_BITS as usize
    + EXTENSION_BITS as usize
    + BASE_BITS as usize
    + RESERVED_FUTURE_USE_BITS as usize
    + EXTENSION_BITS as usize;
/// Trial-buffer heap growth factor.
const SAT_BODY_HEAP_GROWTH: usize = 2;
/// Upper bound on the trial-encode heap buffer.
const MAX_TRIAL_BUF_BYTES: usize = 1 << 20;
use super::CRC_LEN;

fn pad_to_byte(bits: usize) -> usize {
    (u8::BITS as usize - (bits % u8::BITS as usize)) % u8::BITS as usize
}

// ── Bit-level reader/writer ──────────────────────────────────────────────────

struct BitReader<'a> {
    inner: broadcast_common::bits::BitReader<'a>,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            inner: broadcast_common::bits::BitReader::new(data),
        }
    }
    fn remaining_bits(&self) -> usize {
        self.inner.bits_remaining()
    }
    fn bits_consumed(&self) -> usize {
        self.inner.bits_read()
    }
    // `need`/`have` are whole bytes, the unit of every
    // `Error::BufferTooShort` in the workspace: `need` is the byte the read
    // must reach, `have` the whole bytes the body holds.
    fn overrun(&self, bits: usize) -> Error {
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: (self.inner.bits_read() + bits).div_ceil(u8::BITS as usize),
            have: self.inner.total_bits() / u8::BITS as usize,
            what: "SatSection bit reader overrun",
        })
    }
    fn read_u(&mut self, bits: u8) -> Result<u64> {
        self.inner
            .read_bits(u32::from(bits))
            .map_err(|_| self.overrun(usize::from(bits)))
    }
    fn read_i(&mut self, bits: u8) -> Result<i64> {
        let raw = self.read_u(bits)?;
        let bits = bits as usize;
        if raw & (1u64 << (bits - 1)) != 0 {
            Ok((raw as i64) | (!0i64 << bits))
        } else {
            Ok(raw as i64)
        }
    }
    fn seek_forward(&mut self, target_bits: usize) {
        let target = target_bits.min(self.inner.total_bits());
        let _ = self.inner.skip_bits(target - self.inner.bits_read());
    }
    fn skip(&mut self, bits: u8) -> Result<()> {
        self.inner
            .skip_bits(usize::from(bits))
            .map_err(|_| self.overrun(usize::from(bits)))
    }
}

// The writer is [`broadcast_common::bits::BitWriter`], the one MSB-first
// writer in the workspace. This adapter keeps the SAT's own contract on top
// of it: a value wider than its field is `Error::FieldOverflow` (#1129
// class, so a count such as `frag.delivery_system_ids.len()` into a 10-bit
// field is an error instead of a wrapped, misframed section), and a run that
// does not fit the output is `Error::BufferTooShort` in whole bytes, never a
// silent truncation.
struct BitWriter<'a> {
    inner: broadcast_common::bits::BitWriter<'a>,
}

impl<'a> BitWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self {
            inner: broadcast_common::bits::BitWriter::new(buf),
        }
    }
    fn bits_written(&self) -> usize {
        self.inner.bits_written()
    }
    fn overrun(&self, bits: usize) -> Error {
        Error::BufferTooShort(
            broadcast_common::bits::BitError::OutOfBounds {
                needed_bits: bits,
                remaining_bits: self.inner.capacity_bits() - self.inner.bits_written(),
            }
            .into(),
        )
    }
    fn write_u(&mut self, bits: u8, val: u64) -> Result<()> {
        broadcast_common::len::fit_bits(val, u32::from(bits), "SatSection field")?;
        self.inner
            .write_bits(val, u32::from(bits))
            .map_err(|_| self.overrun(usize::from(bits)))
    }
    fn write_i(&mut self, bits: u8, val: i64) -> Result<()> {
        // Two's-complement range of a `bits`-wide signed field.
        let max = (1i64 << (bits - 1)) - 1;
        let min = -(1i64 << (bits - 1));
        if val < min || val > max {
            return Err(broadcast_common::len::FieldOverflow {
                field: "SatSection signed field",
                value: val.unsigned_abs(),
                max: max as u64,
            }
            .into());
        }
        let mask = (1u64 << bits) - 1;
        self.write_u(bits, val as u64 & mask)
    }
    fn write_zero(&mut self, bits: u8) -> Result<()> {
        self.inner
            .write_bits(0, u32::from(bits))
            .map_err(|_| self.overrun(usize::from(bits)))
    }
}

// ── SatTableId discriminant ─────────────────────────────────────────────────

wire_enum! {
    /// `satellite_table_id` discriminant — selects the SAT body structure
    /// (§5.2.11.1, Table 11b).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    #[non_exhaustive]
    pub enum SatTableId: u8, error(crate::error::Error), invalid(value) => crate::error::Error::InvalidValue(
        broadcast_common::error::InvalidValue {
            field: "satellite_table_id",
            value: u64::from(value),
            reason: "reserved (EN 300 468 §5.2.11.1 Table 11b)",
        },
    ),
    {
        /// `satellite_position_v2_info` — TLE/SGP4 orbital elements (§5.2.11.2).
        PositionV2 = 0,
        /// `cell_fragment_info` — earth-surface cell coverage areas (§5.2.11.3).
        CellFragment = 1,
        /// `time_association_info` — NCR↔UTC time association (§5.2.11.4).
        TimeAssociation = 2,
        /// `beamhopping_time_plan_info` — beam illumination schedule (§5.2.11.5).
        BeamhoppingTimePlan = 3,
        /// `satellite_position_v3_info` — ephemeris state vectors (§5.2.11.6).
        PositionV3 = 4,
    }
}

// ── Position V2 (Table 11c) ─────────────────────────────────────────────────

/// Position system selector for PositionV2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PositionSystem {
    /// `position_system == 0`: orbital position (BCD 16-bit, west_east_flag).
    Orbital {
        /// `orbital_position` (16 bits, BCD-encoded as 4 digits).
        orbital_position: u16,
        /// `west_east_flag`.
        west_east_flag: bool,
        /// `reserved_zero_future_use` (7 bits, right-aligned) after `west_east_flag`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
    },
    /// `position_system == 1`: SGP4 TLE elements.
    Sgp4 {
        /// `epoch_year` (8 bits).
        epoch_year: u8,
        /// `day_of_the_year` (16 bits).
        day_of_the_year: u16,
        /// `day_fraction` (32 bits, raw).
        day_fraction: u32,
        /// `mean_motion_first_derivative` (32 bits, raw spfmsbf).
        mean_motion_first_derivative: u32,
        /// `mean_motion_second_derivative` (32 bits, raw spfmsbf).
        mean_motion_second_derivative: u32,
        /// `drag_term` (32 bits, raw spfmsbf).
        drag_term: u32,
        /// `inclination` (32 bits, raw spfmsbf).
        inclination: u32,
        /// `right_ascension_of_the_ascending_node` (32 bits, raw spfmsbf).
        right_ascension: u32,
        /// `eccentricity` (32 bits, raw spfmsbf).
        eccentricity: u32,
        /// `argument_of_perigree` (32 bits, raw spfmsbf).
        argument_of_perigree: u32,
        /// `mean_anomaly` (32 bits, raw spfmsbf).
        mean_anomaly: u32,
        /// `mean_motion` (32 bits, raw spfmsbf).
        mean_motion: u32,
    },
}

/// A satellite entry in the PositionV2 body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PositionV2Satellite {
    /// `satellite_id` (24 bits).
    pub satellite_id: u32,
    /// Position data (orbital or SGP4).
    pub position: PositionSystem,
    /// `reserved_zero_future_use` (7 bits, right-aligned) after `satellite_id`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
}

/// Position V2 body (Table 11c, §5.2.11.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PositionV2Body {
    /// Satellite entries.
    pub satellites: Vec<PositionV2Satellite>,
}

// ── Cell Fragment (Table 11d) ────────────────────────────────────────────────

/// Centre coordinates for a cell fragment (present when `first_occurrence == 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CellCentre {
    /// `centre_latitude` (18 bits, two's complement, `tcimsbf`).
    pub centre_latitude: i32,
    /// `centre_longitude` (19 bits, two's complement, `tcimsbf`).
    pub centre_longitude: i32,
    /// `max_distance` (24 bits).
    pub max_distance: u32,
    /// `reserved_zero_future_use` (4 bits, right-aligned) before `centre_latitude`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_4: ReservedZero<{ RESERVED_FIELD_BITS as u32 }>,
    /// `reserved_zero_future_use` (5 bits, right-aligned) before `centre_longitude`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_5: ReservedZero<{ CELL_CENTRE_RESERVED_BITS as u32 }>,
    /// `reserved_zero_future_use` (6 bits, right-aligned) after `max_distance`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_6: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
}

/// A new delivery system entry in a cell fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NewDeliverySystem {
    /// `new_delivery_system_id` (32 bits).
    pub new_delivery_system_id: u32,
    /// `time_of_application_base` (33 bits).
    pub time_of_application_base: u64,
    /// `time_of_application_ext` (9 bits).
    pub time_of_application_ext: u16,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `time_of_application_ext`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
}

/// An obsolescent delivery system entry in a cell fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ObsolescentDeliverySystem {
    /// `obsolescent_delivery_system_id` (32 bits).
    pub obsolescent_delivery_system_id: u32,
    /// `time_of_obsolescence_base` (33 bits).
    pub time_of_obsolescence_base: u64,
    /// `time_of_obsolescence_ext` (9 bits).
    pub time_of_obsolescence_ext: u16,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `time_of_obsolescence_ext`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
}

/// A cell fragment entry (Table 11d, §5.2.11.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CellFragment {
    /// `cell_fragment_id` (32 bits).
    pub cell_fragment_id: u32,
    /// `first_occurrence`.
    pub first_occurrence: bool,
    /// `last_occurrence`.
    pub last_occurrence: bool,
    /// Centre coordinates (present iff `first_occurrence`).
    pub centre: Option<CellCentre>,
    /// `delivery_system_id` entries (each 32 bits).
    pub delivery_system_ids: Vec<u32>,
    /// New delivery system entries.
    pub new_delivery_systems: Vec<NewDeliverySystem>,
    /// Obsolescent delivery system entries.
    pub obsolescent_delivery_systems: Vec<ObsolescentDeliverySystem>,
    /// `reserved_zero_future_use` (4 bits, right-aligned) of the not-`first_occurrence` branch (unused, `0`, when `centre` is present). Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_4: ReservedZero<{ RESERVED_FIELD_BITS as u32 }>,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `new_delivery_system_id_loop_count`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_new: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `obsolescent_delivery_system_id_loop_count`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_obsolescent: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
}

/// Cell Fragment body (Table 11d, §5.2.11.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CellFragmentBody {
    /// Cell fragment entries.
    pub fragments: Vec<CellFragment>,
}

// ── Time Association (Table 11e) ────────────────────────────────────────────

/// Association type coding — ETSI EN 300 468 §5.2.11.4 Table 11f.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AssociationType {
    /// 0 — UTC without leap second signalling.
    UtcWithoutLeap,
    /// 1 — UTC with leap second signalling.
    UtcWithLeap,
    /// 2..=15 — reserved.
    Reserved(u8),
}

impl AssociationType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v & ASSOCIATION_TYPE_MASK {
            0 => Self::UtcWithoutLeap,
            1 => Self::UtcWithLeap,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::UtcWithoutLeap => 0,
            Self::UtcWithLeap => 1,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::UtcWithoutLeap => "UTC without leap second",
            Self::UtcWithLeap => "UTC with leap second",
            Self::Reserved(_) => "Reserved",
        }
    }
}
broadcast_common::impl_spec_display!(AssociationType, Reserved);

/// Leap-second signalling info (present when `association_type == 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LeapInfo {
    /// `leap59`.
    pub leap59: bool,
    /// `leap61`.
    pub leap61: bool,
    /// `pastleap59`.
    pub pastleap59: bool,
    /// `pastleap61`.
    pub pastleap61: bool,
}

/// Time Association body (Table 11e, §5.2.11.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimeAssociationBody {
    /// `association_type` (4 bits, Table 11f).
    pub association_type: AssociationType,
    /// Leap info (present iff `association_type == 1`).
    pub leap_info: Option<LeapInfo>,
    /// `ncr_base` (33 bits).
    pub ncr_base: u64,
    /// `ncr_ext` (9 bits).
    pub ncr_ext: u16,
    /// `association_timestamp_seconds` (64 bits).
    pub association_timestamp_seconds: u64,
    /// `association_timestamp_nanoseconds` (32 bits).
    pub association_timestamp_nanoseconds: u32,
    /// `reserved_zero_future_use` (4 bits, right-aligned) of the non-leap `association_type` branch (unused, `0`, when `leap_info` is present). Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_4: ReservedZero<{ RESERVED_FIELD_BITS as u32 }>,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `ncr_ext`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub ncr_reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
}

// ── Beamhopping Time Plan (Table 11g) ───────────────────────────────────────

/// Time plan mode — ETSI EN 300 468 §5.2.11.5 Table 11g
/// (`private/specs-verified/dvb/dvb-si/tables/en_300_468/11g-beamhopping-time-plan-info.md`).
///
/// 2-bit field. Selects the body structure of a beamhopping plan entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TimePlanMode {
    /// `0b00` — simple dwell/on-time plan (mode 0).
    DwellOnTime,
    /// `0b01` — bitmap plan (mode 1).
    Bitmap,
    /// `0b10` — grid/revisit/sleep plan (mode 2).
    GridRevisitSleep,
    /// `0b11` — reserved.
    Reserved(u8),
}

impl TimePlanMode {
    #[must_use]
    /// Creates a value from a 2-bit wire nibble (upper bits masked off).
    pub fn from_u8(v: u8) -> Self {
        match v & TIME_PLAN_MODE_MASK {
            0 => Self::DwellOnTime,
            1 => Self::Bitmap,
            2 => Self::GridRevisitSleep,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the 2-bit wire nibble for this value.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::DwellOnTime => 0,
            Self::Bitmap => 1,
            Self::GridRevisitSleep => 2,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns the spec token for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::DwellOnTime => "dwell/on-time",
            Self::Bitmap => "bitmap",
            Self::GridRevisitSleep => "grid/revisit/sleep",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(TimePlanMode, Reserved);

/// Mode-specific data in a beamhopping plan entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum BeamhoppingMode {
    /// `time_plan_mode == 0`: simple dwell/on-time.
    Mode0 {
        /// `dwell_duration_base` (33 bits).
        dwell_duration_base: u64,
        /// `dwell_duration_ext` (9 bits).
        dwell_duration_ext: u16,
        /// `on_time_base` (33 bits).
        on_time_base: u64,
        /// `on_time_ext` (9 bits).
        on_time_ext: u16,
        /// `reserved_zero_future_use` (6 bits, right-aligned) before `dwell_duration_ext`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        dwell_duration_reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
        /// `reserved_zero_future_use` (6 bits, right-aligned) before `on_time_ext`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        on_time_reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
    },
    /// `time_plan_mode == 1`: bitmap.
    Mode1 {
        /// `bit_map_size` (15 bits).
        bit_map_size: u16,
        /// `current_slot` (15 bits).
        current_slot: u16,
        /// `slot_transmission_on` flags (bit_map_size entries).
        slot_transmission_on: Vec<bool>,
        /// `reserved_zero_future_use` (1 bits, right-aligned) before `bit_map_size`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        reserved_zero_future_use_size: ReservedZero<1>,
        /// `reserved_zero_future_use` (1 bits, right-aligned) before `current_slot`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        reserved_zero_future_use_slot: ReservedZero<1>,
    },
    /// `time_plan_mode == 2`: grid/revisit/sleep.
    Mode2 {
        /// `grid_size_base` (33 bits).
        grid_size_base: u64,
        /// `grid_size_ext` (9 bits).
        grid_size_ext: u16,
        /// `revisit_duration_base` (33 bits).
        revisit_duration_base: u64,
        /// `revisit_duration_ext` (9 bits).
        revisit_duration_ext: u16,
        /// `sleep_time_base` (33 bits).
        sleep_time_base: u64,
        /// `sleep_time_ext` (9 bits).
        sleep_time_ext: u16,
        /// `sleep_duration_base` (33 bits).
        sleep_duration_base: u64,
        /// `sleep_duration_ext` (9 bits).
        sleep_duration_ext: u16,
        /// `reserved_zero_future_use` (6 bits, right-aligned) before `grid_size_ext`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        grid_size_reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
        /// `reserved_zero_future_use` (6 bits, right-aligned) before `revisit_duration_ext`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        revisit_duration_reserved_zero_future_use:
            ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
        /// `reserved_zero_future_use` (6 bits, right-aligned) before `sleep_time_ext`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        sleep_time_reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
        /// `reserved_zero_future_use` (6 bits, right-aligned) before `sleep_duration_ext`. Captured on parse and
        /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
        /// sender's bits round-trip). Hand-built values default to `0`.
        sleep_duration_reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
    },
    /// Reserved `time_plan_mode` (3): raw body bytes between the common
    /// header and the plan boundary, preserved for byte-exact round-trip.
    Reserved(Vec<u8>),
}

/// A beamhopping plan entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BeamhoppingPlan {
    /// `beamhopping_time_plan_id` (32 bits).
    pub beamhopping_time_plan_id: u32,
    /// `time_plan_mode` (2 bits) — [`TimePlanMode`].
    pub time_plan_mode: TimePlanMode,
    /// `time_of_application_base` (33 bits).
    pub time_of_application_base: u64,
    /// `time_of_application_ext` (9 bits).
    pub time_of_application_ext: u16,
    /// `cycle_duration_base` (33 bits).
    pub cycle_duration_base: u64,
    /// `cycle_duration_ext` (9 bits).
    pub cycle_duration_ext: u16,
    /// Mode-specific data.
    pub mode: BeamhoppingMode,
    /// `reserved_zero_future_use` (4 bits, right-aligned) before `beamhopping_time_plan_length`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_4: ReservedZero<{ RESERVED_FIELD_BITS as u32 }>,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `time_plan_mode`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use_mode: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `time_of_application_ext`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub time_of_application_reserved_zero_future_use:
        ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
    /// `reserved_zero_future_use` (6 bits, right-aligned) before `cycle_duration_ext`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub cycle_duration_reserved_zero_future_use: ReservedZero<{ RESERVED_FUTURE_USE_BITS as u32 }>,
}

/// Beamhopping Time Plan body (Table 11g, §5.2.11.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BeamhoppingTimePlanBody {
    /// Plan entries.
    pub plans: Vec<BeamhoppingPlan>,
}

// ── Position V3 (Table 11h) ─────────────────────────────────────────────────

/// Usable time range (optional, within metadata).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UsableTime {
    /// `year` (8 bits).
    pub year: u8,
    /// `day` (9 bits).
    pub day: u16,
    /// `day_fraction` (32 bits, spfmsbf raw).
    pub day_fraction: u32,
    /// `reserved_zero_future_use` (7 bits, right-aligned) before `day`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
}

/// Metadata block (optional, within a V3 satellite entry).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PositionV3Metadata {
    /// `total_start_time_year` (8 bits).
    pub total_start_time_year: u8,
    /// `total_start_time_day` (9 bits).
    pub total_start_time_day: u16,
    /// `total_start_time_day_fraction` (32 bits).
    pub total_start_time_day_fraction: u32,
    /// `total_stop_time_year` (8 bits).
    pub total_stop_time_year: u8,
    /// `total_stop_time_day` (9 bits).
    pub total_stop_time_day: u16,
    /// `total_stop_time_day_fraction` (32 bits).
    pub total_stop_time_day_fraction: u32,
    /// `interpolation_flag` — 1 bit.
    pub interpolation_flag: bool,
    /// `interpolation_type` (3 bits, Table 11i). EN 300 468 v1.20.1: when
    /// `interpolation_flag == 0` this field is *undefined and not used* (v1.19.1
    /// said "reserved"); every 3-bit value is preserved verbatim.
    pub interpolation_type: InterpolationType,
    /// `interpolation_degree` (3 bits). Undefined and not used when
    /// `interpolation_flag == 0` (v1.20.1); preserved verbatim.
    pub interpolation_degree: u8,
    /// Usable start time (optional).
    pub usable_start_time: Option<UsableTime>,
    /// Usable stop time (optional).
    pub usable_stop_time: Option<UsableTime>,
    /// `reserved_zero_future_use` (7 bits, right-aligned) before `total_start_time_day`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub total_start_time_reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
    /// `reserved_zero_future_use` (7 bits, right-aligned) before `total_stop_time_day`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub total_stop_time_reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
    /// `reserved_zero_future_use` (1 bits, right-aligned) before `interpolation_flag`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use: ReservedZero<1>,
}

/// Interpolation type coding — ETSI EN 300 468 §5.2.11.6 Table 11i.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum InterpolationType {
    /// 0 — Reserved.
    Reserved0,
    /// 1 — Linear.
    Linear,
    /// 2 — Lagrange.
    Lagrange,
    /// 3 — Reserved.
    Reserved3,
    /// 4 — Hermite.
    Hermite,
    /// 5..=7 — Reserved.
    ReservedOther(u8),
}

impl InterpolationType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v & INTERPOLATION_TYPE_MASK {
            0 => Self::Reserved0,
            1 => Self::Linear,
            2 => Self::Lagrange,
            3 => Self::Reserved3,
            4 => Self::Hermite,
            v => Self::ReservedOther(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Reserved0 => 0,
            Self::Linear => 1,
            Self::Lagrange => 2,
            Self::Reserved3 => 3,
            Self::Hermite => 4,
            Self::ReservedOther(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved0 => "Reserved",
            Self::Linear => "Linear",
            Self::Lagrange => "Lagrange",
            Self::Reserved3 => "Reserved",
            Self::Hermite => "Hermite",
            Self::ReservedOther(_) => "Reserved",
        }
    }
}
broadcast_common::impl_spec_display!(InterpolationType, ReservedOther);

/// Ephemeris acceleration (optional, 3 × 32-bit spfmsbf).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EphemerisAccel {
    /// `ephemeris_x_ddot` (32 bits, spfmsbf raw).
    pub ephemeris_x_ddot: u32,
    /// `ephemeris_y_ddot` (32 bits, spfmsbf raw).
    pub ephemeris_y_ddot: u32,
    /// `ephemeris_z_ddot` (32 bits, spfmsbf raw).
    pub ephemeris_z_ddot: u32,
}

/// A single ephemeris data point.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EphemerisData {
    /// `epoch_year` (8 bits).
    pub epoch_year: u8,
    /// `epoch_day` (9 bits).
    pub epoch_day: u16,
    /// `epoch_day_fraction` (32 bits).
    pub epoch_day_fraction: u32,
    /// `ephemeris_x` (32 bits, spfmsbf raw).
    pub ephemeris_x: u32,
    /// `ephemeris_y` (32 bits, spfmsbf raw).
    pub ephemeris_y: u32,
    /// `ephemeris_z` (32 bits, spfmsbf raw).
    pub ephemeris_z: u32,
    /// `ephemeris_x_dot` (32 bits, spfmsbf raw).
    pub ephemeris_x_dot: u32,
    /// `ephemeris_y_dot` (32 bits, spfmsbf raw).
    pub ephemeris_y_dot: u32,
    /// `ephemeris_z_dot` (32 bits, spfmsbf raw).
    pub ephemeris_z_dot: u32,
    /// Acceleration (optional).
    pub acceleration: Option<EphemerisAccel>,
    /// `reserved_zero_future_use` (7 bits, right-aligned) before `epoch_day`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
}

/// Covariance data (21 × 32-bit elements).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CovarianceData {
    /// `covariance_epoch_year` (8 bits).
    pub covariance_epoch_year: u8,
    /// `covariance_epoch_day` (9 bits).
    pub covariance_epoch_day: u16,
    /// `covariance_epoch_day_fraction` (32 bits).
    pub covariance_epoch_day_fraction: u32,
    /// 21 covariance elements (each 32 bits, spfmsbf raw).
    pub covariance_elements: [u32; COVARIANCE_ELEMENTS],
    /// `reserved_zero_future_use` (7 bits, right-aligned) before `covariance_epoch_day`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub covariance_epoch_reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
}

/// A satellite entry in the PositionV3 body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PositionV3Satellite {
    /// `satellite_id` (24 bits).
    pub satellite_id: u32,
    /// `usable_start_time_flag`. EN 300 468 v1.20.1 (Table 11h semantics): when
    /// `metadata_flag == 0` this flag is *not used* (v1.19.1 said "reserved");
    /// the parsed bit is preserved verbatim either way.
    pub usable_start_time_flag: bool,
    /// `usable_stop_time_flag`. Not used when `metadata_flag == 0` (v1.20.1,
    /// was "reserved" in v1.19.1); the parsed bit is preserved verbatim.
    pub usable_stop_time_flag: bool,
    /// `ephemeris_accel_flag`.
    pub ephemeris_accel_flag: bool,
    /// `covariance_flag`.
    pub covariance_flag: bool,
    /// Metadata block (optional); its presence also drives the
    /// `metadata_flag` bit on the wire.
    pub metadata: Option<PositionV3Metadata>,
    /// Ephemeris data entries; their count is derived on serialization.
    pub ephemeris_data: Vec<EphemerisData>,
    /// Covariance data (optional).
    pub covariance: Option<CovarianceData>,
    /// `reserved_zero_future_use` (3 bits, right-aligned) before `metadata_flag`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub reserved_zero_future_use: ReservedZero<{ POSITION_V3_RESERVED_BITS as u32 }>,
}

/// Position V3 body (Table 11h, §5.2.11.6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PositionV3Body {
    /// `oem_version_major` (4 bits).
    pub oem_version_major: u8,
    /// `oem_version_minor` (4 bits).
    pub oem_version_minor: u8,
    /// `creation_date_year` (8 bits).
    pub creation_date_year: u8,
    /// `creation_date_day` (9 bits).
    pub creation_date_day: u16,
    /// `creation_date_day_fraction` (32 bits).
    pub creation_date_day_fraction: u32,
    /// Satellite entries.
    pub satellites: Vec<PositionV3Satellite>,
    /// `reserved_zero_future_use` (7 bits, right-aligned) before `creation_date_day`. Captured on parse and
    /// re-emitted verbatim (EN 300 468 Table 11x: value `0` by spec; a deviating
    /// sender's bits round-trip). Hand-built values default to `0`.
    pub creation_date_reserved_zero_future_use: ReservedZero<{ TRAILING_RESERVED_BITS as u32 }>,
}

// ── SatBody enum ────────────────────────────────────────────────────────────

/// The typed body of a SAT section, selected by `satellite_table_id`
/// (Tables 11c–11h).
///
/// Every *defined* `satellite_table_id` (`SatTableId`) has a typed variant
/// below, so a caller never has to hand-slice a known layout. [`Self::Raw`]
/// is the reserved-discriminant fallthrough only.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SatBody {
    /// `satellite_table_id == 0`: Position V2 (Table 11c).
    PositionV2(PositionV2Body),
    /// `satellite_table_id == 1`: Cell Fragment (Table 11d).
    CellFragment(CellFragmentBody),
    /// `satellite_table_id == 2`: Time Association (Table 11e).
    TimeAssociation(TimeAssociationBody),
    /// `satellite_table_id == 3`: Beamhopping Time Plan (Table 11g).
    BeamhoppingTimePlan(BeamhoppingTimePlanBody),
    /// `satellite_table_id == 4`: Position V3 (Table 11h).
    PositionV3(PositionV3Body),
    /// Reserved `satellite_table_id` (5–63), i.e. `kind()` is `None`: the
    /// body bytes verbatim.
    ///
    /// This is the one deliberate raw-bytes exposure in this enum, and it is
    /// not a gap in typing (r02-W12): EN 300 468 (V1.18.1) Table 11 defines
    /// layouts only for `satellite_table_id` 0–4, so there is no syntax to
    /// type these bodies against. `Vec` (not a borrow) keeps `SatBody` free
    /// of the input's lifetime, matching `BitReader`'s owned-interior design.
    Raw(Vec<u8>),
}

fn sat_body_serialized_len(body: &SatBody) -> usize {
    match body {
        SatBody::Raw(v) => v.len(),
        _ => {
            // The hardened BitWriter errors (rather than silently truncating)
            // on overrun, so trial-encode into a buffer big enough for any
            // legal body. Real SAT bodies are bounded by the 12-bit
            // section_length (<4 KiB), so a stack buffer covers every valid
            // section with no heap traffic (audit r02-O2: this used to
            // `vec![0; 4096]` three times per `serialize_into`). An
            // over-large constructed body falls through to a heap buffer that
            // grows to the cap and is then rejected by the section_length
            // guard in serialize_into — never a panic in this infallible
            // length calc.
            let mut stack = [0u8; SAT_BODY_STACK_BUF];
            let mut writer = BitWriter::new(&mut stack);
            if sat_body_write(body, &mut writer).is_ok() {
                return writer.bits_written().div_ceil(u8::BITS as usize);
            }
            let mut cap = SAT_BODY_STACK_BUF * SAT_BODY_HEAP_GROWTH;
            loop {
                let mut tmp = vec![0u8; cap];
                let mut writer = BitWriter::new(&mut tmp);
                if sat_body_write(body, &mut writer).is_ok() {
                    break writer.bits_written().div_ceil(u8::BITS as usize);
                }
                if cap >= MAX_TRIAL_BUF_BYTES {
                    break cap; // pathological; serialize_into rejects it
                }
                cap *= SAT_BODY_HEAP_GROWTH;
            }
        }
    }
}

fn sat_body_write(body: &SatBody, w: &mut BitWriter) -> Result<()> {
    match body {
        SatBody::PositionV2(b) => sat_write_position_v2(b, w)?,
        SatBody::CellFragment(b) => sat_write_cell_fragment(b, w)?,
        SatBody::TimeAssociation(b) => sat_write_time_association(b, w)?,
        SatBody::BeamhoppingTimePlan(b) => sat_write_beamhopping_time_plan(b, w)?,
        SatBody::PositionV3(b) => sat_write_position_v3(b, w)?,
        SatBody::Raw(_) => {}
    }
    Ok(())
}

fn sat_write_position_v2(b: &PositionV2Body, w: &mut BitWriter<'_>) -> Result<()> {
    for sat in &b.satellites {
        w.write_u(SATELLITE_ID_BITS, sat.satellite_id as u64)?;
        w.write_u(TRAILING_RESERVED_BITS, sat.reserved_zero_future_use.get())?;
        match &sat.position {
            PositionSystem::Orbital {
                orbital_position,
                west_east_flag,
                reserved_zero_future_use,
            } => {
                w.write_u(1, 0)?;
                w.write_u(ORBITAL_POSITION_BITS, *orbital_position as u64)?;
                w.write_u(1, *west_east_flag as u64)?;
                w.write_u(TRAILING_RESERVED_BITS, reserved_zero_future_use.get())?;
            }
            PositionSystem::Sgp4 {
                epoch_year,
                day_of_the_year,
                day_fraction,
                mean_motion_first_derivative,
                mean_motion_second_derivative,
                drag_term,
                inclination,
                right_ascension,
                eccentricity,
                argument_of_perigree,
                mean_anomaly,
                mean_motion,
            } => {
                w.write_u(1, 1)?;
                w.write_u(YEAR_BITS, *epoch_year as u64)?;
                w.write_u(DAY_OF_YEAR_BITS, *day_of_the_year as u64)?;
                w.write_u(WORD_BITS, *day_fraction as u64)?;
                w.write_u(WORD_BITS, *mean_motion_first_derivative as u64)?;
                w.write_u(WORD_BITS, *mean_motion_second_derivative as u64)?;
                w.write_u(WORD_BITS, *drag_term as u64)?;
                w.write_u(WORD_BITS, *inclination as u64)?;
                w.write_u(WORD_BITS, *right_ascension as u64)?;
                w.write_u(WORD_BITS, *eccentricity as u64)?;
                w.write_u(WORD_BITS, *argument_of_perigree as u64)?;
                w.write_u(WORD_BITS, *mean_anomaly as u64)?;
                w.write_u(WORD_BITS, *mean_motion as u64)?;
            }
        }
    }
    Ok(())
}

fn sat_write_cell_fragment(b: &CellFragmentBody, w: &mut BitWriter<'_>) -> Result<()> {
    for frag in &b.fragments {
        w.write_u(WORD_BITS, frag.cell_fragment_id as u64)?;
        w.write_u(1, frag.first_occurrence as u64)?;
        w.write_u(1, frag.last_occurrence as u64)?;
        if frag.first_occurrence {
            let Some(ref c) = frag.centre else {
                return Err(Error::ValueOutOfRange {
                    field: "centre",
                    reason: "required when first_occurrence is set",
                });
            };
            w.write_u(RESERVED_FIELD_BITS, c.reserved_zero_future_use_4.get())?;
            w.write_i(LATITUDE_BITS, c.centre_latitude as i64)?;
            w.write_u(
                CELL_CENTRE_RESERVED_BITS,
                c.reserved_zero_future_use_5.get(),
            )?;
            w.write_i(LONGITUDE_BITS, c.centre_longitude as i64)?;
            w.write_u(MAX_DISTANCE_BITS, c.max_distance as u64)?;
            w.write_u(RESERVED_FUTURE_USE_BITS, c.reserved_zero_future_use_6.get())?;
        } else {
            w.write_u(RESERVED_FIELD_BITS, frag.reserved_zero_future_use_4.get())?;
        }
        w.write_u(LOOP_COUNT_BITS, frag.delivery_system_ids.len() as u64)?;
        for id in &frag.delivery_system_ids {
            w.write_u(WORD_BITS, *id as u64)?;
        }
        w.write_u(
            RESERVED_FUTURE_USE_BITS,
            frag.reserved_zero_future_use_new.get(),
        )?;
        w.write_u(LOOP_COUNT_BITS, frag.new_delivery_systems.len() as u64)?;
        for nds in &frag.new_delivery_systems {
            w.write_u(WORD_BITS, nds.new_delivery_system_id as u64)?;
            w.write_u(BASE_BITS, nds.time_of_application_base)?;
            w.write_u(RESERVED_FUTURE_USE_BITS, nds.reserved_zero_future_use.get())?;
            w.write_u(EXTENSION_BITS, nds.time_of_application_ext as u64)?;
        }
        w.write_u(
            RESERVED_FUTURE_USE_BITS,
            frag.reserved_zero_future_use_obsolescent.get(),
        )?;
        w.write_u(
            LOOP_COUNT_BITS,
            frag.obsolescent_delivery_systems.len() as u64,
        )?;
        for ods in &frag.obsolescent_delivery_systems {
            w.write_u(WORD_BITS, ods.obsolescent_delivery_system_id as u64)?;
            w.write_u(BASE_BITS, ods.time_of_obsolescence_base)?;
            w.write_u(RESERVED_FUTURE_USE_BITS, ods.reserved_zero_future_use.get())?;
            w.write_u(EXTENSION_BITS, ods.time_of_obsolescence_ext as u64)?;
        }
    }
    Ok(())
}

fn sat_write_time_association(b: &TimeAssociationBody, w: &mut BitWriter<'_>) -> Result<()> {
    w.write_u(ASSOCIATION_TYPE_BITS, b.association_type.to_u8() as u64)?;
    if b.association_type.to_u8() == 1 {
        if let Some(ref li) = b.leap_info {
            w.write_u(1, li.leap59 as u64)?;
            w.write_u(1, li.leap61 as u64)?;
            w.write_u(1, li.pastleap59 as u64)?;
            w.write_u(1, li.pastleap61 as u64)?;
        } else {
            w.write_u(RESERVED_FIELD_BITS, b.reserved_zero_future_use_4.get())?;
        }
    } else {
        w.write_u(RESERVED_FIELD_BITS, b.reserved_zero_future_use_4.get())?;
    }
    w.write_u(BASE_BITS, b.ncr_base)?;
    w.write_u(
        RESERVED_FUTURE_USE_BITS,
        b.ncr_reserved_zero_future_use.get(),
    )?;
    w.write_u(EXTENSION_BITS, b.ncr_ext as u64)?;
    w.write_u(TIMESTAMP_SECONDS_BITS, b.association_timestamp_seconds)?;
    w.write_u(WORD_BITS, b.association_timestamp_nanoseconds as u64)?;
    Ok(())
}

fn sat_write_beamhopping_time_plan(
    b: &BeamhoppingTimePlanBody,
    w: &mut BitWriter<'_>,
) -> Result<()> {
    for plan in &b.plans {
        w.write_u(WORD_BITS, plan.beamhopping_time_plan_id as u64)?;
        w.write_u(RESERVED_FIELD_BITS, plan.reserved_zero_future_use_4.get())?;
        let mode_bits = match &plan.mode {
            BeamhoppingMode::Mode0 { .. } => MODE0_BITS,
            BeamhoppingMode::Mode1 { bit_map_size, .. } => {
                let bm = *bit_map_size as usize;
                let raw = MODE1_HEADER_BITS + bm;
                raw + pad_to_byte(raw)
            }
            BeamhoppingMode::Mode2 { .. } => MODE2_BITS,
            BeamhoppingMode::Reserved(v) => v.len() * u8::BITS as usize,
        };
        let total_bits_after_length = PLAN_FIELDS_BITS + mode_bits;
        let plan_length_bytes = total_bits_after_length / u8::BITS as usize;
        w.write_u(PLAN_LENGTH_BITS, plan_length_bytes as u64)?;
        w.write_u(
            RESERVED_FUTURE_USE_BITS,
            plan.reserved_zero_future_use_mode.get(),
        )?;
        w.write_u(TIME_PLAN_MODE_BITS, plan.time_plan_mode.to_u8() as u64)?;
        w.write_u(BASE_BITS, plan.time_of_application_base)?;
        w.write_u(
            RESERVED_FUTURE_USE_BITS,
            plan.time_of_application_reserved_zero_future_use.get(),
        )?;
        w.write_u(EXTENSION_BITS, plan.time_of_application_ext as u64)?;
        w.write_u(BASE_BITS, plan.cycle_duration_base)?;
        w.write_u(
            RESERVED_FUTURE_USE_BITS,
            plan.cycle_duration_reserved_zero_future_use.get(),
        )?;
        w.write_u(EXTENSION_BITS, plan.cycle_duration_ext as u64)?;
        match &plan.mode {
            BeamhoppingMode::Mode0 {
                dwell_duration_base,
                dwell_duration_ext,
                on_time_base,
                on_time_ext,
                dwell_duration_reserved_zero_future_use,
                on_time_reserved_zero_future_use,
            } => {
                w.write_u(BASE_BITS, *dwell_duration_base)?;
                w.write_u(
                    RESERVED_FUTURE_USE_BITS,
                    dwell_duration_reserved_zero_future_use.get(),
                )?;
                w.write_u(EXTENSION_BITS, *dwell_duration_ext as u64)?;
                w.write_u(BASE_BITS, *on_time_base)?;
                w.write_u(
                    RESERVED_FUTURE_USE_BITS,
                    on_time_reserved_zero_future_use.get(),
                )?;
                w.write_u(EXTENSION_BITS, *on_time_ext as u64)?;
            }
            BeamhoppingMode::Mode1 {
                bit_map_size,
                current_slot,
                slot_transmission_on,
                reserved_zero_future_use_size,
                reserved_zero_future_use_slot,
            } => {
                w.write_u(1, reserved_zero_future_use_size.get())?;
                w.write_u(BIT_MAP_BITS, *bit_map_size as u64)?;
                w.write_u(1, reserved_zero_future_use_slot.get())?;
                w.write_u(BIT_MAP_BITS, *current_slot as u64)?;
                for &on in slot_transmission_on {
                    w.write_u(1, on as u64)?;
                }
                let total = MODE1_HEADER_BITS + *bit_map_size as usize;
                for _ in 0..pad_to_byte(total) {
                    w.write_zero(1)?;
                }
            }
            BeamhoppingMode::Mode2 {
                grid_size_base,
                grid_size_ext,
                revisit_duration_base,
                revisit_duration_ext,
                sleep_time_base,
                sleep_time_ext,
                sleep_duration_base,
                sleep_duration_ext,
                grid_size_reserved_zero_future_use,
                revisit_duration_reserved_zero_future_use,
                sleep_time_reserved_zero_future_use,
                sleep_duration_reserved_zero_future_use,
            } => {
                w.write_u(BASE_BITS, *grid_size_base)?;
                w.write_u(
                    RESERVED_FUTURE_USE_BITS,
                    grid_size_reserved_zero_future_use.get(),
                )?;
                w.write_u(EXTENSION_BITS, *grid_size_ext as u64)?;
                w.write_u(BASE_BITS, *revisit_duration_base)?;
                w.write_u(
                    RESERVED_FUTURE_USE_BITS,
                    revisit_duration_reserved_zero_future_use.get(),
                )?;
                w.write_u(EXTENSION_BITS, *revisit_duration_ext as u64)?;
                w.write_u(BASE_BITS, *sleep_time_base)?;
                w.write_u(
                    RESERVED_FUTURE_USE_BITS,
                    sleep_time_reserved_zero_future_use.get(),
                )?;
                w.write_u(EXTENSION_BITS, *sleep_time_ext as u64)?;
                w.write_u(BASE_BITS, *sleep_duration_base)?;
                w.write_u(
                    RESERVED_FUTURE_USE_BITS,
                    sleep_duration_reserved_zero_future_use.get(),
                )?;
                w.write_u(EXTENSION_BITS, *sleep_duration_ext as u64)?;
            }
            BeamhoppingMode::Reserved(v) => {
                for &b in v {
                    w.write_u(u8::BITS as u8, b as u64)?;
                }
            }
        }
    }
    Ok(())
}

fn sat_write_position_v3(b: &PositionV3Body, w: &mut BitWriter<'_>) -> Result<()> {
    w.write_u(OEM_VERSION_BITS, b.oem_version_major as u64)?;
    w.write_u(OEM_VERSION_BITS, b.oem_version_minor as u64)?;
    w.write_u(YEAR_BITS, b.creation_date_year as u64)?;
    w.write_u(
        TRAILING_RESERVED_BITS,
        b.creation_date_reserved_zero_future_use.get(),
    )?;
    w.write_u(DAY_BITS, b.creation_date_day as u64)?;
    w.write_u(WORD_BITS, b.creation_date_day_fraction as u64)?;
    for sat in &b.satellites {
        w.write_u(SATELLITE_ID_BITS, sat.satellite_id as u64)?;
        w.write_u(
            POSITION_V3_RESERVED_BITS,
            sat.reserved_zero_future_use.get(),
        )?;
        w.write_u(1, u8::from(sat.metadata.is_some()) as u64)?;
        w.write_u(1, sat.usable_start_time_flag as u64)?;
        w.write_u(1, sat.usable_stop_time_flag as u64)?;
        w.write_u(1, sat.ephemeris_accel_flag as u64)?;
        w.write_u(1, sat.covariance_flag as u64)?;
        if let Some(ref md) = sat.metadata {
            w.write_u(YEAR_BITS, md.total_start_time_year as u64)?;
            w.write_u(
                TRAILING_RESERVED_BITS,
                md.total_start_time_reserved_zero_future_use.get(),
            )?;
            w.write_u(DAY_BITS, md.total_start_time_day as u64)?;
            w.write_u(WORD_BITS, md.total_start_time_day_fraction as u64)?;
            w.write_u(YEAR_BITS, md.total_stop_time_year as u64)?;
            w.write_u(
                TRAILING_RESERVED_BITS,
                md.total_stop_time_reserved_zero_future_use.get(),
            )?;
            w.write_u(DAY_BITS, md.total_stop_time_day as u64)?;
            w.write_u(WORD_BITS, md.total_stop_time_day_fraction as u64)?;
            w.write_u(1, md.reserved_zero_future_use.get())?;
            w.write_u(1, md.interpolation_flag as u64)?;
            w.write_u(INTERPOLATION_BITS, md.interpolation_type.to_u8() as u64)?;
            w.write_u(INTERPOLATION_BITS, md.interpolation_degree as u64)?;
            if sat.usable_start_time_flag {
                if let Some(ref ut) = md.usable_start_time {
                    w.write_u(YEAR_BITS, ut.year as u64)?;
                    w.write_u(TRAILING_RESERVED_BITS, ut.reserved_zero_future_use.get())?;
                    w.write_u(DAY_BITS, ut.day as u64)?;
                    w.write_u(WORD_BITS, ut.day_fraction as u64)?;
                } else {
                    w.write_zero(YEAR_BITS)?;
                    w.write_zero(TRAILING_RESERVED_BITS)?;
                    w.write_zero(DAY_BITS)?;
                    w.write_zero(WORD_BITS)?;
                }
            }
            if sat.usable_stop_time_flag {
                if let Some(ref ut) = md.usable_stop_time {
                    w.write_u(YEAR_BITS, ut.year as u64)?;
                    w.write_u(TRAILING_RESERVED_BITS, ut.reserved_zero_future_use.get())?;
                    w.write_u(DAY_BITS, ut.day as u64)?;
                    w.write_u(WORD_BITS, ut.day_fraction as u64)?;
                } else {
                    w.write_zero(YEAR_BITS)?;
                    w.write_zero(TRAILING_RESERVED_BITS)?;
                    w.write_zero(DAY_BITS)?;
                    w.write_zero(WORD_BITS)?;
                }
            }
        }
        w.write_u(EPHEMERIS_COUNT_BITS, sat.ephemeris_data.len() as u64)?;
        for ed in &sat.ephemeris_data {
            w.write_u(YEAR_BITS, ed.epoch_year as u64)?;
            w.write_u(TRAILING_RESERVED_BITS, ed.reserved_zero_future_use.get())?;
            w.write_u(DAY_BITS, ed.epoch_day as u64)?;
            w.write_u(WORD_BITS, ed.epoch_day_fraction as u64)?;
            w.write_u(WORD_BITS, ed.ephemeris_x as u64)?;
            w.write_u(WORD_BITS, ed.ephemeris_y as u64)?;
            w.write_u(WORD_BITS, ed.ephemeris_z as u64)?;
            w.write_u(WORD_BITS, ed.ephemeris_x_dot as u64)?;
            w.write_u(WORD_BITS, ed.ephemeris_y_dot as u64)?;
            w.write_u(WORD_BITS, ed.ephemeris_z_dot as u64)?;
            if sat.ephemeris_accel_flag {
                if let Some(ref acc) = ed.acceleration {
                    w.write_u(WORD_BITS, acc.ephemeris_x_ddot as u64)?;
                    w.write_u(WORD_BITS, acc.ephemeris_y_ddot as u64)?;
                    w.write_u(WORD_BITS, acc.ephemeris_z_ddot as u64)?;
                } else {
                    w.write_zero(WORD_BITS)?;
                    w.write_zero(WORD_BITS)?;
                    w.write_zero(WORD_BITS)?;
                }
            }
        }
        if sat.covariance_flag {
            if let Some(ref cov) = sat.covariance {
                w.write_u(YEAR_BITS, cov.covariance_epoch_year as u64)?;
                w.write_u(
                    TRAILING_RESERVED_BITS,
                    cov.covariance_epoch_reserved_zero_future_use.get(),
                )?;
                w.write_u(DAY_BITS, cov.covariance_epoch_day as u64)?;
                w.write_u(WORD_BITS, cov.covariance_epoch_day_fraction as u64)?;
                for elem in &cov.covariance_elements {
                    w.write_u(WORD_BITS, *elem as u64)?;
                }
            } else {
                w.write_zero(YEAR_BITS)?;
                w.write_zero(TRAILING_RESERVED_BITS)?;
                w.write_zero(DAY_BITS)?;
                w.write_zero(WORD_BITS)?;
                for _ in 0..COVARIANCE_ELEMENTS {
                    w.write_zero(WORD_BITS)?;
                }
            }
        }
    }
    Ok(())
}

/// Parse a SAT body. The typed loop bodies stop when fewer bits than one entry remain; those
/// leftover bytes belong to no field and would be dropped on re-serialize, so
/// a body whose re-serialized length differs from `data` is rejected.
fn sat_body_parse(sat_table_id: u8, data: &[u8]) -> Result<SatBody> {
    let body = sat_body_parse_entries(sat_table_id, data)?;
    let used = sat_body_serialized_len(&body);
    if used != data.len() {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: data.len().saturating_sub(used),
                have: 0,
                what: "SatSection trailing body bytes after the last whole entry",
            },
        ));
    }
    Ok(body)
}

fn sat_body_parse_entries(sat_table_id: u8, data: &[u8]) -> Result<SatBody> {
    if data.is_empty() && sat_table_id <= MAX_TYPED_SAT_TABLE_ID {
        return Ok(match sat_table_id {
            0 => SatBody::PositionV2(PositionV2Body {
                satellites: Vec::new(),
            }),
            1 => SatBody::CellFragment(CellFragmentBody {
                fragments: Vec::new(),
            }),
            3 => SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody { plans: Vec::new() }),
            _ => {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: "SatSection body (non-loop type requires data)",
                    },
                ));
            }
        });
    }
    let mut r = BitReader::new(data);
    match sat_table_id {
        0 => sat_parse_position_v2(&mut r),
        1 => sat_parse_cell_fragment(&mut r),
        2 => sat_parse_time_association(&mut r),
        3 => sat_parse_beamhopping_time_plan(&mut r, data),
        4 => sat_parse_position_v3(&mut r),
        _ => Ok(SatBody::Raw(data.to_vec())),
    }
}

fn sat_parse_position_v2(r: &mut BitReader<'_>) -> Result<SatBody> {
    let mut satellites = Vec::new();
    while r.remaining_bits() > SATELLITE_ID_BITS as usize + TRAILING_RESERVED_BITS as usize {
        let satellite_id = r.read_u(SATELLITE_ID_BITS)? as u32;
        let sat_reserved = r.read_u(TRAILING_RESERVED_BITS)? as u8;
        let position_system = r.read_u(1)?;
        let position = if position_system == 0 {
            if r.remaining_bits() < ORBITAL_BITS {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: (ORBITAL_BITS).div_ceil(u8::BITS as usize),
                        have: r.remaining_bits() / u8::BITS as usize,
                        what: "SatSection PositionV2 Orbital fields",
                    },
                ));
            }
            let orbital_position = r.read_u(ORBITAL_POSITION_BITS)? as u16;
            let west_east_flag = r.read_u(1)? != 0;
            let orbital_reserved = r.read_u(TRAILING_RESERVED_BITS)? as u8;
            PositionSystem::Orbital {
                orbital_position,
                west_east_flag,
                reserved_zero_future_use: ReservedZero::from_bits(u64::from(orbital_reserved)),
            }
        } else {
            if r.remaining_bits() < SGP4_BITS {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: (SGP4_BITS).div_ceil(u8::BITS as usize),
                        have: r.remaining_bits() / u8::BITS as usize,
                        what: "SatSection PositionV2 SGP4 fields",
                    },
                ));
            }
            let epoch_year = r.read_u(YEAR_BITS)? as u8;
            let day_of_the_year = r.read_u(DAY_OF_YEAR_BITS)? as u16;
            let day_fraction = r.read_u(WORD_BITS)? as u32;
            let mean_motion_first_derivative = r.read_u(WORD_BITS)? as u32;
            let mean_motion_second_derivative = r.read_u(WORD_BITS)? as u32;
            let drag_term = r.read_u(WORD_BITS)? as u32;
            let inclination = r.read_u(WORD_BITS)? as u32;
            let right_ascension = r.read_u(WORD_BITS)? as u32;
            let eccentricity = r.read_u(WORD_BITS)? as u32;
            let argument_of_perigree = r.read_u(WORD_BITS)? as u32;
            let mean_anomaly = r.read_u(WORD_BITS)? as u32;
            let mean_motion = r.read_u(WORD_BITS)? as u32;
            PositionSystem::Sgp4 {
                epoch_year,
                day_of_the_year,
                day_fraction,
                mean_motion_first_derivative,
                mean_motion_second_derivative,
                drag_term,
                inclination,
                right_ascension,
                eccentricity,
                argument_of_perigree,
                mean_anomaly,
                mean_motion,
            }
        };
        satellites.push(PositionV2Satellite {
            satellite_id,
            reserved_zero_future_use: ReservedZero::from_bits(u64::from(sat_reserved)),
            position,
        });
    }
    Ok(SatBody::PositionV2(PositionV2Body { satellites }))
}

fn sat_parse_cell_fragment(r: &mut BitReader<'_>) -> Result<SatBody> {
    let mut fragments = Vec::new();
    while r.remaining_bits() >= WORD_BITS as usize + TIME_PLAN_MODE_BITS as usize {
        let cell_fragment_id = r.read_u(WORD_BITS)? as u32;
        let first_occurrence = r.read_u(1)? != 0;
        let last_occurrence = r.read_u(1)? != 0;
        let mut frag_r4 = 0u8;
        let centre = if first_occurrence {
            if r.remaining_bits() < CELL_CENTRE_BITS {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: (CELL_CENTRE_BITS).div_ceil(u8::BITS as usize),
                        have: r.remaining_bits() / u8::BITS as usize,
                        what: "SatSection CellFragment centre",
                    },
                ));
            }
            let center_r4 = r.read_u(RESERVED_FIELD_BITS)? as u8;
            let centre_latitude = r.read_i(LATITUDE_BITS)? as i32;
            let center_r5 = r.read_u(CELL_CENTRE_RESERVED_BITS)? as u8;
            let centre_longitude = r.read_i(LONGITUDE_BITS)? as i32;
            let max_distance = r.read_u(MAX_DISTANCE_BITS)? as u32;
            let center_r6 = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
            Some(CellCentre {
                centre_latitude,
                centre_longitude,
                max_distance,
                reserved_zero_future_use_4: ReservedZero::from_bits(u64::from(center_r4)),
                reserved_zero_future_use_5: ReservedZero::from_bits(u64::from(center_r5)),
                reserved_zero_future_use_6: ReservedZero::from_bits(u64::from(center_r6)),
            })
        } else {
            frag_r4 = r.read_u(RESERVED_FIELD_BITS)? as u8;
            None
        };
        let dsid_count = r.read_u(LOOP_COUNT_BITS)? as usize;
        if r.remaining_bits() < dsid_count * usize::from(WORD_BITS) {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: (dsid_count * usize::from(WORD_BITS)).div_ceil(u8::BITS as usize),
                    have: r.remaining_bits() / u8::BITS as usize,
                    what: "SatSection CellFragment delivery_system_ids",
                },
            ));
        }
        let mut delivery_system_ids =
            Vec::with_capacity(dsid_count.min(r.remaining_bits() / usize::from(WORD_BITS)));
        for _ in 0..dsid_count {
            delivery_system_ids.push(r.read_u(WORD_BITS)? as u32);
        }
        let frag_new_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
        let nds_count = r.read_u(LOOP_COUNT_BITS)? as usize;
        if r.remaining_bits() < nds_count * DELIVERY_SYSTEM_ENTRY_BITS {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: (nds_count * DELIVERY_SYSTEM_ENTRY_BITS).div_ceil(u8::BITS as usize),
                    have: r.remaining_bits() / u8::BITS as usize,
                    what: "SatSection CellFragment new_delivery_systems",
                },
            ));
        }
        let mut new_delivery_systems =
            Vec::with_capacity(nds_count.min(r.remaining_bits() / DELIVERY_SYSTEM_ENTRY_BITS));
        for _ in 0..nds_count {
            let new_delivery_system_id = r.read_u(WORD_BITS)? as u32;
            let time_of_application_base = r.read_u(BASE_BITS)?;
            let nds_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
            let time_of_application_ext = r.read_u(EXTENSION_BITS)? as u16;
            new_delivery_systems.push(NewDeliverySystem {
                new_delivery_system_id,
                time_of_application_base,
                time_of_application_ext,
                reserved_zero_future_use: ReservedZero::from_bits(u64::from(nds_r)),
            });
        }
        let frag_obs_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
        let ods_count = r.read_u(LOOP_COUNT_BITS)? as usize;
        if r.remaining_bits() < ods_count * DELIVERY_SYSTEM_ENTRY_BITS {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: (ods_count * DELIVERY_SYSTEM_ENTRY_BITS).div_ceil(u8::BITS as usize),
                    have: r.remaining_bits() / u8::BITS as usize,
                    what: "SatSection CellFragment obsolescent_delivery_systems",
                },
            ));
        }
        let mut obsolescent_delivery_systems =
            Vec::with_capacity(ods_count.min(r.remaining_bits() / DELIVERY_SYSTEM_ENTRY_BITS));
        for _ in 0..ods_count {
            let obsolescent_delivery_system_id = r.read_u(WORD_BITS)? as u32;
            let time_of_obsolescence_base = r.read_u(BASE_BITS)?;
            let ods_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
            let time_of_obsolescence_ext = r.read_u(EXTENSION_BITS)? as u16;
            obsolescent_delivery_systems.push(ObsolescentDeliverySystem {
                obsolescent_delivery_system_id,
                time_of_obsolescence_base,
                time_of_obsolescence_ext,
                reserved_zero_future_use: ReservedZero::from_bits(u64::from(ods_r)),
            });
        }
        fragments.push(CellFragment {
            cell_fragment_id,
            first_occurrence,
            last_occurrence,
            centre,
            delivery_system_ids,
            new_delivery_systems,
            obsolescent_delivery_systems,
            reserved_zero_future_use_4: ReservedZero::from_bits(u64::from(frag_r4)),
            reserved_zero_future_use_new: ReservedZero::from_bits(u64::from(frag_new_r)),
            reserved_zero_future_use_obsolescent: ReservedZero::from_bits(u64::from(frag_obs_r)),
        });
    }
    Ok(SatBody::CellFragment(CellFragmentBody { fragments }))
}

fn sat_parse_time_association(r: &mut BitReader<'_>) -> Result<SatBody> {
    if r.remaining_bits() < TIME_ASSOC_MIN_BITS {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: (TIME_ASSOC_MIN_BITS).div_ceil(u8::BITS as usize),
                have: r.remaining_bits() / u8::BITS as usize,
                what: "SatSection TimeAssociation body",
            },
        ));
    }
    let association_type = AssociationType::from_u8(r.read_u(ASSOCIATION_TYPE_BITS)? as u8);
    let mut assoc_r4 = 0u8;
    let leap_info = if association_type.to_u8() == 1 {
        Some(LeapInfo {
            leap59: r.read_u(1)? != 0,
            leap61: r.read_u(1)? != 0,
            pastleap59: r.read_u(1)? != 0,
            pastleap61: r.read_u(1)? != 0,
        })
    } else {
        assoc_r4 = r.read_u(RESERVED_FIELD_BITS)? as u8;
        None
    };
    let ncr_base = r.read_u(BASE_BITS)?;
    let ncr_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
    let ncr_ext = r.read_u(EXTENSION_BITS)? as u16;
    let association_timestamp_seconds = r.read_u(TIMESTAMP_SECONDS_BITS)?;
    let association_timestamp_nanoseconds = r.read_u(WORD_BITS)? as u32;
    Ok(SatBody::TimeAssociation(TimeAssociationBody {
        association_type,
        leap_info,
        ncr_base,
        ncr_ext,
        association_timestamp_seconds,
        association_timestamp_nanoseconds,
        reserved_zero_future_use_4: ReservedZero::from_bits(u64::from(assoc_r4)),
        ncr_reserved_zero_future_use: ReservedZero::from_bits(u64::from(ncr_r)),
    }))
}

fn sat_parse_beamhopping_time_plan(r: &mut BitReader<'_>, data: &[u8]) -> Result<SatBody> {
    let mut plans = Vec::new();
    while r.remaining_bits()
        >= WORD_BITS as usize + RESERVED_FIELD_BITS as usize + PLAN_LENGTH_BITS as usize
    {
        let beamhopping_time_plan_id = r.read_u(WORD_BITS)? as u32;
        let plan_r4 = r.read_u(RESERVED_FIELD_BITS)? as u8;
        let plan_length = r.read_u(PLAN_LENGTH_BITS)? as usize;
        let plan_end_bits = r.bits_consumed() + plan_length * u8::BITS as usize;
        let plan_mode_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
        let time_plan_mode = TimePlanMode::from_u8(r.read_u(TIME_PLAN_MODE_BITS)? as u8);
        let time_of_application_base = r.read_u(BASE_BITS)?;
        let toa_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
        let time_of_application_ext = r.read_u(EXTENSION_BITS)? as u16;
        let cycle_duration_base = r.read_u(BASE_BITS)?;
        let cycle_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
        let cycle_duration_ext = r.read_u(EXTENSION_BITS)? as u16;
        let mode = match time_plan_mode {
            TimePlanMode::DwellOnTime => {
                if r.remaining_bits() < MODE0_BITS {
                    return Err(Error::BufferTooShort(
                        broadcast_common::error::BufferTooShort {
                            need: (MODE0_BITS).div_ceil(u8::BITS as usize),
                            have: r.remaining_bits() / u8::BITS as usize,
                            what: "SatSection Beamhopping Mode0",
                        },
                    ));
                }
                let dwell_duration_base = r.read_u(BASE_BITS)?;
                let dwell_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
                let dwell_duration_ext = r.read_u(EXTENSION_BITS)? as u16;
                let on_time_base = r.read_u(BASE_BITS)?;
                let on_time_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
                let on_time_ext = r.read_u(EXTENSION_BITS)? as u16;
                BeamhoppingMode::Mode0 {
                    dwell_duration_base,
                    dwell_duration_ext,
                    on_time_base,
                    on_time_ext,
                    dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                        dwell_r,
                    )),
                    on_time_reserved_zero_future_use: ReservedZero::from_bits(u64::from(on_time_r)),
                }
            }
            TimePlanMode::Bitmap => {
                if r.remaining_bits() < MODE1_HEADER_BITS {
                    return Err(Error::BufferTooShort(
                        broadcast_common::error::BufferTooShort {
                            need: (MODE1_HEADER_BITS).div_ceil(u8::BITS as usize),
                            have: r.remaining_bits() / u8::BITS as usize,
                            what: "SatSection Beamhopping Mode1 header",
                        },
                    ));
                }
                let m1_size_r = r.read_u(1)? as u8;
                let bit_map_size = r.read_u(BIT_MAP_BITS)? as u16;
                let m1_slot_r = r.read_u(1)? as u8;
                let current_slot = r.read_u(BIT_MAP_BITS)? as u16;
                if r.remaining_bits() < bit_map_size as usize {
                    return Err(Error::BufferTooShort(
                        broadcast_common::error::BufferTooShort {
                            need: (bit_map_size as usize).div_ceil(u8::BITS as usize),
                            have: r.remaining_bits() / u8::BITS as usize,
                            what: "SatSection Beamhopping Mode1 bitmap",
                        },
                    ));
                }
                let mut slot_transmission_on =
                    Vec::with_capacity((bit_map_size as usize).min(r.remaining_bits()));
                for _ in 0..bit_map_size {
                    slot_transmission_on.push(r.read_u(1)? != 0);
                }
                let total = MODE1_HEADER_BITS + bit_map_size as usize;
                r.skip(pad_to_byte(total) as u8)?;
                BeamhoppingMode::Mode1 {
                    bit_map_size,
                    current_slot,
                    slot_transmission_on,
                    reserved_zero_future_use_size: ReservedZero::from_bits(u64::from(m1_size_r)),
                    reserved_zero_future_use_slot: ReservedZero::from_bits(u64::from(m1_slot_r)),
                }
            }
            TimePlanMode::GridRevisitSleep => {
                if r.remaining_bits() < MODE2_BITS {
                    return Err(Error::BufferTooShort(
                        broadcast_common::error::BufferTooShort {
                            need: (MODE2_BITS).div_ceil(u8::BITS as usize),
                            have: r.remaining_bits() / u8::BITS as usize,
                            what: "SatSection Beamhopping Mode2",
                        },
                    ));
                }
                let grid_size_base = r.read_u(BASE_BITS)?;
                let grid_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
                let grid_size_ext = r.read_u(EXTENSION_BITS)? as u16;
                let revisit_duration_base = r.read_u(BASE_BITS)?;
                let revisit_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
                let revisit_duration_ext = r.read_u(EXTENSION_BITS)? as u16;
                let sleep_time_base = r.read_u(BASE_BITS)?;
                let sleep_time_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
                let sleep_time_ext = r.read_u(EXTENSION_BITS)? as u16;
                let sleep_duration_base = r.read_u(BASE_BITS)?;
                let sleep_dur_r = r.read_u(RESERVED_FUTURE_USE_BITS)? as u8;
                let sleep_duration_ext = r.read_u(EXTENSION_BITS)? as u16;
                BeamhoppingMode::Mode2 {
                    grid_size_base,
                    grid_size_ext,
                    revisit_duration_base,
                    revisit_duration_ext,
                    sleep_time_base,
                    sleep_time_ext,
                    sleep_duration_base,
                    sleep_duration_ext,
                    grid_size_reserved_zero_future_use: ReservedZero::from_bits(u64::from(grid_r)),
                    revisit_duration_reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                        revisit_r,
                    )),
                    sleep_time_reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                        sleep_time_r,
                    )),
                    sleep_duration_reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                        sleep_dur_r,
                    )),
                }
            }
            _ => {
                let start_byte = r.bits_consumed().div_ceil(u8::BITS as usize);
                let end_byte = plan_end_bits / u8::BITS as usize;
                let raw = if start_byte < end_byte && end_byte <= data.len() {
                    data[start_byte..end_byte].to_vec()
                } else {
                    Vec::new()
                };
                BeamhoppingMode::Reserved(raw)
            }
        };
        // A `plan_length` shorter than the fixed + mode-specific
        // fields already consumed would move the cursor BACKWARDS,
        // re-reading part of this plan's own body as the next plan
        // (r02-W14) — reject it instead.
        if r.bits_consumed() > plan_end_bits {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: r.bits_consumed() - plan_end_bits,
                    have: 0,
                    what: "SatSection Beamhopping plan_length shorter than plan fields",
                },
            ));
        }
        r.seek_forward(plan_end_bits);
        plans.push(BeamhoppingPlan {
            beamhopping_time_plan_id,
            time_plan_mode,
            time_of_application_base,
            time_of_application_ext,
            cycle_duration_base,
            cycle_duration_ext,
            mode,
            reserved_zero_future_use_4: ReservedZero::from_bits(u64::from(plan_r4)),
            reserved_zero_future_use_mode: ReservedZero::from_bits(u64::from(plan_mode_r)),
            time_of_application_reserved_zero_future_use: ReservedZero::from_bits(u64::from(toa_r)),
            cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(u64::from(cycle_r)),
        });
    }
    Ok(SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
        plans,
    }))
}

fn sat_parse_position_v3(r: &mut BitReader<'_>) -> Result<SatBody> {
    if r.remaining_bits() < POS_V3_HEADER_BITS {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: (POS_V3_HEADER_BITS).div_ceil(u8::BITS as usize),
                have: r.remaining_bits() / u8::BITS as usize,
                what: "SatSection PositionV3 body header",
            },
        ));
    }
    let oem_version_major = r.read_u(OEM_VERSION_BITS)? as u8;
    let oem_version_minor = r.read_u(OEM_VERSION_BITS)? as u8;
    let creation_date_year = r.read_u(YEAR_BITS)? as u8;
    let creation_r = r.read_u(TRAILING_RESERVED_BITS)? as u8;
    let creation_date_day = r.read_u(DAY_BITS)? as u16;
    let creation_date_day_fraction = r.read_u(WORD_BITS)? as u32;
    let mut satellites = Vec::new();
    while r.remaining_bits()
        >= SATELLITE_ID_BITS as usize
            + POSITION_V3_RESERVED_BITS as usize
            + POSITION_V3_FLAGS_BITS as usize
    {
        let satellite_id = r.read_u(SATELLITE_ID_BITS)? as u32;
        let v3sat_r = r.read_u(POSITION_V3_RESERVED_BITS)? as u8;
        let metadata_flag = r.read_u(1)? != 0;
        let usable_start_time_flag = r.read_u(1)? != 0;
        let usable_stop_time_flag = r.read_u(1)? != 0;
        let ephemeris_accel_flag = r.read_u(1)? != 0;
        let covariance_flag = r.read_u(1)? != 0;
        let metadata = if metadata_flag {
            if r.remaining_bits() < METADATA_FIXED_BITS {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: (METADATA_FIXED_BITS).div_ceil(u8::BITS as usize),
                        have: r.remaining_bits() / u8::BITS as usize,
                        what: "SatSection PositionV3 metadata",
                    },
                ));
            }
            let total_start_time_year = r.read_u(YEAR_BITS)? as u8;
            let start_r = r.read_u(TRAILING_RESERVED_BITS)? as u8;
            let total_start_time_day = r.read_u(DAY_BITS)? as u16;
            let total_start_time_day_fraction = r.read_u(WORD_BITS)? as u32;
            let total_stop_time_year = r.read_u(YEAR_BITS)? as u8;
            let stop_r = r.read_u(TRAILING_RESERVED_BITS)? as u8;
            let total_stop_time_day = r.read_u(DAY_BITS)? as u16;
            let total_stop_time_day_fraction = r.read_u(WORD_BITS)? as u32;
            let interp_r = r.read_u(1)? as u8;
            let interpolation_flag = r.read_u(1)? != 0;
            let interpolation_type =
                InterpolationType::from_u8(r.read_u(INTERPOLATION_BITS)? as u8);
            let interpolation_degree = r.read_u(INTERPOLATION_BITS)? as u8;
            let usable_start_time = if usable_start_time_flag {
                if r.remaining_bits() < USABLE_TIME_BITS {
                    return Err(Error::BufferTooShort(
                        broadcast_common::error::BufferTooShort {
                            need: (USABLE_TIME_BITS).div_ceil(u8::BITS as usize),
                            have: r.remaining_bits() / u8::BITS as usize,
                            what: "SatSection PositionV3 usable_start_time",
                        },
                    ));
                }
                let year = r.read_u(YEAR_BITS)? as u8;
                let ustart_r = r.read_u(TRAILING_RESERVED_BITS)? as u8;
                let day = r.read_u(DAY_BITS)? as u16;
                let day_fraction = r.read_u(WORD_BITS)? as u32;
                Some(UsableTime {
                    year,
                    day,
                    day_fraction,
                    reserved_zero_future_use: ReservedZero::from_bits(u64::from(ustart_r)),
                })
            } else {
                None
            };
            let usable_stop_time = if usable_stop_time_flag {
                if r.remaining_bits() < USABLE_TIME_BITS {
                    return Err(Error::BufferTooShort(
                        broadcast_common::error::BufferTooShort {
                            need: (USABLE_TIME_BITS).div_ceil(u8::BITS as usize),
                            have: r.remaining_bits() / u8::BITS as usize,
                            what: "SatSection PositionV3 usable_stop_time",
                        },
                    ));
                }
                let year = r.read_u(YEAR_BITS)? as u8;
                let ustop_r = r.read_u(TRAILING_RESERVED_BITS)? as u8;
                let day = r.read_u(DAY_BITS)? as u16;
                let day_fraction = r.read_u(WORD_BITS)? as u32;
                Some(UsableTime {
                    year,
                    day,
                    day_fraction,
                    reserved_zero_future_use: ReservedZero::from_bits(u64::from(ustop_r)),
                })
            } else {
                None
            };
            Some(PositionV3Metadata {
                total_start_time_year,
                total_start_time_day,
                total_start_time_day_fraction,
                total_stop_time_year,
                total_stop_time_day,
                total_stop_time_day_fraction,
                interpolation_flag,
                interpolation_type,
                interpolation_degree,
                usable_start_time,
                usable_stop_time,
                total_start_time_reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                    start_r,
                )),
                total_stop_time_reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                    stop_r,
                )),
                reserved_zero_future_use: ReservedZero::from_bits(u64::from(interp_r)),
            })
        } else {
            None
        };
        let ephemeris_data_count = r.read_u(EPHEMERIS_COUNT_BITS)? as u16;
        let entry_bits: usize = USABLE_TIME_BITS
            + WORD_BITS as usize * EPHEMERIS_VECTOR_FIELDS
            + if ephemeris_accel_flag {
                WORD_BITS as usize * EPHEMERIS_ACCEL_FIELDS
            } else {
                0
            };
        let mut ephemeris_data = Vec::with_capacity(
            (ephemeris_data_count as usize)
                .min(r.remaining_bits().saturating_sub(entry_bits) / entry_bits + 1),
        );
        for _ in 0..ephemeris_data_count {
            if r.remaining_bits() < entry_bits {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: (entry_bits).div_ceil(u8::BITS as usize),
                        have: r.remaining_bits() / u8::BITS as usize,
                        what: "SatSection PositionV3 ephemeris_data entry",
                    },
                ));
            }
            let epoch_year = r.read_u(YEAR_BITS)? as u8;
            let eph_r = r.read_u(TRAILING_RESERVED_BITS)? as u8;
            let epoch_day = r.read_u(DAY_BITS)? as u16;
            let epoch_day_fraction = r.read_u(WORD_BITS)? as u32;
            let ephemeris_x = r.read_u(WORD_BITS)? as u32;
            let ephemeris_y = r.read_u(WORD_BITS)? as u32;
            let ephemeris_z = r.read_u(WORD_BITS)? as u32;
            let ephemeris_x_dot = r.read_u(WORD_BITS)? as u32;
            let ephemeris_y_dot = r.read_u(WORD_BITS)? as u32;
            let ephemeris_z_dot = r.read_u(WORD_BITS)? as u32;
            let acceleration = if ephemeris_accel_flag {
                Some(EphemerisAccel {
                    ephemeris_x_ddot: r.read_u(WORD_BITS)? as u32,
                    ephemeris_y_ddot: r.read_u(WORD_BITS)? as u32,
                    ephemeris_z_ddot: r.read_u(WORD_BITS)? as u32,
                })
            } else {
                None
            };
            ephemeris_data.push(EphemerisData {
                epoch_year,
                epoch_day,
                epoch_day_fraction,
                ephemeris_x,
                ephemeris_y,
                ephemeris_z,
                ephemeris_x_dot,
                ephemeris_y_dot,
                ephemeris_z_dot,
                acceleration,
                reserved_zero_future_use: ReservedZero::from_bits(u64::from(eph_r)),
            });
        }
        let covariance = if covariance_flag {
            if r.remaining_bits() < COV_BITS {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: (COV_BITS).div_ceil(u8::BITS as usize),
                        have: r.remaining_bits() / u8::BITS as usize,
                        what: "SatSection PositionV3 covariance",
                    },
                ));
            }
            let covariance_epoch_year = r.read_u(YEAR_BITS)? as u8;
            let cov_r = r.read_u(TRAILING_RESERVED_BITS)? as u8;
            let covariance_epoch_day = r.read_u(DAY_BITS)? as u16;
            let covariance_epoch_day_fraction = r.read_u(WORD_BITS)? as u32;
            let mut covariance_elements = [0u32; COVARIANCE_ELEMENTS];
            for elem in &mut covariance_elements {
                *elem = r.read_u(WORD_BITS)? as u32;
            }
            Some(CovarianceData {
                covariance_epoch_year,
                covariance_epoch_day,
                covariance_epoch_day_fraction,
                covariance_elements,
                covariance_epoch_reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                    cov_r,
                )),
            })
        } else {
            None
        };
        satellites.push(PositionV3Satellite {
            satellite_id,
            usable_start_time_flag,
            usable_stop_time_flag,
            ephemeris_accel_flag,
            covariance_flag,
            metadata,
            ephemeris_data,
            covariance,
            reserved_zero_future_use: ReservedZero::from_bits(u64::from(v3sat_r)),
        });
    }
    Ok(SatBody::PositionV3(PositionV3Body {
        oem_version_major,
        oem_version_minor,
        creation_date_year,
        creation_date_day,
        creation_date_day_fraction,
        satellites,
        creation_date_reserved_zero_future_use: ReservedZero::from_bits(u64::from(creation_r)),
    }))
}

// ── SatSection ──────────────────────────────────────────────────────────────

/// Satellite Access Table section (EN 300 468 §5.2.11.1, Table 11a).
///
/// The body is typed as [`SatBody`], selected by `satellite_table_id`.
/// All body fields are owned numeric values; the section does not borrow
/// from the input buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SatSection {
    /// 6-bit discriminant selecting the body structure (see [`SatTableId`]).
    pub satellite_table_id: u8,
    /// `section_syntax_indicator` (byte 1 bit 7): `1` for a constructed value,
    /// whatever the wire carried for a parsed one (parse tolerant, serialize
    /// preserves).
    pub section_syntax_indicator: bool,
    /// `private_indicator` — byte 1 bit 6 (Table 11a).
    pub private_indicator: bool,
    /// 10-bit sub_table discriminator.
    pub table_count: u16,
    /// 5-bit sub_table version number.
    pub version_number: u8,
    /// When `true`, this sub_table is currently applicable.
    pub current_next_indicator: bool,
    /// Section number within the sub_table.
    pub section_number: u8,
    /// Highest section number of the sub_table.
    pub last_section_number: u8,
    /// Typed body — interpret per `satellite_table_id`.
    pub body: SatBody,
    /// `reserved(2)` in byte 1 bits `[5:4]` (`11`); preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_b1: ReservedOnes<{ HEADER_RESERVED_BITS as u32 }>,
    /// `reserved(2)` in front of `version_number` (`11`); preserved.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_b5: ReservedOnes<{ HEADER_RESERVED_BITS as u32 }>,
    /// `reserved_zero_future_use(8)` at byte 8 (`0x00`, EN 300 468 Table 11a);
    /// preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_zero_future_use: ReservedZero<{ HEADER_RESERVED_BYTE_BITS as u32 }>,
}

impl SatSection {
    /// Construct a SAT section with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        satellite_table_id: u8,
        private_indicator: bool,
        table_count: u16,
        section: crate::tables::SectionVersion,
        body: SatBody,
    ) -> Self {
        Self {
            satellite_table_id,
            section_syntax_indicator: true,
            private_indicator,
            table_count,
            version_number: section.version_number,
            current_next_indicator: section.current_next_indicator,
            section_number: section.section_number,
            last_section_number: section.last_section_number,
            body,
            reserved_b1: ReservedOnes::FILL,
            reserved_b5: ReservedOnes::FILL,
            reserved_zero_future_use: ReservedZero::FILL,
        }
    }

    /// Typed view of `satellite_table_id`, or `None` if reserved (5–63).
    #[must_use]
    pub fn kind(&self) -> Option<SatTableId> {
        SatTableId::try_from(self.satellite_table_id).ok()
    }
}

impl<'a> Parse<'a> for SatSection {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + CRC_LEN;
        need_in(bytes, min_len, "SatSection")?;
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "SatSection",
                expected: &[TABLE_ID],
            });
        }
        let section_length = super::section_length_of(bytes);
        let total = super::check_section_length(
            bytes.len(),
            SECTION_LENGTH_PREFIX,
            section_length,
            HEADER_LEN + CRC_LEN,
        )?;
        let satellite_table_id = bytes[SATELLITE_TABLE_ID_OFFSET] >> SATELLITE_TABLE_ID_SHIFT;
        let private_indicator = (bytes[1] & super::SECTION_B1_PRIVATE_INDICATOR) != 0;
        let table_count = (((bytes[SATELLITE_TABLE_ID_OFFSET] & TABLE_COUNT_HI_MASK) as u16)
            << YEAR_BITS)
            | bytes[TABLE_COUNT_LO_OFFSET] as u16;
        let version_number = super::version_number_of(bytes[super::VERSION_FLAGS_OFFSET]);
        let current_next_indicator = super::current_next_of(bytes[super::VERSION_FLAGS_OFFSET]);
        let section_number = bytes[super::SECTION_NUMBER_OFFSET];
        let last_section_number = bytes[super::LAST_SECTION_NUMBER_OFFSET];
        let body_data = &bytes[HEADER_LEN..total - CRC_LEN];
        let body = sat_body_parse(satellite_table_id, body_data)?;
        Ok(SatSection {
            satellite_table_id,
            section_syntax_indicator: bytes[1] & super::SECTION_B1_SSI != 0,
            private_indicator,
            table_count,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            body,
            reserved_b1: ReservedOnes::from_bits(u64::from(super::b1_reserved_of(bytes[1]))),
            reserved_b5: ReservedOnes::from_bits(u64::from(super::b5_reserved_of(
                bytes[super::VERSION_FLAGS_OFFSET],
            ))),
            reserved_zero_future_use: ReservedZero::from_bits(u64::from(
                bytes[RESERVED_TRAILER_OFFSET],
            )),
        })
    }
}

impl Serialize for SatSection {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + sat_body_serialized_len(&self.body) + CRC_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let section_length_usize = len - SECTION_LENGTH_PREFIX;
        if section_length_usize > MAX_SECTION_LENGTH {
            return Err(Error::SectionLengthOverflow {
                declared: section_length_usize,
                available: MAX_SECTION_LENGTH,
            });
        }
        let section_length = section_length_usize as u16;
        if let SatBody::PositionV3(ref v3) = self.body {
            for sat in &v3.satellites {
                if sat.ephemeris_data.len() > u16::MAX as usize {
                    return Err(Error::SectionLengthOverflow {
                        declared: sat.ephemeris_data.len(),
                        available: u16::MAX as usize,
                    });
                }
            }
        }
        buf[0] = TABLE_ID;
        buf[1] = super::b1_of(
            self.section_syntax_indicator,
            self.private_indicator,
            self.reserved_b1.get() as u8,
        )? | ((section_length >> YEAR_BITS) as u8 & super::SECTION_LENGTH_HI_MASK);
        buf[SECTION_LENGTH_LO_OFFSET] = (section_length & u16::from(u8::MAX)) as u8;
        broadcast_common::len::fit_mask(
            self.table_count,
            (u16::from(TABLE_COUNT_HI_MASK) << YEAR_BITS) | u16::from(u8::MAX),
            "table_count",
        )?;
        buf[SATELLITE_TABLE_ID_OFFSET] = (broadcast_common::len::fit_mask(
            self.satellite_table_id,
            SATELLITE_TABLE_ID_MASK,
            "satellite_table_id",
        )? << SATELLITE_TABLE_ID_SHIFT)
            | ((self.table_count >> YEAR_BITS) as u8 & TABLE_COUNT_HI_MASK);
        buf[TABLE_COUNT_LO_OFFSET] = (self.table_count & u16::from(u8::MAX)) as u8;
        buf[super::VERSION_FLAGS_OFFSET] = super::b5_of(
            self.reserved_b5.get() as u8,
            self.version_number,
            self.current_next_indicator,
        )?;
        buf[super::SECTION_NUMBER_OFFSET] = self.section_number;
        buf[super::LAST_SECTION_NUMBER_OFFSET] = self.last_section_number;
        buf[RESERVED_TRAILER_OFFSET] = self.reserved_zero_future_use.get() as u8;
        let body_start = HEADER_LEN;
        match &self.body {
            SatBody::Raw(v) => {
                buf[body_start..body_start + v.len()].copy_from_slice(v);
            }
            _ => {
                let body_byte_len = sat_body_serialized_len(&self.body);
                for b in &mut buf[body_start..body_start + body_byte_len] {
                    *b = 0;
                }
                let mut writer = BitWriter::new(&mut buf[body_start..body_start + body_byte_len]);
                sat_body_write(&self.body, &mut writer)?;
            }
        }
        let body_end = HEADER_LEN + sat_body_serialized_len(&self.body);
        super::write_crc(buf, body_end);
        Ok(len)
    }
}
impl crate::traits::TableDef<'_> for SatSection {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "SATELLITE_ACCESS";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_sat(stid: u8, table_count: u16, body: &SatBody) -> Vec<u8> {
        let sat = SatSection::new(
            stid,
            true,
            table_count,
            crate::tables::SectionVersion::new(5, true, 0, 0),
            body.clone(),
        );
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        buf
    }

    fn build_sat_private_indicator_false(stid: u8, body: &SatBody) -> Vec<u8> {
        let sat = SatSection::new(
            stid,
            false,
            0,
            crate::tables::SectionVersion::new(5, true, 0, 0),
            body.clone(),
        );
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        buf
    }

    #[test]
    fn parse_raw_body() {
        let body_data = [0xAA, 0xBB, 0xCC, 0xDD];
        let bytes = build_sat(7, 0, &SatBody::Raw(body_data.to_vec()));
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 7);
        assert_eq!(sat.kind(), None);
        assert_eq!(sat.body, SatBody::Raw(body_data.to_vec()));
    }

    #[test]
    fn section_length_high_nibble_is_written_for_a_300_byte_body() {
        let body_data = vec![0xAAu8; 300];
        let bytes = build_sat(7, 0, &SatBody::Raw(body_data.clone()));

        assert_eq!(bytes[1], 0xF1);
        assert_eq!(bytes[2], 0x36);

        let sat = SatSection::parse(&bytes).expect("reparse");
        assert_eq!(sat.body, SatBody::Raw(body_data));
    }

    #[test]
    fn private_indicator_false_round_trip() {
        let body = SatBody::TimeAssociation(TimeAssociationBody {
            association_type: AssociationType::UtcWithoutLeap,
            leap_info: None,
            ncr_base: 0,
            ncr_ext: 0,
            association_timestamp_seconds: 0,
            association_timestamp_nanoseconds: 0,
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            ncr_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat_private_indicator_false(2, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        assert!(!sat.private_indicator);
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(
            bytes, buf2,
            "byte-exact round-trip with private_indicator=false"
        );
    }

    #[test]
    fn parse_position_v3_discriminant() {
        let body = SatBody::PositionV3(PositionV3Body {
            oem_version_major: 1,
            oem_version_minor: 0,
            creation_date_year: 25,
            creation_date_day: 100,
            creation_date_day_fraction: 0,
            satellites: Vec::new(),
            creation_date_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat(4, 0x1A3, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 4);
        assert_eq!(sat.kind(), Some(SatTableId::PositionV3));
        assert_eq!(sat.table_count, 0x1A3);
    }

    #[test]
    fn time_association_round_trip() {
        let body = SatBody::TimeAssociation(TimeAssociationBody {
            association_type: AssociationType::UtcWithLeap,
            leap_info: Some(LeapInfo {
                leap59: true,
                leap61: false,
                pastleap59: false,
                pastleap61: true,
            }),
            ncr_base: 0x0000_AAAA_AAAA_u64,
            ncr_ext: 0x1AA,
            association_timestamp_seconds: 0x12345678_9ABCDEF0,
            association_timestamp_nanoseconds: 0xDEADBEEF,
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            ncr_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat(2, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::TimeAssociation(ta) => {
                assert_eq!(ta.association_type, AssociationType::UtcWithLeap);
                let li = ta.leap_info.as_ref().unwrap();
                assert!(li.leap59);
                assert!(!li.leap61);
                assert!(!li.pastleap59);
                assert!(li.pastleap61);
                assert_eq!(ta.ncr_base, 0x0000_AAAA_AAAA);
                assert_eq!(ta.ncr_ext, 0x1AA);
                assert_eq!(ta.association_timestamp_seconds, 0x12345678_9ABCDEF0);
                assert_eq!(ta.association_timestamp_nanoseconds, 0xDEADBEEF);
            }
            other => panic!("expected TimeAssociation, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact re-serialize");
    }

    #[test]
    fn position_v2_orbital_round_trip() {
        let body = SatBody::PositionV2(PositionV2Body {
            satellites: vec![PositionV2Satellite {
                satellite_id: 0x123456,
                position: PositionSystem::Orbital {
                    orbital_position: 0x1234,
                    west_east_flag: true,
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(0, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::PositionV2(pv2) => {
                assert_eq!(pv2.satellites.len(), 1);
                assert_eq!(pv2.satellites[0].satellite_id, 0x123456);
                match &pv2.satellites[0].position {
                    PositionSystem::Orbital {
                        orbital_position,
                        west_east_flag,
                        ..
                    } => {
                        assert_eq!(*orbital_position, 0x1234);
                        assert!(*west_east_flag);
                    }
                    other => panic!("expected Orbital, got {other:?}"),
                }
            }
            other => panic!("expected PositionV2, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact re-serialize");
    }

    #[test]
    fn beamhopping_mode0_round_trip() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![BeamhoppingPlan {
                beamhopping_time_plan_id: 0xDEADBEEF,
                time_plan_mode: TimePlanMode::DwellOnTime,
                time_of_application_base: 0x0000_AAAA_AAAA,
                time_of_application_ext: 0x100,
                cycle_duration_base: 0x0000_5555_5555,
                cycle_duration_ext: 0x080,
                mode: BeamhoppingMode::Mode0 {
                    dwell_duration_base: 0x0000_1111_1111,
                    dwell_duration_ext: 0x111,
                    on_time_base: 0x0000_2222_2222,
                    on_time_ext: 0x022,
                    dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                    on_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 1);
                assert_eq!(bhp.plans[0].beamhopping_time_plan_id, 0xDEADBEEF);
                assert_eq!(bhp.plans[0].time_plan_mode, TimePlanMode::DwellOnTime);
                match &bhp.plans[0].mode {
                    BeamhoppingMode::Mode0 {
                        dwell_duration_base,
                        ..
                    } => {
                        assert_eq!(*dwell_duration_base, 0x0000_1111_1111);
                    }
                    other => panic!("expected Mode0, got {other:?}"),
                }
            }
            other => panic!("expected BeamhoppingTimePlan, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact re-serialize");
    }

    /// Regression for r02-W14: a `plan_length` shorter than the fixed +
    /// mode-specific fields the plan actually carries must be rejected, not
    /// silently accepted with the cursor moved backwards into the plan's
    /// own body (which would then be misread as a second, fabricated plan).
    #[test]
    fn beamhopping_plan_length_shorter_than_fields_is_rejected() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![BeamhoppingPlan {
                beamhopping_time_plan_id: 0xDEAD_BEEF,
                time_plan_mode: TimePlanMode::DwellOnTime,
                time_of_application_base: 0x0000_AAAA_AAAA,
                time_of_application_ext: 0x100,
                cycle_duration_base: 0x0000_5555_5555,
                cycle_duration_ext: 0x080,
                mode: BeamhoppingMode::Mode0 {
                    dwell_duration_base: 0x0000_1111_1111,
                    dwell_duration_ext: 0x111,
                    on_time_base: 0x0000_2222_2222,
                    on_time_ext: 0x022,
                    dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                    on_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
        });
        let mut bytes = build_sat(3, 0, &body);

        // Locate `beamhopping_time_plan_id` (0xDEADBEEF) on the wire; the
        // next 16 bits are `reserved(4)` + `plan_length(12)`.
        let marker = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let id_pos = bytes
            .windows(4)
            .position(|w| w == marker)
            .expect("beamhopping_time_plan_id marker found");
        let plan_length_pos = id_pos + 4;
        // Zero the plan_length (keep the top reserved nibble as-is): far
        // shorter than the fixed+Mode0 fields that actually follow.
        bytes[plan_length_pos] &= 0xF0;
        bytes[plan_length_pos + 1] = 0x00;

        let err = SatSection::parse(&bytes).unwrap_err();
        assert!(
            matches!(err, Error::BufferTooShort(broadcast_common::error::BufferTooShort { what, .. }) if what.contains("plan_length")),
            "expected a plan_length error, got {err:?}"
        );
    }

    #[test]
    fn reserved_discriminant_has_no_kind() {
        let bytes = build_sat(7, 0, &SatBody::Raw(Vec::new()));
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 7);
        assert_eq!(sat.kind(), None);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_sat(0, 0, &SatBody::Raw(vec![1, 2, 3]));
        bytes[0] = 0x40;
        assert!(matches!(
            SatSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x40, .. }
        ));
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(matches!(
            SatSection::parse(&[0x4D, 0xF0]).unwrap_err(),
            Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                what: "SatSection",
                ..
            })
        ));
    }

    #[test]
    fn serialize_round_trip_raw() {
        let body_data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let sat = SatSection::new(
            10,
            true,
            0x2FF,
            crate::tables::SectionVersion::new(5, true, 0, 0),
            SatBody::Raw(body_data.clone()),
        );
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        let re = SatSection::parse(&buf).unwrap();
        assert_eq!(re.body, SatBody::Raw(body_data));
        assert_eq!(re.table_count, 0x2FF);
    }

    #[test]
    fn parse_handwritten_sat_raw() {
        let mut bytes: Vec<u8> = vec![
            0x4D, 0xF0, 0x0E, 0x1C, 0x00, 0xCB, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD,
        ];
        let crc = broadcast_common::crc32_mpeg2::compute(&bytes);
        bytes.extend_from_slice(&crc.to_be_bytes());
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 7);
        assert_eq!(sat.table_count, 0);
        assert_eq!(sat.version_number, 5);
        assert!(sat.current_next_indicator);
        match sat.body {
            SatBody::Raw(v) => assert_eq!(v, &[0xAA, 0xBB, 0xCC, 0xDD]),
            other => panic!("expected Raw, got {other:?}"),
        }
    }

    #[test]
    fn beamhopping_multi_plan_round_trip() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![
                BeamhoppingPlan {
                    beamhopping_time_plan_id: 0x11111111,
                    time_plan_mode: TimePlanMode::DwellOnTime,
                    time_of_application_base: 0x0000_AAAA_AAAA,
                    time_of_application_ext: 0x100,
                    cycle_duration_base: 0x0000_5555_5555,
                    cycle_duration_ext: 0x080,
                    mode: BeamhoppingMode::Mode0 {
                        dwell_duration_base: 0x0000_1111_1111,
                        dwell_duration_ext: 0x111,
                        on_time_base: 0x0000_2222_2222,
                        on_time_ext: 0x022,
                        dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                        on_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                    },
                    reserved_zero_future_use_4: ReservedZero::from_bits(0),
                    reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                    time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                    cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                BeamhoppingPlan {
                    beamhopping_time_plan_id: 0x22222222,
                    time_plan_mode: TimePlanMode::DwellOnTime,
                    time_of_application_base: 0x0000_BBBB_BBBB,
                    time_of_application_ext: 0x000,
                    cycle_duration_base: 0x0000_6666_6666,
                    cycle_duration_ext: 0x090,
                    mode: BeamhoppingMode::Mode0 {
                        dwell_duration_base: 0x0000_3333_3333,
                        dwell_duration_ext: 0x133,
                        on_time_base: 0x0000_4444_4444,
                        on_time_ext: 0x044,
                        dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                        on_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                    },
                    reserved_zero_future_use_4: ReservedZero::from_bits(0),
                    reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                    time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                    cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
            ],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 2);
                assert_eq!(bhp.plans[0].beamhopping_time_plan_id, 0x11111111);
                assert_eq!(bhp.plans[1].beamhopping_time_plan_id, 0x22222222);
            }
            other => panic!("expected BeamhoppingTimePlan, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact multi-plan round-trip");
    }

    /// EN 300 468 v1.20.1 Table 11h semantics: when `metadata_flag == 0` the
    /// usable_*_time flags are "not used", and when `interpolation_flag == 0`
    /// the `interpolation_type`/`interpolation_degree` are "undefined and not
    /// used" (v1.19.1: "reserved"). Whatever a sender puts there must survive
    /// parse -> serialize byte-for-byte.
    #[test]
    fn position_v3_not_used_fields_round_trip_verbatim() {
        let sat_with = |flags: bool, md: Option<PositionV3Metadata>| PositionV3Satellite {
            satellite_id: 0x00_1234,
            usable_start_time_flag: flags,
            usable_stop_time_flag: flags,
            ephemeris_accel_flag: false,
            covariance_flag: false,
            metadata: md,
            ephemeris_data: Vec::new(),
            covariance: None,
            reserved_zero_future_use: ReservedZero::from_bits(0),
        };
        let md = PositionV3Metadata {
            total_start_time_year: 26,
            total_start_time_day: 1,
            total_start_time_day_fraction: 0,
            total_stop_time_year: 27,
            total_stop_time_day: 2,
            total_stop_time_day_fraction: 0,
            interpolation_flag: false,
            interpolation_type: InterpolationType::ReservedOther(6),
            interpolation_degree: 5,
            usable_start_time: None,
            usable_stop_time: None,
            total_start_time_reserved_zero_future_use: ReservedZero::from_bits(0),
            total_stop_time_reserved_zero_future_use: ReservedZero::from_bits(0),
            reserved_zero_future_use: ReservedZero::from_bits(0),
        };
        let body = SatBody::PositionV3(PositionV3Body {
            oem_version_major: 1,
            oem_version_minor: 0,
            creation_date_year: 26,
            creation_date_day: 1,
            creation_date_day_fraction: 0,
            // Satellite 1: metadata_flag == 0 but both usable flags set.
            // Satellite 2: interpolation_flag == 0 with a non-zero type/degree.
            satellites: vec![sat_with(true, None), sat_with(false, Some(md))],
            creation_date_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat(4, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        let SatBody::PositionV3(v3) = &sat.body else {
            panic!("expected PositionV3");
        };
        assert!(v3.satellites[0].usable_start_time_flag && v3.satellites[0].usable_stop_time_flag);
        assert!(v3.satellites[0].metadata.is_none());
        let md = v3.satellites[1].metadata.as_ref().unwrap();
        assert!(!md.interpolation_flag);
        assert_eq!(md.interpolation_type, InterpolationType::ReservedOther(6));
        assert_eq!(md.interpolation_degree, 5);
        let mut out = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn position_v3_one_sat_with_metadata_round_trip() {
        let body = SatBody::PositionV3(PositionV3Body {
            oem_version_major: 2,
            oem_version_minor: 1,
            creation_date_year: 26,
            creation_date_day: 42,
            creation_date_day_fraction: 0,
            satellites: vec![PositionV3Satellite {
                satellite_id: 0xABCDEF,
                usable_start_time_flag: true,
                usable_stop_time_flag: false,
                ephemeris_accel_flag: false,
                covariance_flag: false,
                metadata: Some(PositionV3Metadata {
                    total_start_time_year: 26,
                    total_start_time_day: 1,
                    total_start_time_day_fraction: 0,
                    total_stop_time_year: 27,
                    total_stop_time_day: 100,
                    total_stop_time_day_fraction: 0,
                    interpolation_flag: true,
                    interpolation_type: InterpolationType::Linear,
                    interpolation_degree: 2,
                    usable_start_time: Some(UsableTime {
                        year: 26,
                        day: 10,
                        day_fraction: 0,
                        reserved_zero_future_use: ReservedZero::from_bits(0),
                    }),
                    usable_stop_time: None,
                    total_start_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                    total_stop_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                }),
                ephemeris_data: Vec::new(),
                covariance: None,
                reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
            creation_date_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat(4, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::PositionV3(v3) => {
                assert_eq!(v3.satellites.len(), 1);
                assert_eq!(v3.satellites[0].satellite_id, 0xABCDEF);
                let md = v3.satellites[0].metadata.as_ref().unwrap();
                assert!(md.interpolation_flag);
                assert_eq!(md.interpolation_type, InterpolationType::Linear);
                assert_eq!(md.interpolation_degree, 2);
                assert!(md.usable_start_time.is_some());
            }
            other => panic!("expected PositionV3, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact PositionV3 round-trip");
    }

    #[test]
    fn cell_fragment_round_trip() {
        let body = SatBody::CellFragment(CellFragmentBody {
            fragments: vec![CellFragment {
                cell_fragment_id: 0x11223344,
                first_occurrence: true,
                last_occurrence: false,
                centre: Some(CellCentre {
                    centre_latitude: 1000,
                    centre_longitude: -2000,
                    max_distance: 500000,
                    reserved_zero_future_use_4: ReservedZero::from_bits(0),
                    reserved_zero_future_use_5: ReservedZero::from_bits(0),
                    reserved_zero_future_use_6: ReservedZero::from_bits(0),
                }),
                delivery_system_ids: vec![0x55667788],
                new_delivery_systems: vec![NewDeliverySystem {
                    new_delivery_system_id: 0xAABBCCDD,
                    time_of_application_base: 0x0000_1234_5678,
                    time_of_application_ext: 0x100,
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                }],
                obsolescent_delivery_systems: vec![ObsolescentDeliverySystem {
                    obsolescent_delivery_system_id: 0xEEFF0011,
                    time_of_obsolescence_base: 0x0000_9ABC_DEF0,
                    time_of_obsolescence_ext: 0x1FF,
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                }],
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_new: ReservedZero::from_bits(0),
                reserved_zero_future_use_obsolescent: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(1, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::CellFragment(cf) => {
                assert_eq!(cf.fragments.len(), 1);
                assert_eq!(cf.fragments[0].cell_fragment_id, 0x11223344);
                assert!(cf.fragments[0].first_occurrence);
                assert!(cf.fragments[0].centre.is_some());
                assert_eq!(cf.fragments[0].delivery_system_ids.len(), 1);
                assert_eq!(cf.fragments[0].new_delivery_systems.len(), 1);
                assert_eq!(cf.fragments[0].obsolescent_delivery_systems.len(), 1);
            }
            other => panic!("expected CellFragment, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact CellFragment round-trip");
    }

    #[test]
    fn beamhopping_mode1_round_trip() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![BeamhoppingPlan {
                beamhopping_time_plan_id: 0x12345678,
                time_plan_mode: TimePlanMode::Bitmap,
                time_of_application_base: 0x0000_AAAA_AAAA,
                time_of_application_ext: 0x100,
                cycle_duration_base: 0x0000_5555_5555,
                cycle_duration_ext: 0x080,
                mode: BeamhoppingMode::Mode1 {
                    bit_map_size: 8,
                    current_slot: 3,
                    slot_transmission_on: vec![true, false, true, true, false, false, true, false],
                    reserved_zero_future_use_size: ReservedZero::from_bits(0),
                    reserved_zero_future_use_slot: ReservedZero::from_bits(0),
                },
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 1);
                assert_eq!(bhp.plans[0].time_plan_mode, TimePlanMode::Bitmap);
                match &bhp.plans[0].mode {
                    BeamhoppingMode::Mode1 {
                        bit_map_size,
                        current_slot,
                        slot_transmission_on,
                        ..
                    } => {
                        assert_eq!(*bit_map_size, 8);
                        assert_eq!(*current_slot, 3);
                        assert_eq!(
                            slot_transmission_on,
                            &[true, false, true, true, false, false, true, false]
                        );
                    }
                    other => panic!("expected Mode1, got {other:?}"),
                }
            }
            other => panic!("expected BeamhoppingTimePlan, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact Mode1 round-trip");
    }

    #[test]
    fn beamhopping_mode2_round_trip() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![BeamhoppingPlan {
                beamhopping_time_plan_id: 0x87654321,
                time_plan_mode: TimePlanMode::GridRevisitSleep,
                time_of_application_base: 0x0000_BBBB_BBBB,
                time_of_application_ext: 0x000,
                cycle_duration_base: 0x0000_6666_6666,
                cycle_duration_ext: 0x090,
                mode: BeamhoppingMode::Mode2 {
                    grid_size_base: 0x0000_1111_1111,
                    grid_size_ext: 0x111,
                    revisit_duration_base: 0x0000_2222_2222,
                    revisit_duration_ext: 0x022,
                    sleep_time_base: 0x0000_3333_3333,
                    sleep_time_ext: 0x133,
                    sleep_duration_base: 0x0000_4444_4444,
                    sleep_duration_ext: 0x044,
                    grid_size_reserved_zero_future_use: ReservedZero::from_bits(0),
                    revisit_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                    sleep_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                    sleep_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 1);
                assert_eq!(bhp.plans[0].time_plan_mode, TimePlanMode::GridRevisitSleep);
                match &bhp.plans[0].mode {
                    BeamhoppingMode::Mode2 { grid_size_base, .. } => {
                        assert_eq!(*grid_size_base, 0x0000_1111_1111);
                    }
                    other => panic!("expected Mode2, got {other:?}"),
                }
            }
            other => panic!("expected BeamhoppingTimePlan, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact Mode2 round-trip");
    }

    #[test]
    fn beamhopping_reserved_mode_round_trip() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![
                BeamhoppingPlan {
                    beamhopping_time_plan_id: 0x11111111,
                    time_plan_mode: TimePlanMode::DwellOnTime,
                    time_of_application_base: 0x0000_AAAA_AAAA,
                    time_of_application_ext: 0x100,
                    cycle_duration_base: 0x0000_5555_5555,
                    cycle_duration_ext: 0x080,
                    mode: BeamhoppingMode::Mode0 {
                        dwell_duration_base: 0x0000_1111_1111,
                        dwell_duration_ext: 0x111,
                        on_time_base: 0x0000_2222_2222,
                        on_time_ext: 0x022,
                        dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                        on_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                    },
                    reserved_zero_future_use_4: ReservedZero::from_bits(0),
                    reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                    time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                    cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                BeamhoppingPlan {
                    beamhopping_time_plan_id: 0x22222222,
                    time_plan_mode: TimePlanMode::Reserved(3),
                    time_of_application_base: 0x0000_CCCC_CCCC,
                    time_of_application_ext: 0x100,
                    cycle_duration_base: 0x0000_DDDD_DDDD,
                    cycle_duration_ext: 0x000,
                    mode: BeamhoppingMode::Reserved(vec![0xAA, 0xBB, 0xCC]),
                    reserved_zero_future_use_4: ReservedZero::from_bits(0),
                    reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                    time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                    cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
            ],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 2);
                assert_eq!(bhp.plans[0].time_plan_mode, TimePlanMode::DwellOnTime);
                assert_eq!(bhp.plans[1].time_plan_mode, TimePlanMode::Reserved(3));
                match &bhp.plans[1].mode {
                    BeamhoppingMode::Reserved(v) => {
                        assert_eq!(v, &[0xAA, 0xBB, 0xCC]);
                    }
                    other => panic!("expected Reserved, got {other:?}"),
                }
            }
            other => panic!("expected BeamhoppingTimePlan, got {other:?}"),
        }
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2, "byte-exact Reserved mode round-trip");
    }

    #[test]
    fn cell_fragment_truncated_dsid_count() {
        let body = SatBody::CellFragment(CellFragmentBody {
            fragments: vec![CellFragment {
                cell_fragment_id: 1,
                first_occurrence: false,
                last_occurrence: false,
                centre: None,
                delivery_system_ids: vec![0x11111111],
                new_delivery_systems: Vec::new(),
                obsolescent_delivery_systems: Vec::new(),
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_new: ReservedZero::from_bits(0),
                reserved_zero_future_use_obsolescent: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(1, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2);

        let corrupt_sat = SatSection::new(
            1,
            true,
            0,
            crate::tables::SectionVersion::new(5, true, 0, 0),
            SatBody::CellFragment(CellFragmentBody {
                fragments: vec![CellFragment {
                    cell_fragment_id: 1,
                    first_occurrence: false,
                    last_occurrence: false,
                    centre: None,
                    delivery_system_ids: vec![0x11111111; 50],
                    new_delivery_systems: Vec::new(),
                    obsolescent_delivery_systems: Vec::new(),
                    reserved_zero_future_use_4: ReservedZero::from_bits(0),
                    reserved_zero_future_use_new: ReservedZero::from_bits(0),
                    reserved_zero_future_use_obsolescent: ReservedZero::from_bits(0),
                }],
            }),
        );
        let mut corrupt_buf = vec![0u8; corrupt_sat.serialized_len()];
        corrupt_sat.serialize_into(&mut corrupt_buf).unwrap();
        let section_length = (corrupt_buf.len() - SECTION_LENGTH_PREFIX) as u16;
        corrupt_buf[1] = 0x80 | 0x40 | 0x30 | ((section_length >> 8) as u8 & 0x0F);
        corrupt_buf[2] = (section_length & 0xFF) as u8;
        let crc_end = corrupt_buf.len();
        let crc = broadcast_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
        corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
        let original_len = corrupt_buf.len();
        corrupt_buf.truncate(original_len - 100);
        let sl = (corrupt_buf.len() - SECTION_LENGTH_PREFIX) as u16;
        corrupt_buf[1] = (corrupt_buf[1] & 0xF0) | ((sl >> 8) as u8 & 0x0F);
        corrupt_buf[2] = (sl & 0xFF) as u8;
        let crc_end = corrupt_buf.len();
        let crc2 = broadcast_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
        corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc2.to_be_bytes());
        assert!(matches!(
            &(SatSection::parse(&corrupt_buf)),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn beamhopping_mode1_truncated_bit_map_size() {
        let corrupt_sat = SatSection::new(
            3,
            true,
            0,
            crate::tables::SectionVersion::new(5, true, 0, 0),
            SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
                plans: vec![BeamhoppingPlan {
                    beamhopping_time_plan_id: 1,
                    time_plan_mode: TimePlanMode::Bitmap,
                    time_of_application_base: 0,
                    time_of_application_ext: 0,
                    cycle_duration_base: 0,
                    cycle_duration_ext: 0,
                    mode: BeamhoppingMode::Mode1 {
                        bit_map_size: 200,
                        current_slot: 0,
                        slot_transmission_on: vec![true; 200],
                        reserved_zero_future_use_size: ReservedZero::from_bits(0),
                        reserved_zero_future_use_slot: ReservedZero::from_bits(0),
                    },
                    reserved_zero_future_use_4: ReservedZero::from_bits(0),
                    reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                    time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                    cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                }],
            }),
        );
        let mut corrupt_buf = vec![0u8; corrupt_sat.serialized_len()];
        corrupt_sat.serialize_into(&mut corrupt_buf).unwrap();
        let original_len = corrupt_buf.len();
        let truncate_at = HEADER_LEN + 20;
        assert!(
            truncate_at + CRC_LEN < original_len,
            "fixture must be large enough to truncate meaningfully"
        );
        {
            corrupt_buf.truncate(truncate_at + CRC_LEN);
            let sl = (corrupt_buf.len() - SECTION_LENGTH_PREFIX) as u16;
            corrupt_buf[1] = (corrupt_buf[1] & 0xF0) | ((sl >> 8) as u8 & 0x0F);
            corrupt_buf[2] = (sl & 0xFF) as u8;
            let crc_end = corrupt_buf.len();
            let crc = broadcast_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
            corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
            assert!(matches!(
                &(SatSection::parse(&corrupt_buf)),
                Err(crate::error::Error::BufferTooShort(..))
            ));
        }
    }

    #[test]
    fn position_v3_truncated_ephemeris_data_count() {
        let corrupt_sat = SatSection::new(
            4,
            true,
            0,
            crate::tables::SectionVersion::new(5, true, 0, 0),
            SatBody::PositionV3(PositionV3Body {
                oem_version_major: 1,
                oem_version_minor: 0,
                creation_date_year: 25,
                creation_date_day: 1,
                creation_date_day_fraction: 0,
                satellites: vec![PositionV3Satellite {
                    satellite_id: 1,
                    usable_start_time_flag: false,
                    usable_stop_time_flag: false,
                    ephemeris_accel_flag: false,
                    covariance_flag: false,
                    metadata: None,
                    ephemeris_data: vec![
                        EphemerisData {
                            epoch_year: 25,
                            epoch_day: 1,
                            epoch_day_fraction: 0,
                            ephemeris_x: 0,
                            ephemeris_y: 0,
                            ephemeris_z: 0,
                            ephemeris_x_dot: 0,
                            ephemeris_y_dot: 0,
                            ephemeris_z_dot: 0,
                            acceleration: None,
                            reserved_zero_future_use: ReservedZero::from_bits(0),
                        };
                        5
                    ],
                    covariance: None,
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                }],
                creation_date_reserved_zero_future_use: ReservedZero::from_bits(0),
            }),
        );
        let mut corrupt_buf = vec![0u8; corrupt_sat.serialized_len()];
        corrupt_sat.serialize_into(&mut corrupt_buf).unwrap();
        let original_len = corrupt_buf.len();
        let truncate_at = HEADER_LEN + 30;
        assert!(
            truncate_at + CRC_LEN < original_len,
            "fixture must be large enough to truncate meaningfully"
        );
        {
            corrupt_buf.truncate(truncate_at + CRC_LEN);
            let sl = (corrupt_buf.len() - SECTION_LENGTH_PREFIX) as u16;
            corrupt_buf[1] = (corrupt_buf[1] & 0xF0) | ((sl >> 8) as u8 & 0x0F);
            corrupt_buf[2] = (sl & 0xFF) as u8;
            let crc_end = corrupt_buf.len();
            let crc = broadcast_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
            corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
            assert!(matches!(
                &(SatSection::parse(&corrupt_buf)),
                Err(crate::error::Error::BufferTooShort(..))
            ));
        }
    }

    #[test]
    fn hand_byte_time_association() {
        let body = SatBody::TimeAssociation(TimeAssociationBody {
            association_type: AssociationType::UtcWithoutLeap,
            leap_info: None,
            ncr_base: 0x0000_AAAA_AAAA_u64,
            ncr_ext: 0x1AA,
            association_timestamp_seconds: 0,
            association_timestamp_nanoseconds: 0,
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            ncr_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat(2, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 2);
        match &sat.body {
            SatBody::TimeAssociation(ta) => {
                assert_eq!(ta.association_type, AssociationType::UtcWithoutLeap);
                assert_eq!(ta.ncr_base, 0x0000_AAAA_AAAA);
                assert_eq!(ta.ncr_ext, 0x1AA);
            }
            other => panic!("expected TimeAssociation, got {other:?}"),
        }
        assert_eq!((bytes[1] >> 6) & 1, 1);
        assert_eq!((bytes[3] >> 2) & 0x3F, 2);
    }

    #[test]
    fn hand_byte_position_v2_orbital() {
        let body = SatBody::PositionV2(PositionV2Body {
            satellites: vec![PositionV2Satellite {
                satellite_id: 0x010203,
                position: PositionSystem::Orbital {
                    orbital_position: 0x1920,
                    west_east_flag: true,
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(0, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::PositionV2(pv2) => {
                assert_eq!(pv2.satellites[0].satellite_id, 0x010203);
                match &pv2.satellites[0].position {
                    PositionSystem::Orbital {
                        orbital_position, ..
                    } => {
                        assert_eq!(*orbital_position, 0x1920);
                    }
                    other => panic!("expected Orbital, got {other:?}"),
                }
            }
            other => panic!("expected PositionV2, got {other:?}"),
        }
        assert_eq!((bytes[3] >> 2) & 0x3F, 0);
    }

    #[test]
    fn hand_byte_cell_fragment() {
        let body = SatBody::CellFragment(CellFragmentBody {
            fragments: vec![CellFragment {
                cell_fragment_id: 0xAABBCCDD,
                first_occurrence: false,
                last_occurrence: true,
                centre: None,
                delivery_system_ids: Vec::new(),
                new_delivery_systems: Vec::new(),
                obsolescent_delivery_systems: Vec::new(),
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_new: ReservedZero::from_bits(0),
                reserved_zero_future_use_obsolescent: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(1, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::CellFragment(cf) => {
                assert_eq!(cf.fragments[0].cell_fragment_id, 0xAABBCCDD);
                assert!(cf.fragments[0].last_occurrence);
            }
            other => panic!("expected CellFragment, got {other:?}"),
        }
        assert_eq!((bytes[3] >> 2) & 0x3F, 1);
    }

    #[test]
    fn hand_byte_beamhopping_mode0() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![BeamhoppingPlan {
                beamhopping_time_plan_id: 0xDEADBEEF,
                time_plan_mode: TimePlanMode::DwellOnTime,
                time_of_application_base: 0,
                time_of_application_ext: 0,
                cycle_duration_base: 0,
                cycle_duration_ext: 0,
                mode: BeamhoppingMode::Mode0 {
                    dwell_duration_base: 0,
                    dwell_duration_ext: 0,
                    on_time_base: 0,
                    on_time_ext: 0,
                    dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                    on_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                },
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_mode: ReservedZero::from_bits(0),
                time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
                cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans[0].beamhopping_time_plan_id, 0xDEADBEEF);
                assert_eq!(bhp.plans[0].time_plan_mode, TimePlanMode::DwellOnTime);
            }
            other => panic!("expected BeamhoppingTimePlan, got {other:?}"),
        }
        assert_eq!((bytes[3] >> 2) & 0x3F, 3);
    }

    #[test]
    fn hand_byte_position_v3() {
        let body = SatBody::PositionV3(PositionV3Body {
            oem_version_major: 1,
            oem_version_minor: 2,
            creation_date_year: 26,
            creation_date_day: 42,
            creation_date_day_fraction: 0,
            satellites: Vec::new(),
            creation_date_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat(4, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::PositionV3(v3) => {
                assert_eq!(v3.oem_version_major, 1);
                assert_eq!(v3.oem_version_minor, 2);
            }
            other => panic!("expected PositionV3, got {other:?}"),
        }
        assert_eq!((bytes[3] >> 2) & 0x3F, 4);
    }

    // ── Hand-built byte-literal anchor tests ──────────────────────────────────
    // Each constructs a wire byte array by hand (no serializer) and verifies
    // that the bit-packed parser maps fields to the expected bit positions.
    // Re-serialization must then produce byte-identical output.

    fn crc_section(bytes: &[u8]) -> Vec<u8> {
        let mut v = bytes.to_vec();
        let crc = broadcast_common::crc32_mpeg2::compute(&v);
        v.extend_from_slice(&crc.to_be_bytes());
        v
    }

    #[test]
    fn hand_built_time_association_anchor() {
        // section_length = body_len(19) + 10 = 29 = 0x001D
        // Bit breakdown in body (association_type=0, ncr_base=1, all else 0):
        //   [0:3]   association_type(4)=0
        //   [4:7]   reserved/ncr_leap(4)=0
        //   [8:40]  ncr_base(33)=1    →  LSB lands at bit 40 → byte 5 bit 7
        //   [41:46] skip(6)
        //   [47:55] ncr_ext(9)=0
        //   [56:119] timestamp_seconds(64)=0
        //   [120:151] timestamp_nanoseconds(32)=0
        let bytes = crc_section(&[
            0x4D, 0xF0, 0x1D, 0x08, 0x00, 0xCB, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 2);
        match &sat.body {
            SatBody::TimeAssociation(ta) => {
                assert_eq!(ta.association_type, AssociationType::UtcWithoutLeap);
                assert_eq!(ta.ncr_base, 1);
                assert_eq!(ta.ncr_ext, 0);
                assert_eq!(ta.association_timestamp_seconds, 0);
                assert_eq!(ta.association_timestamp_nanoseconds, 0);
            }
            other => panic!("expected TimeAssociation, got {other:?}"),
        }
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "byte-identical re-serialize");
    }

    #[test]
    fn hand_built_position_v2_orbital_anchor() {
        // section_length = body_len(7) + 10 = 17 = 0x0011
        // Bit breakdown in body (one satellite, orbital position):
        //   [0:23]   satellite_id(24)=0x010203
        //   [24:30]  skip(7)
        //   [31]     position_system(1)=0 → orbital
        //   [32:47]  orbital_position(16)=0x1234
        //   [48]     west_east_flag(1)=1
        //   [49:55]  skip(7)
        let bytes = crc_section(&[
            0x4D, 0xF0, 0x11, 0x00, 0x00, 0xCB, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x00, 0x12,
            0x34, 0x80,
        ]);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 0);
        match &sat.body {
            SatBody::PositionV2(pv2) => {
                assert_eq!(pv2.satellites.len(), 1);
                let s = &pv2.satellites[0];
                assert_eq!(s.satellite_id, 0x010203);
                match &s.position {
                    PositionSystem::Orbital {
                        orbital_position,
                        west_east_flag,
                        ..
                    } => {
                        assert_eq!(*orbital_position, 0x1234);
                        assert!(*west_east_flag);
                    }
                    other => panic!("expected Orbital, got {other:?}"),
                }
            }
            other => panic!("expected PositionV2, got {other:?}"),
        }
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "byte-identical re-serialize");
    }

    #[test]
    fn hand_built_cell_fragment_anchor() {
        // section_length = body_len(10) + 10 = 20 = 0x0014
        // Bit breakdown in body (one fragment, no centre, empty lists):
        //   [0:31]   cell_fragment_id(32)=0xAABBCCDD
        //   [32]     first_occurrence(1)=0
        //   [33]     last_occurrence(1)=1
        //   [34:37]  skip(4)
        //   [38:47]  delivery_system_ids_count(10)=0
        //   [48:53]  skip(6)
        //   [54:63]  new_delivery_systems_count(10)=0
        //   [64:69]  skip(6)
        //   [70:79]  obsolescent_delivery_systems_count(10)=0
        let bytes = crc_section(&[
            0x4D, 0xF0, 0x14, 0x04, 0x00, 0xCB, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD, 0x40,
            0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 1);
        match &sat.body {
            SatBody::CellFragment(cf) => {
                assert_eq!(cf.fragments.len(), 1);
                let f = &cf.fragments[0];
                assert_eq!(f.cell_fragment_id, 0xAABBCCDD);
                assert!(!f.first_occurrence);
                assert!(f.last_occurrence);
                assert!(f.centre.is_none());
                assert!(f.delivery_system_ids.is_empty());
                assert!(f.new_delivery_systems.is_empty());
                assert!(f.obsolescent_delivery_systems.is_empty());
            }
            other => panic!("expected CellFragment, got {other:?}"),
        }
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "byte-identical re-serialize");
    }

    #[test]
    fn hand_built_beamhopping_mode0_anchor() {
        // section_length = body_len(31) + 10 = 41 = 0x0029
        // Bit breakdown in body (one plan, Mode0, all times zero):
        //   [0:31]   plan_id(32)=0xDEADBEEF
        //   [32:35]  skip(4)
        //   [36:47]  plan_length(12)=25 → 0x019
        //   [48:53]  skip(6)
        //   [54:55]  plan_mode(2)=0
        //   [56:247] all zero (times, durations)
        let bytes = crc_section(&[
            0x4D, 0xF0, 0x29, 0x0C, 0x00, 0xCB, 0x00, 0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x00,
            0x19, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 3);
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 1);
                let p = &bhp.plans[0];
                assert_eq!(p.beamhopping_time_plan_id, 0xDEADBEEF);
                assert_eq!(p.time_plan_mode, TimePlanMode::DwellOnTime);
                assert_eq!(p.time_of_application_base, 0);
                assert_eq!(p.cycle_duration_base, 0);
                match &p.mode {
                    BeamhoppingMode::Mode0 {
                        dwell_duration_base,
                        dwell_duration_ext,
                        on_time_base,
                        on_time_ext,
                        ..
                    } => {
                        assert_eq!(*dwell_duration_base, 0);
                        assert_eq!(*dwell_duration_ext, 0);
                        assert_eq!(*on_time_base, 0);
                        assert_eq!(*on_time_ext, 0);
                    }
                    other => panic!("expected Mode0, got {other:?}"),
                }
            }
            other => panic!("expected BeamhoppingTimePlan, got {other:?}"),
        }
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "byte-identical re-serialize");
    }

    #[test]
    fn hand_built_position_v3_anchor() {
        // section_length = body_len(8) + 10 = 18 = 0x0012
        // Bit breakdown in body (empty satellites):
        //   [0:3]   oem_version_major(4)=1
        //   [4:7]   oem_version_minor(4)=2        → byte 0 = 0x12
        //   [8:15]  creation_date_year(8)=26       → byte 1 = 0x1A
        //   [16:22] skip(7)
        //   [23:31] creation_date_day(9)=42        → byte 2 bit 0=0, byte 3 = 0x2A
        //   [32:63] creation_date_day_fraction(32)=0 → bytes 4-7
        let bytes = crc_section(&[
            0x4D, 0xF0, 0x12, 0x10, 0x00, 0xCB, 0x00, 0x00, 0x00, 0x12, 0x1A, 0x00, 0x2A, 0x00,
            0x00, 0x00, 0x00,
        ]);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.satellite_table_id, 4);
        match &sat.body {
            SatBody::PositionV3(v3) => {
                assert_eq!(v3.oem_version_major, 1);
                assert_eq!(v3.oem_version_minor, 2);
                assert_eq!(v3.creation_date_year, 26);
                assert_eq!(v3.creation_date_day, 42);
                assert_eq!(v3.creation_date_day_fraction, 0);
                assert!(v3.satellites.is_empty());
            }
            other => panic!("expected PositionV3, got {other:?}"),
        }
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "byte-identical re-serialize");
    }

    #[test]
    fn parse_rejects_truncated_time_association_body() {
        let body = SatBody::TimeAssociation(TimeAssociationBody {
            association_type: AssociationType::UtcWithoutLeap,
            leap_info: None,
            ncr_base: 0,
            ncr_ext: 0,
            association_timestamp_seconds: 0,
            association_timestamp_nanoseconds: 0,
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            ncr_reserved_zero_future_use: ReservedZero::from_bits(0),
        });
        let bytes = build_sat(2, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        buf.truncate(HEADER_LEN + 4 + CRC_LEN);
        let sl = (buf.len() - SECTION_LENGTH_PREFIX) as u16;
        buf[1] = (buf[1] & 0xF0) | ((sl >> 8) as u8 & 0x0F);
        buf[2] = (sl & 0xFF) as u8;
        let crc_end = buf.len();
        let crc = broadcast_common::crc32_mpeg2::compute(&buf[..crc_end - CRC_LEN]);
        buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
        assert!(matches!(
            &(SatSection::parse(&buf)),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn sat_table_id_wire_to_name() {
        let stid = SatTableId::try_from(0u8).unwrap();
        assert_eq!(stid, SatTableId::PositionV2);
        let stid = SatTableId::try_from(2u8).unwrap();
        assert_eq!(stid, SatTableId::TimeAssociation);
        let stid = SatTableId::try_from(4u8).unwrap();
        assert_eq!(stid, SatTableId::PositionV3);
    }

    #[test]
    fn association_type_wire_to_name() {
        assert_eq!(
            AssociationType::from_u8(0).name(),
            "UTC without leap second"
        );
        assert_eq!(AssociationType::from_u8(1).name(), "UTC with leap second");
    }

    #[test]
    fn interpolation_type_wire_to_name() {
        assert_eq!(InterpolationType::from_u8(1).name(), "Linear");
        assert_eq!(InterpolationType::from_u8(2).name(), "Lagrange");
        assert_eq!(InterpolationType::from_u8(4).name(), "Hermite");
        assert_eq!(InterpolationType::from_u8(0).name(), "Reserved");
    }

    #[test]
    fn time_plan_mode_full_range_round_trip() {
        for v in 0u8..=0x03 {
            let tpm = TimePlanMode::from_u8(v);
            assert_eq!(tpm.to_u8(), v, "TimePlanMode round-trip failed for {v}");
        }
    }

    #[test]
    fn time_plan_mode_known_values() {
        assert_eq!(TimePlanMode::from_u8(0), TimePlanMode::DwellOnTime);
        assert_eq!(TimePlanMode::from_u8(1), TimePlanMode::Bitmap);
        assert_eq!(TimePlanMode::from_u8(2), TimePlanMode::GridRevisitSleep);
        assert_eq!(TimePlanMode::from_u8(3), TimePlanMode::Reserved(3));
        assert_eq!(TimePlanMode::DwellOnTime.name(), "dwell/on-time");
        assert_eq!(TimePlanMode::Bitmap.name(), "bitmap");
        assert_eq!(TimePlanMode::GridRevisitSleep.name(), "grid/revisit/sleep");
        assert_eq!(TimePlanMode::Reserved(3).name(), "reserved");
    }

    fn mode1_plan(id: u32, bit_map_size: u16) -> BeamhoppingPlan {
        BeamhoppingPlan {
            beamhopping_time_plan_id: id,
            time_plan_mode: TimePlanMode::Bitmap,
            time_of_application_base: 0,
            time_of_application_ext: 0,
            cycle_duration_base: 0,
            cycle_duration_ext: 0,
            mode: BeamhoppingMode::Mode1 {
                bit_map_size,
                current_slot: 0,
                slot_transmission_on: vec![true; bit_map_size as usize],
                reserved_zero_future_use_size: ReservedZero::from_bits(0),
                reserved_zero_future_use_slot: ReservedZero::from_bits(0),
            },
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            reserved_zero_future_use_mode: ReservedZero::from_bits(0),
            time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
            cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
        }
    }

    // ── Signed-field range guard (§5.2.11.3: 18-bit latitude, 19-bit longitude)

    #[test]
    fn write_i_rejects_out_of_range_values() {
        let mut buf = [0u8; 8];
        let mut w = BitWriter::new(&mut buf);
        assert!(matches!(
            w.write_i(18, 1 << 17),
            Err(Error::FieldOverflow(_))
        ));
        let mut buf = [0u8; 8];
        let mut w = BitWriter::new(&mut buf);
        assert!(matches!(
            w.write_i(18, -(1 << 17) - 1),
            Err(Error::FieldOverflow(_))
        ));
        let mut buf = [0u8; 8];
        let mut w = BitWriter::new(&mut buf);
        assert!(matches!(
            w.write_i(19, 1 << 18),
            Err(Error::FieldOverflow(_))
        ));
        let mut buf = [0u8; 8];
        let mut w = BitWriter::new(&mut buf);
        assert!(matches!(
            w.write_i(19, -(1 << 18) - 1),
            Err(Error::FieldOverflow(_))
        ));
    }

    #[test]
    fn write_i_accepts_both_boundaries() {
        let mut buf = [0u8; 16];
        let mut w = BitWriter::new(&mut buf);
        w.write_i(18, (1 << 17) - 1).unwrap();
        w.write_i(18, -(1 << 17)).unwrap();
        w.write_i(19, (1 << 18) - 1).unwrap();
        w.write_i(19, -(1 << 18)).unwrap();
        assert_eq!(w.bits_written(), 18 + 18 + 19 + 19);
    }

    fn centre_fragment(lat: i32, lon: i32) -> CellFragment {
        CellFragment {
            cell_fragment_id: 1,
            first_occurrence: true,
            last_occurrence: false,
            centre: Some(CellCentre {
                centre_latitude: lat,
                centre_longitude: lon,
                max_distance: 0,
                reserved_zero_future_use_4: ReservedZero::from_bits(0),
                reserved_zero_future_use_5: ReservedZero::from_bits(0),
                reserved_zero_future_use_6: ReservedZero::from_bits(0),
            }),
            delivery_system_ids: Vec::new(),
            new_delivery_systems: Vec::new(),
            obsolescent_delivery_systems: Vec::new(),
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            reserved_zero_future_use_new: ReservedZero::from_bits(0),
            reserved_zero_future_use_obsolescent: ReservedZero::from_bits(0),
        }
    }

    #[test]
    fn cell_centre_rejects_out_of_range_coordinates() {
        for frag in [centre_fragment(1 << 17, 0), centre_fragment(0, 1 << 18)] {
            let body = SatBody::CellFragment(CellFragmentBody {
                fragments: vec![frag],
            });
            let mut buf = [0u8; 32];
            let mut w = BitWriter::new(&mut buf);
            assert!(matches!(
                sat_body_write(&body, &mut w),
                Err(Error::FieldOverflow(_))
            ));
        }
    }

    #[test]
    fn cell_centre_boundary_coordinates_round_trip() {
        let centre = centre_fragment((1 << 17) - 1, -(1 << 18));
        let body = SatBody::CellFragment(CellFragmentBody {
            fragments: vec![centre],
        });
        let bytes = build_sat(1, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::CellFragment(cf) => {
                let c = cf.fragments[0].centre.as_ref().unwrap();
                assert_eq!(c.centre_latitude, 131_071);
                assert_eq!(c.centre_longitude, -262_144);
            }
            other => panic!("expected CellFragment, got {other:?}"),
        }
        let mut again = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut again).unwrap();
        assert_eq!(again, bytes);
    }

    // ── Bit-level helper hygiene ─────────────────────────────────────────────

    /// `write_zero` must clear its bits and advance the cursor: emitting into a
    /// 0xFF buffer yields the same bytes as into a zeroed one.
    #[test]
    fn bit_writer_zero_runs_clear_a_dirty_buffer() {
        let bodies = [
            SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
                plans: vec![mode1_plan(1, 13), mode0_plan(2)],
            }),
            SatBody::PositionV3(PositionV3Body {
                oem_version_major: 1,
                oem_version_minor: 0,
                creation_date_year: 24,
                creation_date_day: 1,
                creation_date_day_fraction: 0,
                satellites: vec![PositionV3Satellite {
                    satellite_id: 1,
                    usable_start_time_flag: true,
                    usable_stop_time_flag: true,
                    ephemeris_accel_flag: true,
                    covariance_flag: true,
                    metadata: Some(PositionV3Metadata {
                        total_start_time_year: 1,
                        total_start_time_day: 2,
                        total_start_time_day_fraction: 3,
                        total_stop_time_year: 4,
                        total_stop_time_day: 5,
                        total_stop_time_day_fraction: 6,
                        interpolation_flag: true,
                        interpolation_type: InterpolationType::Linear,
                        interpolation_degree: 2,
                        usable_start_time: None,
                        usable_stop_time: None,
                        total_start_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                        total_stop_time_reserved_zero_future_use: ReservedZero::from_bits(0),
                        reserved_zero_future_use: ReservedZero::from_bits(0),
                    }),
                    ephemeris_data: Vec::new(),
                    covariance: None,
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                }],
                creation_date_reserved_zero_future_use: ReservedZero::from_bits(0),
            }),
        ];
        for body in &bodies {
            let len = sat_body_serialized_len(body);
            let mut dirty = vec![0xFFu8; len];
            sat_body_write(body, &mut BitWriter::new(&mut dirty)).unwrap();
            let mut clean = vec![0x00u8; len];
            sat_body_write(body, &mut BitWriter::new(&mut clean)).unwrap();
            assert_eq!(dirty, clean);
        }
    }

    fn mode0_plan(id: u32) -> BeamhoppingPlan {
        BeamhoppingPlan {
            beamhopping_time_plan_id: id,
            time_plan_mode: TimePlanMode::DwellOnTime,
            time_of_application_base: 0,
            time_of_application_ext: 0,
            cycle_duration_base: 0,
            cycle_duration_ext: 0,
            mode: BeamhoppingMode::Mode0 {
                dwell_duration_base: 0,
                dwell_duration_ext: 0,
                on_time_base: 0,
                on_time_ext: 0,
                dwell_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
                on_time_reserved_zero_future_use: ReservedZero::from_bits(0),
            },
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            reserved_zero_future_use_mode: ReservedZero::from_bits(0),
            time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
            cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
        }
    }

    /// `seek_forward` consumes the reserved bytes of a reserved-mode plan only
    /// when a following plan must be reached; a body of two reserved plans makes
    /// a no-op seek re-read the reserved bytes as a plan.
    #[test]
    fn beamhopping_reserved_mode_seek_forward() {
        let plan = BeamhoppingPlan {
            beamhopping_time_plan_id: 0x33333333,
            time_plan_mode: TimePlanMode::Reserved(3),
            time_of_application_base: 0,
            time_of_application_ext: 0,
            cycle_duration_base: 0,
            cycle_duration_ext: 0,
            mode: BeamhoppingMode::Reserved(vec![0xFF; 8]),
            reserved_zero_future_use_4: ReservedZero::from_bits(0),
            reserved_zero_future_use_mode: ReservedZero::from_bits(0),
            time_of_application_reserved_zero_future_use: ReservedZero::from_bits(0),
            cycle_duration_reserved_zero_future_use: ReservedZero::from_bits(0),
        };
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
            plans: vec![plan.clone(), mode0_plan(0x44)],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.body, body);
        let mut again = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut again).unwrap();
        assert_eq!(again, bytes);
    }

    // ── Empty bodies for the loop-shaped layouts ─────────────────────────────

    #[test]
    fn parse_empty_position_v2_body() {
        let body = SatBody::PositionV2(PositionV2Body {
            satellites: Vec::new(),
        });
        let bytes = build_sat(0, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.body, body);
        let mut again = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut again).unwrap();
        assert_eq!(again, bytes);
    }

    #[test]
    fn parse_empty_cell_fragment_body() {
        let body = SatBody::CellFragment(CellFragmentBody {
            fragments: Vec::new(),
        });
        let bytes = build_sat(1, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.body, body);
        let mut again = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut again).unwrap();
        assert_eq!(again, bytes);
    }

    #[test]
    fn parse_empty_beamhopping_body() {
        let body = SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody { plans: Vec::new() });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        assert_eq!(sat.body, body);
        let mut again = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut again).unwrap();
        assert_eq!(again, bytes);
    }

    // ── Oversize fallback in sat_body_serialized_len ─────────────────────────

    fn ephemeris_entry() -> EphemerisData {
        EphemerisData {
            epoch_year: 0,
            epoch_day: 0,
            epoch_day_fraction: 0,
            ephemeris_x: 0,
            ephemeris_y: 0,
            ephemeris_z: 0,
            ephemeris_x_dot: 0,
            ephemeris_y_dot: 0,
            ephemeris_z_dot: 0,
            acceleration: None,
            reserved_zero_future_use: ReservedZero::from_bits(0),
        }
    }

    fn oversize_v3(entries: usize) -> SatSection {
        SatSection::new(
            4,
            true,
            0,
            crate::tables::SectionVersion::new(0, true, 0, 0),
            SatBody::PositionV3(PositionV3Body {
                oem_version_major: 1,
                oem_version_minor: 0,
                creation_date_year: 24,
                creation_date_day: 1,
                creation_date_day_fraction: 0,
                satellites: vec![PositionV3Satellite {
                    satellite_id: 1,
                    usable_start_time_flag: false,
                    usable_stop_time_flag: false,
                    ephemeris_accel_flag: false,
                    covariance_flag: false,
                    metadata: None,
                    ephemeris_data: vec![ephemeris_entry(); entries],
                    covariance: None,
                    reserved_zero_future_use: ReservedZero::from_bits(0),
                }],
                creation_date_reserved_zero_future_use: ReservedZero::from_bits(0),
            }),
        )
    }

    #[test]
    fn serialized_len_grows_the_heap_buffer_for_an_oversized_body() {
        // Each ephemeris entry is 31 bytes; 2 258 entries is ~70 000 bytes.
        let sat = oversize_v3(2_258);
        let len = sat.serialized_len();
        assert!(len > 70_000, "got {len}");
        let mut buf = vec![0u8; len];
        assert!(matches!(
            sat.serialize_into(&mut buf),
            Err(Error::SectionLengthOverflow { .. })
        ));
    }

    #[test]
    fn serialized_len_stops_at_the_one_mib_cap() {
        // ~2 MiB of entries never fits the 1 MiB body cap, so the body length
        // is the cap (1 048 576) plus the 9-byte header and 4-byte CRC.
        let sat = oversize_v3(67_764);
        assert_eq!(sat.serialized_len(), 1_048_589);
        let mut buf = vec![0u8; 1_048_589];
        assert!(matches!(
            sat.serialize_into(&mut buf),
            Err(Error::SectionLengthOverflow { .. })
        ));
    }

    /// Each reserved run is width-limited by its type, so a wider value is
    /// unrepresentable and cannot spill into the adjacent syntax bits.
    #[test]
    fn reserved_zero_runs_cannot_hold_more_than_their_width() {
        let frag = CellFragment {
            cell_fragment_id: 0,
            first_occurrence: false,
            last_occurrence: false,
            centre: None,
            delivery_system_ids: vec![],
            new_delivery_systems: vec![],
            obsolescent_delivery_systems: vec![],
            reserved_zero_future_use_4: ReservedZero::from_bits(0xFF),
            reserved_zero_future_use_new: ReservedZero::from_bits(0xFF),
            reserved_zero_future_use_obsolescent: ReservedZero::from_bits(0xFF),
        };
        assert_eq!(frag.reserved_zero_future_use_4.get(), 0x0F);
        assert_eq!(frag.reserved_zero_future_use_new.get(), 0x3F);
        assert_eq!(frag.reserved_zero_future_use_obsolescent.get(), 0x3F);
    }
}
