# media-doctor 0.4.3 — 2026-07-26

Patch. Dependency-floor bump only — no functional or behaviour change.

## Changed

- Widen the `transmux` dependency from `0.18` to `0.20` (commit 28c8d7a1), picking up
  transmux's media-plane IR changes: `Sample.data` is now `bytes::Bytes` and the IR
  types (`Media`, `Track`, `TrackSpec`, `Sample`, `DemuxEvent`, …) moved into a
  `transmux::ir` module (re-exported at the crate root). media-doctor's own public
  API is unchanged — no transmux type crosses its boundary — so this remains a
  patch release.

## Compatibility

MSRV 1.86. Requires `transmux ≥ 0.20`.
