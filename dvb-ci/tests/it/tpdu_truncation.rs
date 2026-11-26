//! TPDU truncation boundaries and the segmented-data (`Tdata_more`) tag —
//! ETSI EN 50221 Annex A §A.4.1, Tables A.1/A.2/A.16.

use broadcast_core::error::{BufferTooShort, LengthMismatch};
use broadcast_core::{Parse, Serialize};
use dvb_ci::Error;
use dvb_ci::tpdu::{self, CommandTpdu, DataBlock, NewTc, ResponseTpdu, TcObject};

fn buffer_too_short(err: &Error) -> Option<(usize, usize)> {
    match err {
        Error::BufferTooShort(BufferTooShort { need, have, .. }) => Some((*need, *have)),
        _ => None,
    }
}

#[test]
fn tc_object_missing_t_c_id_reports_the_exact_need() {
    let err = TcObject::parse(&[0x82, 0x01]).unwrap_err();
    assert_eq!(buffer_too_short(&err), Some((3, 2)), "{err:?}");
}

#[test]
fn every_response_tpdu_prefix_is_rejected() {
    let wire = [0xA0, 0x02, 0x05, 0xAA, 0x80, 0x02, 0x05, 0x80];
    for cut in 0..wire.len() {
        assert!(
            ResponseTpdu::parse(&wire[..cut]).is_err(),
            "prefix of {cut} bytes must be rejected"
        );
    }
    // Status trailer cuts: one byte short of the SB tag / of the SB_value.
    let err = ResponseTpdu::parse(&wire[..5]).unwrap_err();
    assert_eq!(buffer_too_short(&err), Some((6, 5)), "{err:?}");
    let err = ResponseTpdu::parse(&wire[..6]).unwrap_err();
    assert_eq!(buffer_too_short(&err), Some((8, 6)), "{err:?}");
    let err = ResponseTpdu::parse(&wire[..7]).unwrap_err();
    assert_eq!(buffer_too_short(&err), Some((8, 7)), "{err:?}");
}

#[test]
fn every_command_tpdu_prefix_is_rejected() {
    let wire = [0xA0, 0x04, 0x05, 0xAA, 0xBB];
    for cut in 0..wire.len() {
        assert!(
            CommandTpdu::parse(&wire[..cut]).is_err(),
            "prefix of {cut} bytes must be rejected"
        );
    }
    let err = CommandTpdu::parse(&wire[..4]).unwrap_err();
    match err {
        Error::LengthMismatch(LengthMismatch {
            declared, actual, ..
        }) => assert_eq!((declared, actual), (4, 2)),
        other => panic!("expected LengthMismatch, got {other:?}"),
    }
}

#[test]
fn every_fixed_object_prefix_is_rejected() {
    let wire = [0x87, 0x02, 0x01, 0x02];
    for cut in 0..wire.len() {
        assert!(
            NewTc::parse(&wire[..cut]).is_err(),
            "prefix of {cut} bytes must be rejected"
        );
    }
    let err = NewTc::parse(&wire[..3]).unwrap_err();
    assert_eq!(buffer_too_short(&err), Some((4, 3)), "{err:?}");
    let error = [0x88, 0x02, 0x01];
    assert!(tc_error_prefix_rejected(&error));
    let err = dvb_ci::tpdu::TcError::parse(&error).unwrap_err();
    assert_eq!(buffer_too_short(&err), Some((4, 3)), "{err:?}");
}

fn tc_error_prefix_rejected(wire: &[u8]) -> bool {
    dvb_ci::tpdu::TcError::parse(wire).is_err()
}

#[test]
fn data_more_tag_round_trips() {
    let wire = [0xA1, 0x02, 0x05, 0xAA, 0x80, 0x02, 0x05, 0x80];
    let r = ResponseTpdu::parse(&wire).unwrap();
    assert_eq!(r.block, Some(DataBlock::More));
    assert_eq!(r.block.unwrap().to_tag(), tpdu::tags::DATA_MORE);
    assert_eq!(r.block.unwrap().to_tag(), 0xA1);
    assert_eq!(r.block.unwrap().name(), "data_more");
    assert_eq!(r.try_to_bytes().unwrap(), wire);
}

#[test]
fn data_last_tag_round_trips() {
    let wire = [0xA0, 0x02, 0x05, 0xAA, 0x80, 0x02, 0x05, 0x80];
    let r = ResponseTpdu::parse(&wire).unwrap();
    assert_eq!(r.block, Some(DataBlock::Last));
    assert_eq!(r.block.unwrap().to_tag(), tpdu::tags::DATA_LAST);
    assert_eq!(r.block.unwrap().to_tag(), 0xA0);
    assert_eq!(r.block.unwrap().name(), "data_last");
}

#[test]
fn response_tpdu_serialize_into_returns_the_bytes_written() {
    let r = ResponseTpdu {
        tag: tpdu::tags::DATA_LAST,
        t_c_id: 5,
        data: &[0xAA, 0xBB],
        sb_value: tpdu::SbValue::new(true),
        block: Some(DataBlock::Last),
        length_form: dvb_ci::length::LengthForm::Minimal,
        status_length_form: dvb_ci::length::LengthForm::Minimal,
    };
    let mut buf = [0u8; 32];
    let n = r.serialize_into(&mut buf).unwrap();
    assert_eq!(n, r.serialized_len());
    assert_eq!(n, 9);
    assert_eq!(
        &buf[..n],
        &[0xA0, 0x03, 0x05, 0xAA, 0xBB, 0x80, 0x02, 0x05, 0x80]
    );
}
