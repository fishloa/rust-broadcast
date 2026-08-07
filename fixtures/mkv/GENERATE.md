# Regenerating the Matroska (MKV) muxer fixtures (issue #915)

Real ffmpeg `-c copy` remuxes of already-committed captures — no re-encoding,
so every codec byte (SPS/PPS, AudioSpecificConfig, coded frames) is a genuine
capture, not synthesized.

```bash
# H.264 + AAC: -c copy remux of the existing TS capture.
ffmpeg -y -i fixtures/ts/h264_aac.ts -c copy fixtures/mkv/h264_aac.mkv

# HEVC + AAC: HEVC video from the existing (video-only) HEVC TS capture,
# paired with the AAC audio from the H.264/AAC TS above — two genuine
# captures muxed into one file (there is no committed HEVC+AAC capture).
ffmpeg -y -i fixtures/ts/hevc/main.ts -i fixtures/ts/h264_aac.ts \
  -map 0:v -map 1:a -c copy -shortest fixtures/mkv/hevc_aac.mkv

# VP9 + Opus: -c copy remux of the existing WebM capture (WebM is a
# Matroska profile, so this is a straight Segment/Cluster/Track re-wrap).
ffmpeg -y -i fixtures/webm/vp9_opus.webm -c copy fixtures/mkv/vp9_opus.mkv
```
