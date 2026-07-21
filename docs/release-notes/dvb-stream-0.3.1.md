# dvb-stream 0.3.1 — 2026-07-21

Patch. Dependency-floor bump only — no functional or behaviour change.

## Changed

- Widen the internal `mpeg-ts` dependency from `0.2` to `0.3` (issue #663;
  private dependency, used only internally by `SectionStream` — no public
  API change to `dvb-stream`).

## Compatibility

MSRV 1.86. Requires `mpeg-ts ≥ 0.3`.
