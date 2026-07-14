# media-doctor 0.4.1 — 2026-07-14

Patch. Dependency-floor bump only — no code or behaviour change.

## Changed
- Widen the `transmux` dependency from `0.15` to `0.16`. transmux 0.16.0 adds
  the CENC/CBCS encrypt path (issue #564) and makes one breaking struct-literal
  change (`dash::ContentProtectionSystem` gained a `pssh` field); media-doctor
  does not touch that type, so this is a floor bump to keep the crate building
  against current transmux.

## Compatibility
- MSRV 1.86. Requires `transmux ≥ 0.16`, `broadcast-common ≥ 8.4`.
