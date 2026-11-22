# broadcast-dash 0.3.0

_Released 2026-11-22._

Minor for a 0.x crate, breaking: the MPD model's availability and publish times are `jiff::Timestamp` (jiff 1.0), and the crate builds against `broadcast-common` 14. Two model-derived feature flags change meaning so that `parse -> to_xml -> parse` is an identity for a document the model keeps.

## Breaking changes

- `jiff` 0.2 -> 1.0. The `Mpd` availability and publish times and the `xs_datetime` readers and writers take and return jiff 1.x `Timestamp`s. Migration: depend on `jiff` 1.x; the spelling of the values is unchanged.
- Builds against `broadcast-common` 14 (the `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single `broadcast-common` 14.
- Behaviour change: `MpdFeatures::xlink_resolve_to_zero` and `MpdFeatures::inband_event_stream` are derived from what the model keeps, not from any element anywhere in the tree. `xlink_resolve_to_zero` reports an xlink the model carries (a `SegmentList`, `EventStream` or `RequestParam`), not the Period or AdaptationSet the xlink removes; `inband_event_stream` reports only a stream on a retained AdaptationSet, Representation or Preselection. Content the parser drops no longer raises a flag the writer cannot reproduce (#3139).

## Fixed

- `MpdFeatures::service_description` is derived from the `ServiceDescription` elements the model keeps (MPD, Period, `Event`), so an element the model drops no longer raises a flag the writer cannot reproduce and `parse -> to_xml -> parse` is an identity (#3138).
- `xs_duration`'s doc claimed a hand-rolled `PT{secs}S` spelling while the function delegates to `jiff`'s ISO 8601 duration printer; the doc now states the printed form, the exact output for the common shapes is asserted, and three dead `let _` statements are removed (audit 22 finding 69).

## Tests

The golden refuses to re-record under `CI` and names `xmllint --schema` as its independent oracle. The role tokens and their round trip, the `SegmentTemplate` grammar and inverse matching, `xs:dateTime`'s zone/range/hour-24 rules, the writer's per-element branches, the parser ceilings at exactly max and max+1, the attribute edges, and the inheritance helpers are pinned (audit 21/22 findings 25-35).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4020 (`thiserror`), plus #4008/#3992's epoch truthfulness in the CHANGELOG.

---

Published from tag `broadcast-dash-v0.3.0`.
