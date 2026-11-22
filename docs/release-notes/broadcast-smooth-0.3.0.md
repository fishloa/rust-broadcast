# broadcast-smooth 0.3.0

_Released 2026-11-22._

Minor for a 0.x crate. No functional change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. Internally `SmoothParseError` now derives `thiserror::Error` (messages unchanged) and implements `core::error::Error` without `std`.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Changed

- Internal: `SmoothParseError` derives `thiserror::Error` (messages unchanged) (#3962).

## Tests

A second `<Protection>`/`ProtectionHeader` never replaces the first, malformed XML keeps its reader position and message, every `SmoothParseError` variant's `Display` text is asserted literally (#3040), and `hex_decode` accepts exactly `MAX_CODEC_PRIVATE_DATA_HEX_LEN` characters while `default_limits` admits twice that so an oversized `CodecPrivateData` reports the typed error rather than a generic limit (#3039).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `broadcast-smooth-v0.3.0`.
