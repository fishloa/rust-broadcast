# `tests/fixtures/ts/`

## `h264-two-resolutions.ts`

The mid-stream codec-config-change fixture for the `ts_demux` tests
(issue #1080, audit r04-W51): H.264 320x240 for 2 s immediately followed by
H.264 640x480, concatenated at the transport-stream level so a single video
PID carries two different SPS/PPS sets. Produced with ffmpeg 8.1.2
(libx264 0.165.3222):

```bash
ffmpeg -y -f lavfi -i "testsrc2=size=320x240:rate=25:duration=2" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -x264-params keyint=25 \
  -f mpegts lo.ts
ffmpeg -y -f lavfi -i "testsrc2=size=640x480:rate=25:duration=2" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -x264-params keyint=25 \
  -f mpegts hi.ts
printf "file 'lo.ts'\nfile 'hi.ts'\n" > list.txt
ffmpeg -y -f concat -safe 0 -i list.txt -c copy h264-two-resolutions.ts
```

Oracle values from ffprobe 8.1.2 and a raw byte scan:

- first SPS (in `00 00 00 01 67`) codes 320x240, second codes 640x480;
  `ffprobe -select_streams v -show_entries packet=flags` reports 4 keyframes
  (2 clips x `keyint=25`), and each keyframe carries one SPS/PPS pair
- `ffprobe -select_streams v -show_entries stream=width,height` reports the
  first (320x240), since the container-level description is the first one
