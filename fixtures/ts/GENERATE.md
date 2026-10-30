# Regenerating the codec-config fixtures

All fixtures are real ffmpeg-encoded bitstreams (real SPS/VPS/PTL, real syncframes),
kept tiny (≤16 frames, 320×240 unless the branch needs otherwise). Requires `ffmpeg`
with `libx264` + `libx265`, and the `ac3`/`eac3` encoders.

## H.264 profile matrix (`fixtures/ts/h264/`)

```bash
GEN() {  # profile pixfmt extra outfile
  ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=320x240:rate=25" \
    -frames:v 15 -c:v libx264 -profile:v "$1" -pix_fmt "$2" \
    -x264-params "keyint=15:scenecut=0$3" -an -f mpegts "$4"
}
GEN baseline yuv420p     "" fixtures/ts/h264/baseline.ts
GEN main     yuv420p     "" fixtures/ts/h264/main.ts
GEN high     yuv420p     "" fixtures/ts/h264/high.ts
GEN high10   yuv420p10le "" fixtures/ts/h264/high10.ts
GEN high422  yuv422p     "" fixtures/ts/h264/high422.ts
GEN high444  yuv444p     "" fixtures/ts/h264/high444.ts

# interlaced (frame_mbs_only_flag=0, MBAFF) — needs a broadcast size
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=720x576:rate=25" \
  -frames:v 16 -c:v libx264 -profile:v high -pix_fmt yuv420p \
  -flags +ilme+ildct -x264-params "interlaced=1:tff=1:keyint=16:scenecut=0" \
  -an -f mpegts fixtures/ts/h264/interlaced.ts

# frame_cropping_flag=1 (1088 coded → 1080 displayed)
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=1920x1080:rate=25" \
  -frames:v 12 -c:v libx264 -profile:v high -pix_fmt yuv420p \
  -x264-params "keyint=12:scenecut=0" -an -f mpegts fixtures/ts/h264/high_1080_cropped.ts
```

## HEVC (`fixtures/ts/hevc/`)

```bash
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=320x240:rate=25" \
  -frames:v 15 -c:v libx265 -profile:v main -pix_fmt yuv420p \
  -x265-params "keyint=15:scenecut=0" -an -f mpegts fixtures/ts/hevc/main.ts
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=320x240:rate=25" \
  -frames:v 15 -c:v libx265 -profile:v main10 -pix_fmt yuv420p10le \
  -x265-params "keyint=15:scenecut=0" -an -f mpegts fixtures/ts/hevc/main10.ts
```

## Dolby (`fixtures/ts/dolby/`)

```bash
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
  -c:a ac3  -b:a 192k -f mpegts fixtures/ts/dolby/ac3.ts
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
  -c:a eac3 -b:a 192k -f mpegts fixtures/ts/dolby/eac3.ts
```

## HE-AAC in TS (`fixtures/ts/heaac/heaac_v{1,2}_adts.ts`, issue #1428)

Real HE-AAC (ADTS in TS) from the macOS AudioToolbox encoder, audio of this
workspace's own `fixtures/ts/h264_aac.ts`, video stream-copied. ffmpeg 8.1.2
(`aac_at`) rejects the profile names `aac_he`/`aac_he_v2`; the numeric
AudioToolbox profile values `4` (HE-AAC) and `28` (HE-AAC v2) work (same values
as the `heaac_v{1,2}.mp4` oracles). The source audio is mono; both profiles are
asked for stereo with `-ac 2` (v1 given mono produces parametric stereo).

```bash
ffmpeg -y -i fixtures/ts/h264_aac.ts -map 0:v:0 -map 0:a:0 -c:v copy \
  -c:a aac_at -profile:a 4 -ac 2 -b:a 48k -f mpegts fixtures/ts/heaac/heaac_v1_adts.ts
ffmpeg -y -i fixtures/ts/h264_aac.ts -map 0:v:0 -map 0:a:0 -c:v copy \
  -c:a aac_at -profile:a 28 -ac 2 -b:a 32k -f mpegts fixtures/ts/heaac/heaac_v2_adts.ts
```

Verified with `ffprobe -v error -analyzeduration 10M -probesize 10M -select_streams a:0
-show_entries stream=codec_name,profile,sample_rate,channels,channel_layout -of default=nw=1`:

| File | codec_name | profile | sample_rate | channels | ADTS header |
|---|---|---|---|---|---|
| `heaac_v1_adts.ts` | aac | HE-AAC | 44100 | 2 (stereo) | `fff1 5c80`: profile LC, sfi 7 (22050 core), channel_configuration 2 |
| `heaac_v2_adts.ts` | aac | HE-AACv2 | 44100 | 2 (stereo) | `fff1 5c40`: profile LC, sfi 7 (22050 core), channel_configuration 1 |

The ADTS header of an HE-AAC stream states only the core rate (22050 Hz) and
core channels; the SBR/PS signalling is inside the raw_data_block, which
transmux never decodes. These files carry no DVB `AAC_descriptor`, so they
exercise the per-route override path.

## Re-derive the oracle

```bash
# H.264/H.265 SPS/VPS fields (authoritative):
ffmpeg -hide_banner -loglevel trace -i <fixture> -c copy -bsf:v trace_headers -f null - 2>&1 \
  | grep -E 'profile_idc|constraint_set|level_idc|chroma_format_idc|bit_depth|pic_(width|height)|frame_mbs_only|frame_crop'
# RFC 6381 avc1 bytes (profile.constraint.level):
ffmpeg -loglevel error -i <fixture> -c:v copy -bsf:v filter_units=pass_types=7 -f h264 - | xxd | head
```
