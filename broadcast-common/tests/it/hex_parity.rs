//! `hex_encode` is the public API the hex crate now backs: pin its exact
//! contract so the delegation cannot change it.
use broadcast_common::hex::hex_encode;

#[test]
fn every_byte_value_is_two_lowercase_digits() {
    for b in 0..=255u8 {
        assert_eq!(hex_encode(&[b]), format!("{b:02x}"));
    }
}

#[test]
fn long_input_has_no_separators_or_prefix() {
    let data: Vec<u8> = (0..=255u8).cycle().take(1021).collect();
    let text = hex_encode(&data);
    assert_eq!(text.len(), 2042);
    assert!(
        text.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    );
    assert_eq!(hex_encode(&[]), "");
}
