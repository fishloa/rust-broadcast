# ttml-subtitle 0.1.0 — 2026-07-30

**Initial release.** New crate.

W3C TTML2 / IMSC 1.1 timed-text subtitle parser and validator.

- Full TTML2 element tree: 26 element types, 56 style properties.
- Exhaustive `<time-expression>` grammar: clock-time, offset-time, wallclock-time with frame/tick/SMPTE constraint enforcement.
- IMSC 1.1 profile validation: 159-row feature disposition table, §7.12 "must reject" structural constraints.
- Parse/validate split: inspect non-conformant documents before rejecting.
- Semantic round-trip (parse → serialize → re-parse → equal); no raw-passthrough. Enumerated divergence in README.
- From-scratch authoring via `Default` on all element types.
- Real-fixture tested against 11 W3C IMSC conformance suite documents.
- `no_std` + `alloc` compatible. Optional `serde`.
- `#[non_exhaustive]` on all public enums; `#204` label convention.

See the [lockstep 9.1.0 release note](v9.1.0.md) for context.
