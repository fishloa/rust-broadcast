# cmaf-fmp4 fixture provenance

Real ffmpeg fragmented-MP4 HLS output generated from the workspace's own
committed `fixtures/ts/h264_aac.ts` capture (no external source), via:

```bash
ffmpeg -i fixtures/ts/h264_aac.ts -c copy -bsf:a aac_adtstoasc -f hls -hls_time 1 \
  -hls_segment_type fmp4 -hls_playlist_type vod -hls_list_size 0 index.m3u8
```

ffmpeg 8.1.2. Kept: `init.mp4` (the `EXT-X-MAP` init segment) and
`index0.m4s`..`index2.m4s` (three 1.000 s media segments). The playlist
itself is not kept.

Independent oracle for the codec strings (used by
`tests/origin_hardening.rs`): `MP4Box -info init.mp4` reports
`RFC6381 Codec Parameters: avc1.4D400D` (video) and `mp4a.40.2` (audio).
