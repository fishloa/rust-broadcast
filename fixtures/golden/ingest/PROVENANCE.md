# PROVENANCE

Behaviour goldens for the ingest session drive loop (issue #2373), produced by
`multimux/tests/it/ingest_drive_golden.rs`. Each file is the ordered
`SessionEvent` sequence one real publish produces at the session boundary,
followed by the program and track specs it lands and the session's final
health. The shared drive loop both `run_whip` and `run_rtmp` use must keep
them byte-identical.

## Inputs (both real, both committed)

- `whip-publish.txt` — the H.264 access units of `fixtures/ts/h264-aac.ts`
  (Main profile, 320x240, 25 fps; see `fixtures/ts/PROVENANCE.md`), demuxed
  with `transmux::TsDemux` and sent as RFC 6184 single-NAL RTP packets through
  a `WhipIngestSession`. The session's `avcC` is the capture's own; the
  publish announces its program only after capturing a real SPS/PPS pair
  in-band.
- `rtmp-publish.txt` — the ffmpeg publish capture `fixtures/rtmp/obs-publish.bin`
  (see `fixtures/rtmp/PROVENANCE.md`), replayed through a real
  `rtmp_runtime::io::RtmpConnection` over an in-memory pipe in 509-byte reads.

## Regenerating

```bash
UPDATE_INGEST_GOLDENS=1 cargo test -p multimux --all-features --locked \
  --test it ingest_drive_golden
```

A plain run never writes: a mismatch fails with both documents. Regenerate
only for an intended behaviour change, and say why in the commit.
