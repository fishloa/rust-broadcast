//! AFD and bar-data decoding from T.35 / GA94 payloads (ETSI TS 101 154
//! Annex B, Tables B.1-B.7): every accept and reject branch of
//! `afd_from_t35_payload`, `bar_data_from_t35_payload` and
//! `bar_data_from_ga94_payload`.

use transmux::user_data::{
    AfdSignal, afd_from_t35_payload, bar_data_from_ga94_payload, bar_data_from_t35_payload,
};
use transmux::{ActiveFormatDescription, BarData};

const T35_PREFIX: [u8; 3] = [0xB5, 0x00, 0x31];
const DTG1: [u8; 4] = *b"DTG1";
const GA94: [u8; 4] = *b"GA94";

fn afd_payload(rest: &[u8]) -> Vec<u8> {
    [T35_PREFIX.as_slice(), DTG1.as_slice(), rest].concat()
}

fn bar_payload(rest: &[u8]) -> Vec<u8> {
    [T35_PREFIX.as_slice(), GA94.as_slice(), rest].concat()
}

#[test]
fn afd_with_the_active_format_flag_decodes_the_code() {
    let got = afd_from_t35_payload(&afd_payload(&[0x40, 0xF4]));
    assert_eq!(
        got,
        Some(AfdSignal::Format(ActiveFormatDescription::BoxWiderThan16x9Centre))
    );
    let got = afd_from_t35_payload(&afd_payload(&[0x40, 0xA8]));
    assert_eq!(got, Some(AfdSignal::Format(ActiveFormatDescription::SameAsCodedFrame)));
}

#[test]
fn afd_with_the_flag_cleared_is_present_but_empty() {
    assert_eq!(
        afd_from_t35_payload(&afd_payload(&[0x00, 0xFF])),
        Some(AfdSignal::NoFormat)
    );
    assert_eq!(afd_from_t35_payload(&afd_payload(&[0x00])), Some(AfdSignal::NoFormat));
}

#[test]
fn afd_rejects_foreign_or_short_payloads() {
    assert_eq!(afd_from_t35_payload(&[]), None);
    assert_eq!(afd_from_t35_payload(&afd_payload(&[])), None);
    assert_eq!(
        afd_from_t35_payload(&afd_payload(&[0x40])),
        None,
        "flag set, code byte missing"
    );
    let mut wrong_country = afd_payload(&[0x40, 0x04]);
    wrong_country[0] = 0xB4;
    assert_eq!(afd_from_t35_payload(&wrong_country), None);
    let mut wrong_provider = afd_payload(&[0x40, 0x04]);
    wrong_provider[2] = 0x32;
    assert_eq!(afd_from_t35_payload(&wrong_provider), None);
    let mut wrong_identifier = afd_payload(&[0x40, 0x04]);
    wrong_identifier[3..7].copy_from_slice(&GA94);
    assert_eq!(afd_from_t35_payload(&wrong_identifier), None);
    assert_eq!(afd_from_t35_payload(&afd_payload(&[0x40, 0x04])[..6]), None);
}

#[test]
fn bar_data_decodes_each_flagged_value_in_order() {
    let rest = [0x06, 0xF0, 0xC0, 0x10, 0xC0, 0x20, 0xC0, 0x30, 0xC0, 0x40];
    let want = BarData {
        top_bar_end_line: Some(0x10),
        bottom_bar_start_line: Some(0x20),
        left_bar_end_pixel: Some(0x30),
        right_bar_start_pixel: Some(0x40),
    };
    assert_eq!(
        bar_data_from_t35_payload(&bar_payload(&rest)),
        Some(want.clone())
    );
    assert_eq!(bar_data_from_ga94_payload(&rest), Some(want.clone()));
    assert_eq!(
        bar_data_from_ga94_payload(&[GA94.as_slice(), &rest].concat()),
        Some(want)
    );
}

#[test]
fn bar_values_are_masked_to_fourteen_bits() {
    let rest = [0x06, 0x80, 0xFF, 0xFF];
    assert_eq!(
        bar_data_from_ga94_payload(&rest),
        Some(BarData {
            top_bar_end_line: Some(0x3FFF),
            bottom_bar_start_line: None,
            left_bar_end_pixel: None,
            right_bar_start_pixel: None,
        })
    );
}

#[test]
fn only_the_flagged_bars_are_read() {
    let rest = [0x06, 0x50, 0xC0, 0x08, 0xC0, 0x09];
    assert_eq!(
        bar_data_from_ga94_payload(&rest),
        Some(BarData {
            top_bar_end_line: None,
            bottom_bar_start_line: Some(8),
            left_bar_end_pixel: None,
            right_bar_start_pixel: Some(9),
        })
    );
}

#[test]
fn bar_data_rejects_foreign_short_or_wrong_type_payloads() {
    assert_eq!(bar_data_from_ga94_payload(&[]), None);
    assert_eq!(bar_data_from_ga94_payload(&[0x06]), None);
    assert_eq!(
        bar_data_from_ga94_payload(&[0x03, 0x80, 0xC0, 0x10]),
        None,
        "not bar_data"
    );
    assert_eq!(
        bar_data_from_t35_payload(&[0xB5, 0x00, 0x31, b'G', b'A', b'9', b'4', 0x06]),
        None
    );
    let mut wrong_country = bar_payload(&[0x06, 0x80, 0xC0, 0x10]);
    wrong_country[0] = 0xB4;
    assert_eq!(bar_data_from_t35_payload(&wrong_country), None);
    let mut wrong_provider = bar_payload(&[0x06, 0x80, 0xC0, 0x10]);
    wrong_provider[1] = 0x01;
    assert_eq!(bar_data_from_t35_payload(&wrong_provider), None);
}
