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
use dvb_common::{Parse, Serialize};

/// table_id for the Satellite Access Table.
pub const TABLE_ID: u8 = 0x4D;
/// Well-known PID on which the SAT is carried (EN 300 468 Table 1, §5.1.3).
pub const PID: u16 = 0x001B;

const HEADER_LEN: usize = 9;
const SECTION_LENGTH_PREFIX: usize = 3;
const CRC_LEN: usize = 4;

fn pad_to_byte(bits: usize) -> usize {
    (8 - (bits % 8)) % 8
}

// ── Bit-level reader/writer ──────────────────────────────────────────────────

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }
    fn remaining_bits(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.bit_pos)
    }
    fn bits_consumed(&self) -> usize {
        self.bit_pos
    }
    fn read_u(&mut self, bits: u8) -> Result<u64> {
        let bits = bits as usize;
        if self.bit_pos + bits > self.data.len() * 8 {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + bits,
                have: self.data.len() * 8,
                what: "SatSection bit reader overrun",
            });
        }
        let mut val: u64 = 0;
        for i in 0..bits {
            let byte_idx = (self.bit_pos + i) / 8;
            let bit_idx = 7 - ((self.bit_pos + i) % 8);
            val = (val << 1) | ((self.data[byte_idx] >> bit_idx) & 1) as u64;
        }
        self.bit_pos += bits;
        Ok(val)
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
    fn skip(&mut self, bits: u8) -> Result<()> {
        if self.bit_pos + bits as usize > self.data.len() * 8 {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + bits as usize,
                have: self.data.len() * 8,
                what: "SatSection bit reader overrun",
            });
        }
        self.bit_pos += bits as usize;
        Ok(())
    }
}

struct BitWriter<'a> {
    buf: &'a mut [u8],
    bit_pos: usize,
}

impl<'a> BitWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, bit_pos: 0 }
    }
    fn bits_written(&self) -> usize {
        self.bit_pos
    }
    fn write_u(&mut self, bits: u8, val: u64) -> Result<()> {
        let bits = bits as usize;
        if self.bit_pos + bits > self.buf.len() * 8 {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + bits,
                have: self.buf.len() * 8,
                what: "SatSection bit writer overrun",
            });
        }
        for i in 0..bits {
            let byte_idx = (self.bit_pos + i) / 8;
            let bit_idx = 7 - ((self.bit_pos + i) % 8);
            let bit_val = ((val >> (bits - 1 - i)) & 1) as u8;
            self.buf[byte_idx] |= bit_val << bit_idx;
        }
        self.bit_pos += bits;
        Ok(())
    }
    fn write_i(&mut self, bits: u8, val: i64) -> Result<()> {
        self.write_u(bits, val as u64 & ((1u64 << bits) - 1))
    }
    fn write_zero(&mut self, bits: u8) -> Result<()> {
        if self.bit_pos + bits as usize > self.buf.len() * 8 {
            return Err(Error::BufferTooShort {
                need: self.bit_pos + bits as usize,
                have: self.buf.len() * 8,
                what: "SatSection bit writer overrun",
            });
        }
        self.bit_pos += bits as usize;
        Ok(())
    }
}

// ── SatTableId discriminant ─────────────────────────────────────────────────

/// `satellite_table_id` discriminant — selects the SAT body structure
/// (§5.2.11.1, Table 11b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, num_enum::TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum SatTableId {
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
pub struct CellCenter {
    /// `center_latitude` (18 bits, two's complement, `tcimsbf`).
    pub center_latitude: i32,
    /// `center_longitude` (19 bits, two's complement, `tcimsbf`).
    pub center_longitude: i32,
    /// `max_distance` (24 bits).
    pub max_distance: u32,
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
    pub center: Option<CellCenter>,
    /// `delivery_system_id` entries (each 32 bits).
    pub delivery_system_ids: Vec<u32>,
    /// New delivery system entries.
    pub new_delivery_systems: Vec<NewDeliverySystem>,
    /// Obsolescent delivery system entries.
    pub obsolescent_delivery_systems: Vec<ObsolescentDeliverySystem>,
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
        match v & 0x0F {
            0 => Self::UtcWithoutLeap,
            1 => Self::UtcWithLeap,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
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
}

// ── Beamhopping Time Plan (Table 11g) ───────────────────────────────────────

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
    },
    /// `time_plan_mode == 1`: bitmap.
    Mode1 {
        /// `bit_map_size` (15 bits).
        bit_map_size: u16,
        /// `current_slot` (15 bits).
        current_slot: u16,
        /// `slot_transmission_on` flags (bit_map_size entries).
        slot_transmission_on: Vec<bool>,
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
    /// `time_plan_mode` (2 bits).
    pub time_plan_mode: u8,
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
    /// `interpolation_type` (3 bits, Table 11i).
    pub interpolation_type: InterpolationType,
    /// `interpolation_degree` (3 bits).
    pub interpolation_degree: u8,
    /// Usable start time (optional).
    pub usable_start_time: Option<UsableTime>,
    /// Usable stop time (optional).
    pub usable_stop_time: Option<UsableTime>,
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
        match v & 0x07 {
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
    pub fn to_u8(self) -> u8 {
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
    pub covariance_elements: [u32; 21],
}

/// A satellite entry in the PositionV3 body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PositionV3Satellite {
    /// `satellite_id` (24 bits).
    pub satellite_id: u32,
    /// `usable_start_time_flag`.
    pub usable_start_time_flag: bool,
    /// `usable_stop_time_flag`.
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
}

// ── SatBody enum ────────────────────────────────────────────────────────────

/// The typed body of a SAT section, selected by `satellite_table_id`
/// (Tables 11c–11h).
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
    /// Reserved `satellite_table_id` (5–63): raw body bytes.
    Raw(Vec<u8>),
}

fn sat_body_serialized_len(body: &SatBody) -> usize {
    match body {
        SatBody::Raw(v) => v.len(),
        _ => {
            // The hardened BitWriter errors (rather than silently truncating)
            // on overrun, so feed it a buffer that's grown until the body fits.
            // Real SAT bodies are bounded by the 12-bit section_length (<4 KiB);
            // an over-large constructed body grows to the cap and is then
            // rejected by the section_length guard in serialize_into — never a
            // panic in this infallible length calc.
            let mut cap = 4096usize;
            loop {
                let mut tmp = vec![0u8; cap];
                let mut writer = BitWriter::new(&mut tmp);
                if sat_body_write(body, &mut writer).is_ok() {
                    break writer.bits_written().div_ceil(8);
                }
                if cap >= 1 << 20 {
                    break cap; // pathological; serialize_into rejects it
                }
                cap *= 2;
            }
        }
    }
}

