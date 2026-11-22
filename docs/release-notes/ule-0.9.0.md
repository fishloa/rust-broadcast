# ule 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is a test pass over SNDU framing and reassembly.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

`UleReceiver` multi-packet reassembly, packed follow-on SNDUs and the `PUSI=0` leftover rules, the `Sndu::Length` boundaries, and the `TypeField`/extension-header wire values and lengths are pinned (#3179, #3180, #3181).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `ule-v0.9.0`.
