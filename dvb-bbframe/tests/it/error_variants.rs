use dvb_bbframe::Error;
use dvb_bbframe::header::TsGs;

#[test]
fn a_ts_gs_code_outside_the_two_bit_field_is_unsupported() {
    let err = TsGs::try_from(0x04).unwrap_err();
    assert_eq!(Error::from(err), Error::UnsupportedTsGs { ts_gs: 0x04 });
}
