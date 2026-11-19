//! SAT golden byte vectors derived by hand from ETSI EN 300 468 Tables 11a–11h
//! (§5.2.11), transcribed under
//! `private/specs-verified/dvb/dvb-si/tables/en_300_468/`.
//!
//! Every vector pins the bit layout and the computed
//! `beamhopping_time_plan_length` without going through this crate's
//! serializer: the section header, the body bits and the CRC placement are
//! literal, and a round trip must reproduce the same bytes. There is no real
//! SAT capture in the fixtures.

use broadcast_common::{Parse, Serialize};
use dvb_si::tables::sat::*;

/// Build the Table 11a framing around a hand-packed body:
/// `4D | SSI=1, private=1, reserved=11, length_hi | length_lo | stid(6),
/// table_count(10) | reserved(2), version(5), current_next(1) |
/// section_number | last_section_number | reserved_zero_future_use | body |
/// CRC_32`.
fn wrap(stid: u8, table_count: u16, version: u8, current_next: bool, body: &[u8]) -> Vec<u8> {
    let section_length = body.len() + 10;
    let mut v = vec![
        0x4D,
        0xF0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        ((stid & 0x3F) << 2) | ((table_count >> 8) as u8 & 0x03),
        (table_count & 0xFF) as u8,
        0xC0 | ((version & 0x1F) << 1) | u8::from(current_next),
        0x00,
        0x00,
        0x00,
    ];
    v.extend_from_slice(body);
    let crc = broadcast_common::crc32_mpeg2::compute(&v);
    v.extend_from_slice(&crc.to_be_bytes());
    v
}

/// Parse `golden`, assert the section length against the literal vector, and
/// check that re-serialising the parsed value reproduces it byte for byte.
fn parse_and_reserialize(golden: &[u8]) -> SatSection {
    let sat = SatSection::parse(golden).expect("golden parses");
    assert_eq!(sat.serialized_len(), golden.len());
    let mut out = vec![0u8; golden.len()];
    sat.serialize_into(&mut out).expect("re-serialize");
    assert_eq!(out, golden, "hand-derived bytes are reproduced");
    sat
}

fn zeroes(n: usize) -> Vec<u8> {
    vec![0u8; n]
}

// ── Table 11c: satellite_position_v2_info ───────────────────────────────────

#[test]
fn position_v2_orbital_two_satellites() {
    // satellite_id(24)=0x010203 | reserved(7)=0 | position_system(1)=0 |
    // orbital_position(16)=0x1234 | west_east_flag=1 | reserved(7)=0
    // satellite_id(24)=0x040506 | reserved(7)=0 | position_system(1)=0 |
    // orbital_position(16)=0x5678 | west_east_flag=0 | reserved(7)=1111111
    let body = [
        0x01, 0x02, 0x03, 0x00, 0x12, 0x34, 0x80, 0x04, 0x05, 0x06, 0x00, 0x56, 0x78, 0x7F,
    ];
    let golden = wrap(0, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::PositionV2(pv2) = &sat.body else {
        panic!("expected PositionV2, got {:?}", sat.body);
    };
    assert_eq!(pv2.satellites.len(), 2);
    assert_eq!(pv2.satellites[0].satellite_id, 0x010203);
    assert_eq!(pv2.satellites[1].satellite_id, 0x040506);
    match (&pv2.satellites[0].position, &pv2.satellites[1].position) {
        (
            PositionSystem::Orbital {
                orbital_position: p0,
                west_east_flag: w0,
                ..
            },
            PositionSystem::Orbital {
                orbital_position: p1,
                west_east_flag: w1,
                ..
            },
        ) => {
            assert_eq!(*p0, 0x1234);
            assert!(*w0);
            assert_eq!(*p1, 0x5678);
            assert!(!*w1);
        }
        other => panic!("expected two orbital positions, got {other:?}"),
    }
}

fn sgp4_satellite(id: [u8; 3], epoch_year: u8, day_of_the_year: u16) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&id);
    // reserved_zero_future_use(7)=0 then position_system(1)=1.
    v.push(0x01);
    v.push(epoch_year);
    v.extend_from_slice(&day_of_the_year.to_be_bytes());
    // Ten 32-bit orbital elements (day_fraction .. mean_motion) all zero.
    v.extend_from_slice(&zeroes(40));
    v
}

