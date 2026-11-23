# playout-runtime 0.8.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against the wave's epochs (`broadcast-common` 14, `mp4-emsg` 0.9, `scte35-splice` 8, `ssai-runtime` 0.8), which is the breaking part; the `splice_insert()` sections it emits are `scte35-splice` 8 ones. The work is the audit-22 test pass.

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Tests

- **`Schedule::is_empty` is checked while the schedule is populated.** It was only asserted empty, so a constant answer could make a host skip a populated schedule (audit 22 finding 48, #3016).
- **The 33-bit `break_duration()` cap sits exactly at 2^33 - 1 ticks.** A duration of 2^33 ticks or more is refused with `BreakDurationTooLong`, and the largest expressible one round-trips through the section serializer (audit 22 finding 47, #3015).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4021 (test bullets under `### Tests`).

---

Published from tag `playout-runtime-v0.8.0`.
