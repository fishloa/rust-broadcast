//! Satellite Access Table (SAT) — ETSI EN 300 468 §5.2.11.
//!
//! Long-form private section on PID 0x001B with table_id 0x4D. The SAT is a
//! *family*: a common `satellite_access_section()` header carries a 6-bit
//! `satellite_table_id` discriminant ([`SatTableId`]) that selects one of five
//! body structures (position v2, cell fragment, time association, beamhopping
//! time plan, position v3).
//!
//! The body is now fully typed as [`SatBody`] — an enum with one variant per
//! defined layout plus a [`SatBody::Raw`] fallthrough for reserved
//! `satellite_table_id` values 5–63. All five layouts use bit-packed fields; a
//! private bit-level reader/writer handles the extraction and emission.

use crate::error::{Error, Result};
use crate::traits::Table;
use dvb_common::{Parse, Serialize};

/// table_id for the Satellite Access Table.
pub const TABLE_ID: u8 = 0x4D;
/// Well-known PID on which the SAT is carried (EN 300 468 Table 1, §5.1.3).
pub const PID: u16 = 0x001B;

const HEADER_LEN: usize = 9;
const SECTION_LENGTH_PREFIX: usize = 3;
const CRC_LEN: usize = 4;

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
        self.data.len().saturating_sub(self.bit_pos / 8) * 8 - (self.bit_pos % 8)
    }
    fn bits_consumed(&self) -> usize {
        self.bit_pos
    }
    fn read_u(&mut self, bits: u8) -> u64 {
        let bits = bits as usize;
        let mut val: u64 = 0;
        for i in 0..bits {
            let byte_idx = (self.bit_pos + i) / 8;
            let bit_idx = 7 - ((self.bit_pos + i) % 8);
            if byte_idx < self.data.len() {
                val = (val << 1) | ((self.data[byte_idx] >> bit_idx) & 1) as u64;
            }
        }
        self.bit_pos += bits;
        val
    }
    fn read_i(&mut self, bits: u8) -> i64 {
        let raw = self.read_u(bits);
        let bits = bits as usize;
        if raw & (1u64 << (bits - 1)) != 0 {
            (raw as i64) | (!0i64 << bits)
        } else {
            raw as i64
        }
    }
    fn skip(&mut self, bits: u8) {
        self.bit_pos += bits as usize;
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
    fn write_u(&mut self, bits: u8, val: u64) {
        let bits = bits as usize;
        for i in 0..bits {
            let byte_idx = (self.bit_pos + i) / 8;
            let bit_idx = 7 - ((self.bit_pos + i) % 8);
            if byte_idx < self.buf.len() {
                let bit_val = ((val >> (bits - 1 - i)) & 1) as u8;
                self.buf[byte_idx] |= bit_val << bit_idx;
            }
        }
        self.bit_pos += bits;
    }
    fn write_i(&mut self, bits: u8, val: i64) {
        self.write_u(bits, val as u64 & ((1u64 << bits) - 1));
    }
    fn write_zero(&mut self, bits: u8) {
        self.bit_pos += bits as usize;
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
    pub association_type: u8,
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
    /// `interpolation_type` (3 bits, Table 11i).
    pub interpolation_type: u8,
    /// `interpolation_degree` (3 bits).
    pub interpolation_degree: u8,
    /// Usable start time (optional).
    pub usable_start_time: Option<UsableTime>,
    /// Usable stop time (optional).
    pub usable_stop_time: Option<UsableTime>,
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
    /// `metadata_flag`.
    pub metadata_flag: bool,
    /// `usable_start_time_flag`.
    pub usable_start_time_flag: bool,
    /// `usable_stop_time_flag`.
    pub usable_stop_time_flag: bool,
    /// `ephemeris_accel_flag`.
    pub ephemeris_accel_flag: bool,
    /// `covariance_flag`.
    pub covariance_flag: bool,
    /// Metadata block (optional).
    pub metadata: Option<PositionV3Metadata>,
    /// `ephemeris_data_count` (16 bits).
    pub ephemeris_data_count: u16,
    /// Ephemeris data entries.
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
            let mut tmp = vec![0u8; 0];
            let mut writer = BitWriter::new(&mut tmp);
            sat_body_write(body, &mut writer, true);
            writer.bits_written().div_ceil(8)
        }
    }
}

