# `mpeg2_mp2_pes_misaligned.ts` provenance (issue #638)

Derived from the real MP2 audio elementary stream already captured in
`mpeg2_mp2.ts` (a real broadcast MPEG-2 video + MP2 audio TS): its own PES
packetization happens to be frame-aligned, so it never exercises
`split_mpeg_audio_frames`'s resync path. Real DVB-S broadcast multiplexers
routinely split PES payloads without regard to audio frame boundaries (see
issue #638), so this fixture reproduces that condition deterministically from
real captured audio bytes rather than fabricated data.

## How it was generated

1. Demuxed `mpeg2_mp2.ts` with `TsDemux` and concatenated its 39 real,
   decoded MP2 (Layer II) frames back into one continuous elementary stream
   (48,901 bytes).
2. Re-chunked that stream into fixed 2000-byte pieces, with no regard for the
   ~1253/1254-byte MP2 frame length — so most PES payload boundaries land
   mid-frame (verified: the 2nd and 3rd PES payloads in the resulting stream
   both start with non-syncword bytes).
3. Re-muxed those chunks as a single-track `CodecConfig::MpegAudio` `Media`
   through `transmux::ts_mux::TsMux` (the crate's own spec-correct TS/PES/
   PAT/PMT builder — same machinery the crate already trusts for muxing), one
   chunk = one PES.
4. Independently verified structurally valid with TSDuck's `tsanalyze`: no
   invalid syncs, no discontinuities, PID 0x0100 correctly recognized as
   "MPEG-1 Audio (Audio layer II, ... @44,100Hz)" by TSDuck's own independent
   parser.

The one-off generator script (`transmux/examples/_gen_638_fixture.rs`) is not
committed — it was run once and deleted; this file documents the recipe.
Regenerate by reproducing the steps above (see
`transmux/tests/mpeg_legacy.rs`'s `mpeg_audio_resyncs_across_pes_boundaries`
for the exact chunking parameters).

## Used by

- `transmux/tests/mpeg_legacy.rs::mpeg_audio_resyncs_across_pes_boundaries`
