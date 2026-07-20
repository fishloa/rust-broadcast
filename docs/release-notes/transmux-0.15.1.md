# transmux 0.15.1 — 2026-07-07

Patch release: a real bug found by production use (issue #638). No breaking
changes, no new API surface.

## Fixed

- **MPEG audio / ADTS frame splitting never resynced past a bad sync**
  (#638). `split_mpeg_audio_frames` and `split_adts_frames` assumed a PES
  payload always starts exactly on a frame boundary. Real DVB-S broadcast
  multiplexers routinely split PES payloads without regard to audio frame
  length, so a misaligned payload silently yielded zero frames — a track
  stuck in `Probing` forever if no buffered PES happened to align, or
  silently dropped samples on an already-live track. Both splitters, and the
  MP2/AAC config-probe backlog scans, now resync forward to the next valid
  frame header instead of bailing on the first byte that isn't one.

  New fixture `fixtures/ts/legacy/mpeg2_mp2_pes_misaligned.ts`: the same 39
  real MP2 frames already captured in `mpeg2_mp2.ts`, re-chunked into
  PES-payload-sized pieces with no regard for frame length — reproducing the
  misalignment `mpeg2_mp2.ts`'s own frame-aligned packetisation never
  exercised. Both the MP2 and the same-class ADTS/AAC regression test were
  verified to fail against the pre-fix code (TDD).

## Compatibility

No breaking changes — bugfix only. MSRV 1.86, edition 2024. `media-doctor`
and `timed-metadata`'s existing `transmux = { version = "0.15" }` path-dep
requirement already covers this release; no version-req bump needed there.
