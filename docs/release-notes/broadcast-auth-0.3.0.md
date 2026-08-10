# broadcast-auth 0.3.0

**Release date:** 2026-08-10

MSRV bump plus a documentation fix. No public API or behaviour change.

## What changed

- MSRV raised to **1.95.0** (issue #949), workspace-wide. No functional or API change from this alone.
- Doc accuracy fix (#941 row 5): the README's "Schemes" section now lists the already-shipped `SignedUrl` scheme (`Verifier::signed_url`, issue #747) — it was documented in the crate-root doc comment but had been omitted from the README. The scheme itself is unchanged; only the README was out of date.

## Migration

No API changes; no action required.
