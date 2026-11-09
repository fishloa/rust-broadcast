use broadcast_common::clock33::{WRAP_33BIT_HALF, signed_distance, unwrap_delta};
use broadcast_common::seq::SeqSpace;

#[test]
fn unwrap_delta_half_range_step_agrees_with_signed_distance() {
    let half = WRAP_33BIT_HALF;
    let step = signed_distance(half, 0);
    assert_eq!(step, half as i64, "signed_distance documents (-2^32, 2^32]");
    assert_eq!(
        unwrap_delta(0, half, 0),
        i128::from(step),
        "unwrap_delta returned -2^32, outside the documented (-2^32, 2^32]"
    );
}

#[test]
fn seq_ordering_is_antisymmetric_at_exactly_half_the_space() {
    for bits in [8u32, 16, 31] {
        let s = SeqSpace::new(bits);
        let half = (s.modulus() / 2) as u32;
        assert!(
            !(s.gt(half, 0) && s.gt(0, half)),
            "bits={bits}: gt(a,b) and gt(b,a) both true at distance 2^(bits-1)"
        );
    }
}
