# ttml-subtitle 0.2.0

**Release date:** 2026-08-10

A documentation-accuracy fix and the workspace-wide MSRV bump. No behaviour
or API change.

## What changed

- Doc accuracy fix (#941 row 3): the crate-root doc comment and the
  README's "Round-Trip Guarantee" table now disclose that foreign/unknown
  child elements are silently dropped during parsing and never re-emitted.
  This behaviour already existed and is permitted by TTML2 §7.2's
  extensibility rule — only the disclosure was missing, not the behaviour.
- MSRV raised to **1.95.0** (issue #949), the workspace-wide MSRV
  unification.

## Migration

No API changes; no action required. Consumers must build with rustc 1.95.0
or newer.
