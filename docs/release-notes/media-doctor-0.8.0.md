# media-doctor 0.8.0

**Release date:** 2026-08-10

Fixes two under-enforcement bugs in the `cc-anomaly` check's legal-duplicate
rule, consolidates PTS wrap-math onto a shared workspace primitive, and moves
the version specifically to close an epoch-purity gap against `transmux`.
Carries the workspace-wide MSRV bump. No breaking changes to this crate's
public API.

## Why the version moved

In-tree `media-doctor` had drifted to requiring `transmux ^0.23` without a
version bump of its own, while the *published* 0.7.0 on crates.io still
requires `transmux ^0.22` — two different `transmux` epochs sharing one
published version bucket. That is unsafe for consumers on either side of the
line: someone who pins `media-doctor = "0.7"` today can be handed either a
build against `transmux` 0.22 or one against 0.23 depending on when they
resolved it, with no way to tell from the version number alone which
`transmux` API surface they actually get. This release fixes that by moving
to 0.8.0 for the epoch change (issue #858) and raising the floor again to
`transmux ^0.24` in the same release, so the bucket is unambiguous going
forward. A consumer who needs the old `transmux` 0.22/0.23 line must stay on
`media-doctor` 0.7.0 and not float to `^0.7`; this was caught by
`tools/check-published-dep-consistency.py` when `compliance-probe` became the
first in-tree consumer to notice.

## What changed

- **`cc-anomaly` (`CcAnomalyCheck`) under-enforced ITU-T H.222.0 /
  ISO/IEC 13818-1 §2.4.3.3's "legal duplicate" rule in two ways**, both now
  delegated to the shared `broadcast_common::ts_dup::check_duplicate` (the
  same primitive `dvb-conformance` uses for issue #956):
  - Byte-identity was payload-only: a packet whose adaptation-field content
    changed (e.g. `splice_countdown` or OPCR) while the payload stayed
    identical was wrongly accepted as a legal duplicate. Per §2.4.3.3, only
    the PCR field is exempt from byte-for-byte identity.
  - "Two, and only two" was never enforced: an unbounded run of
    byte-identical (PCR excepted) repeats on the same continuity counter was
    silently accepted forever; a third consecutive repeat is now itself
    flagged.
  - On the committed `m6-duplicate.ts` fixture this does not change the
    finding count (879, unchanged), confirmed by an independent oracle added
    to `tests/integration.rs`; a stream that does carry an AF-body-only
    difference or a third consecutive repeat will now report additional
    findings this check previously missed.
  - Also corrects a stale doc comment on the fixture test that claimed
    `m6-duplicate.ts` has "4 true legal duplicates" — the correct count
    under the strict byte-identity rule is 5.
- `pts_check`'s wrap-aware backward-jump delta now delegates to
  `broadcast_common::clock33::wrapping_forward_distance`, the shared owner
  of this math across `timed-metadata`, `transmux`, and `compliance-probe`.
  Identical computation, no behaviour change; internal only, no public API
  change.
- `transmux` floor raised to `^0.24` (see above).
- MSRV raised to **1.95.0** (issue #949) — the workspace-wide move that
  removes the separate MSRV lane `webrtc-runtime`'s optional `media` feature
  previously required. Only let-chain / `is_multiple_of` style adoption
  where the 1.95 lints require it.

## Migration

No API changes to this crate's public surface. Pin `media-doctor = "0.8"` to
get the `transmux ^0.24` line; a consumer that still needs `transmux`
0.22/0.23 must stay on the exact `media-doctor = "=0.7.0"` and not resolve
against a floating `^0.7`, since the published 0.7.0 bucket does not
reliably mean one `transmux` epoch.