fn sat_body_write(body: &SatBody, w: &mut BitWriter) -> Result<()> {
    match body {
        SatBody::PositionV2(b) => {
            for sat in &b.satellites {
                w.write_u(24, sat.satellite_id as u64)?;
                w.write_zero(7)?;
                match &sat.position {
                    PositionSystem::Orbital {
                        orbital_position,
                        west_east_flag,
                    } => {
                        w.write_u(1, 0)?;
                        w.write_u(16, *orbital_position as u64)?;
                        w.write_u(1, *west_east_flag as u64)?;
                        w.write_zero(7)?;
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
                        w.write_u(8, *epoch_year as u64)?;
                        w.write_u(16, *day_of_the_year as u64)?;
                        w.write_u(32, *day_fraction as u64)?;
                        w.write_u(32, *mean_motion_first_derivative as u64)?;
                        w.write_u(32, *mean_motion_second_derivative as u64)?;
                        w.write_u(32, *drag_term as u64)?;
                        w.write_u(32, *inclination as u64)?;
                        w.write_u(32, *right_ascension as u64)?;
                        w.write_u(32, *eccentricity as u64)?;
                        w.write_u(32, *argument_of_perigree as u64)?;
                        w.write_u(32, *mean_anomaly as u64)?;
                        w.write_u(32, *mean_motion as u64)?;
                    }
                }
            }
        }
        SatBody::CellFragment(b) => {
            for frag in &b.fragments {
                w.write_u(32, frag.cell_fragment_id as u64)?;
                w.write_u(1, frag.first_occurrence as u64)?;
                w.write_u(1, frag.last_occurrence as u64)?;
                if frag.first_occurrence {
                    if let Some(ref c) = frag.center {
                        w.write_zero(4)?;
                        w.write_i(18, c.center_latitude as i64)?;
                        w.write_zero(5)?;
                        w.write_i(19, c.center_longitude as i64)?;
                        w.write_u(24, c.max_distance as u64)?;
                        w.write_zero(6)?;
                    }
                } else {
                    w.write_zero(4)?;
                }
                w.write_u(10, frag.delivery_system_ids.len() as u64)?;
                for id in &frag.delivery_system_ids {
                    w.write_u(32, *id as u64)?;
                }
                w.write_zero(6)?;
                w.write_u(10, frag.new_delivery_systems.len() as u64)?;
                for nds in &frag.new_delivery_systems {
                    w.write_u(32, nds.new_delivery_system_id as u64)?;
                    w.write_u(33, nds.time_of_application_base)?;
                    w.write_zero(6)?;
                    w.write_u(9, nds.time_of_application_ext as u64)?;
                }
                w.write_zero(6)?;
                w.write_u(10, frag.obsolescent_delivery_systems.len() as u64)?;
                for ods in &frag.obsolescent_delivery_systems {
                    w.write_u(32, ods.obsolescent_delivery_system_id as u64)?;
                    w.write_u(33, ods.time_of_obsolescence_base)?;
                    w.write_zero(6)?;
                    w.write_u(9, ods.time_of_obsolescence_ext as u64)?;
                }
            }
        }
        SatBody::TimeAssociation(b) => {
            w.write_u(4, b.association_type.to_u8() as u64)?;
            if b.association_type.to_u8() == 1 {
                if let Some(ref li) = b.leap_info {
                    w.write_u(1, li.leap59 as u64)?;
                    w.write_u(1, li.leap61 as u64)?;
                    w.write_u(1, li.pastleap59 as u64)?;
                    w.write_u(1, li.pastleap61 as u64)?;
                } else {
                    w.write_zero(4)?;
                }
            } else {
                w.write_zero(4)?;
            }
            w.write_u(33, b.ncr_base)?;
            w.write_zero(6)?;
            w.write_u(9, b.ncr_ext as u64)?;
            w.write_u(64, b.association_timestamp_seconds)?;
            w.write_u(32, b.association_timestamp_nanoseconds as u64)?;
        }
        SatBody::BeamhoppingTimePlan(b) => {
            for plan in &b.plans {
                w.write_u(32, plan.beamhopping_time_plan_id as u64)?;
                w.write_zero(4)?;
                let mode_bits = match &plan.mode {
                    BeamhoppingMode::Mode0 { .. } => 33 + 6 + 9 + 33 + 6 + 9,
                    BeamhoppingMode::Mode1 { bit_map_size, .. } => {
                        let bm = *bit_map_size as usize;
                        let raw = 1 + 15 + 1 + 15 + bm;
                        raw + pad_to_byte(raw)
                    }
                    BeamhoppingMode::Mode2 { .. } => {
                        33 + 6 + 9 + 33 + 6 + 9 + 33 + 6 + 9 + 33 + 6 + 9
                    }
                    BeamhoppingMode::Reserved(v) => v.len() * 8,
                };
                let total_bits_after_length = 8 + 48 + 48 + mode_bits;
                let plan_length_bytes = total_bits_after_length / 8;
                w.write_u(12, plan_length_bytes as u64)?;
                w.write_zero(6)?;
                w.write_u(2, plan.time_plan_mode as u64)?;
                w.write_u(33, plan.time_of_application_base)?;
                w.write_zero(6)?;
                w.write_u(9, plan.time_of_application_ext as u64)?;
                w.write_u(33, plan.cycle_duration_base)?;
                w.write_zero(6)?;
                w.write_u(9, plan.cycle_duration_ext as u64)?;
                match &plan.mode {
                    BeamhoppingMode::Mode0 {
                        dwell_duration_base,
                        dwell_duration_ext,
                        on_time_base,
                        on_time_ext,
                    } => {
                        w.write_u(33, *dwell_duration_base)?;
                        w.write_zero(6)?;
                        w.write_u(9, *dwell_duration_ext as u64)?;
                        w.write_u(33, *on_time_base)?;
                        w.write_zero(6)?;
                        w.write_u(9, *on_time_ext as u64)?;
                    }
                    BeamhoppingMode::Mode1 {
                        bit_map_size,
                        current_slot,
                        slot_transmission_on,
                    } => {
                        w.write_zero(1)?;
                        w.write_u(15, *bit_map_size as u64)?;
                        w.write_zero(1)?;
                        w.write_u(15, *current_slot as u64)?;
                        for &on in slot_transmission_on {
                            w.write_u(1, on as u64)?;
                        }
                        let total = 1 + 15 + 1 + 15 + *bit_map_size as usize;
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
                    } => {
                        w.write_u(33, *grid_size_base)?;
                        w.write_zero(6)?;
                        w.write_u(9, *grid_size_ext as u64)?;
                        w.write_u(33, *revisit_duration_base)?;
                        w.write_zero(6)?;
                        w.write_u(9, *revisit_duration_ext as u64)?;
                        w.write_u(33, *sleep_time_base)?;
                        w.write_zero(6)?;
                        w.write_u(9, *sleep_time_ext as u64)?;
                        w.write_u(33, *sleep_duration_base)?;
                        w.write_zero(6)?;
                        w.write_u(9, *sleep_duration_ext as u64)?;
                    }
                    BeamhoppingMode::Reserved(v) => {
                        for &b in v {
                            w.write_u(8, b as u64)?;
                        }
                    }
                }
            }
        }
        SatBody::PositionV3(b) => {
            w.write_u(4, b.oem_version_major as u64)?;
            w.write_u(4, b.oem_version_minor as u64)?;
            w.write_u(8, b.creation_date_year as u64)?;
            w.write_zero(7)?;
            w.write_u(9, b.creation_date_day as u64)?;
            w.write_u(32, b.creation_date_day_fraction as u64)?;
            for sat in &b.satellites {
                w.write_u(24, sat.satellite_id as u64)?;
                w.write_zero(3)?;
                w.write_u(1, u8::from(sat.metadata.is_some()) as u64)?;
                w.write_u(1, sat.usable_start_time_flag as u64)?;
                w.write_u(1, sat.usable_stop_time_flag as u64)?;
                w.write_u(1, sat.ephemeris_accel_flag as u64)?;
                w.write_u(1, sat.covariance_flag as u64)?;
                if let Some(ref md) = sat.metadata {
                    w.write_u(8, md.total_start_time_year as u64)?;
                    w.write_zero(7)?;
                    w.write_u(9, md.total_start_time_day as u64)?;
                    w.write_u(32, md.total_start_time_day_fraction as u64)?;
                    w.write_u(8, md.total_stop_time_year as u64)?;
                    w.write_zero(7)?;
                    w.write_u(9, md.total_stop_time_day as u64)?;
                    w.write_u(32, md.total_stop_time_day_fraction as u64)?;
                    w.write_zero(1)?;
                    w.write_u(1, md.interpolation_flag as u64)?;
                    w.write_u(3, md.interpolation_type.to_u8() as u64)?;
                    w.write_u(3, md.interpolation_degree as u64)?;
                    if sat.usable_start_time_flag {
                        if let Some(ref ut) = md.usable_start_time {
                            w.write_u(8, ut.year as u64)?;
                            w.write_zero(7)?;
                            w.write_u(9, ut.day as u64)?;
                            w.write_u(32, ut.day_fraction as u64)?;
                        } else {
                            w.write_zero(8)?;
                            w.write_zero(7)?;
                            w.write_zero(9)?;
                            w.write_zero(32)?;
                        }
                    }
                    if sat.usable_stop_time_flag {
                        if let Some(ref ut) = md.usable_stop_time {
                            w.write_u(8, ut.year as u64)?;
                            w.write_zero(7)?;
                            w.write_u(9, ut.day as u64)?;
                            w.write_u(32, ut.day_fraction as u64)?;
                        } else {
                            w.write_zero(8)?;
                            w.write_zero(7)?;
                            w.write_zero(9)?;
                            w.write_zero(32)?;
                        }
                    }
                }
                w.write_u(16, sat.ephemeris_data.len() as u64)?;
                for ed in &sat.ephemeris_data {
                    w.write_u(8, ed.epoch_year as u64)?;
                    w.write_zero(7)?;
                    w.write_u(9, ed.epoch_day as u64)?;
                    w.write_u(32, ed.epoch_day_fraction as u64)?;
                    w.write_u(32, ed.ephemeris_x as u64)?;
                    w.write_u(32, ed.ephemeris_y as u64)?;
                    w.write_u(32, ed.ephemeris_z as u64)?;
                    w.write_u(32, ed.ephemeris_x_dot as u64)?;
                    w.write_u(32, ed.ephemeris_y_dot as u64)?;
                    w.write_u(32, ed.ephemeris_z_dot as u64)?;
                    if sat.ephemeris_accel_flag {
                        if let Some(ref acc) = ed.acceleration {
                            w.write_u(32, acc.ephemeris_x_ddot as u64)?;
                            w.write_u(32, acc.ephemeris_y_ddot as u64)?;
                            w.write_u(32, acc.ephemeris_z_ddot as u64)?;
                        } else {
                            w.write_zero(32)?;
                            w.write_zero(32)?;
                            w.write_zero(32)?;
                        }
                    }
                }
                if sat.covariance_flag {
                    if let Some(ref cov) = sat.covariance {
                        w.write_u(8, cov.covariance_epoch_year as u64)?;
                        w.write_zero(7)?;
                        w.write_u(9, cov.covariance_epoch_day as u64)?;
                        w.write_u(32, cov.covariance_epoch_day_fraction as u64)?;
                        for elem in &cov.covariance_elements {
                            w.write_u(32, *elem as u64)?;
                        }
                    } else {
                        w.write_zero(8)?;
                        w.write_zero(7)?;
                        w.write_zero(9)?;
                        w.write_zero(32)?;
                        for _ in 0..21 {
                            w.write_zero(32)?;
                        }
                    }
                }
            }
        }
        SatBody::Raw(_) => {}
    }
    Ok(())
}

