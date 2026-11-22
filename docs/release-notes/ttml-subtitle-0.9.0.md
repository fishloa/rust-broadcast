# ttml-subtitle 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate. One real fix (namespace preservation no longer rewrites a foreign attribute or element to an inner-scope `xmlns:` override), and the breaking part is the epoch move to `broadcast-common` 14 (the jiff 1.0 epoch).

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Fixed

- Namespace preservation no longer rewrites an outer-scope foreign attribute or element to an inner-scope `xmlns:` override URI. Each item keeps its recorded prefix only while that prefix still resolves to its namespace, otherwise a fallback prefix is emitted. A foreign namespace bound to a core prefix (`tt`, `ttp`, `tts`, `ttm`, or the default namespace) now gets a fallback binding instead of silently changing the core declaration's meaning.

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `ttml-subtitle-v0.9.0`.
