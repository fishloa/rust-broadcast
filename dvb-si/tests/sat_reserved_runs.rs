//! SAT (EN 300 468 §5.2.11, Tables 11a-11h) body `reserved_zero_future_use`
//! runs are captured on parse into typed fields and re-emitted verbatim
//! (workspace reserved-bit policy, #1139): a sender that writes ones in a
//! reserved run round-trips byte-identically. Each case sets EVERY run of one
//! body shape to all-ones, so a serializer that wrote zeros (the old
//! behaviour) fails the parse-back equality, and a hand-derived byte check on
//! the smallest shape pins the wire positions.
//!
//! No real capture of a SAT carrying non-zero reserved bits is available (the
//! SAT is new in EN 300 468 V1.18); the layouts are spec-derived (Tables 11c,
//! 11d, 11e, 11g, 11h) and the byte check below is derived by hand from
//! Table 11e, not from this crate's serializer.

use broadcast_common::{Parse, Serialize};
use dvb_si::tables::sat::*;

fn section(stid: u8, body: SatBody) -> Vec<u8> {
    let sat = SatSection::new(stid, true, 1, 0, true, 0, 0, body);
    let mut buf = vec![0u8; sat.serialized_len()];
    sat.serialize_into(&mut buf).unwrap();
    buf
}

fn time_association(r4: u8, ncr_r6: u8) -> TimeAssociationBody {
    TimeAssociationBody {
        association_type: AssociationType::from_u8(2),
        leap_info: None,
        ncr_base: 0,
        ncr_ext: 0,
        association_timestamp_seconds: 0,
        association_timestamp_nanoseconds: 0,
        reserved_zero_future_use_4: r4,
        ncr_reserved_zero_future_use: ncr_r6,
    }
}

/// serialize -> parse -> equal body -> serialize again byte-identical.
fn round_trip(stid: u8, body: SatBody) -> Vec<u8> {
    let bytes = section(stid, body.clone());
    let parsed = SatSection::parse(&bytes).unwrap();
    assert_eq!(parsed.body, body, "reserved runs must survive parse");
    let mut again = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut again).unwrap();
    assert_eq!(again, bytes, "re-serialize must be byte-identical");
    bytes
}

#[test]
fn time_association_reserved_runs_hand_derived_bytes() {
    let b = time_association(0xF, 0x3F);
    let bytes = round_trip(2, SatBody::TimeAssociation(b));
    // Table 11e: association_type(4)=2 | reserved_zero_future_use(4)=1111 -> 0x2F;
    // ncr_base(33)=0 | reserved(6)=111111 | ncr_ext(9)=0 -> 00 00 00 00 7E 00.
    assert_eq!(
        &bytes[9..9 + 7],
        &[0x2F, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00]
    );
}

#[test]
fn position_v2_orbital_reserved_runs() {
    let sat = PositionV2Satellite {
        satellite_id: 1,
        position: PositionSystem::Orbital {
            orbital_position: 0x0192,
            west_east_flag: true,
            reserved_zero_future_use: 0x7F,
        },
        reserved_zero_future_use: 0x7F,
    };
    round_trip(
        0,
        SatBody::PositionV2(PositionV2Body {
            satellites: vec![sat],
        }),
    );
}

#[test]
fn cell_fragment_reserved_runs() {
    let first = CellFragment {
        cell_fragment_id: 1,
        first_occurrence: true,
        last_occurrence: false,
        centre: Some(CellCentre {
            centre_latitude: 5,
            centre_longitude: -7,
            max_distance: 9,
            reserved_zero_future_use_4: 0xF,
            reserved_zero_future_use_5: 0x1F,
            reserved_zero_future_use_6: 0x3F,
        }),
        delivery_system_ids: vec![11, 12],
        new_delivery_systems: vec![NewDeliverySystem {
            new_delivery_system_id: 1,
            time_of_application_base: 2,
            time_of_application_ext: 3,
            reserved_zero_future_use: 0x3F,
        }],
        obsolescent_delivery_systems: vec![ObsolescentDeliverySystem {
            obsolescent_delivery_system_id: 4,
            time_of_obsolescence_base: 5,
            time_of_obsolescence_ext: 6,
            reserved_zero_future_use: 0x3F,
        }],
        reserved_zero_future_use_4: 0,
        reserved_zero_future_use_new: 0x3F,
        reserved_zero_future_use_obsolescent: 0x3F,
    };
    // Not-first fragment: the 4-bit run in the else branch.
    let later = CellFragment {
        cell_fragment_id: 2,
        first_occurrence: false,
        last_occurrence: true,
        centre: None,
        delivery_system_ids: vec![],
        new_delivery_systems: vec![],
        obsolescent_delivery_systems: vec![],
        reserved_zero_future_use_4: 0xF,
        reserved_zero_future_use_new: 0x3F,
        reserved_zero_future_use_obsolescent: 0x3F,
    };
    round_trip(
        1,
        SatBody::CellFragment(CellFragmentBody {
            fragments: vec![first, later],
        }),
    );
}

