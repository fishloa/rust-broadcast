# ts-fix 0.3.1 — 2026-07-21

Patch. Dependency-floor bump only — no behaviour change.

## Changed

- Widen the internal `mpeg-ts` dependency from `0.2` to `0.3` (issue #663).
  `ops::psi_regen::PsiRegenOp`'s internal PAT rebuild now calls the renamed
  `mpeg_ts::mux::SectionPacketiser`/`packetise` (was `SectionPacketizer`/
  `packetize`) — an internal identifier rename following `mpeg-ts` 0.3's
  British-spelling rename. No public API or behaviour change to `ts-fix`.

## Compatibility

MSRV 1.86. Requires `mpeg-ts ≥ 0.3`.
