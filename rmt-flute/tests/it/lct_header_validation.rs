//! LCT header validation (RFC 5651 §5.1): CCI/TSI/TOI lengths against the
//! flag-derived field sizes, `HDR_LEN` consistency, and the close flags.

use rmt_flute::{Error, LCT_VERSION, LctHeader, Toi, Tsi};

fn header<'a>(cci: &'a [u8], tsi: Tsi, toi: Toi) -> LctHeader<'a> {
    LctHeader {
        version: LCT_VERSION,
        psi: 0,
        reserved: 0,
        close_session: false,
        close_object: false,
        codepoint: 0,
        cci,
        tsi,
        toi,
        extensions: Vec::new(),
    }
}

fn assert_field(err: Error, expected: &str) {
    match err {
        Error::InvalidField(field) => assert_eq!(field.field, expected, "got {field:?}"),
        other => panic!("expected InvalidField({expected}), got {other:?}"),
    }
}

#[test]
fn a_zero_or_non_word_cci_length_is_rejected() {
    let mut buf = [0u8; 16];
    let empty = header(&[], Tsi::new(0, 0).unwrap(), Toi::new(0, 0).unwrap());
    assert_field(empty.serialize_into(&mut buf).unwrap_err(), "CCI");

    let six = header(&[0u8; 6], Tsi::new(0, 0).unwrap(), Toi::new(0, 0).unwrap());
    assert_field(six.serialize_into(&mut buf).unwrap_err(), "CCI");
}

#[test]
fn a_three_word_toi_round_trips() {
    let toi = Toi::new(0x0102_0304_0506, 12).expect("12-byte TOI");
    let h = header(&[0u8; 4], Tsi::new(0, 0).unwrap(), toi);
    assert_eq!(h.o_flag(), 3);
    assert_eq!(h.h_flag(), 0);
    assert_eq!(h.hdr_len(), 5);

    let mut out = vec![0u8; h.serialized_len()];
    assert_eq!(h.serialize_into(&mut out).unwrap(), 20);
    let (parsed, used) = LctHeader::parse(&out).unwrap();
    assert_eq!(used, 20);
    assert_eq!(parsed, h);
}

#[test]
fn an_hdr_len_too_small_for_the_flags_is_inconsistent() {
    // V=1, C=0, PSI=0, S=1, O=1, H=0; HDR_LEN=3 words (12 bytes) while the
    // flag-derived base alone is 16 bytes.
    let mut data = [0u8; 12];
    data[0] = 0x10;
    data[1] = 0xA0;
    data[2] = 3;
    data[3] = 0;
    assert!(matches!(
        LctHeader::parse(&data),
        Err(Error::InconsistentLength { .. })
    ));
}

#[test]
fn the_close_session_and_close_object_bits_are_written_exactly() {
    let mut h = header(&[0u8; 4], Tsi::new(0, 0).unwrap(), Toi::new(0, 0).unwrap());
    h.close_session = true;
    h.close_object = true;
    let mut out = vec![0u8; h.serialized_len()];
    h.serialize_into(&mut out).unwrap();
    let word = u16::from_be_bytes([out[0], out[1]]);
    assert_eq!(word & 0x0003, 0x0003, "A and B must both be set");
    assert!(
        LctHeader::parse(&out).unwrap().0.close_object,
        "close object survives parse"
    );

    let mut off = header(&[0u8; 4], Tsi::new(0, 0).unwrap(), Toi::new(0, 0).unwrap());
    off.close_object = true;
    let mut out = vec![0u8; off.serialized_len()];
    off.serialize_into(&mut out).unwrap();
    let word = u16::from_be_bytes([out[0], out[1]]);
    assert_eq!(word & 0x0003, 0x0001, "only B is set");
}
