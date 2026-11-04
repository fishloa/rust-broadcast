# byterange-hls fixture provenance

Real ffmpeg single-file byte-range HLS playlist generated from the workspace's
own `fixtures/ts/h264_aac_40s.ts` capture, via:

```bash
ffmpeg -i fixtures/ts/h264_aac_40s.ts -t 14 -c copy -f hls -hls_time 2 \
  -hls_flags single_file -hls_playlist_type vod -hls_list_size 0 index.m3u8
```

ffmpeg 8.1.2. Only the playlist is kept (the 311 KB `index.ts` it references is
not needed): seven `EXT-X-BYTERANGE` segments of the single resource
`index.ts`, with ffmpeg's own explicit offsets, used by
`tests/client_state.rs` as the oracle for the byte ranges the client must
request when the offsets are omitted (RFC 8216 §4.3.2.2).
