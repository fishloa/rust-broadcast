# media-doctor 0.3.0 — 2026-07-04

Additive (minor). Adds codec-level (signalling-vs-bitstream) diagnostics on top
of the v1 container/timestamp checks.

## New checks (#567) — reuse transmux decoders, no duplicated parsing
- **`CodecSignallingCheck`** — PMT `stream_type` vs actual ES codec; `esds` ASC
  vs ADTS.
- **`check_container_codec`** — `avcC`/`hvcC` profile/level/chroma/bit-depth vs
  in-band SPS; sample-entry dims vs SPS-decoded dims.
- **`FpsCadenceCheck`** — VUI fps vs track timescale cadence.
- **`ParamSetsCheck`** — missing SPS/PPS/VPS before the first IDR/IRAP.
- **`InterlaceCheck`** — `frame_mbs_only_flag == 0` content fact.

Each check has clean-negative + corrupted-positive tests; a hard gate asserts
zero false positives across all 35 committed real captures.

## Compatibility
New dependency: `transmux` (≥ 0.13). Requires broadcast-common ≥ 8.4. MSRV 1.86.
