//! SCTE 104 time-type and splice/audio command byte mappings (ANSI/SCTE 104
//! 2023 §12.4/§12.5 and Tables 9-5, 9-22, 9-35; transcriptions under
//! `private/specs-verified/scte/scte104/`).

use broadcast_common::{Parse, Serialize};
use scte104::operations::insert_audio_provisioning::CodecIndex;
use scte104::operations::schedule_definition::SpliceScheduleCommand;
use scte104::operations::splice_request::SpliceInsertType;
use scte104::time::{GpiTimestamp, SmpteVitcTimestamp, Time, TimeType, Timestamp, UtcTimestamp};

#[test]
fn time_zero_is_eight_zero_bytes() {
    let zero = Time::zero();
    assert_eq!(zero, Time::default());
    let mut out = [0xFFu8; 8];
    assert_eq!(zero.serialize_into(&mut out).unwrap(), 8);
    assert_eq!(out, [0u8; 8]);
    assert_eq!(Time::parse(&out).unwrap(), zero);
}

#[test]
fn time_type_bytes_and_labels_are_pinned() {
    for (byte, ty, label) in [
        (0u8, TimeType::None, "none"),
        (1, TimeType::Utc, "utc"),
        (2, TimeType::SmpteVitc, "smpte vitc"),
        (3, TimeType::Gpi, "gpi"),
    ] {
        assert_eq!(TimeType::from_byte(byte), ty);
        assert_eq!(ty.to_byte(), byte);
        assert_eq!(ty.name(), label);
        assert_eq!(ty.to_string(), label);
    }
    assert_eq!(TimeType::from_byte(4), TimeType::Reserved(4));
    assert_eq!(TimeType::Reserved(4).to_byte(), 4);
    assert_eq!(TimeType::Reserved(4).name(), "reserved");
    assert_eq!(TimeType::Reserved(4).to_string(), "reserved(0x04)");
}

#[test]
fn every_timestamp_form_round_trips_with_its_literal_time_type() {
    let cases: &[(Timestamp, u8, &[u8])] = &[
        (Timestamp::None, 0, &[0x00]),
        (
            Timestamp::Utc(UtcTimestamp {
                seconds: 0x1234_5678,
                microseconds: 0x9ABC,
            }),
            1,
            &[0x01, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC],
        ),
        (
            Timestamp::SmpteVitc(SmpteVitcTimestamp {
                hours: 1,
                minutes: 2,
                seconds: 3,
                frames: 4,
            }),
            2,
            &[0x02, 0x01, 0x02, 0x03, 0x04],
        ),
        (
            Timestamp::Gpi(GpiTimestamp {
                gpi_number: 5,
                gpi_edge: 1,
            }),
            3,
            &[0x03, 0x05, 0x01],
        ),
    ];

    for (ts, time_type, wire) in cases {
        assert_eq!(ts.time_type(), *time_type);
        let mut out = vec![0u8; ts.serialized_len()];
        let n = ts.serialize_into(&mut out).unwrap();
        assert_eq!(n, wire.len());
        assert_eq!(&out, wire, "timestamp wire bytes");
        assert_eq!(&Timestamp::parse(wire).unwrap(), ts);
    }
}

#[test]
fn splice_insert_type_bytes_and_labels_are_pinned() {
    for (byte, ty, label) in [
        (0u8, SpliceInsertType::Reserved, "reserved"),
        (
            1,
            SpliceInsertType::SpliceStartNormal,
            "splice start normal",
        ),
        (
            2,
            SpliceInsertType::SpliceStartImmediate,
            "splice start immediate",
        ),
        (3, SpliceInsertType::SpliceEndNormal, "splice end normal"),
        (
            4,
            SpliceInsertType::SpliceEndImmediate,
            "splice end immediate",
        ),
        (5, SpliceInsertType::SpliceCancel, "splice cancel"),
    ] {
        assert_eq!(SpliceInsertType::from_byte(byte), ty);
        assert_eq!(ty.to_byte(), byte);
        assert_eq!(ty.name(), label);
        assert_eq!(ty.to_string(), label);
    }
    assert_eq!(SpliceInsertType::from_byte(9), SpliceInsertType::Unknown(9));
    assert_eq!(SpliceInsertType::Unknown(9).to_byte(), 9);
    assert_eq!(SpliceInsertType::Unknown(9).name(), "unknown");
}

#[test]
fn splice_schedule_command_bytes_and_labels_are_pinned() {
    for (byte, cmd, label) in [
        (0u8, SpliceScheduleCommand::Reserved, "reserved"),
        (1, SpliceScheduleCommand::SpliceInsert, "splice insert"),
        (3, SpliceScheduleCommand::SpliceReturn, "splice return"),
        (5, SpliceScheduleCommand::SpliceCancel, "splice cancel"),
    ] {
        assert_eq!(SpliceScheduleCommand::from_byte(byte), cmd);
        assert_eq!(cmd.to_byte(), byte);
        assert_eq!(cmd.name(), label);
        assert_eq!(cmd.to_string(), label);
    }
    // 0x02 is not defined by Table 9-22 and must stay unknown.
    assert_eq!(
        SpliceScheduleCommand::from_byte(2),
        SpliceScheduleCommand::Unknown(2)
    );
    assert_eq!(SpliceScheduleCommand::Unknown(2).to_byte(), 2);
}

#[test]
fn codec_index_bytes_and_labels_are_pinned() {
    for (byte, codec, label) in [
        (0u8, CodecIndex::Reserved, "reserved"),
        (1, CodecIndex::Ac3, "AC-3"),
        (2, CodecIndex::Eac3, "E-AC-3"),
        (3, CodecIndex::Aac, "AAC"),
        (5, CodecIndex::Ac4, "AC-4"),
        (6, CodecIndex::MpegH, "MPEG-H"),
    ] {
        assert_eq!(CodecIndex::from_u8(byte), codec);
        assert_eq!(codec.to_u8(), byte);
        assert_eq!(codec.name(), label);
        assert_eq!(codec.to_string(), label);
    }
    assert_eq!(CodecIndex::from_u8(4), CodecIndex::Unknown(4));
    assert_eq!(CodecIndex::Unknown(4).to_u8(), 4);
    assert_eq!(CodecIndex::Unknown(4).name(), "reserved");
}
