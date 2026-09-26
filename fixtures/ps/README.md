# `fixtures/ps/` — MPEG-1/2 Program Stream fixtures

- `h264_ac3.ps` + `h264_ac3.packets.csv` — H.264 video + AC-3 audio PS, with an
  ffprobe per-packet oracle CSV. Used by `transmux/tests/ps_demux.rs`.

- `ffmpeg-mpeg2video-mp2.ps` — MPEG-2 video + MPEG-1 Layer II audio PS (issue
  #1009, `ps_demux` C6: MPEG-2 video was misidentified as H.264). Generated
  with:

  ```sh
  ffmpeg -f lavfi -i testsrc2=duration=1:size=352x288:rate=25 \
         -f lavfi -i sine=frequency=440:duration=1 \
         -c:v mpeg2video -c:a mp2 -f vob out.mpg
  ```

  (ffmpeg 8.1.2, `testsrc2`/`sine` synthetic sources, `vob`/MPEG-PS muxer).
  ffprobe oracle: `codec_name=mpeg2video`, 352x288, 25 video packets (3
  keyframes), `codec_name=mp2`, 44100 Hz mono, 39 audio packets. Released
  under the workspace licence.
