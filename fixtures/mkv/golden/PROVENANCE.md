# PROVENANCE

These are differential goldens for `broadcast-matroska`'s sans-IO reader and
writer (streaming-first Phase 5, slice 3). They were recorded on 2026-11-25 from
`origin/main` commit `08593e64c`, before either crate code was changed, by a
throwaway program in a `.worktrees/` checkout of that commit:

- `track` lines are `transmux::WebmDemux`'s output for the fixture: per track,
  the Matroska `CodecID`, sample count, first and last presentation time in the
  crate's IR ticks (milliseconds), and the FNV-1a 64-bit hash of the
  concatenated sample bytes in decode order.
- `mkv` is `transmux::MkvMuxStage::new(Duration::from_millis(5_000))` fed the
  demuxed `Media` through `drive_media`, with the `Info.Duration` patch
  applied: the output's byte length and FNV-1a 64-bit hash.

The recorded program is not committed (it depends on `transmux`, which this
crate must stay below). Regenerate only against the same `origin/main` commit;
never re-pin these values to make a test pass.

The fixture bytes themselves are unchanged (`fixtures/mkv/*`, six files across
H.264/HEVC, VP8/VP9 and Opus/Vorbis; see `../PROVENANCE.md`).