fn sat_body_parse(sat_table_id: u8, data: &[u8]) -> Result<SatBody> {
    if data.is_empty() && sat_table_id <= 4 {
        return Ok(match sat_table_id {
            0 => SatBody::PositionV2(PositionV2Body {
                satellites: Vec::new(),
            }),
            1 => SatBody::CellFragment(CellFragmentBody {
                fragments: Vec::new(),
            }),
            3 => SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody { plans: Vec::new() }),
            _ => {
                return Err(Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "SatSection body (non-loop type requires data)",
                });
            }
        });
    }
    let mut r = BitReader::new(data);
    match sat_table_id {
        0 => {
            let mut satellites = Vec::new();
            while r.remaining_bits() > 24 + 7 {
                let satellite_id = r.read_u(24)? as u32;
                r.skip(7)?;
                let position_system = r.read_u(1)?;
                let position = if position_system == 0 {
                    const ORBITAL_BITS: usize = 16 + 1 + 7;
                    if r.remaining_bits() < ORBITAL_BITS {
                        return Err(Error::BufferTooShort {
                            need: ORBITAL_BITS,
                            have: r.remaining_bits(),
                            what: "SatSection PositionV2 Orbital fields",
                        });
                    }
                    let orbital_position = r.read_u(16)? as u16;
                    let west_east_flag = r.read_u(1)? != 0;
                    r.skip(7)?;
                    PositionSystem::Orbital {
                        orbital_position,
                        west_east_flag,
                    }
                } else {
                    const SGP4_BITS: usize = 8 + 16 + 32 * 10;
                    if r.remaining_bits() < SGP4_BITS {
                        return Err(Error::BufferTooShort {
                            need: SGP4_BITS,
                            have: r.remaining_bits(),
                            what: "SatSection PositionV2 SGP4 fields",
                        });
                    }
                    let epoch_year = r.read_u(8)? as u8;
                    let day_of_the_year = r.read_u(16)? as u16;
                    let day_fraction = r.read_u(32)? as u32;
                    let mean_motion_first_derivative = r.read_u(32)? as u32;
                    let mean_motion_second_derivative = r.read_u(32)? as u32;
                    let drag_term = r.read_u(32)? as u32;
                    let inclination = r.read_u(32)? as u32;
                    let right_ascension = r.read_u(32)? as u32;
                    let eccentricity = r.read_u(32)? as u32;
                    let argument_of_perigree = r.read_u(32)? as u32;
                    let mean_anomaly = r.read_u(32)? as u32;
                    let mean_motion = r.read_u(32)? as u32;
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
                    position,
                });
            }
            Ok(SatBody::PositionV2(PositionV2Body { satellites }))
        }
        1 => {
            let mut fragments = Vec::new();
            while r.remaining_bits() >= 32 + 2 {
                let cell_fragment_id = r.read_u(32)? as u32;
                let first_occurrence = r.read_u(1)? != 0;
                let last_occurrence = r.read_u(1)? != 0;
                let center = if first_occurrence {
                    const CENTER_BITS: usize = 4 + 18 + 5 + 19 + 24 + 6;
                    if r.remaining_bits() < CENTER_BITS {
                        return Err(Error::BufferTooShort {
                            need: CENTER_BITS,
                            have: r.remaining_bits(),
                            what: "SatSection CellFragment center",
                        });
                    }
                    r.skip(4)?;
                    let center_latitude = r.read_i(18)? as i32;
                    r.skip(5)?;
                    let center_longitude = r.read_i(19)? as i32;
                    let max_distance = r.read_u(24)? as u32;
                    r.skip(6)?;
                    Some(CellCenter {
                        center_latitude,
                        center_longitude,
                        max_distance,
                    })
                } else {
                    r.skip(4)?;
                    None
                };
                let dsid_count = r.read_u(10)? as usize;
                if r.remaining_bits() < dsid_count * 32 {
                    return Err(Error::BufferTooShort {
                        need: dsid_count * 32,
                        have: r.remaining_bits(),
                        what: "SatSection CellFragment delivery_system_ids",
                    });
                }
                let mut delivery_system_ids =
                    Vec::with_capacity(dsid_count.min(r.remaining_bits() / 32));
                for _ in 0..dsid_count {
                    delivery_system_ids.push(r.read_u(32)? as u32);
                }
                r.skip(6)?;
                let nds_count = r.read_u(10)? as usize;
                const NDS_ENTRY_BITS: usize = 32 + 33 + 6 + 9;
                if r.remaining_bits() < nds_count * NDS_ENTRY_BITS {
                    return Err(Error::BufferTooShort {
                        need: nds_count * NDS_ENTRY_BITS,
                        have: r.remaining_bits(),
                        what: "SatSection CellFragment new_delivery_systems",
                    });
                }
                let mut new_delivery_systems =
                    Vec::with_capacity(nds_count.min(r.remaining_bits() / NDS_ENTRY_BITS));
                for _ in 0..nds_count {
                    let new_delivery_system_id = r.read_u(32)? as u32;
                    let time_of_application_base = r.read_u(33)?;
                    r.skip(6)?;
                    let time_of_application_ext = r.read_u(9)? as u16;
                    new_delivery_systems.push(NewDeliverySystem {
                        new_delivery_system_id,
                        time_of_application_base,
                        time_of_application_ext,
                    });
                }
                r.skip(6)?;
                let ods_count = r.read_u(10)? as usize;
                if r.remaining_bits() < ods_count * NDS_ENTRY_BITS {
                    return Err(Error::BufferTooShort {
                        need: ods_count * NDS_ENTRY_BITS,
                        have: r.remaining_bits(),
                        what: "SatSection CellFragment obsolescent_delivery_systems",
                    });
                }
                let mut obsolescent_delivery_systems =
                    Vec::with_capacity(ods_count.min(r.remaining_bits() / NDS_ENTRY_BITS));
                for _ in 0..ods_count {
                    let obsolescent_delivery_system_id = r.read_u(32)? as u32;
                    let time_of_obsolescence_base = r.read_u(33)?;
                    r.skip(6)?;
                    let time_of_obsolescence_ext = r.read_u(9)? as u16;
                    obsolescent_delivery_systems.push(ObsolescentDeliverySystem {
                        obsolescent_delivery_system_id,
                        time_of_obsolescence_base,
                        time_of_obsolescence_ext,
                    });
                }
                fragments.push(CellFragment {
                    cell_fragment_id,
                    first_occurrence,
                    last_occurrence,
                    center,
                    delivery_system_ids,
                    new_delivery_systems,
                    obsolescent_delivery_systems,
                });
            }
            Ok(SatBody::CellFragment(CellFragmentBody { fragments }))
        }
        2 => {
            const TIME_ASSOC_MIN_BITS: usize = 4 + 4 + 33 + 6 + 9 + 64 + 32;
            if r.remaining_bits() < TIME_ASSOC_MIN_BITS {
                return Err(Error::BufferTooShort {
                    need: TIME_ASSOC_MIN_BITS,
                    have: r.remaining_bits(),
                    what: "SatSection TimeAssociation body",
                });
            }
            let association_type = AssociationType::from_u8(r.read_u(4)? as u8);
            let leap_info = if association_type.to_u8() == 1 {
                Some(LeapInfo {
                    leap59: r.read_u(1)? != 0,
                    leap61: r.read_u(1)? != 0,
                    pastleap59: r.read_u(1)? != 0,
                    pastleap61: r.read_u(1)? != 0,
                })
            } else {
                r.skip(4)?;
                None
            };
            let ncr_base = r.read_u(33)?;
            r.skip(6)?;
            let ncr_ext = r.read_u(9)? as u16;
            let association_timestamp_seconds = r.read_u(64)?;
            let association_timestamp_nanoseconds = r.read_u(32)? as u32;
            Ok(SatBody::TimeAssociation(TimeAssociationBody {
                association_type,
                leap_info,
                ncr_base,
                ncr_ext,
                association_timestamp_seconds,
                association_timestamp_nanoseconds,
            }))
        }
        3 => {
            let mut plans = Vec::new();
            while r.remaining_bits() >= 32 + 4 + 12 {
                let beamhopping_time_plan_id = r.read_u(32)? as u32;
                r.skip(4)?;
                let plan_length = r.read_u(12)? as usize;
                let plan_end_bits = r.bits_consumed() + plan_length * 8;
                r.skip(6)?;
                let time_plan_mode = r.read_u(2)? as u8;
                let time_of_application_base = r.read_u(33)?;
                r.skip(6)?;
                let time_of_application_ext = r.read_u(9)? as u16;
                let cycle_duration_base = r.read_u(33)?;
                r.skip(6)?;
                let cycle_duration_ext = r.read_u(9)? as u16;
                let mode = match time_plan_mode {
                    0 => {
                        const MODE0_BITS: usize = 33 + 6 + 9 + 33 + 6 + 9;
                        if r.remaining_bits() < MODE0_BITS {
                            return Err(Error::BufferTooShort {
                                need: MODE0_BITS,
                                have: r.remaining_bits(),
                                what: "SatSection Beamhopping Mode0",
                            });
                        }
                        let dwell_duration_base = r.read_u(33)?;
                        r.skip(6)?;
                        let dwell_duration_ext = r.read_u(9)? as u16;
                        let on_time_base = r.read_u(33)?;
                        r.skip(6)?;
                        let on_time_ext = r.read_u(9)? as u16;
                        BeamhoppingMode::Mode0 {
                            dwell_duration_base,
                            dwell_duration_ext,
                            on_time_base,
                            on_time_ext,
                        }
                    }
                    1 => {
                        const MODE1_HEADER_BITS: usize = 1 + 15 + 1 + 15;
                        if r.remaining_bits() < MODE1_HEADER_BITS {
                            return Err(Error::BufferTooShort {
                                need: MODE1_HEADER_BITS,
                                have: r.remaining_bits(),
                                what: "SatSection Beamhopping Mode1 header",
                            });
                        }
                        r.skip(1)?;
                        let bit_map_size = r.read_u(15)? as u16;
                        r.skip(1)?;
                        let current_slot = r.read_u(15)? as u16;
                        if r.remaining_bits() < bit_map_size as usize {
                            return Err(Error::BufferTooShort {
                                need: bit_map_size as usize,
                                have: r.remaining_bits(),
                                what: "SatSection Beamhopping Mode1 bitmap",
                            });
                        }
                        let mut slot_transmission_on =
                            Vec::with_capacity((bit_map_size as usize).min(r.remaining_bits()));
                        for _ in 0..bit_map_size {
                            slot_transmission_on.push(r.read_u(1)? != 0);
                        }
                        let total = 1 + 15 + 1 + 15 + bit_map_size as usize;
                        r.skip(pad_to_byte(total) as u8)?;
                        BeamhoppingMode::Mode1 {
                            bit_map_size,
                            current_slot,
                            slot_transmission_on,
                        }
                    }
                    2 => {
                        const MODE2_BITS: usize = 33 + 6 + 9 + 33 + 6 + 9 + 33 + 6 + 9 + 33 + 6 + 9;
                        if r.remaining_bits() < MODE2_BITS {
                            return Err(Error::BufferTooShort {
                                need: MODE2_BITS,
                                have: r.remaining_bits(),
                                what: "SatSection Beamhopping Mode2",
                            });
                        }
                        let grid_size_base = r.read_u(33)?;
                        r.skip(6)?;
                        let grid_size_ext = r.read_u(9)? as u16;
                        let revisit_duration_base = r.read_u(33)?;
                        r.skip(6)?;
                        let revisit_duration_ext = r.read_u(9)? as u16;
                        let sleep_time_base = r.read_u(33)?;
                        r.skip(6)?;
                        let sleep_time_ext = r.read_u(9)? as u16;
                        let sleep_duration_base = r.read_u(33)?;
                        r.skip(6)?;
                        let sleep_duration_ext = r.read_u(9)? as u16;
                        BeamhoppingMode::Mode2 {
                            grid_size_base,
                            grid_size_ext,
                            revisit_duration_base,
                            revisit_duration_ext,
                            sleep_time_base,
                            sleep_time_ext,
                            sleep_duration_base,
                            sleep_duration_ext,
                        }
                    }
                    _ => {
                        let start_byte = r.bits_consumed().div_ceil(8);
                        let end_byte = plan_end_bits / 8;
                        let raw = if start_byte < end_byte && end_byte <= data.len() {
                            data[start_byte..end_byte].to_vec()
                        } else {
                            Vec::new()
                        };
                        BeamhoppingMode::Reserved(raw)
                    }
                };
                r.bit_pos = plan_end_bits;
                plans.push(BeamhoppingPlan {
                    beamhopping_time_plan_id,
                    time_plan_mode,
                    time_of_application_base,
                    time_of_application_ext,
                    cycle_duration_base,
                    cycle_duration_ext,
                    mode,
                });
            }
            Ok(SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
                plans,
            }))
        }
        4 => {
            const POS_V3_HEADER_BITS: usize = 4 + 4 + 8 + 7 + 9 + 32;
            if r.remaining_bits() < POS_V3_HEADER_BITS {
                return Err(Error::BufferTooShort {
                    need: POS_V3_HEADER_BITS,
                    have: r.remaining_bits(),
                    what: "SatSection PositionV3 body header",
                });
            }
            let oem_version_major = r.read_u(4)? as u8;
            let oem_version_minor = r.read_u(4)? as u8;
            let creation_date_year = r.read_u(8)? as u8;
            r.skip(7)?;
            let creation_date_day = r.read_u(9)? as u16;
            let creation_date_day_fraction = r.read_u(32)? as u32;
            let mut satellites = Vec::new();
            while r.remaining_bits() >= 24 + 3 + 5 {
                let satellite_id = r.read_u(24)? as u32;
                r.skip(3)?;
                let metadata_flag = r.read_u(1)? != 0;
                let usable_start_time_flag = r.read_u(1)? != 0;
                let usable_stop_time_flag = r.read_u(1)? != 0;
                let ephemeris_accel_flag = r.read_u(1)? != 0;
                let covariance_flag = r.read_u(1)? != 0;
                let metadata = if metadata_flag {
                    const METADATA_FIXED_BITS: usize =
                        8 + 7 + 9 + 32 + 8 + 7 + 9 + 32 + 1 + 1 + 3 + 3;
                    if r.remaining_bits() < METADATA_FIXED_BITS {
                        return Err(Error::BufferTooShort {
                            need: METADATA_FIXED_BITS,
                            have: r.remaining_bits(),
                            what: "SatSection PositionV3 metadata",
                        });
                    }
                    let total_start_time_year = r.read_u(8)? as u8;
                    r.skip(7)?;
                    let total_start_time_day = r.read_u(9)? as u16;
                    let total_start_time_day_fraction = r.read_u(32)? as u32;
                    let total_stop_time_year = r.read_u(8)? as u8;
                    r.skip(7)?;
                    let total_stop_time_day = r.read_u(9)? as u16;
                    let total_stop_time_day_fraction = r.read_u(32)? as u32;
                    r.skip(1)?;
                    let interpolation_flag = r.read_u(1)? != 0;
                    let interpolation_type = InterpolationType::from_u8(r.read_u(3)? as u8);
                    let interpolation_degree = r.read_u(3)? as u8;
                    let usable_start_time = if usable_start_time_flag {
                        const USABLE_TIME_BITS: usize = 8 + 7 + 9 + 32;
                        if r.remaining_bits() < USABLE_TIME_BITS {
                            return Err(Error::BufferTooShort {
                                need: USABLE_TIME_BITS,
                                have: r.remaining_bits(),
                                what: "SatSection PositionV3 usable_start_time",
                            });
                        }
                        let year = r.read_u(8)? as u8;
                        r.skip(7)?;
                        let day = r.read_u(9)? as u16;
                        let day_fraction = r.read_u(32)? as u32;
                        Some(UsableTime {
                            year,
                            day,
                            day_fraction,
                        })
                    } else {
                        None
                    };
                    let usable_stop_time = if usable_stop_time_flag {
                        const USABLE_TIME_BITS: usize = 8 + 7 + 9 + 32;
                        if r.remaining_bits() < USABLE_TIME_BITS {
                            return Err(Error::BufferTooShort {
                                need: USABLE_TIME_BITS,
                                have: r.remaining_bits(),
                                what: "SatSection PositionV3 usable_stop_time",
                            });
                        }
                        let year = r.read_u(8)? as u8;
                        r.skip(7)?;
                        let day = r.read_u(9)? as u16;
                        let day_fraction = r.read_u(32)? as u32;
                        Some(UsableTime {
                            year,
                            day,
                            day_fraction,
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
                    })
                } else {
                    None
                };
                let ephemeris_data_count = r.read_u(16)? as u16;
                let entry_bits: usize =
                    8 + 7 + 9 + 32 + 32 * 6 + if ephemeris_accel_flag { 32 * 3 } else { 0 };
                let mut ephemeris_data = Vec::with_capacity(
                    (ephemeris_data_count as usize)
                        .min(r.remaining_bits().saturating_sub(entry_bits) / entry_bits + 1),
                );
                for _ in 0..ephemeris_data_count {
                    if r.remaining_bits() < entry_bits {
                        return Err(Error::BufferTooShort {
                            need: entry_bits,
                            have: r.remaining_bits(),
                            what: "SatSection PositionV3 ephemeris_data entry",
                        });
                    }
                    let epoch_year = r.read_u(8)? as u8;
                    r.skip(7)?;
                    let epoch_day = r.read_u(9)? as u16;
                    let epoch_day_fraction = r.read_u(32)? as u32;
                    let ephemeris_x = r.read_u(32)? as u32;
                    let ephemeris_y = r.read_u(32)? as u32;
                    let ephemeris_z = r.read_u(32)? as u32;
                    let ephemeris_x_dot = r.read_u(32)? as u32;
                    let ephemeris_y_dot = r.read_u(32)? as u32;
                    let ephemeris_z_dot = r.read_u(32)? as u32;
                    let acceleration = if ephemeris_accel_flag {
                        Some(EphemerisAccel {
                            ephemeris_x_ddot: r.read_u(32)? as u32,
                            ephemeris_y_ddot: r.read_u(32)? as u32,
                            ephemeris_z_ddot: r.read_u(32)? as u32,
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
                    });
                }
                let covariance = if covariance_flag {
                    const COV_HEADER_BITS: usize = 8 + 7 + 9 + 32;
                    const COV_ELEMENTS_BITS: usize = 21 * 32;
                    const COV_BITS: usize = COV_HEADER_BITS + COV_ELEMENTS_BITS;
                    if r.remaining_bits() < COV_BITS {
                        return Err(Error::BufferTooShort {
                            need: COV_BITS,
                            have: r.remaining_bits(),
                            what: "SatSection PositionV3 covariance",
                        });
                    }
                    let covariance_epoch_year = r.read_u(8)? as u8;
                    r.skip(7)?;
                    let covariance_epoch_day = r.read_u(9)? as u16;
                    let covariance_epoch_day_fraction = r.read_u(32)? as u32;
                    let mut covariance_elements = [0u32; 21];
                    for elem in &mut covariance_elements {
                        *elem = r.read_u(32)? as u32;
                    }
                    Some(CovarianceData {
                        covariance_epoch_year,
                        covariance_epoch_day,
                        covariance_epoch_day_fraction,
                        covariance_elements,
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
                });
            }
            Ok(SatBody::PositionV3(PositionV3Body {
                oem_version_major,
                oem_version_minor,
                creation_date_year,
                creation_date_day,
                creation_date_day_fraction,
                satellites,
            }))
        }
        _ => Ok(SatBody::Raw(data.to_vec())),
    }
}

// ── SatSection ──────────────────────────────────────────────────────────────

/// Satellite Access Table section (EN 300 468 §5.2.11.1, Table 11a).
///
/// The body is typed as [`SatBody`], selected by `satellite_table_id`.
/// All body fields are owned numeric values; the section does not borrow
/// from the input buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SatSection {
    /// 6-bit discriminant selecting the body structure (see [`SatTableId`]).
    pub satellite_table_id: u8,
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
}

