# Regenerating the fMP4 gap-tier fixtures

Real ffmpeg-authored MP4s carrying the config boxes the gap-tier features build.
Requires ffmpeg with libx264/libx265/libsvtav1/libopus/flac/libvpx-vp9 + the mp4
muxer's `-encryption_scheme`.

```bash
# avcC / hvcC (from the SPS-fixture TS)
ffmpeg -y -i fixtures/ts/h264/high.ts -c:v copy -f mp4 fixtures/mp4/h264_high.mp4
ffmpeg -y -i fixtures/ts/hevc/main.ts -c:v copy -f mp4 fixtures/mp4/hevc_main.mp4

# #429 CENC (cenc-aes-ctr)
ffmpeg -y -i fixtures/ts/h264/main.ts -c:v copy \
  -encryption_scheme cenc-aes-ctr \
  -encryption_key 76a6c65c5ea762046bd749a2e632ccbb \
  -encryption_kid a7e61c373e219033c21091fa607bf3b8 -f mp4 fixtures/mp4/cenc.mp4

# #430 captions (TTML → stpp; wvtt needs GPAC MP4Box, absent → spec vector)
printf 'WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nHello CMAF\n' > /tmp/cap.vtt
ffmpeg -y -i /tmp/cap.vtt -c:s ttml -f mp4 fixtures/mp4/stpp.mp4

# #434 colr(nclx) / pasp
ffmpeg -y -f lavfi -i testsrc2=size=320x240:rate=25 -frames:v 8 -c:v libx264 -pix_fmt yuv420p \
  -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc -color_range tv \
  -movflags +write_colr -f mp4 fixtures/mp4/colr_hdr.mp4

# #435 prft (fragmented + wallclock) / sgpd+sbgp (roll)
ffmpeg -y -i fixtures/ts/h264_aac.ts -map 0:v:0 -c:v copy \
  -movflags +frag_keyframe+empty_moov+separate_moof -frag_duration 1000000 \
  -write_prft wallclock -f mp4 fixtures/mp4/prft.mp4
ffmpeg -y -i fixtures/ts/h264_aac.ts -map 0:a:0 -c:a copy -f mp4 fixtures/mp4/aac_sgpd.mp4

# #436 AV1
ffmpeg -y -f lavfi -i testsrc2=size=320x240:rate=25 -frames:v 12 -c:v libsvtav1 -f mp4 fixtures/mp4/av1.mp4

# #437 Opus / FLAC / VP9
ffmpeg -y -f lavfi -i "sine=frequency=440:duration=1" -c:a libopus -f mp4 fixtures/mp4/opus.mp4
ffmpeg -y -f lavfi -i "sine=frequency=440:duration=1" -c:a flac   -f mp4 fixtures/mp4/flac.mp4
ffmpeg -y -f lavfi -i testsrc2=size=320x240:rate=25 -frames:v 12 -c:v libvpx-vp9 -f mp4 fixtures/mp4/vp9.mp4
```

Extract a config-box body: find the fourcc, read the big-endian `u32` size 4 bytes
before it, slice `[i-4 .. i-4+size]`, body = `[8..]`. See `FMP4-GAPS-ORACLE.md`.

# #1010 (C9) sidx — real MP4Box -dash onDemand-profile output
ffmpeg -y -f lavfi -i testsrc2=duration=2:size=160x120:rate=25 -preset ultrafast -crf 40 \
  -g 25 -pix_fmt yuv420p -c:v libx264 in.mp4
MP4Box -dash 500 -rap -frag-rap -profile onDemand in.mp4 -out out_dash.mpd
# -> in_dashinit.mp4 carries one full sidx box (2 references). Extract with:
#   MP4Box -diso in_dashinit.mp4 -out in_dashinit.xml   # oracle: SegmentIndexBox attrs
#   python3: find b'sidx', read the big-endian u32 size 4 bytes before it,
#   slice [i-4 .. i-4+size] -> fixtures/mp4/sidx_real.bin (the full sidx box,
#   header included). GPAC's own MP4Box (not this crate) is the independent
#   oracle: reference_ID=1, timescale=12800, 2 references
#   (size=11401/dur=12800/SAP_type=1, size=10306/dur=12800/SAP_type=1).
