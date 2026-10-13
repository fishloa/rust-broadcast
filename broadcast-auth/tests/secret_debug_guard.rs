//! Tripwire (audit umbrella #1142): no derived `Debug` over a secret-named field.
//!
//! See `tools/secret_debug_guard.inc.rs` for what it scans and its limits.

include!("../../tools/secret_debug_guard.inc.rs");

/// `(file suffix, type, reason)`: types whose secret-looking field name is not a secret.
const ALLOW: &[(&str, &str, &str)] = &[];

#[test]
fn no_derived_debug_over_secret_fields() {
    assert_no_derived_debug_on_secrets(env!("CARGO_MANIFEST_DIR"), ALLOW);
}

#[test]
fn no_handrolled_constant_time_compare() {
    assert_no_handrolled_ct_compare(env!("CARGO_MANIFEST_DIR"));
}
