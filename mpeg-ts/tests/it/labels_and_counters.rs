//! mpeg-ts labels, counter arithmetic and the generic `Serialize` impl of
//! `TsHeader`: every `name()`/`Display` label is asserted literally, the
//! continuity-counter wrap distance is exercised in both directions, and the
//! trait `serialize_into` really writes the four header bytes.

use broadcast_core::Serialize;
use mpeg_ts::pid::Pid;
use mpeg_ts::resync::PacketStride;
use mpeg_ts::ts::{
    AdaptationFieldControl, ContinuityCounter, LtwOffset, PiecewiseRate, ScramblingControl,
    TsHeader,
};
use mpeg_ts::ts_dup::DuplicateVerdict;

fn cc(value: u8) -> ContinuityCounter {
    ContinuityCounter::try_new(value).expect("4-bit counter")
}

#[test]
fn continuity_counter_forward_distance_wraps_and_detects_repeats() {
    assert_eq!(cc(0).forward_distance_from(cc(15)), 1);
    assert_eq!(cc(5).forward_distance_from(cc(5)), 0);
    assert_eq!(cc(3).forward_distance_from(cc(1)), 2);
    assert_eq!(cc(1).forward_distance_from(cc(1)), 0);
    assert_eq!(cc(15).forward_distance_from(cc(0)), 15);
}

#[test]
fn continuity_counter_conversions_and_formatting() {
    assert_eq!(u8::from(cc(9)), 9);
    assert_eq!(
        ContinuityCounter::try_from(15u8).expect("in range").value(),
        15
    );
    assert!(ContinuityCounter::try_from(16u8).is_err());

    assert_eq!(format!("{}", cc(9)), "9");
    assert_eq!(format!("{:?}", cc(9)), "ContinuityCounter(9)");

    let err = ContinuityCounter::try_new(16).expect_err("out of range");
    assert_eq!(
        err.to_string(),
        "continuity_counter 16 exceeds the 4-bit range 0..=15"
    );
}

#[test]
fn adaptation_field_control_labels_and_flags() {
    let cases = [
        (AdaptationFieldControl::Reserved, "reserved", (false, false)),
        (
            AdaptationFieldControl::PayloadOnly,
            "payload_only",
            (false, true),
        ),
        (
            AdaptationFieldControl::AdaptationOnly,
            "adaptation_only",
            (true, false),
        ),
        (
            AdaptationFieldControl::AdaptationAndPayload,
            "adaptation_and_payload",
            (true, true),
        ),
    ];
    for (value, label, flags) in cases {
        assert_eq!(value.name(), label);
        assert_eq!(value.to_string(), label);
        assert_eq!(value.to_flags(), flags);
        assert_eq!(AdaptationFieldControl::from_flags(flags.0, flags.1), value);
    }
}

#[test]
fn scrambling_control_labels() {
    let cases = [
        (ScramblingControl::NotScrambled, "not_scrambled"),
        (ScramblingControl::Reserved, "reserved"),
        (ScramblingControl::EvenKey, "even_key"),
        (ScramblingControl::OddKey, "odd_key"),
    ];
    for (value, label) in cases {
        assert_eq!(value.name(), label);
        assert_eq!(value.to_string(), label);
    }
    assert_eq!(ScramblingControl::EvenKey.to_bits(), 0b10);
    assert_eq!(
        ScramblingControl::from_bits(0b11),
        ScramblingControl::OddKey
    );
}

#[test]
fn packet_stride_and_duplicate_verdict_labels() {
    assert_eq!(PacketStride::Ts188.name(), "ts_188");
    assert_eq!(PacketStride::Ts188.to_string(), "ts_188");
    assert_eq!(PacketStride::Rs204.name(), "rs_204");
    assert_eq!(PacketStride::Rs204.to_string(), "rs_204");
    assert_eq!(PacketStride::M2ts192.name(), "m2ts_192");
    assert_eq!(PacketStride::M2ts192.to_string(), "m2ts_192");

    assert_eq!(DuplicateVerdict::NotDuplicate.name(), "not duplicate");
    assert_eq!(DuplicateVerdict::NotDuplicate.to_string(), "not duplicate");
    assert_eq!(DuplicateVerdict::Legal.name(), "legal");
    assert_eq!(DuplicateVerdict::Legal.to_string(), "legal");
    assert_eq!(
        DuplicateVerdict::IllegalThirdRepeat.name(),
        "illegal third repeat"
    );
    assert_eq!(
        DuplicateVerdict::IllegalThirdRepeat.to_string(),
        "illegal third repeat"
    );
}

#[test]
fn ltw_offset_and_piecewise_rate_value_conversions() {
    let offset = LtwOffset::new(0x1234).expect("15-bit");
    assert_eq!(u16::from(offset), 0x1234);
    assert_eq!(format!("{offset}"), "4660");

    let rate = PiecewiseRate::new(0x12_3456).expect("22-bit");
    assert_eq!(u32::from(rate), 0x12_3456);
    assert_eq!(format!("{rate}"), "1193046");
}

#[test]
fn pid_debug_prints_the_hex_value() {
    assert_eq!(format!("{:?}", Pid::from_const(0x0100)), "Pid(0x0100)");
    assert_eq!(format!("{}", Pid::from_const(0x1FFF)), "0x1FFF");
}

fn header() -> TsHeader {
    TsHeader {
        tei: true,
        pusi: true,
        pid: Pid::from_const(0x0123),
        scrambling: ScramblingControl::OddKey,
        adaptation_field_control: AdaptationFieldControl::AdaptationAndPayload,
        continuity_counter: cc(0xA),
    }
}

#[test]
fn ts_header_serialized_len_is_four_for_both_forms() {
    assert_eq!(TsHeader::serialized_len(), 4);
    assert_eq!(<TsHeader as Serialize>::serialized_len(&header()), 4);
}

#[test]
fn the_generic_serialize_impl_writes_the_header_bytes() {
    let header = header();
    let mut buf = [0u8; 4];
    let written =
        <TsHeader as Serialize>::serialize_into(&header, &mut buf).expect("4-byte header");
    assert_eq!(written, 4);
    assert_eq!(buf, header.to_bytes());
    assert_eq!(buf, [0x47, 0xC1, 0x23, 0xFA]);
}
