# dvb-vbi 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit test pass over the EN 301 775 data-unit rules.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

- `DataUnit::validate`'s generation rules are pinned one deviation at a time: Teletext line offsets 0/7/22 pass and 6/23 fail, WSS requires first-field line 23, Closed Captioning requires line 21, monochrome `first_pixel_position` 719 passes and 720 fails, `n_pixels` 1/251 pass and 0/252 fail, and `padded_to` accepts exactly the current length (#3226).
- Every monochrome `first_segment`/`last_segment`/`field_parity` combination decodes and re-encodes byte-exact, the `Serialize` impl of `DataUnitPayload` is exercised through the trait (not just the inherent methods), and a 255-byte stuffing body passes while 256 is rejected (#3227).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `dvb-vbi-v0.9.0`.
