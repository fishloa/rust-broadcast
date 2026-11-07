use broadcast_common::Parse;
use rmt_flute::{Error, NORM_INFO_FIXED_LEN, NormInfo};

#[test]
fn a_norm_info_whose_hdr_len_is_below_the_fixed_header_is_inconsistent() {
    let mut bytes = [0u8; NORM_INFO_FIXED_LEN];
    bytes[0] = 0x11;
    bytes[1] = 1;
    assert_eq!(
        NormInfo::parse(&bytes).unwrap_err(),
        Error::InconsistentLength {
            length: 1,
            reason: "hdr_len smaller than the NORM_INFO fixed header",
        }
    );
}
