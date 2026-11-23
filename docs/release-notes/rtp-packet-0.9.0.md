# rtp-packet 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit-23 test pass over the RTP/RTCP codecs.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

The RFC 3550 §A.1 cycle counter and sequence arithmetic, the header extension (one-byte and two-byte RFC 8285 forms), the CSRC list bounds, the RTCP SR/RR/SDES/BYE/APP layouts and the compound-packet rules are pinned at their boundaries, with truncations asserted as typed errors rather than panics.

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `rtp-packet-v0.9.0`.
