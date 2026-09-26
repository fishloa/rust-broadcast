# `tests/fixtures/timeline/`

Fixtures for the output-timeline tests in `tests/timeline_oracle.rs` (issues
#1019-#1023). Every file here is generated from synthetic input and is released
under the workspace licence.

## `av_offset.ts`

H.264 video (64x64, 25 fps, 5 s, 2 s GOP, 2 B-frames) plus AAC-LC audio
(48 kHz mono sine) whose start is delayed by 0.3 s with `-itsoffset`, so the
first audio PTS is 25 080 ticks (90 kHz) after the first video PTS. Generated
with ffmpeg 8.1.2 (libx264 0.165.3222, native `aac` encoder):

```bash
ffmpeg -y -f lavfi -i "testsrc2=size=64x64:rate=25:duration=5" \
  -itsoffset 0.3 -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=4.7" \
  -map 0:v -map 1:a -c:v libx264 -preset ultrafast -tune zerolatency \
  -x264-params "bframes=2:keyint=50:min-keyint=50:scenecut=0" -pix_fmt yuv420p \
  -c:a aac -b:a 32k -f mpegts av_offset.ts
```

Oracle values recorded in the tests come from ffprobe 8.1.2:

```bash
ffprobe -v error -show_entries stream=index,codec_type,start_pts,start_time,duration \
  -show_entries format=duration -of compact av_offset.ts
ffprobe -v error -select_streams v -show_entries packet=pts,dts,duration,flags -of csv av_offset.ts
ffprobe -v error -select_streams a -show_entries packet=pts,duration -of csv av_offset.ts
```

- video: `start_pts=133200` (`start_time=1.480000`), first DTS 126000, 125
  packets of duration 3600, keyframes at packet 0/50/100 (PTS 133200 / 313200
  / 493200, DTS 126000 / 306000 / 486000), `duration=5.000000`
- audio: `start_pts=158280` (`start_time=1.758667`), 222 packets of duration
  1920, `duration=4.736000`
- format: `duration=5.014667`
