# buffering-runtime 0.5.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit-22 test-hardening pass: every label, the PDV/jitter estimator's boundaries, the config validators and the controller's arithmetic are now pinned by value.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

- The label accessors (`State`/`Reason`/`Input`/`SampleOutcome`), `EventQueue::clear`, `MinWindow::is_empty` and the input-log error text arms are pinned literally, so an empty label, a no-op `clear` or a dropped field fails the suite (audit 22 finding 62).
- The jitter estimator's exact PDV sequence, its discontinuity and outlier boundaries, the minimum-window overflow count, the anchored stall memory, the forgetting histogram's weights/quantile/interval count and the per-track liveness and step rules are pinned by value (audit 22 finding 61).
- The config validators' boundaries (min window, bucket, quantile, sample cap, policy order, histogram range), the policy accessors, the controller's over-max/re-arm arithmetic, the target-reporting boundary and granularity floor, and the re-prime/starve hold rules are pinned by value (audit 22 finding 60).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `buffering-runtime-v0.5.0`.
