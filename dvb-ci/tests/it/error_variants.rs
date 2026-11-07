use broadcast_common::Parse;
use dvb_ci::Error;
use dvb_ci::tpdu::{NewTc, ResponseTpdu};

#[test]
fn a_response_status_trailer_without_the_sb_tag_is_an_unexpected_tpdu_tag() {
    let bytes = [0xA0, 0x01, 0x01, 0x00, 0x02, 0x01, 0x00];
    assert_eq!(
        ResponseTpdu::parse(&bytes).unwrap_err(),
        Error::UnexpectedTpduTag {
            got: 0x00,
            expected: 0x80,
            what: "R_TPDU SB_tag",
        }
    );
}

#[test]
fn a_new_t_c_with_another_tag_is_an_unexpected_tpdu_tag() {
    let bytes = [0x82, 0x01, 0x01];
    assert_eq!(
        NewTc::parse(&bytes).unwrap_err(),
        Error::UnexpectedTpduTag {
            got: 0x82,
            expected: 0x87,
            what: "New_T_C",
        }
    );
}
