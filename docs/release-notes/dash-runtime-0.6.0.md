# dash-runtime 0.6.0

_Released 2026-11-17._

Minor-class release (0.5 -> 0.6) with no runtime behaviour change: the only functional change is an over-limit test pinning `MAX_BASE_CANDIDATES`, so removing the cap no longer passes the suite. The bump follows the workspace, and the `media-plane` requirement moves to 0.13.

## Upgrading

- `media-plane` 0.12 -> 0.13 (the requirement is now `0.13.0`).

## Changed

- Test-only: `MAX_BASE_CANDIDATES` has an over-limit test that fails if the cap is removed (audit 20 finding 52, #2863).

---

Published from tag `dash-runtime-v0.6.0`.
