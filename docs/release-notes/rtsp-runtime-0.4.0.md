# rtsp-runtime 0.4.0

**Release date:** 2026-07-29

Breaking dependency bump: requires `broadcast-common` 9. No functional or API change in this crate itself — the bump exists because staying on `broadcast-common` 8 caused trait-resolution errors when a consumer mixed this crate with 9-based siblings.

## What changed

- Requires `broadcast-common` 9.

## Migration

Requires `broadcast-common` 9. No API changes in this crate.
