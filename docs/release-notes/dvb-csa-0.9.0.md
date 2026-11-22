# dvb-csa 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate. No functional change: the crate builds against `mpeg-ts` 0.10 (and transitively `broadcast-common` 14, the jiff 1.0 epoch), which is the breaking part, and the `rand` dev-dependency is gone.

## Breaking changes

- Builds against `mpeg-ts` 0.10 (transitively `broadcast-common` 14, the `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each.

## Changed

- Internal: the `rand` dev-dependency is dropped (#3972, #4025); no consumer-visible change.

## Tests

The zeroize-on-drop guarantee is pinned for all five key-bearing types (round-key schedules, control word, both stream ciphers), so a removed `Drop` fails the suite (#3206), and the `KeyParity` labels and `Display` output are asserted (#3207).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4025 (dev-dependency removal recorded).

---

Published from tag `dvb-csa-v0.9.0`.
