//! Drift-guard for the spec/field-enum label convention (issue #204): every
//! public enum has `impl_spec_display!` (delegating to a hand-written
//! `name()`) or a hand-written `Display`, unless it is in the skip list.
//!
//! Skip list (structured errors and data-carrying ADTs a static label would
//! make lossy):
//! - `MpdError`, `AddressError`, `PlanError`, `TimingError`, `DateTimeError`,
//!   `InheritError`: `thiserror` errors, not spec labels.
//! - `OfferOutcome`: wraps an `AdoptReason` or a `RejectReason`, both labelled.
//! - `SegmentInfo`: data-carrying (`Template`/`List`/`Base`) resolution result.

mod common;

use std::collections::BTreeSet;

const SKIP: &[&str] = &[
    "MpdError",
    "AddressError",
    "PlanError",
    "TimingError",
    "DateTimeError",
    "InheritError",
    "OfferOutcome",
    "SegmentInfo",
];

fn has_display(all: &str, name: &str) -> bool {
    let macro_needle = format!("impl_spec_display!({name}");
    let boundary =
        |rest: &str| !matches!(rest.chars().next(), Some(c) if c.is_alphanumeric() || c == '_');
    all.lines().any(|line| {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(&macro_needle)
            && boundary(rest)
        {
            return true;
        }
        t.strip_prefix("impl")
            .and_then(|after| {
                after
                    .find(&format!("Display for {name}"))
                    .map(|p| (after, p))
            })
            .is_some_and(|(after, p)| {
                let rest = &after[p + format!("Display for {name}").len()..];
                boundary(rest)
            })
    })
}

#[test]
fn every_public_spec_enum_has_a_display_impl() {
    let all: String = common::scan::src_files()
        .into_iter()
        .map(|(_, s)| s)
        .collect::<Vec<_>>()
        .join("\n");
    let mut enums = BTreeSet::new();
    for line in all.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("pub enum ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                enums.insert(name);
            }
        }
    }
    assert!(enums.len() >= 15, "scanner found only {enums:?}");
    let missing: Vec<_> = enums
        .iter()
        .filter(|n| !SKIP.contains(&n.as_str()) && !has_display(&all, n))
        .collect();
    assert!(
        missing.is_empty(),
        "public enum(s) missing a #204 Display/name(): {missing:?}"
    );
}

#[test]
fn display_matches_name_for_a_sample() {
    use dash_runtime::timing::clock::{AdoptReason, ClockSource};
    use dash_runtime::timing::utc_timing::UtcScheme;
    assert_eq!(
        ClockSource::UtcTiming.to_string(),
        ClockSource::UtcTiming.name()
    );
    assert_eq!(AdoptReason::Step.to_string(), "step");
    assert_eq!(UtcScheme::HttpXsdate.to_string(), "http_xsdate");
}