#[test]
fn beamhopping_reserved_runs_all_modes() {
    for mode in [
        BeamhoppingMode::Mode0 {
            dwell_duration_base: 1,
            dwell_duration_ext: 2,
            on_time_base: 3,
            on_time_ext: 4,
            dwell_duration_reserved_zero_future_use: 0x3F,
            on_time_reserved_zero_future_use: 0x3F,
        },
        BeamhoppingMode::Mode1 {
            bit_map_size: 5,
            current_slot: 1,
            slot_transmission_on: vec![true, false, true, true, false],
            reserved_zero_future_use_size: 1,
            reserved_zero_future_use_slot: 1,
        },
        BeamhoppingMode::Mode2 {
            grid_size_base: 1,
            grid_size_ext: 2,
            revisit_duration_base: 3,
            revisit_duration_ext: 4,
            sleep_time_base: 5,
            sleep_time_ext: 6,
            sleep_duration_base: 7,
            sleep_duration_ext: 8,
            grid_size_reserved_zero_future_use: 0x3F,
            revisit_duration_reserved_zero_future_use: 0x3F,
            sleep_time_reserved_zero_future_use: 0x3F,
            sleep_duration_reserved_zero_future_use: 0x3F,
        },
    ] {
        let mode_num = match &mode {
            BeamhoppingMode::Mode0 { .. } => 0,
            BeamhoppingMode::Mode1 { .. } => 1,
            _ => 2,
        };
        let plan = BeamhoppingPlan {
            beamhopping_time_plan_id: 7,
            time_plan_mode: TimePlanMode::from_u8(mode_num),
            time_of_application_base: 1,
            time_of_application_ext: 2,
            cycle_duration_base: 3,
            cycle_duration_ext: 4,
            mode,
            reserved_zero_future_use_4: 0xF,
            reserved_zero_future_use_mode: 0x3F,
            time_of_application_reserved_zero_future_use: 0x3F,
            cycle_duration_reserved_zero_future_use: 0x3F,
        };
        round_trip(
            3,
            SatBody::BeamhoppingTimePlan(BeamhoppingTimePlanBody { plans: vec![plan] }),
        );
    }
}

#[test]
fn position_v3_reserved_runs() {
    let usable = |r| UsableTime {
        year: 1,
        day: 2,
        day_fraction: 3,
        reserved_zero_future_use: r,
    };
    let sat = PositionV3Satellite {
        satellite_id: 9,
        usable_start_time_flag: true,
        usable_stop_time_flag: true,
        ephemeris_accel_flag: false,
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
            interpolation_degree: 1,
            usable_start_time: Some(usable(0x7F)),
            usable_stop_time: Some(usable(0x7F)),
            total_start_time_reserved_zero_future_use: 0x7F,
            total_stop_time_reserved_zero_future_use: 0x7F,
            reserved_zero_future_use: 1,
        }),
        ephemeris_data: vec![EphemerisData {
            epoch_year: 1,
            epoch_day: 2,
            epoch_day_fraction: 3,
            ephemeris_x: 4,
            ephemeris_y: 5,
            ephemeris_z: 6,
            ephemeris_x_dot: 7,
            ephemeris_y_dot: 8,
            ephemeris_z_dot: 9,
            acceleration: None,
            reserved_zero_future_use: 0x7F,
        }],
        covariance: Some(CovarianceData {
            covariance_epoch_year: 1,
            covariance_epoch_day: 2,
            covariance_epoch_day_fraction: 3,
            covariance_elements: [0; 21],
            covariance_epoch_reserved_zero_future_use: 0x7F,
        }),
        reserved_zero_future_use: 0x7,
    };
    round_trip(
        4,
        SatBody::PositionV3(PositionV3Body {
            oem_version_major: 1,
            oem_version_minor: 2,
            creation_date_year: 3,
            creation_date_day: 4,
            creation_date_day_fraction: 5,
            satellites: vec![sat],
            creation_date_reserved_zero_future_use: 0x7F,
        }),
    );
}

#[test]
fn over_range_reserved_run_is_an_error_not_a_wrap() {
    let b = time_association(0xF, 0x40); // 6-bit run given 0x40
    let sat = SatSection::new(2, true, 1, 0, true, 0, 0, SatBody::TimeAssociation(b));
    let mut buf = vec![0u8; sat.serialized_len()];
    assert!(sat.serialize_into(&mut buf).is_err());
}
