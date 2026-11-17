//! PTS/DTS decoding edges: each of the three marker bits is checked, the
//! prefix is checked, and `seconds()` divides by the 90 kHz clock.

use mpeg_pes::{Dts, Error, Pts};

#[test]
fn every_marker_of_a_pts_field_is_checked() {
    let pts = Pts::try_new(0x1_2345_6789).expect("33-bit");
    let field = pts.to_field_bytes();
    assert_eq!(Pts::from_field_bytes(&field).expect("markers set"), pts);

    for index in [0usize, 2, 4] {
        let mut bad = field;
        bad[index] &= !0x01;
        assert_eq!(
            Pts::from_field_bytes(&bad),
            Err(Error::BadTimestampMarker("PTS")),
            "marker in byte {index}"
        );
    }
}

#[test]
fn every_marker_of_a_dts_field_is_checked() {
    let dts = Dts::try_new(0x0_1234_5678).expect("33-bit");
    let field = dts.to_field_bytes();
    assert_eq!(Dts::from_field_bytes(&field).expect("markers set"), dts);

    for index in [0usize, 2, 4] {
        let mut bad = field;
        bad[index] &= !0x01;
        assert_eq!(
            Dts::from_field_bytes(&bad),
            Err(Error::BadTimestampMarker("DTS")),
            "marker in byte {index}"
        );
    }
}

#[test]
fn a_pts_field_read_as_dts_reports_the_prefix() {
    let pts = Pts::try_new(90_000).expect("33-bit");
    assert_eq!(
        Dts::from_field_bytes(&pts.to_field_bytes()),
        Err(Error::BadTimestampPrefix("DTS"))
    );
}

#[test]
fn seconds_divides_by_the_90khz_clock() {
    assert_eq!(Pts::try_new(90_000).expect("33-bit").seconds(), 1.0);
    assert_eq!(Pts::try_new(270_000).expect("33-bit").seconds(), 3.0);
    assert_eq!(Dts::try_new(45_000).expect("33-bit").seconds(), 0.5);
    assert_eq!(Dts::try_new(180_000).expect("33-bit").seconds(), 2.0);
}