#[test]
fn position_v2_sgp4_two_satellites() {
    let mut body = Vec::new();
    body.extend_from_slice(&sgp4_satellite([0x0A, 0x0B, 0x0C], 24, 1));
    body.extend_from_slice(&sgp4_satellite([0x0D, 0x0E, 0x0F], 25, 0x0102));
    let golden = wrap(0, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::PositionV2(pv2) = &sat.body else {
        panic!("expected PositionV2, got {:?}", sat.body);
    };
    assert_eq!(pv2.satellites.len(), 2);
    match (&pv2.satellites[0].position, &pv2.satellites[1].position) {
        (
            PositionSystem::Sgp4 {
                epoch_year: y0,
                day_of_the_year: d0,
                mean_motion: m0,
                ..
            },
            PositionSystem::Sgp4 {
                epoch_year: y1,
                day_of_the_year: d1,
                mean_motion: m1,
                ..
            },
        ) => {
            assert_eq!(*y0, 24);
            assert_eq!(*d0, 1);
            assert_eq!(*m0, 0);
            assert_eq!(*y1, 25);
            assert_eq!(*d1, 0x0102);
            assert_eq!(*m1, 0);
        }
        other => panic!("expected two SGP4 positions, got {other:?}"),
    }
}

// ── Table 11d: cell_fragment_info ───────────────────────────────────────────

#[test]
fn cell_fragment_two_fragments_first_then_last() {
    // First fragment: id=0x11223344 | first=1 last=0 | reserved(4)=0 |
    // centre_latitude(18)=1 | reserved(5)=0 | centre_longitude(19)=-1 |
    // max_distance(24)=1 | reserved(6)=0 | dsid_count(10)=1 |
    // delivery_system_id(32)=0xAABBCCDD | reserved(6)=0 | new_count(10)=0 |
    // reserved(6)=0 | obsolescent_count(10)=0.
    // Second fragment: id=0x55667788 | first=0 last=1 | reserved(4)=0 |
    // dsid_count=0 | new_count=0 | obsolescent_count=0.
    let body = [
        0x11, 0x22, 0x33, 0x44, 0x80, 0x00, 0x01, 0x07, 0xFF, 0xFF, 0x00, 0x00, 0x01, 0x00, 0x01,
        0xAA, 0xBB, 0xCC, 0xDD, 0x00, 0x00, 0x00, 0x00, 0x55, 0x66, 0x77, 0x88, 0x40, 0x00, 0x00,
        0x00, 0x00, 0x00,
    ];
    let golden = wrap(1, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::CellFragment(cf) = &sat.body else {
        panic!("expected CellFragment, got {:?}", sat.body);
    };
    assert_eq!(cf.fragments.len(), 2);
    let first = &cf.fragments[0];
    assert_eq!(first.cell_fragment_id, 0x11223344);
    assert!(first.first_occurrence);
    assert!(!first.last_occurrence);
    let centre = first
        .centre
        .as_ref()
        .expect("first occurrence has a centre");
    assert_eq!(centre.centre_latitude, 1);
    assert_eq!(centre.centre_longitude, -1);
    assert_eq!(centre.max_distance, 1);
    assert_eq!(first.delivery_system_ids, [0xAABBCCDD]);
    let last = &cf.fragments[1];
    assert_eq!(last.cell_fragment_id, 0x55667788);
    assert!(!last.first_occurrence);
    assert!(last.last_occurrence);
    assert!(last.centre.is_none());
    assert!(last.delivery_system_ids.is_empty());
}

// ── Table 11e: time_association_info ────────────────────────────────────────

#[test]
fn time_association_with_leap_second_signalling() {
    // association_type(4)=1 | leap59=1 leap61=0 pastleap59=1 pastleap61=0 |
    // ncr_base(33)=2 | reserved(6)=0 | ncr_ext(9)=0x1FF |
    // association_timestamp_seconds(64)=0x0102030405060708 |
    // association_timestamp_nanoseconds(32)=0x0A0B0C0D.
    let body = [
        0x1A, 0x00, 0x00, 0x00, 0x01, 0x01, 0xFF, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        0x0A, 0x0B, 0x0C, 0x0D,
    ];
    let golden = wrap(2, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::TimeAssociation(ta) = &sat.body else {
        panic!("expected TimeAssociation, got {:?}", sat.body);
    };
    assert_eq!(ta.association_type, AssociationType::UtcWithLeap);
    let leap = ta.leap_info.as_ref().expect("leap flags present");
    assert!(leap.leap59);
    assert!(!leap.leap61);
    assert!(leap.pastleap59);
    assert!(!leap.pastleap61);
    assert_eq!(ta.ncr_base, 2);
    assert_eq!(ta.ncr_ext, 0x1FF);
    assert_eq!(ta.association_timestamp_seconds, 0x0102_0304_0506_0708);
    assert_eq!(ta.association_timestamp_nanoseconds, 0x0A0B_0C0D);
}

// ── Table 11g: beamhopping_time_plan_info ───────────────────────────────────

#[test]
fn beamhopping_mode0_plan_length_is_25() {
    // plan_id(32)=0xDEADBEEF | reserved(4)=0 | plan_length(12)=25 (0x019) |
    // reserved(6)=0 | time_plan_mode(2)=0 | time_of_application_base(33)=0 |
    // reserved(6)=0 | ext(9)=0 | cycle_duration_base(33)=0 | reserved(6)=0 |
    // ext(9)=0 | dwell_duration_base(33)=0 | reserved(6)=0 | ext(9)=0 |
    // on_time_base(33)=0 | reserved(6)=0 | ext(9)=0.
    let mut body = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x19, 0x00];
    body.extend_from_slice(&zeroes(24));
    assert_eq!(body.len(), 31);
    let golden = wrap(3, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::BeamhoppingTimePlan(bhp) = &sat.body else {
        panic!("expected BeamhoppingTimePlan, got {:?}", sat.body);
    };
    assert_eq!(bhp.plans.len(), 1);
    assert_eq!(bhp.plans[0].beamhopping_time_plan_id, 0xDEAD_BEEF);
    assert_eq!(bhp.plans[0].time_plan_mode, TimePlanMode::DwellOnTime);
}

#[test]
fn beamhopping_mode1_bit_map_size_13_plan_length_is_19() {
    // plan_id=0x12345678 | reserved(4)=0 | plan_length(12)=19 (0x013) |
    // reserved(6)=0 | mode(2)=1 | toa base/ext zero (48 bits) | cycle zero
    // (48 bits) | size_r(1)=0 | bit_map_size(15)=13 | slot_r(1)=0 |
    // current_slot(15)=5 | bitmap=1011001 1001 | padding(3)=000.
    let body = [
        0x12, 0x34, 0x56, 0x78, 0x00, 0x13, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x0D, 0x00, 0x05, 0xB2, 0xC8,
    ];
    assert_eq!(body.len(), 25);
    let golden = wrap(3, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::BeamhoppingTimePlan(bhp) = &sat.body else {
        panic!("expected BeamhoppingTimePlan, got {:?}", sat.body);
    };
    let BeamhoppingMode::Mode1 {
        bit_map_size,
        current_slot,
        slot_transmission_on,
        ..
    } = &bhp.plans[0].mode
    else {
        panic!("expected Mode1");
    };
    assert_eq!(*bit_map_size, 13);
    assert_eq!(*current_slot, 5);
    assert_eq!(
        slot_transmission_on,
        &[
            true, false, true, true, false, false, true, false, true, true, false, false, true
        ]
    );
}

#[test]
fn beamhopping_mode1_bit_map_size_9_pins_the_padding_count() {
    // Same shape as the bit_map_size=13 case: raw mode bits = 1+15+1+15+9 = 41,
    // padded to 48, so plan_length is still 19 (0x013). The nine bitmap bits are
    // followed by seven padding bits.
    let mut body = vec![0x1A, 0x2B, 0x3C, 0x4D, 0x00, 0x13, 0x01];
    body.extend_from_slice(&zeroes(12));
    body.extend_from_slice(&[0x00, 0x09, 0x00, 0x01, 0xAA, 0x80]);
    assert_eq!(body.len(), 25);
    let golden = wrap(3, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::BeamhoppingTimePlan(bhp) = &sat.body else {
        panic!("expected BeamhoppingTimePlan, got {:?}", sat.body);
    };
    let BeamhoppingMode::Mode1 {
        bit_map_size,
        current_slot,
        slot_transmission_on,
        ..
    } = &bhp.plans[0].mode
    else {
        panic!("expected Mode1");
    };
    assert_eq!(*bit_map_size, 9);
    assert_eq!(*current_slot, 1);
    assert_eq!(
        slot_transmission_on,
        &[true, false, true, false, true, false, true, false, true]
    );
}

#[test]
fn beamhopping_mode1_then_mode0_keeps_plan_boundaries() {
    // Two plans: a Mode1 plan (bit_map_size=13) then a Mode0 plan. The padding
    // and the declared plan_length must both land the second plan on its
    // byte-aligned boundary.
    let mut body = vec![
        0x12, 0x34, 0x56, 0x78, 0x00, 0x13, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x0D, 0x00, 0x05, 0xB2, 0xC8,
    ];
    body.extend_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0x19, 0x00]);
    body.extend_from_slice(&zeroes(24));
    assert_eq!(body.len(), 56);
    let golden = wrap(3, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::BeamhoppingTimePlan(bhp) = &sat.body else {
        panic!("expected BeamhoppingTimePlan, got {:?}", sat.body);
    };
    assert_eq!(bhp.plans.len(), 2);
    assert_eq!(bhp.plans[0].time_plan_mode, TimePlanMode::Bitmap);
    assert_eq!(bhp.plans[1].time_plan_mode, TimePlanMode::DwellOnTime);
    assert_eq!(bhp.plans[1].beamhopping_time_plan_id, 0xCAFE_BABE);
}

#[test]
fn beamhopping_mode2_plan_length_is_37() {
    // plan_id=0xCAFEBABE | reserved(4)=0 | plan_length(12)=37 (0x025) |
    // reserved(6)=0 | mode(2)=2 | toa zero (48) | cycle zero (48) |
    // grid_size_base(33)=1 | reserved(6)=0 | grid_size_ext(9)=0x1FF |
    // the three remaining 48-bit groups are zero.
    let mut body = vec![0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0x25, 0x02];
    body.extend_from_slice(&zeroes(12));
    body.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x81, 0xFF]);
    body.extend_from_slice(&zeroes(18));
    assert_eq!(body.len(), 43);
    let golden = wrap(3, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::BeamhoppingTimePlan(bhp) = &sat.body else {
        panic!("expected BeamhoppingTimePlan, got {:?}", sat.body);
    };
    let BeamhoppingMode::Mode2 {
        grid_size_base,
        grid_size_ext,
        ..
    } = &bhp.plans[0].mode
    else {
        panic!("expected Mode2");
    };
    assert_eq!(*grid_size_base, 1);
    assert_eq!(*grid_size_ext, 0x1FF);
}

// ── Table 11h: satellite_position_v3_info ───────────────────────────────────

fn v3_header() -> Vec<u8> {
    // oem_version_major(4)=1, minor(4)=2 | creation_date_year(8)=24 |
    // reserved(7)=0, creation_date_day(9)=100 | day_fraction(32)=0.
    vec![0x12, 0x18, 0x00, 0x64, 0x00, 0x00, 0x00, 0x00]
}

fn v3_ephemeris(accel: bool) -> Vec<u8> {
    // epoch_year(8)=24 | reserved(7)=0, epoch_day(9)=0 | day_fraction(32)=0 |
    // x, y, z, x_dot, y_dot, z_dot (6 x 32)=0.
    let mut v = vec![0x18, 0x00, 0x00];
    v.extend_from_slice(&zeroes(28));
    if accel {
        // ephemeris_x_ddot=1, ephemeris_y_ddot=2, ephemeris_z_ddot=3.
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x02]);
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x03]);
    }
    v
}