impl SatSection {
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
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "SatSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "SatSection",
                expected: &[TABLE_ID],
            });
        }
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = super::check_section_length(
            bytes.len(),
            SECTION_LENGTH_PREFIX,
            section_length,
            HEADER_LEN + CRC_LEN,
        )?;
        let satellite_table_id = bytes[3] >> 2;
        let private_indicator = (bytes[1] & 0x40) != 0;
        let table_count = (((bytes[3] & 0x03) as u16) << 8) | bytes[4] as u16;
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = bytes[5] & 0x01 != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let body_data = &bytes[HEADER_LEN..total - CRC_LEN];
        let body = sat_body_parse(satellite_table_id, body_data)?;
        Ok(SatSection {
            satellite_table_id,
            private_indicator,
            table_count,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            body,
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
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length_usize = len - SECTION_LENGTH_PREFIX;
        if section_length_usize > 0x0FFF {
            return Err(Error::SectionLengthOverflow {
                declared: section_length_usize,
                available: 0x0FFF,
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
        buf[1] = super::SECTION_B1_SSI
            | (u8::from(self.private_indicator) << 6)
            | super::SECTION_B1_RESERVED_HI
            | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3] = (self.satellite_table_id << 2) | ((self.table_count >> 8) as u8 & 0x03);
        buf[4] = (self.table_count & 0xFF) as u8;
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8] = 0x00;
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
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..body_end]);
        buf[body_end..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for SatSection {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "SATELLITE_ACCESS";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_sat(stid: u8, table_count: u16, body: &SatBody) -> Vec<u8> {
        let sat = SatSection {
            satellite_table_id: stid,
            private_indicator: true,
            table_count,
            version_number: 5,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            body: body.clone(),
        };
        let mut buf = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf).unwrap();
        buf
    }

    fn build_sat_private_indicator_false(stid: u8, body: &SatBody) -> Vec<u8> {
        let sat = SatSection {
            satellite_table_id: stid,
            private_indicator: false,
            table_count: 0,
            version_number: 5,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            body: body.clone(),
        };
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
    fn private_indicator_false_round_trip() {
        let body = SatBody::TimeAssociation(TimeAssociationBody {
            association_type: AssociationType::UtcWithoutLeap,
            leap_info: None,
            ncr_base: 0,
            ncr_ext: 0,
            association_timestamp_seconds: 0,
            association_timestamp_nanoseconds: 0,
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
                },
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
                time_plan_mode: 0,
                time_of_application_base: 0x0000_AAAA_AAAA,
                time_of_application_ext: 0x100,
                cycle_duration_base: 0x0000_5555_5555,
                cycle_duration_ext: 0x080,
                mode: BeamhoppingMode::Mode0 {
                    dwell_duration_base: 0x0000_1111_1111,
                    dwell_duration_ext: 0x111,
                    on_time_base: 0x0000_2222_2222,
                    on_time_ext: 0x222,
                },
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 1);
                assert_eq!(bhp.plans[0].beamhopping_time_plan_id, 0xDEADBEEF);
                assert_eq!(bhp.plans[0].time_plan_mode, 0);
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
            Error::BufferTooShort {
                what: "SatSection",
                ..
            }
        ));
    }

    #[test]
    fn serialize_round_trip_raw() {
        let body_data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let sat = SatSection {
            satellite_table_id: 10,
            private_indicator: true,
            table_count: 0x2FF,
            version_number: 5,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            body: SatBody::Raw(body_data.clone()),
        };
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
        let crc = dvb_common::crc32_mpeg2::compute(&bytes);
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
                    time_plan_mode: 0,
                    time_of_application_base: 0x0000_AAAA_AAAA,
                    time_of_application_ext: 0x100,
                    cycle_duration_base: 0x0000_5555_5555,
                    cycle_duration_ext: 0x080,
                    mode: BeamhoppingMode::Mode0 {
                        dwell_duration_base: 0x0000_1111_1111,
                        dwell_duration_ext: 0x111,
                        on_time_base: 0x0000_2222_2222,
                        on_time_ext: 0x222,
                    },
                },
                BeamhoppingPlan {
                    beamhopping_time_plan_id: 0x22222222,
                    time_plan_mode: 0,
                    time_of_application_base: 0x0000_BBBB_BBBB,
                    time_of_application_ext: 0x200,
                    cycle_duration_base: 0x0000_6666_6666,
                    cycle_duration_ext: 0x090,
                    mode: BeamhoppingMode::Mode0 {
                        dwell_duration_base: 0x0000_3333_3333,
                        dwell_duration_ext: 0x333,
                        on_time_base: 0x0000_4444_4444,
                        on_time_ext: 0x444,
                    },
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
                    }),
                    usable_stop_time: None,
                }),
                ephemeris_data: Vec::new(),
                covariance: None,
            }],
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
                center: Some(CellCenter {
                    center_latitude: 1000,
                    center_longitude: -2000,
                    max_distance: 500000,
                }),
                delivery_system_ids: vec![0x55667788],
                new_delivery_systems: vec![NewDeliverySystem {
                    new_delivery_system_id: 0xAABBCCDD,
                    time_of_application_base: 0x0000_1234_5678,
                    time_of_application_ext: 0x100,
                }],
                obsolescent_delivery_systems: vec![ObsolescentDeliverySystem {
                    obsolescent_delivery_system_id: 0xEEFF0011,
                    time_of_obsolescence_base: 0x0000_9ABC_DEF0,
                    time_of_obsolescence_ext: 0x1FF,
                }],
            }],
        });
        let bytes = build_sat(1, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::CellFragment(cf) => {
                assert_eq!(cf.fragments.len(), 1);
                assert_eq!(cf.fragments[0].cell_fragment_id, 0x11223344);
                assert!(cf.fragments[0].first_occurrence);
                assert!(cf.fragments[0].center.is_some());
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
                time_plan_mode: 1,
                time_of_application_base: 0x0000_AAAA_AAAA,
                time_of_application_ext: 0x100,
                cycle_duration_base: 0x0000_5555_5555,
                cycle_duration_ext: 0x080,
                mode: BeamhoppingMode::Mode1 {
                    bit_map_size: 8,
                    current_slot: 3,
                    slot_transmission_on: vec![true, false, true, true, false, false, true, false],
                },
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 1);
                assert_eq!(bhp.plans[0].time_plan_mode, 1);
                match &bhp.plans[0].mode {
                    BeamhoppingMode::Mode1 {
                        bit_map_size,
                        current_slot,
                        slot_transmission_on,
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
                time_plan_mode: 2,
                time_of_application_base: 0x0000_BBBB_BBBB,
                time_of_application_ext: 0x200,
                cycle_duration_base: 0x0000_6666_6666,
                cycle_duration_ext: 0x090,
                mode: BeamhoppingMode::Mode2 {
                    grid_size_base: 0x0000_1111_1111,
                    grid_size_ext: 0x111,
                    revisit_duration_base: 0x0000_2222_2222,
                    revisit_duration_ext: 0x222,
                    sleep_time_base: 0x0000_3333_3333,
                    sleep_time_ext: 0x333,
                    sleep_duration_base: 0x0000_4444_4444,
                    sleep_duration_ext: 0x444,
                },
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 1);
                assert_eq!(bhp.plans[0].time_plan_mode, 2);
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
                    time_plan_mode: 0,
                    time_of_application_base: 0x0000_AAAA_AAAA,
                    time_of_application_ext: 0x100,
                    cycle_duration_base: 0x0000_5555_5555,
                    cycle_duration_ext: 0x080,
                    mode: BeamhoppingMode::Mode0 {
                        dwell_duration_base: 0x0000_1111_1111,
                        dwell_duration_ext: 0x111,
                        on_time_base: 0x0000_2222_2222,
                        on_time_ext: 0x222,
                    },
                },
                BeamhoppingPlan {
                    beamhopping_time_plan_id: 0x22222222,
                    time_plan_mode: 3,
                    time_of_application_base: 0x0000_CCCC_CCCC,
                    time_of_application_ext: 0x300,
                    cycle_duration_base: 0x0000_DDDD_DDDD,
                    cycle_duration_ext: 0x400,
                    mode: BeamhoppingMode::Reserved(vec![0xAA, 0xBB, 0xCC]),
                },
            ],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans.len(), 2);
                assert_eq!(bhp.plans[0].time_plan_mode, 0);
                assert_eq!(bhp.plans[1].time_plan_mode, 3);
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
                center: None,
                delivery_system_ids: vec![0x11111111],
                new_delivery_systems: Vec::new(),
                obsolescent_delivery_systems: Vec::new(),
            }],
        });
        let bytes = build_sat(1, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        let mut buf2 = vec![0u8; sat.serialized_len()];
        sat.serialize_into(&mut buf2).unwrap();
        assert_eq!(bytes, buf2);

        let corrupt_sat = SatSection {
            satellite_table_id: 1,
            private_indicator: true,
            table_count: 0,
            version_number: 5,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            body: SatBody::CellFragment(CellFragmentBody {
                fragments: vec![CellFragment {
                    cell_fragment_id: 1,
                    first_occurrence: false,
                    last_occurrence: false,
                    center: None,
                    delivery_system_ids: vec![0x11111111; 50],
                    new_delivery_systems: Vec::new(),
                    obsolescent_delivery_systems: Vec::new(),
                }],
            }),
        };
        let mut corrupt_buf = vec![0u8; corrupt_sat.serialized_len()];
        corrupt_sat.serialize_into(&mut corrupt_buf).unwrap();
        let section_length = (corrupt_buf.len() - SECTION_LENGTH_PREFIX) as u16;
        corrupt_buf[1] = 0x80 | 0x40 | 0x30 | ((section_length >> 8) as u8 & 0x0F);
        corrupt_buf[2] = (section_length & 0xFF) as u8;
        let crc_end = corrupt_buf.len();
        let crc = dvb_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
        corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
        let original_len = corrupt_buf.len();
        corrupt_buf.truncate(original_len - 100);
        let sl = (corrupt_buf.len() - SECTION_LENGTH_PREFIX) as u16;
        corrupt_buf[1] = (corrupt_buf[1] & 0xF0) | ((sl >> 8) as u8 & 0x0F);
        corrupt_buf[2] = (sl & 0xFF) as u8;
        let crc_end = corrupt_buf.len();
        let crc2 = dvb_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
        corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc2.to_be_bytes());
        assert!(SatSection::parse(&corrupt_buf).is_err());
    }

    #[test]
    fn beamhopping_mode1_truncated_bit_map_size() {
        let corrupt_sat = SatSection {
            satellite_table_id: 3,
            private_indicator: true,
            table_count: 0,
            version_number: 5,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            body: SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody {
                plans: vec![BeamhoppingPlan {
                    beamhopping_time_plan_id: 1,
                    time_plan_mode: 1,
                    time_of_application_base: 0,
                    time_of_application_ext: 0,
                    cycle_duration_base: 0,
                    cycle_duration_ext: 0,
                    mode: BeamhoppingMode::Mode1 {
                        bit_map_size: 200,
                        current_slot: 0,
                        slot_transmission_on: vec![true; 200],
                    },
                }],
            }),
        };
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
            let crc = dvb_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
            corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
            assert!(SatSection::parse(&corrupt_buf).is_err());
        }
    }

    #[test]
    fn position_v3_truncated_ephemeris_data_count() {
        let corrupt_sat = SatSection {
            satellite_table_id: 4,
            private_indicator: true,
            table_count: 0,
            version_number: 5,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            body: SatBody::PositionV3(PositionV3Body {
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
                        };
                        5
                    ],
                    covariance: None,
                }],
            }),
        };
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
            let crc = dvb_common::crc32_mpeg2::compute(&corrupt_buf[..crc_end - CRC_LEN]);
            corrupt_buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
            assert!(SatSection::parse(&corrupt_buf).is_err());
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
                },
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
                center: None,
                delivery_system_ids: Vec::new(),
                new_delivery_systems: Vec::new(),
                obsolescent_delivery_systems: Vec::new(),
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
                time_plan_mode: 0,
                time_of_application_base: 0,
                time_of_application_ext: 0,
                cycle_duration_base: 0,
                cycle_duration_ext: 0,
                mode: BeamhoppingMode::Mode0 {
                    dwell_duration_base: 0,
                    dwell_duration_ext: 0,
                    on_time_base: 0,
                    on_time_ext: 0,
                },
            }],
        });
        let bytes = build_sat(3, 0, &body);
        let sat = SatSection::parse(&bytes).unwrap();
        match &sat.body {
            SatBody::BeamhoppingTimePlan(bhp) => {
                assert_eq!(bhp.plans[0].beamhopping_time_plan_id, 0xDEADBEEF);
                assert_eq!(bhp.plans[0].time_plan_mode, 0);
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
        let crc = dvb_common::crc32_mpeg2::compute(&v);
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
        // Bit breakdown in body (one fragment, no center, empty lists):
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
                assert!(f.center.is_none());
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
                assert_eq!(p.time_plan_mode, 0);
                assert_eq!(p.time_of_application_base, 0);
                assert_eq!(p.cycle_duration_base, 0);
                match &p.mode {
                    BeamhoppingMode::Mode0 {
                        dwell_duration_base,
                        dwell_duration_ext,
                        on_time_base,
                        on_time_ext,
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
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_end - CRC_LEN]);
        buf[crc_end - CRC_LEN..crc_end].copy_from_slice(&crc.to_be_bytes());
        assert!(SatSection::parse(&buf).is_err());
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
}
