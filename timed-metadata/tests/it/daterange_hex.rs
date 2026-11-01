//! `SCTE35-*` attribute hex is attacker-reachable (it comes from a remote
//! playlist): it must never panic and must be strict hex.
use timed_metadata::DateRange;

fn line(value: &str) -> String {
    format!("#EXT-X-DATERANGE:ID=\"x\",START-DATE=\"2020-01-01T00:00:00Z\",SCTE35-OUT={value}")
}

#[test]
fn malformed_hex_is_an_error_never_a_panic() {
    let cases = [
        // 4 bytes, even length: the old `&h[0..2]` cut the `é` in half.
        ("multibyte character", "0xaéb"),
        // old: from_str_radix("+1", 16) == Ok(1) => bytes [1, 1]
        ("plus sign", "0x+1+1"),
        ("minus sign", "0x-1-1"),
        ("odd length", "0xABC"),
        ("non-hex digits", "0xZZ"),
    ];
    let accepted: Vec<_> = cases
        .iter()
        .filter(|(_, value)| DateRange::parse_tag_line(&line(value)).is_ok())
        .map(|(label, _)| *label)
        .collect();
    assert!(accepted.is_empty(), "accepted malformed hex: {accepted:?}");
}

#[test]
fn valid_hex_round_trips_uppercase_and_accepts_either_prefix_case() {
    let dr = DateRange::parse_tag_line(&line("0xFC3021")).unwrap();
    assert_eq!(dr.scte35.as_ref().unwrap().raw, vec![0xFC, 0x30, 0x21]);
    assert!(dr.to_tag_line().unwrap().contains("SCTE35-OUT=0xFC3021"));
    let lower = DateRange::parse_tag_line(&line("0Xfc3021")).unwrap();
    assert_eq!(lower.scte35.unwrap().raw, vec![0xFC, 0x30, 0x21]);
}
