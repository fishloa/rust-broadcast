# rist-runtime 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate. One real fix (an SDES with RFC 3550 P-bit padding no longer fails the whole compound packet), the rest is the epoch move and the audit-23 test pass. The crate builds against `broadcast-common` 14 and `rtp-packet` 0.9; the RTCP packets it returns are 0.9 ones.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `rtp-packet` 0.8 -> 0.9). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Fixed

- A compound packet whose SDES carries RFC 3550 §6.4.1 P-bit padding no longer fails the whole parse with `BufferTooShort`: the SDES parser is handed the full claimed sub-packet (its own length field covers the padding) and the region is preserved verbatim alongside the sub-packet instead of being taken as the CNAME (#3334).

## Tests

Compound-packet padding, ordering and length contracts are pinned: a padded SR, a non-zero padding fill (rejected with the exact count/body), a longer zero fill, unmodelled packets keeping non-zero padding in a dirty output buffer, wire-position order for unknown packets between modelled slots, per-direction preservation of unmodelled RTT feedback and NACKs, a padded sole SDES and a padded second SDES, a header-only 4-byte sub-packet, trailing bytes below a common header, `serialized_len()` equal to the bytes written, and `RistPadding`'s accessors (audit 23 #3330-#3336, #3341). Standalone NACK / RTT Echo parsing is pinned against hostile and empty bodies, with every truncation a `BufferTooShort` and the exact `need`/`have` asserted (audit 23 #3337-#3340).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `rist-runtime-v0.9.0`.
