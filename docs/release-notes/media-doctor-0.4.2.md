# media-doctor 0.4.2 — 2026-07-21

Patch. Dependency-floor bumps only — no functional or behaviour change.

## Changed

- Widen the `transmux` dependency from `0.17` to `0.18` and the internal
  `mpeg-ts` dependency from `0.2` to `0.3` (issue #663). `transmux` 0.18.0
  is a breaking release for its own `Rtp(Stream)Packetizer`/`Depacketizer`
  family (British-spelling rename) and `mpeg-ts` 0.3.0 similarly renames
  `SectionPacketizer`/`packetize`; `media-doctor` does not use either
  renamed identifier directly except in its own `#[cfg(test)]` fixture
  helpers, which are updated to `mpeg_ts::mux::SectionPacketiser`/
  `packetise`.

## Compatibility

MSRV 1.86. Requires `transmux ≥ 0.18`, `mpeg-ts ≥ 0.3`.
