//! Tripwire (audit umbrella #1142): no derived `Debug` over a secret-named
//! field, and (where `ct_compare`) no hand-rolled constant-time compare.
//!
//! See `tools/secret_debug_guard.inc.rs` for what it scans and its limits. The
//! cipher crates (`dvb-csa`, `rist-runtime`, ...) XOR legitimately, so only the
//! crates listed with `ct_compare: true` get the compare scan.

include!("../../../tools/secret_debug_guard.inc.rs");

pub struct Crate {
    pub name: &'static str,
    /// `(file suffix, type, reason)`: types whose secret-looking field name is
    /// not a secret.
    pub allow: &'static [(&'static str, &'static str, &'static str)],
    pub ct_compare: bool,
}

fn crate_manifest_dir(name: &str) -> String {
    workspace_guards::crate_dir(name).display().to_string()
}

#[test]
fn no_derived_debug_over_secret_fields() {
    for c in crate::secret_debug_data::TABLE {
        assert_no_derived_debug_on_secrets(&crate_manifest_dir(c.name), c.allow);
    }
}

#[test]
fn no_handrolled_constant_time_compare() {
    for c in crate::secret_debug_data::TABLE
        .iter()
        .filter(|c| c.ct_compare)
    {
        assert_no_handrolled_ct_compare(&crate_manifest_dir(c.name));
    }
}
