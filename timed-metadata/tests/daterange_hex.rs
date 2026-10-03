//! `SCTE35-*` attribute hex is attacker-reachable (it comes from a remote
//! playlist): it must never panic and must be strict hex.
use timed_metadata::DateRange;

fn line(value: &str) -> String {
    format!("#EXT-X-DATERANGE:ID=\"x\",START-DATE=\"2020-01-01T00:00:00Z\",SCTE35-OUT={value}")
}

#[test]
fn multibyte_character_in_hex_is_an_error_not_a_panic() {
    // 4 bytes, even length: the old `&h[0..2]` cuts the `é` in half.
    assert!(DateRange::parse_tag_line(&line("0xaéb")).is_err());
}

#[test]
fn a_sign_is_not_a_hex_digit() {
    // old: from_str_radix("+1", 16) == Ok(1) => bytes [1, 1]
    assert!(DateRange::parse_tag_line(&line("0x+1+1")).is_err());
    assert!(DateRange::parse_tag_line(&line("0x-1-1")).is_err());
}

#[test]
fn odd_length_and_non_hex_are_errors() {
    assert!(DateRange::parse_tag_line(&line("0xABC")).is_err());
    assert!(DateRange::parse_tag_line(&line("0xZZ")).is_err());
}

#[test]
fn valid_hex_round_trips_uppercase_and_accepts_either_prefix_case() {
    let dr = DateRange::parse_tag_line(&line("0xFC3021")).unwrap();
    assert_eq!(dr.scte35.as_ref().unwrap().raw, vec![0xFC, 0x30, 0x21]);
    assert!(dr.to_tag_line().unwrap().contains("SCTE35-OUT=0xFC3021"));
    let lower = DateRange::parse_tag_line(&line("0Xfc3021")).unwrap();
    assert_eq!(lower.scte35.unwrap().raw, vec![0xFC, 0x30, 0x21]);
}
