//! The typed SRT numbers: circular ordering in both directions, the 26-bit
//! Message Number wrap, and every label/format conversion's literal output
//! (audit 21 #2931).

use srt_runtime::packet::{AckNumber, MessageNumber, SeqNo, SocketId, SrtTimestamp};

fn sn(v: u32) -> SeqNo {
    SeqNo::try_new(v).expect("fits 31 bits")
}

#[test]
fn seq_no_orders_circularly_in_both_directions() {
    assert!(sn(5).lt(sn(6)));
    assert!(!sn(6).lt(sn(5)));
    assert!(!sn(5).lt(sn(5)));
    assert!(sn(5).leq(sn(5)));
    assert!(sn(5).leq(sn(6)));
    assert!(!sn(6).leq(sn(5)));
    assert!(sn(6).gt(sn(5)));
    assert!(!sn(5).gt(sn(6)));
    assert!(!sn(5).gt(sn(5)));
    assert!(sn(5).geq(sn(5)));
    assert!(sn(6).geq(sn(5)));
    assert!(!sn(5).geq(sn(6)));
    // Across the 31-bit wrap.
    assert!(SeqNo::MAX.lt(sn(0)));
    assert!(!sn(0).lt(SeqNo::MAX));
    assert!(sn(0).gt(SeqNo::MAX));
    assert!(!SeqNo::MAX.gt(sn(0)));
}

#[test]
fn seq_no_diff_is_the_signed_circular_distance() {
    assert_eq!(sn(10).diff(sn(4)), 6);
    assert_eq!(sn(4).diff(sn(10)), -6);
    assert_eq!(sn(5).diff(sn(5)), 0);
    assert_eq!(sn(0).diff(SeqNo::MAX), 1);
    assert_eq!(SeqNo::MAX.diff(sn(0)), -1);
}

#[test]
fn message_number_wraps_to_one_at_the_26_bit_boundary() {
    assert_eq!(MessageNumber::try_new(7).unwrap().next().value(), 8);
    assert_eq!(MessageNumber::try_new(0).unwrap().next().value(), 1);
    let max = MessageNumber::try_new(0x03FF_FFFF).unwrap();
    assert_eq!(max.next().value(), 1);
    assert!(MessageNumber::try_new(1 << 26).is_err());
}

#[test]
fn message_number_try_from_keeps_the_value() {
    assert_eq!(MessageNumber::try_from(0).unwrap().value(), 0);
    assert_eq!(MessageNumber::try_from(7).unwrap().value(), 7);
    assert_eq!(
        MessageNumber::try_from(0x03FF_FFFF).unwrap().value(),
        0x03FF_FFFF
    );
    assert!(MessageNumber::try_from(1 << 26).is_err());
}

#[test]
fn seq_no_display_debug_and_from_are_the_wire_value() {
    let seven = sn(7);
    assert_eq!(format!("{seven}"), "7");
    assert_eq!(format!("{seven:?}"), "SeqNo(0x00000007)");
    assert_eq!(u32::from(seven), 7);
    assert_eq!(SeqNo::try_from(7).unwrap(), seven);
}

#[test]
fn socket_id_display_debug_and_from_are_the_wire_value() {
    let id = SocketId::from_wire(42);
    assert_eq!(format!("{id}"), "42");
    assert_eq!(format!("{id:?}"), "SocketId(42)");
    assert_eq!(u32::from(id), 42);
    assert_eq!(SocketId::from(42), id);
}

#[test]
fn ack_number_display_debug_and_from_are_the_wire_value() {
    let ack = AckNumber::from_wire(7);
    assert_eq!(format!("{ack}"), "7");
    assert_eq!(format!("{ack:?}"), "AckNumber(7)");
    assert_eq!(u32::from(ack), 7);
    assert_eq!(AckNumber::from(7), ack);
}

#[test]
fn message_number_display_debug_and_from_are_the_wire_value() {
    let n = MessageNumber::try_new(7).unwrap();
    assert_eq!(format!("{n}"), "7");
    assert_eq!(format!("{n:?}"), "MessageNumber(7)");
    assert_eq!(u32::from(n), 7);
}

#[test]
fn timestamp_display_debug_and_from_are_the_wire_value() {
    let ts = SrtTimestamp::from_wire(7);
    assert_eq!(format!("{ts}"), "7");
    assert_eq!(format!("{ts:?}"), "SrtTimestamp(7)");
    assert_eq!(u32::from(ts), 7);
    assert_eq!(SrtTimestamp::from(7), ts);
}

#[test]
fn out_of_range_errors_display_their_value() {
    let Err(seq_err) = SeqNo::try_new(0x8000_0000) else {
        panic!("0x8000_0000 does not fit 31 bits");
    };
    assert_eq!(
        format!("{seq_err}"),
        "sequence number 0x80000000 exceeds the 31-bit range"
    );
    let Err(number_err) = MessageNumber::try_new(1 << 26) else {
        panic!("2^26 does not fit 26 bits");
    };
    assert_eq!(
        format!("{number_err}"),
        "message number 0x04000000 exceeds the 26-bit range"
    );
}