#[test]
fn position_v3_with_acceleration() {
    let mut body = v3_header();
    // satellite_id(24)=1 | reserved(3)=0 metadata=0 start=0 stop=0 accel=1
    // covariance=0.
    body.extend_from_slice(&[0x00, 0x00, 0x01, 0x02]);
    body.extend_from_slice(&[0x00, 0x01]); // ephemeris_data_count = 1
    body.extend_from_slice(&v3_ephemeris(true));
    assert_eq!(body.len(), 57);
    let golden = wrap(4, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::PositionV3(v3) = &sat.body else {
        panic!("expected PositionV3, got {:?}", sat.body);
    };
    assert_eq!(v3.oem_version_major, 1);
    assert_eq!(v3.oem_version_minor, 2);
    assert_eq!(v3.creation_date_year, 24);
    assert_eq!(v3.creation_date_day, 100);
    assert_eq!(v3.satellites.len(), 1);
    let s = &v3.satellites[0];
    assert!(s.ephemeris_accel_flag);
    assert_eq!(s.ephemeris_data.len(), 1);
    assert_eq!(s.ephemeris_data[0].epoch_year, 24);
    let accel = s.ephemeris_data[0]
        .acceleration
        .as_ref()
        .expect("accel flag set");
    assert_eq!(accel.ephemeris_x_ddot, 1);
    assert_eq!(accel.ephemeris_y_ddot, 2);
    assert_eq!(accel.ephemeris_z_ddot, 3);
}

#[test]
fn position_v3_without_acceleration() {
    let mut body = v3_header();
    body.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]);
    body.extend_from_slice(&[0x00, 0x01]);
    body.extend_from_slice(&v3_ephemeris(false));
    assert_eq!(body.len(), 45);
    let golden = wrap(4, 0, 0, true, &body);
    let sat = parse_and_reserialize(&golden);
    let SatBody::PositionV3(v3) = &sat.body else {
        panic!("expected PositionV3, got {:?}", sat.body);
    };
    let s = &v3.satellites[0];
    assert!(!s.ephemeris_accel_flag);
    assert_eq!(s.ephemeris_data.len(), 1);
    assert!(s.ephemeris_data[0].acceleration.is_none());
    assert_eq!(s.ephemeris_data[0].epoch_year, 24);
}
