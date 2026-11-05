# transmux 0.15.2 — 2026-07-09

Patch release: a real bug found by production use (issue #641). No breaking
changes, no new API surface.

## Fixed

- **DVB `stream_type 0x06`/`0x15` Dolby/DTS audio never classified past
  opaque data** (#641). AC-3/E-AC-3/DTS carried the standard DVB way —
  `stream_type 0x06` (PES private data) or `0x15` (metadata in PES) plus an
  AC-3 (`0x6A`), enhanced AC-3 (`0x7A`), or DTS (`0x7B`) ES_info descriptor,
  per ETSI EN 300 468 — fell through to opaque `CodecConfig::Data` and was
  silently dropped from HLS/fMP4 output, exactly like the native
  `0x81`/`0x87`/`0x8*` stream_types would have been recognised. The PMT
  parser now consults the ES_info descriptor loop for those two
  `stream_type`s and reclassifies to the matching audio codec, reaching the
  existing `ConfigProbe::Ac3`/`Eac3`/`Dts` syncframe recovery unchanged.

  New fixture `fixtures/ts/dolby/eac3_dvb_0x06.ts`: real E-AC-3 syncframes
  (already captured in `eac3.ts`) re-muxed under `stream_type 0x06` + a real
  `enhanced_AC3_descriptor` instead of the native `0x87`. Independently
  verified structurally valid with TSDuck's `tstables`.

  Fixing the classifier surfaced a genuine, previously-undiscovered issue in
  an existing committed fixture: `fixtures/ts/m6-single.ts` (a real M6
  French TV capture used throughout the test suite) turns out to carry 3
  real E-AC-3 audio tracks that were being silently dropped by every test
  that demuxed it. Test expectations in `any_stream.rs`/`ir_timing.rs` were
  corrected to match the fixed (real) behavior.

## Compatibility

No breaking changes — bugfix only. MSRV 1.86, edition 2024. `media-doctor`
and `timed-metadata`'s existing `transmux = { version = "0.15" }` path-dep
requirement already covers this release; no version-req bump needed there.
