//! `SeqSpace`'s circular comparison at its boundaries (audit 21, #2922). A
//! `lt` that admits equality inverts duplicate/retransmission detection, and
//! the half-space tie rule is what keeps the ordering antisymmetric.

use broadcast_common::seq::SeqSpace;

fn space16() -> SeqSpace {
    broadcast_common::seq_space!(16)
}

fn space31() -> SeqSpace {
    broadcast_common::seq_space!(31)
}

#[test]
fn a_sequence_neither_precedes_nor_follows_itself() {
    for space in [space16(), space31()] {
        assert!(!space.lt(5, 5));
        assert!(space.leq(5, 5));
        assert!(!space.gt(5, 5));
        assert!(space.geq(5, 5));
        assert!(!space.is_tie(5, 5));
        assert_eq!(space.diff(5, 5), 0);
    }
}

#[test]
fn exactly_half_the_space_apart_is_no_ordering_in_either_direction() {
    for space in [space16(), space31()] {
        let half = (space.modulus() / 2) as u32;
        for (a, b) in [(half, 0u32), (0, half), (half + 1, 1), (1, half + 1)] {
            assert!(space.is_tie(a, b), "bits {}: {a}/{b}", space.bits());
            assert!(!space.lt(a, b), "bits {}: {a}/{b}", space.bits());
            assert!(!space.leq(a, b), "bits {}: {a}/{b}", space.bits());
            assert!(!space.gt(a, b), "bits {}: {a}/{b}", space.bits());
            assert!(!space.geq(a, b), "bits {}: {a}/{b}", space.bits());
        }
        assert_eq!(space.diff(half, 0), half as i32);
        assert_eq!(space.diff(0, half), half as i32);
    }
}

#[test]
fn the_short_way_round_the_wrap_decides_before_and_after() {
    let s16 = space16();
    assert!(s16.lt(0xFFFF, 0));
    assert!(s16.gt(0, 0xFFFF));
    assert!(!s16.lt(0, 0xFFFF));
    assert!(!s16.gt(0xFFFF, 0));
    assert_eq!(s16.diff(0, 0xFFFF), 1);
    let s31 = space31();
    assert!(s31.lt(0x7FFF_FFFF, 0));
    assert!(s31.gt(0, 0x7FFF_FFFF));
    assert!(!s31.lt(0, 0x7FFF_FFFF));
    assert!(!s31.gt(0x7FFF_FFFF, 0));
    assert_eq!(s31.diff(0, 0x7FFF_FFFF), 1);
}

#[test]
fn a_closed_range_walks_forward_from_its_first() {
    let s16 = space16();
    assert!(s16.in_closed_range(5, 5, 5), "a one-element range holds it");
    assert!(!s16.in_closed_range(6, 5, 5));
    assert!(!s16.in_closed_range(4, 5, 5));
    assert!(s16.in_closed_range(7, 5, 9));
    assert!(!s16.in_closed_range(10, 5, 9));
    assert!(
        !s16.in_closed_range(5, 10, 5),
        "a last that precedes first is empty"
    );
    assert!(s16.in_closed_range(0, 0xFFFE, 1));
    assert!(s16.in_closed_range(1, 0xFFFE, 1));
    assert!(!s16.in_closed_range(2, 0xFFFE, 1));
    assert!(s16.in_closed_range(0xFFFE, 0xFFFE, 1));
    assert!(space31().in_closed_range(0, 0x7FFF_FFFE, 1));
    assert!(!space31().in_closed_range(2, 0x7FFF_FFFE, 1));
}
