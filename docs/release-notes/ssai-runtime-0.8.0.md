# ssai-runtime 0.8.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14, `broadcast-hls` 0.9, `mp4-emsg` 0.9 and `scte35-splice` 8, which is the breaking part; the `DateRange` values it renders are `broadcast-hls` 0.9 ones. The work is a test pass.

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Tests

- **`SessionStore::is_empty` is checked while the store is populated.** It was only asserted empty, so a constant answer could make a host discard a populated store (audit 22 finding 48, #3016).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3992 (epoch line in the CHANGELOG), #4021 (test bullets under `### Tests`).

---

Published from tag `ssai-runtime-v0.8.0`.
