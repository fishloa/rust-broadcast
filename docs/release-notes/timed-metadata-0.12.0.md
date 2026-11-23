# timed-metadata 0.12.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own beyond the epoch: the crate builds against `broadcast-common` 14, `broadcast-hls` 0.9, `cc-data` 0.11, `dvb-vbi` 0.9, `mp4-emsg` 0.9 and `scte35-splice` 8, so the splice/`DateRange`/`emsg` values it converts are those versions' types. Hex goes through `broadcast_common::hex`; the direct `hex` dependency is gone and output is unchanged.

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Changed

- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).
- Hex goes through `broadcast_common::hex`; the direct `hex` dependency is removed. `DATERANGE` `SCTE35-*` encoding and decode errors are unchanged (#3957).

## Tests

The Hamming-8/4 encoder is pinned to all 16 spec-equation codewords (single- and double-bit errors included), every `PageHeader` sub-code field and C4-C14 control bit is decoded in isolation, rows 24-31 hit the assembler's row guard, adjacent Teletext boxes keep exactly one separator, the CEA-708 cue extractor's push/close/finalize path and both `open_cue()` views are driven end to end, and DATERANGE's zero/fractional/negative duration edges plus the SCTE-35 BreakEnd-to-`SCTE35-IN` mapping are asserted (#3048-#3055).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3992 (epoch line in the CHANGELOG), #4020 (`thiserror`).

---

Published from tag `timed-metadata-v0.12.0`.
