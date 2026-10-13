//! #1129: the CICAM_player APDUs write 16-bit `service_location_length`,
//! `PMT_length` and `number_of_component_types` fields from `slice.len() as
//! u16`. Those casts are only safe because `write_apdu_header` first rejects
//! any APDU body above 0xFFFF (`length::encode_with`). This test pins that
//! dependency: an over-range field must be an `Err`, and the largest
//! representable field must still round-trip its length byte-exactly, so a
//! change to the header cap can never turn the casts into silent wraps.

use broadcast_common::Serialize;
use dvb_ci::ci_plus::cicam_player::{
    ComponentType, PlayerCapabilitiesReply, PlayerPlayReq, PlayerStartReq, PlayerUpdateReq,
    PlayerVerifyReq,
};
use dvb_ci::length::LengthForm;

const OVER: usize = 65_536;

fn out() -> Vec<u8> {
    vec![0u8; OVER * 3]
}

#[test]
fn over_range_u16_fields_are_errors_not_wraps() {
    let big = vec![0u8; OVER];
    let mut buf = out();
    assert!(
        PlayerVerifyReq {
            length_form: LengthForm::Minimal,
            service_location: &big
        }
        .serialize_into(&mut buf)
        .is_err()
    );
    assert!(
        PlayerPlayReq {
            length_form: LengthForm::Minimal,
            service_location: &big
        }
        .serialize_into(&mut buf)
        .is_err()
    );
    assert!(
        PlayerStartReq {
            length_form: LengthForm::Minimal,
            input_max_bitrate: 0,
            output_max_bitrate: 0,
            linear_channel: false,
            pmt: &big
        }
        .serialize_into(&mut buf)
        .is_err()
    );
    assert!(
        PlayerUpdateReq {
            length_form: LengthForm::Minimal,
            lts_id: 0,
            pmt: &big
        }
        .serialize_into(&mut buf)
        .is_err()
    );
    assert!(
        PlayerCapabilitiesReply {
            length_form: LengthForm::Minimal,
            component_types: vec![
                ComponentType {
                    stream_content: 1,
                    component_type: 1
                };
                OVER
            ],
        }
        .serialize_into(&mut buf)
        .is_err()
    );
}

#[test]
fn largest_pmt_keeps_its_length_field() {
    // body = 3 (LTS_id + PMT_length) + pmt, capped at 0xFFFF by the header.
    let pmt = vec![0xA5u8; 0xFFFF - 3];
    let req = PlayerUpdateReq {
        length_form: LengthForm::Minimal,
        lts_id: 7,
        pmt: &pmt,
    };
    let mut buf = out();
    let n = req.serialize_into(&mut buf).expect("fits");
    // tag(3) + BER 82 FF FF (3) + lts_id + PMT_length(2) + pmt.
    assert_eq!(&buf[3..6], &[0x82, 0xFF, 0xFF]);
    assert_eq!(u16::from_be_bytes([buf[7], buf[8]]) as usize, pmt.len());
    assert_eq!(n, 3 + 3 + 3 + pmt.len());
}
