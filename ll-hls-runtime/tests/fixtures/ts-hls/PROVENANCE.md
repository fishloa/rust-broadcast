# ts-hls fixture provenance

Generated from the workspace's own committed `fixtures/ts/h264_aac.ts` capture
(not a copyrighted external source) via:

```bash
ffmpeg -i fixtures/ts/h264_aac.ts -c copy -f hls -hls_time 2 -hls_segment_type mpegts \
  -hls_list_size 0 -hls_playlist_type vod ll-hls-runtime/tests/fixtures/ts-hls/index.m3u8
```

ffmpeg 8.1.2. Produces a classic MPEG-TS-segment HLS v3 Media Playlist
(`index.m3u8`) referencing two whole `.ts` segments (`index0.ts`,
`index1.ts`) — no `EXT-X-MAP` (no init segment: each segment is a
self-contained PAT/PMT/PES stream starting with the `0x47` sync byte),
used by `ll-hls-runtime/tests/ts_hls_ingest.rs` (issue #760) to prove the
sans-IO client routes classic TS-segment HLS through `transmux::TsDemux`
instead of blocking on an init fetch that will never come.
