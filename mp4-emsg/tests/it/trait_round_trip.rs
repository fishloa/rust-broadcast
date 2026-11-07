//! `EmsgBox` through `broadcast_common::{Parse, Serialize}` on the real v0 and v1 fixtures.

use broadcast_common::{Parse, Serialize};
use mp4_emsg::EmsgBox;

#[test]
fn real_emsg_boxes_round_trip_through_the_traits() {
    for name in [
        "scte35/scte35-emsg-v0.bin",
        "scte35/emsg-v1-scte35-livesim.bin",
    ] {
        let data = test_fixtures::read(name);
        let b = <EmsgBox as Parse>::parse(&data).unwrap();
        let wire = Serialize::try_to_bytes(&b).unwrap();
        assert_eq!(wire, data, "{name}");
        assert_eq!(<EmsgBox as Parse>::parse(&wire).unwrap(), b, "{name}");
    }
}