fn sat_body_write(body: &SatBody, w: &mut BitWriter, count_only: bool) {
    if !count_only {
        let len = sat_body_serialized_len(body);
        let needed = len.div_ceil(8) * 8;
        if w.buf.len() * 8 < w.bit_pos + needed {
            return;
        }
    }
    match body {
        SatBody::PositionV2(b) => {
            for sat in &b.satellites {
                w.write_u(24, sat.satellite_id as u64);
                w.write_zero(7);
                match &sat.position {
                    PositionSystem::Orbital {
                        orbital_position,
                        west_east_flag,
                    } => {
                        w.write_u(1, 0);
                        w.write_u(16, *orbital_position as u64);
                        w.write_u(1, *west_east_flag as u64);
                        w.write_zero(7);
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
                        w.write_u(1, 1);
                        w.write_u(8, *epoch_year as u64);
                        w.write_u(16, *day_of_the_year as u64);
                        w.write_u(32, *day_fraction as u64);
                        w.write_u(32, *mean_motion_first_derivative as u64);
                        w.write_u(32, *mean_motion_second_derivative as u64);
                        w.write_u(32, *drag_term as u64);
                        w.write_u(32, *inclination as u64);
                        w.write_u(32, *right_ascension as u64);
                        w.write_u(32, *eccentricity as u64);
                        w.write_u(32, *argument_of_perigree as u64);
                        w.write_u(32, *mean_anomaly as u64);
                        w.write_u(32, *mean_motion as u64);
                    }
                }
            }
        }
        SatBody::CellFragment(b) => {
            for frag in &b.fragments {
                w.write_u(32, frag.cell_fragment_id as u64);
                w.write_u(1, frag.first_occurrence as u64);
                w.write_u(1, frag.last_occurrence as u64);
                if frag.first_occurrence {
                    if let Some(ref c) = frag.center {
                        w.write_zero(4);
                        w.write_i(18, c.center_latitude as i64);
                        w.write_zero(5);
                        w.write_i(19, c.center_longitude as i64);
                        w.write_u(24, c.max_distance as u64);
                        w.write_zero(6);
                    }
                } else {
                    w.write_zero(4);
                }
                w.write_u(10, frag.delivery_system_ids.len() as u64);
                for id in &frag.delivery_system_ids {
                    w.write_u(32, *id as u64);
                }
                w.write_zero(6);
                w.write_u(10, frag.new_delivery_systems.len() as u64);
                for nds in &frag.new_delivery_systems {
                    w.write_u(32, nds.new_delivery_system_id as u64);
                    w.write_u(33, nds.time_of_application_base);
                    w.write_zero(6);
                    w.write_u(9, nds.time_of_application_ext as u64);
                }
                w.write_zero(6);
                w.write_u(10, frag.obsolescent_delivery_systems.len() as u64);
                for ods in &frag.obsolescent_delivery_systems {
                    w.write_u(32, ods.obsolescent_delivery_system_id as u64);
                    w.write_u(33, ods.time_of_obsolescence_base);
                    w.write_zero(6);
                    w.write_u(9, ods.time_of_obsolescence_ext as u64);
                }
            }
        }
        SatBody::TimeAssociation(b) => {
            w.write_u(4, b.association_type as u64);
            if b.association_type == 1 {
                if let Some(ref li) = b.leap_info {
                    w.write_u(1, li.leap59 as u64);
                    w.write_u(1, li.leap61 as u64);
                    w.write_u(1, li.pastleap59 as u64);
                    w.write_u(1, li.pastleap61 as u64);
                }
            } else {
                w.write_zero(4);
            }
            w.write_u(33, b.ncr_base);
            w.write_zero(6);
            w.write_u(9, b.ncr_ext as u64);
            w.write_u(64, b.association_timestamp_seconds);
            w.write_u(32, b.association_timestamp_nanoseconds as u64);
        }
        SatBody::BeamhoppingTimePlan(b) => {
            for plan in &b.plans {
                w.write_u(32, plan.beamhopping_time_plan_id as u64);
                w.write_zero(4);
                let mode_bits = match &plan.mode {
                    BeamhoppingMode::Mode0 { .. } => 33 + 6 + 9 + 33 + 6 + 9,
                    BeamhoppingMode::Mode1 { bit_map_size, .. } => {
                        1 + 15 + 1 + 15 + *bit_map_size as usize
                    }
                    BeamhoppingMode::Mode2 { .. } => {
                        33 + 6 + 9 + 33 + 6 + 9 + 33 + 6 + 9 + 33 + 6 + 9
                    }
                };
                w.write_u(12, (mode_bits as u64) & 0xFFF);
                w.write_zero(6);
                w.write_u(2, plan.time_plan_mode as u64);
                w.write_u(33, plan.time_of_application_base);
                w.write_zero(6);
                w.write_u(9, plan.time_of_application_ext as u64);
                w.write_u(33, plan.cycle_duration_base);
                w.write_zero(6);
                w.write_u(9, plan.cycle_duration_ext as u64);
                match &plan.mode {
                    BeamhoppingMode::Mode0 {
                        dwell_duration_base,
                        dwell_duration_ext,
                        on_time_base,
                        on_time_ext,
                    } => {
                        w.write_u(33, *dwell_duration_base);
                        w.write_zero(6);
                        w.write_u(9, *dwell_duration_ext as u64);
                        w.write_u(33, *on_time_base);
                        w.write_zero(6);
                        w.write_u(9, *on_time_ext as u64);
                    }
                    BeamhoppingMode::Mode1 {
                        bit_map_size,
                        current_slot,
                        slot_transmission_on,
                    } => {
                        w.write_zero(1);
                        w.write_u(15, *bit_map_size as u64);
                        w.write_zero(1);
                        w.write_u(15, *current_slot as u64);
                        for &on in slot_transmission_on {
                            w.write_u(1, on as u64);
                        }
                        let total = 1 + 15 + 1 + 15 + *bit_map_size as usize;
                        let pad = (8 - (total % 8)) % 8;
                        for _ in 0..pad {
                            w.write_zero(1);
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
                        w.write_u(33, *grid_size_base);
                        w.write_zero(6);
                        w.write_u(9, *grid_size_ext as u64);
                        w.write_u(33, *revisit_duration_base);
                        w.write_zero(6);
                        w.write_u(9, *revisit_duration_ext as u64);
                        w.write_u(33, *sleep_time_base);
                        w.write_zero(6);
                        w.write_u(9, *sleep_time_ext as u64);
                        w.write_u(33, *sleep_duration_base);
                        w.write_zero(6);
                        w.write_u(9, *sleep_duration_ext as u64);
                    }
                }
            }
        }
        SatBody::PositionV3(b) => {
            w.write_u(4, b.oem_version_major as u64);
            w.write_u(4, b.oem_version_minor as u64);
            w.write_u(8, b.creation_date_year as u64);
            w.write_zero(7);
            w.write_u(9, b.creation_date_day as u64);
            w.write_u(32, b.creation_date_day_fraction as u64);
            for sat in &b.satellites {
                w.write_u(24, sat.satellite_id as u64);
                w.write_zero(3);
                w.write_u(1, sat.metadata_flag as u64);
                w.write_u(1, sat.usable_start_time_flag as u64);
                w.write_u(1, sat.usable_stop_time_flag as u64);
                w.write_u(1, sat.ephemeris_accel_flag as u64);
                w.write_u(1, sat.covariance_flag as u64);
                if sat.metadata_flag {
                    if let Some(ref md) = sat.metadata {
                        w.write_u(8, md.total_start_time_year as u64);
                        w.write_zero(7);
                        w.write_u(9, md.total_start_time_day as u64);
                        w.write_u(32, md.total_start_time_day_fraction as u64);
                        w.write_u(8, md.total_stop_time_year as u64);
                        w.write_zero(7);
                        w.write_u(9, md.total_stop_time_day as u64);
                        w.write_u(32, md.total_stop_time_day_fraction as u64);
                        w.write_zero(1);
                        w.write_u(1, 0);
                        w.write_u(3, md.interpolation_type as u64);
                        w.write_u(3, md.interpolation_degree as u64);
                        if sat.usable_start_time_flag {
                            if let Some(ref ut) = md.usable_start_time {
                                w.write_u(8, ut.year as u64);
                                w.write_zero(7);
                                w.write_u(9, ut.day as u64);
                                w.write_u(32, ut.day_fraction as u64);
                            }
                        }
                        if sat.usable_stop_time_flag {
                            if let Some(ref ut) = md.usable_stop_time {
                                w.write_u(8, ut.year as u64);
                                w.write_zero(7);
                                w.write_u(9, ut.day as u64);
                                w.write_u(32, ut.day_fraction as u64);
                            }
                        }
                    }
                }
                w.write_u(16, sat.ephemeris_data_count as u64);
                for ed in &sat.ephemeris_data {
                    w.write_u(8, ed.epoch_year as u64);
                    w.write_zero(7);
                    w.write_u(9, ed.epoch_day as u64);
                    w.write_u(32, ed.epoch_day_fraction as u64);
                    w.write_u(32, ed.ephemeris_x as u64);
                    w.write_u(32, ed.ephemeris_y as u64);
                    w.write_u(32, ed.ephemeris_z as u64);
                    w.write_u(32, ed.ephemeris_x_dot as u64);
                    w.write_u(32, ed.ephemeris_y_dot as u64);
                    w.write_u(32, ed.ephemeris_z_dot as u64);
                    if sat.ephemeris_accel_flag {
                        if let Some(ref acc) = ed.acceleration {
                            w.write_u(32, acc.ephemeris_x_ddot as u64);
                            w.write_u(32, acc.ephemeris_y_ddot as u64);
                            w.write_u(32, acc.ephemeris_z_ddot as u64);
                        }
                    }
                }
                if sat.covariance_flag {
                    if let Some(ref cov) = sat.covariance {
                        w.write_u(8, cov.covariance_epoch_year as u64);
                        w.write_zero(7);
                        w.write_u(9, cov.covariance_epoch_day as u64);
                        w.write_u(32, cov.covariance_epoch_day_fraction as u64);
                        for elem in &cov.covariance_elements {
                            w.write_u(32, *elem as u64);
                        }
                    }
                }
            }
        }
        SatBody::Raw(_) => {}
    }
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
            2 => SatBody::TimeAssociation(TimeAssociationBody {
                association_type: 0,
                leap_info: None,
                ncr_base: 0,
                ncr_ext: 0,
                association_timestamp_seconds: 0,
                association_timestamp_nanoseconds: 0,
            }),
            3 => SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody { plans: Vec::new() }),
            4 => SatBody::PositionV3(PositionV3Body {
                oem_version_major: 0,
                oem_version_minor: 0,
                creation_date_year: 0,
                creation_date_day: 0,
                creation_date_day_fraction: 0,
                satellites: Vec::new(),
            }),
            _ => SatBody::Raw(data.to_vec()),
        });
    }
    let mut r = BitReader::new(data);
    match sat_table_id {
        0 => {
            let mut satellites = Vec::new();
            while r.remaining_bits() > 24 + 7 {
                let satellite_id = r.read_u(24) as u32;
                r.skip(7);
                let position_system = r.read_u(1);
                let position = if position_system == 0 {
                    let orbital_position = r.read_u(16) as u16;
                    let west_east_flag = r.read_u(1) != 0;
                    r.skip(7);
                    PositionSystem::Orbital {
                        orbital_position,
                        west_east_flag,
                    }
                } else {
                    let epoch_year = r.read_u(8) as u8;
                    let day_of_the_year = r.read_u(16) as u16;
                    let day_fraction = r.read_u(32) as u32;
                    let mean_motion_first_derivative = r.read_u(32) as u32;
                    let mean_motion_second_derivative = r.read_u(32) as u32;
                    let drag_term = r.read_u(32) as u32;
                    let inclination = r.read_u(32) as u32;
                    let right_ascension = r.read_u(32) as u32;
                    let eccentricity = r.read_u(32) as u32;
                    let argument_of_perigree = r.read_u(32) as u32;
                    let mean_anomaly = r.read_u(32) as u32;
                    let mean_motion = r.read_u(32) as u32;
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
                let cell_fragment_id = r.read_u(32) as u32;
                let first_occurrence = r.read_u(1) != 0;
                let last_occurrence = r.read_u(1) != 0;
                let center = if first_occurrence {
                    r.skip(4);
                    let center_latitude = r.read_i(18) as i32;
                    r.skip(5);
                    let center_longitude = r.read_i(19) as i32;
                    let max_distance = r.read_u(24) as u32;
                    r.skip(6);
                    Some(CellCenter {
                        center_latitude,
                        center_longitude,
                        max_distance,
                    })
                } else {
                    r.skip(4);
                    None
                };
                let dsid_count = r.read_u(10) as usize;
                let mut delivery_system_ids = Vec::with_capacity(dsid_count);
                for _ in 0..dsid_count {
                    delivery_system_ids.push(r.read_u(32) as u32);
                }
                r.skip(6);
                let nds_count = r.read_u(10) as usize;
                let mut new_delivery_systems = Vec::with_capacity(nds_count);
                for _ in 0..nds_count {
                    let new_delivery_system_id = r.read_u(32) as u32;
                    let time_of_application_base = r.read_u(33);
                    r.skip(6);
                    let time_of_application_ext = r.read_u(9) as u16;
                    new_delivery_systems.push(NewDeliverySystem {
                        new_delivery_system_id,
                        time_of_application_base,
                        time_of_application_ext,
                    });
                }
                r.skip(6);
                let ods_count = r.read_u(10) as usize;
                let mut obsolescent_delivery_systems = Vec::with_capacity(ods_count);
                for _ in 0..ods_count {
                    let obsolescent_delivery_system_id = r.read_u(32) as u32;
                    let time_of_obsolescence_base = r.read_u(33);
                    r.skip(6);
                    let time_of_obsolescence_ext = r.read_u(9) as u16;
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
            let association_type = r.read_u(4) as u8;
            let leap_info = if association_type == 1 {
                Some(LeapInfo {
                    leap59: r.read_u(1) != 0,
                    leap61: r.read_u(1) != 0,
                    pastleap59: r.read_u(1) != 0,
                    pastleap61: r.read_u(1) != 0,
                })
            } else {
                r.skip(4);
                None
            };
            let ncr_base = r.read_u(33);
            r.skip(6);
            let ncr_ext = r.read_u(9) as u16;
            let association_timestamp_seconds = r.read_u(64);
            let association_timestamp_nanoseconds = r.read_u(32) as u32;
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
                let beamhopping_time_plan_id = r.read_u(32) as u32;
                r.skip(4);
                let plan_length = r.read_u(12) as usize;
                let plan_end_bits = r.bits_consumed() + plan_length * 8;
                r.skip(6);
                let time_plan_mode = r.read_u(2) as u8;
                let time_of_application_base = r.read_u(33);
                r.skip(6);
                let time_of_application_ext = r.read_u(9) as u16;
                let cycle_duration_base = r.read_u(33);
                r.skip(6);
                let cycle_duration_ext = r.read_u(9) as u16;
                let mode = match time_plan_mode {
                    0 => {
                        let dwell_duration_base = r.read_u(33);
                        r.skip(6);
                        let dwell_duration_ext = r.read_u(9) as u16;
                        let on_time_base = r.read_u(33);
                        r.skip(6);
                        let on_time_ext = r.read_u(9) as u16;
                        BeamhoppingMode::Mode0 {
                            dwell_duration_base,
                            dwell_duration_ext,
                            on_time_base,
                            on_time_ext,
                        }
                    }
                    1 => {
                        r.skip(1);
                        let bit_map_size = r.read_u(15) as u16;
                        r.skip(1);
                        let current_slot = r.read_u(15) as u16;
                        let mut slot_transmission_on = Vec::with_capacity(bit_map_size as usize);
                        for _ in 0..bit_map_size {
                            slot_transmission_on.push(r.read_u(1) != 0);
                        }
                        let total = 1 + 15 + 1 + 15 + bit_map_size as usize;
                        let pad = (8 - (total % 8)) % 8;
                        r.skip(pad as u8);
                        BeamhoppingMode::Mode1 {
                            bit_map_size,
                            current_slot,
                            slot_transmission_on,
                        }
                    }
                    2 => {
                        let grid_size_base = r.read_u(33);
                        r.skip(6);
                        let grid_size_ext = r.read_u(9) as u16;
                        let revisit_duration_base = r.read_u(33);
                        r.skip(6);
                        let revisit_duration_ext = r.read_u(9) as u16;
                        let sleep_time_base = r.read_u(33);
                        r.skip(6);
                        let sleep_time_ext = r.read_u(9) as u16;
                        let sleep_duration_base = r.read_u(33);
                        r.skip(6);
                        let sleep_duration_ext = r.read_u(9) as u16;
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
                        r.bit_pos = plan_end_bits;
                        BeamhoppingMode::Mode0 {
                            dwell_duration_base: 0,
                            dwell_duration_ext: 0,
                            on_time_base: 0,
                            on_time_ext: 0,
                        }
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
            let oem_version_major = r.read_u(4) as u8;
            let oem_version_minor = r.read_u(4) as u8;
            let creation_date_year = r.read_u(8) as u8;
            r.skip(7);
            let creation_date_day = r.read_u(9) as u16;
            let creation_date_day_fraction = r.read_u(32) as u32;
            let mut satellites = Vec::new();
            while r.remaining_bits() >= 24 + 3 + 5 {
                let satellite_id = r.read_u(24) as u32;
                r.skip(3);
                let metadata_flag = r.read_u(1) != 0;
                let usable_start_time_flag = r.read_u(1) != 0;
                let usable_stop_time_flag = r.read_u(1) != 0;
                let ephemeris_accel_flag = r.read_u(1) != 0;
                let covariance_flag = r.read_u(1) != 0;
                let metadata = if metadata_flag {
                    let total_start_time_year = r.read_u(8) as u8;
                    r.skip(7);
                    let total_start_time_day = r.read_u(9) as u16;
                    let total_start_time_day_fraction = r.read_u(32) as u32;
                    let total_stop_time_year = r.read_u(8) as u8;
                    r.skip(7);
                    let total_stop_time_day = r.read_u(9) as u16;
                    let total_stop_time_day_fraction = r.read_u(32) as u32;
                    r.skip(1);
                    let _interpolation_flag = r.read_u(1);
                    let interpolation_type = r.read_u(3) as u8;
                    let interpolation_degree = r.read_u(3) as u8;
                    let usable_start_time = if usable_start_time_flag {
                        let year = r.read_u(8) as u8;
                        r.skip(7);
                        let day = r.read_u(9) as u16;
                        let day_fraction = r.read_u(32) as u32;
                        Some(UsableTime {
                            year,
                            day,
                            day_fraction,
                        })
                    } else {
                        None
                    };
                    let usable_stop_time = if usable_stop_time_flag {
                        let year = r.read_u(8) as u8;
                        r.skip(7);
                        let day = r.read_u(9) as u16;
                        let day_fraction = r.read_u(32) as u32;
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
                        interpolation_type,
                        interpolation_degree,
                        usable_start_time,
                        usable_stop_time,
                    })
                } else {
                    None
                };
                let ephemeris_data_count = r.read_u(16) as u16;
                let mut ephemeris_data = Vec::with_capacity(ephemeris_data_count as usize);
                for _ in 0..ephemeris_data_count {
                    let epoch_year = r.read_u(8) as u8;
                    r.skip(7);
                    let epoch_day = r.read_u(9) as u16;
                    let epoch_day_fraction = r.read_u(32) as u32;
                    let ephemeris_x = r.read_u(32) as u32;
                    let ephemeris_y = r.read_u(32) as u32;
                    let ephemeris_z = r.read_u(32) as u32;
                    let ephemeris_x_dot = r.read_u(32) as u32;
                    let ephemeris_y_dot = r.read_u(32) as u32;
                    let ephemeris_z_dot = r.read_u(32) as u32;
                    let acceleration = if ephemeris_accel_flag {
                        Some(EphemerisAccel {
                            ephemeris_x_ddot: r.read_u(32) as u32,
                            ephemeris_y_ddot: r.read_u(32) as u32,
                            ephemeris_z_ddot: r.read_u(32) as u32,
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
                    let covariance_epoch_year = r.read_u(8) as u8;
                    r.skip(7);
                    let covariance_epoch_day = r.read_u(9) as u16;
                    let covariance_epoch_day_fraction = r.read_u(32) as u32;
                    let mut covariance_elements = [0u32; 21];
                    for elem in &mut covariance_elements {
                        *elem = r.read_u(32) as u32;
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
                    metadata_flag,
                    usable_start_time_flag,
                    usable_stop_time_flag,
                    ephemeris_accel_flag,
                    covariance_flag,
                    metadata,
                    ephemeris_data_count,
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
/// Since all body fields are owned numeric values, the section no longer
/// borrows from the input buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SatSection {
    /// 6-bit discriminant selecting the body structure (see [`SatTableId`]).
    pub satellite_table_id: u8,
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
        let total = SECTION_LENGTH_PREFIX + section_length;
        if bytes.len() < total || total < HEADER_LEN + CRC_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: section_length,
                available: bytes.len().saturating_sub(SECTION_LENGTH_PREFIX),
            });
        }
        let satellite_table_id = bytes[3] >> 2;
        let table_count = (((bytes[3] & 0x03) as u16) << 8) | bytes[4] as u16;
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = bytes[5] & 0x01 != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];
        let body_data = &bytes[HEADER_LEN..total - CRC_LEN];
        let body = sat_body_parse(satellite_table_id, body_data)?;
        Ok(SatSection {
            satellite_table_id,
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
        let section_length = (len - SECTION_LENGTH_PREFIX) as u16;
        buf[0] = TABLE_ID;
        buf[1] = super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F);
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
                sat_body_write(&self.body, &mut writer, false);
            }
        }
        let body_end = HEADER_LEN + sat_body_serialized_len(&self.body);
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..body_end]);
        buf[body_end..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Table<'a> for SatSection {
    const TABLE_ID: u8 = TABLE_ID;
    const PID: u16 = PID;
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
            association_type: 1,
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
                assert_eq!(ta.association_type, 1);
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
}
