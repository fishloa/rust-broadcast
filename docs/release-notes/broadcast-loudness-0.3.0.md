# broadcast-loudness 0.3.0

**Release date:** 2026-08-10

MSRV bump plus removal of a dead error variant. No behaviour change for any working caller.

## What changed

- MSRV raised to **1.95.0** (issue #949), workspace-wide. No functional or API change from this alone.
- Removed the dead public `Error::NotImplemented` variant (#941 row 4) — it was never constructed anywhere in the crate.

## Migration

No action required for a `match` on `Error` that already carries a wildcard arm — `Error` is `#[non_exhaustive]`, so removing an unused variant is not a breaking change for well-formed callers. A `match` that specifically named `Error::NotImplemented` needs that arm removed.
