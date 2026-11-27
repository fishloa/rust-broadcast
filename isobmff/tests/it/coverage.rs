//! Tier 0 coverage: the crate's typed box set against the private registry of
//! ISO/IEC 14496-12:2026 box types.
//!
//! The registry (`private/specs-verified/mpeg/isobmff/box-registry-14496-12-2026.tsv`,
//! 167 rows) is loaded through [`test_fixtures::spec_string`], so the test skips
//! loudly without the private checkout and hard-fails when it is present but the
//! entry is missing.
//!
//! [`isobmff::coverage::TYPED_BOXES`] is the crate's typed set; each row proves
//! itself. Every registry type not in that table is pinned below under its tier
//! heading from issue #4167. Typing a box without deleting it from the pinned
//! list fails (a type cannot be both), and a registry type in neither list
//! fails (coverage cannot shrink silently).

use std::collections::BTreeSet;

use isobmff::coverage::TYPED_BOXES;

/// Tier 3: metadata tracks.
const UNTYPED_TIER_3: &[&str] = &[
    "bxml", "cprt", "etyp", "hmhd", "imda", "keyd", "loca", "nmhd", "otyp", "setu", "strd", "stri",
    "strk", "uri ", "uriI", "xml ",
];

/// Tier 4: newer signalling.
const UNTYPED_TIER_4: &[&str] = &[
    "aedb", "aelm", "aepp", "aepr", "aesb", "aesd", "amve", "ardi", "assp", "cclv", "cinf", "csch",
    "mehd", "rinf", "scrb", "spki", "vvhd",
];

/// Tier 5: legacy delivery and hint tracks (lowest value).
const UNTYPED_TIER_5: &[&str] = &[
    "btrt", "dimm", "dmax", "dmed", "drep", "extr", "fdpa", "fdsa", "feci", "fecr", "fiin", "fire",
    "fpar", "gitn", "hinf", "hnti", "istm", "maxr", "npck", "nump", "paen", "payt", "pmax", "rcsr",
    "rsot", "rssr", "rtp ", "rtpx", "sdp ", "segr", "snro", "srpp", "stsh", "tPAT", "tPMT", "tims",
    "tmax", "tmin", "totl", "tpay", "tpyl", "trpy", "tsro", "tssy", "tsti", "tyco",
];

const REGISTRY_ENTRY: &str = "mpeg/isobmff/box-registry-14496-12-2026.tsv";
const WHO: &str = "isobmff coverage";

fn registry_types() -> Option<BTreeSet<String>> {
    let text = test_fixtures::spec_string(REGISTRY_ENTRY, WHO)?;
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fourcc = line.split('\t').next().unwrap_or_default();
        if fourcc == "fourcc" {
            continue;
        }
        out.insert(fourcc.to_string());
    }
    Some(out)
}

fn typed_names() -> BTreeSet<String> {
    TYPED_BOXES.iter().map(|row| row.name.to_string()).collect()
}

fn pinned_untyped() -> BTreeSet<String> {
    UNTYPED_TIER_3
        .iter()
        .chain(UNTYPED_TIER_4)
        .chain(UNTYPED_TIER_5)
        .map(|name| (*name).to_string())
        .collect()
}

#[test]
fn each_typed_row_proves_itself() {
    for row in TYPED_BOXES {
        (row.proves)().unwrap_or_else(|err| {
            panic!("{} ({}) does not parse: {err:?}", row.name, row.rust_type)
        });
    }
}

#[test]
fn registry_partitions_into_typed_and_pinned_untyped() {
    let Some(registry) = registry_types() else {
        return;
    };
    assert_eq!(registry.len(), 167, "registry row count");
    let typed = typed_names();
    let untyped = pinned_untyped();

    let both: Vec<_> = typed.intersection(&untyped).cloned().collect();
    assert!(
        both.is_empty(),
        "a type is both typed and pinned untyped: {both:?}"
    );

    let unpinned: Vec<_> = registry
        .difference(&typed)
        .filter(|name| !untyped.contains(*name))
        .cloned()
        .collect();
    assert!(
        unpinned.is_empty(),
        "registry types in neither list: {unpinned:?}"
    );

    let stale: Vec<_> = untyped.difference(&registry).cloned().collect();
    assert!(
        stale.is_empty(),
        "pinned untyped types not in the registry: {stale:?}"
    );

    let unexpected: Vec<_> = typed.difference(&registry).cloned().collect();
    assert!(
        unexpected.is_empty(),
        "typed table rows that are not registry types: {unexpected:?}"
    );
}

#[test]
fn readme_states_the_real_counts() {
    let Some(registry) = registry_types() else {
        return;
    };
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
        .expect("README.md is committed");
    let marker = "Typed ";
    let line = readme
        .lines()
        .find(|line| line.contains(marker) && line.contains("registry box types"))
        .expect("README states 'Typed M of N registry box types'");
    let rest = &line[line.find(marker).expect("marker") + marker.len()..];
    let mut numbers = rest.split_whitespace();
    let typed_count: usize = numbers
        .next()
        .and_then(|token| token.parse().ok())
        .expect("README typed count");
    assert_eq!(typed_count, TYPED_BOXES.len(), "README M");
    let of = numbers.next();
    assert_eq!(of, Some("of"), "README 'of'");
    let total: usize = numbers
        .next()
        .and_then(|token| token.parse().ok())
        .expect("README registry total");
    assert_eq!(total, registry.len(), "README N");
    assert_eq!(TYPED_BOXES.len() + pinned_untyped().len(), registry.len());
}
