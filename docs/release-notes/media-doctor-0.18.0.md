# media-doctor 0.18.0

_Released 2026-11-22._

Minor for a 0.x crate. One behavioural fix (`PcrCheck` no longer carries an unreachable indicator branch), the rest is the epoch move and the audit test pass. The crate builds against `broadcast-common` 14, `broadcast-dash` 0.3, `broadcast-hls` 0.9, `container-probe` 0.6, `dvb-conformance` 16, `dvb-si` 16, `dvb-stream` 0.11, `mpeg-pes` 0.10, `mpeg-ts` 0.10, `scte35-splice` 8, `timed-metadata` 0.12 and `transmux` 0.33.

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Fixed

- `PcrCheck` no longer carries the unreachable TR 101 290 indicator 2.3a (`pcr-repetition`) branch: every forward PCR value step over 100 ms without `discontinuity_indicator` is already reported as 2.3b (`pcr-discontinuity`), so 2.3a can never fire in file mode (#3533).

## Tests

The fMP4/CMAF structural checks, the TS structural checks and the HLS playlist checker are exercised against the committed fixtures; the `mediastreamvalidator_oracle` integration test runs Apple's own validator when it is on PATH and skips loudly otherwise.

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `media-doctor-v0.18.0`.
