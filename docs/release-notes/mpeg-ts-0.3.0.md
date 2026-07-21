# mpeg-ts 0.3.0 — 2026-07-21

**Breaking: British-spelling rename.** `mux::SectionPacketizer` and its
`packetize`/`packetize_into` methods are renamed to `mux::SectionPacketiser`
and `packetise`/`packetise_into` (issue #663, the multimux-hub epic — the
workspace is standardising on British spelling for this family of
identifiers; `transmux`'s `Rtp(Stream)Packetizer`/`Depacketizer` family
follows the same rename in this same release wave). Pure rename —
behaviour-preserving, no functional change.

## Breaking changes

- `mux::SectionPacketizer` → `mux::SectionPacketiser`.
- `SectionPacketizer::packetize` → `SectionPacketiser::packetise`.
- `SectionPacketizer::packetize_into` → `SectionPacketiser::packetise_into`.
- `SiMux`'s internal field/doc references updated to match (no public API
  change beyond the type/method names above).

## Migration

| Old (0.2.x) | New (0.3.0) |
|---|---|
| `mpeg_ts::mux::SectionPacketizer` | `mpeg_ts::mux::SectionPacketiser` |
| `packetizer.packetize(&sections)` | `packetiser.packetise(&sections)` |
| `packetizer.packetize_into(&sections, &mut out)` | `packetiser.packetise_into(&sections, &mut out)` |

A find-and-replace of `Packetizer`→`Packetiser`, `packetize`→`packetise` in
your own call sites is sufficient — the wire format and behaviour are
unchanged.

## Compatibility

MSRV unchanged (1.86). `no_std` + `alloc` posture unchanged.
