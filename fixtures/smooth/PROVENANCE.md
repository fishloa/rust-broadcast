# `fixtures/smooth/` - Smooth Streaming (MS-SSTR) client fixtures

Real output of ffmpeg's `smoothstreaming` muxer, never hand-built bytes. Source material is
`fixtures/ts/h264-aac.ts`, this workspace's own H.264 + AAC capture (own work, MIT OR Apache-2.0),
so the derived files carry the same licence.

Tool: `ffmpeg version 8.1.2 Copyright (c) 2000-2026 the FFmpeg developers` (Homebrew, macOS arm64,
`libx264` enabled), 2026-11-01.

## `av/` - stream copy, one video and one audio quality level

```sh
ffmpeg -y -i fixtures/ts/h264-aac.ts -c copy -f flv mid.flv
ffmpeg -y -i mid.flv -c copy -bsf:v setts=ts=TS-STARTDTS -bsf:a setts=ts=TS-STARTDTS \
  -f smoothstreaming -window_size 0 -extra_window_size 0 -lookahead_count 0 \
  -min_frag_duration 1000000 av
```

- The FLV hop is needed because the muxer rejects the MPEG-TS codec tags and writes
  `CodecPrivateData="(null)"` for AAC read straight from ADTS.
- `setts=ts=TS-STARTDTS` rebases each stream to start at 0: the muxer writes no `t` on the first
  `c` and names fragments by their real start time, so a stream that starts at 1.48 s would be
  addressed as `Fragments(video=14800000)` while the manifest implies `0`. The video and audio
  start offsets (1.48 s and 1.4568 s in the source) are therefore removed independently; a test
  that compares against the source subtracts each track's first decode time.
- `FragmentInfo(...)` side files the muxer also writes are not committed (nothing reads them).
- Video: H.264 Main 320x240, 3 fragments of 1 s (`Fragments(video=0|10000000|20000000)`, bitrate
  64528). Audio: AAC-LC 44.1 kHz mono, 3 fragments (`Fragments(audio=0|10214580|20199161)`,
  bitrate 98429). `TimeScale` is the default 10 000 000.

## `av-low/` - a second, lower video quality level

```sh
ffmpeg -y -i mid.flv -an -vf scale=160:120 -c:v libx264 -profile:v main -b:v 30k -g 25 \
  -keyint_min 25 -sc_threshold 0 -bf 0 -bsf:v setts=ts=TS-STARTDTS \
  -f smoothstreaming -window_size 0 -extra_window_size 0 -lookahead_count 0 \
  -min_frag_duration 1000000 av-low
```

H.264 Main 160x120, bitrate 30000, fragments at `0`, `10000000`, `20000000`. The muxer also wrote a
fourth, 40 ms fragment (`Fragments(video=30000000)`); that file is not committed so the timeline
matches `av/`'s three chunks, and the tests that need a two-quality manifest take the
`QualityLevel` lines of both directories under `av/`'s `c` timeline.

Tests derive live-style and `r`-repeat manifests from `av/Manifest` by editing attributes of that
real document at run time (`IsLive`, `DVRWindowLength`, `t`, `r`); the fragment bodies are never
altered.
