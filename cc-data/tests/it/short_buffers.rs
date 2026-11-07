//! `OutputBufferTooSmall { need, have }` for every buffer shorter than a
//! cc_data() carrying a CEA-608 and a CEA-708 triplet.

use broadcast_common::Serialize;
use cc_data::{CcData, CcTriplet, CcType, Error};

#[test]
fn cc_data_rejects_every_short_buffer() {
    let cc = CcData {
        process_em_data_flag: true,
        process_cc_data_flag: true,
        additional_data_flag: false,
        reserved_byte1: 0xFF,
        triplets: vec![
            CcTriplet {
                cc_valid: true,
                cc_type: CcType::Ntsc608Field1,
                cc_data_1: 0x94,
                cc_data_2: 0x2C,
            },
            CcTriplet {
                cc_valid: true,
                cc_type: CcType::Dtvcc708Start,
                cc_data_1: 0x42,
                cc_data_2: 0x01,
            },
        ],
    };
    test_fixtures::assert_short_buffers_rejected(
        cc.serialized_len(),
        |buf| cc.serialize_into(buf),
        |e| match e {
            Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
                need,
                have,
            }) => Some((*need, *have)),
            _ => None,
        },
    );
}

#[test]
fn a_one_byte_cc_data_is_a_missing_header() {
    assert_eq!(
        <CcData as broadcast_common::Parse>::parse(&[0x40]).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 2,
            have: 1,
            what: "cc_data header",
        })
    );
}
