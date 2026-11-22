# mpeg-pes 0.10.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit-21 regression suite over the PES header.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

Regression tests for the audit-21 survivors: all 256 PES-extension flag subsets round-trip byte-exactly, every ESCR marker bit and both `program_packet_sequence_counter` markers are rejected individually, every DSM trick-mode encoding is pinned, the scrambling-control field round-trips through the header flag byte, the zero-length `PES_packet_length` rule and the 255-byte optional-field limit are pinned, `InvalidSync::found` is asserted, PTS/DTS marker bits are checked per byte and `seconds()` is pinned, and `PesAssembler`'s state accessors plus `StreamId::is_audio` are covered.

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `mpeg-pes-v0.10.0`.
