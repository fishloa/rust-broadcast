# dvb-ci 0.14.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14, `dvb-si` 16 and `mpeg-ts` 0.10, which is the breaking part. The work is a test pass over the EN 50221 and TS 101 699 / TS 103 205 APDU layouts.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `dvb-si` 15 -> 16, `mpeg-ts` 0.9 -> 0.10). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the `dvb-si` sections this crate takes are the 16 ones.

## Tests

The APDU labels, `serialize_into` length returns, 12-bit length fields, the 255/4095 ceilings and the truncation boundaries are pinned against the surviving cargo-mutants (audit 23).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `dvb-ci-v0.14.0`.
