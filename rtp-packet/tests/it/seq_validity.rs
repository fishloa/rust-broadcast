//! RFC 3550 §A.1 sequence-validity cycle accounting and the extended
//! sequence-number distance that feeds RTCP loss reporting (§6.4.1).

use rtp_packet::{ExtendedSeqNo, SeqNo, RTP_SEQ_MOD, RTP_SEQ_MOD_PLUS_ONE, SeqValidity, SeqVerdict};

fn probation_complete_at(v: &mut SeqValidity, first: u16) {
    v.new_source(first);
    assert_eq!(v.update_seq(first), SeqVerdict::Probation);
    assert_eq!(
        v.update_seq(first.wrapping_add(1)),
        SeqVerdict::Valid { restarted: false }
    );
    assert_eq!(v.cycles, 0);
}

#[test]
fn the_cycle_counter_increments_only_on_the_wrapping_packet() {
    let mut v = SeqValidity::default();
    // Probation completes at 65533; 65531 is the new_source seed of 65532.
    probation_complete_at(&mut v, 65532);
    assert_eq!(v.max_seq, 65533);

    assert_eq!(
        v.update_seq(65534),
        SeqVerdict::Valid { restarted: false }
    );
    assert_eq!(v.cycles, 0, "an in-order packet must not add a cycle");

    assert_eq!(
        v.update_seq(65535),
        SeqVerdict::Valid { restarted: false }
    );
    assert_eq!(v.cycles, 0, "an in-order packet must not add a cycle");

    assert_eq!(v.update_seq(0), SeqVerdict::Valid { restarted: false });
    assert_eq!(v.cycles, RTP_SEQ_MOD, "65535 -> 0 adds exactly one cycle");

    assert_eq!(v.update_seq(1), SeqVerdict::Valid { restarted: false });
    assert_eq!(
        v.cycles, RTP_SEQ_MOD,
        "the packet after the wrap must not add another cycle"
    );
}

#[test]
fn a_duplicate_sequence_number_is_not_a_cycle() {
    let mut v = SeqValidity::default();
    probation_complete_at(&mut v, 100);
    assert_eq!(v.update_seq(200), SeqVerdict::Valid { restarted: false });
    assert_eq!(
        v.update_seq(200),
        SeqVerdict::Valid { restarted: false },
        "a duplicate within MAX_DROPOUT is in order"
    );
    assert_eq!(v.cycles, 0, "seq == max_seq must not add a cycle");
}

#[test]
fn bad_seq_sentinel_is_one_past_the_modulus() {
    assert_eq!(RTP_SEQ_MOD_PLUS_ONE, 65_537);
    assert_eq!(RTP_SEQ_MOD, 65_536);
}

#[test]
fn extended_forward_distance_across_a_cycle_boundary() {
    let ext = ExtendedSeqNo::from(0x0001_0005);
    assert_eq!(ext.forward_distance(SeqNo::new(0xFFFE)), 7);
    assert_eq!(ext.forward_distance(SeqNo::new(0x0005)), 0);
    assert_eq!(ExtendedSeqNo::from(0x0001_0000).forward_distance(SeqNo::new(0xFFFF)), 1);
    assert_eq!(ExtendedSeqNo::from(0x0002_0003).forward_distance(SeqNo::new(0xFFFD)), 6);
    assert_eq!(ExtendedSeqNo::from(0x0002_0003).get(), 0x0002_0003);
}
