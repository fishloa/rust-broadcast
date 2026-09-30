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

- `ffmpeg-mpeg2video-2xac3.ps` — MPEG-2 video + **two** AC-3 audio substreams in
  a single `private_stream_1` (0xBD) `stream_id`, distinguished by their
  `substream_id` bytes (0x80 and 0x81). Used by
  `transmux/tests/ps_demux_substreams.rs` (audit r04-W19/W20). Generated with:

  ```sh
  ffmpeg -f lavfi -i testsrc2=duration=2:size=352x288:rate=25 \
         -f lavfi -i sine=frequency=440:duration=2 \
         -f lavfi -i sine=frequency=880:duration=2 \
         -map 0:v -map 1:a -map 2:a -c:v mpeg2video -c:a ac3 -b:a 192k \
         -f vob out.mpg
  ```

  (ffmpeg 8.1.2, `vob`/MPEG-PS muxer). ffprobe oracle: `mpeg2video` 352x288,
  50 video packets, 3 keyframes; two `ac3` streams, 44100 Hz mono, 58 packets
  each — both substreams share `stream_id` 0xBD and are only separable by
  `substream_id`. Released under the workspace licence.

- `ffmpeg-h264-noaud.ps` — H.264 video with **no access-unit delimiter** + AC-3
  audio. Used by `transmux/tests/ps_demux_substreams.rs` (audit r04-W21): an
  AUD-only splitter cannot find a single boundary in it, so the whole file
  collapses into one "access unit". Generated from the H.264/AAC capture above:
  the AUDs are stripped as they are (so no NAL is rewritten, unlike a
  `-c:v copy`/`-bsf` remux of the *decoded* capture, which would also change
  the SPS/PPS):

  ```sh
  ffmpeg -i fixtures/ts/h264_aac.ts -map 0:v -c copy \
         -bsf:v h264_metadata=aud=remove -f h264 noaud.h264
  ffmpeg -i noaud.h264 -i fixtures/ps/h264_ac3.ps -map 0:v -map 1:a -c copy \
         -f vob noaud.ps
  ```

  ffprobe oracle: 75 H.264 pictures, 3 keyframes, and the video ES in one PES
  packet (0xE0 count = 1), so there is no PES boundary to fall back on either.
  Released under the workspace licence.

### Extracted elementary streams (independent oracles)

- `ffmpeg-mpeg2video-2xac3.a0.ac3` / `.a1.ac3` — the two AC-3 elementary streams
  ffmpeg itself extracts from `ffmpeg-mpeg2video-2xac3.ps`:

  ```sh
  ffmpeg -v error -i fixtures/ps/ffmpeg-mpeg2video-2xac3.ps -map 0:a:N          -c copy -f ac3 fixtures/ps/ffmpeg-mpeg2video-2xac3.aN.ac3
  ```

  Used by `transmux/tests/ps_demux_substreams.rs` as a **non-circular** oracle:
  the crate's demuxed AC-3 tracks must be byte-identical to these, so the test
  is not comparing the demuxer against a rule it derived itself. Both are
  48 482 bytes (58 syncframes; the last is trimmed), and they differ from each
  other from byte 4, confirming the two substreams are genuinely distinct
  programmes.

- `ffmpeg-ac3-dts.ps` — MPEG-2 video + AC-3 + **DTS** in one
  `private_stream_1` stream, used by
  `transmux/tests/ps_demux_substreams.rs` (a non-AC-3 audio substream must be
  skipped). Generated with ffmpeg's own DTS encoder, so the 0x88 substream
  carries a real `dca` bitstream rather than relabelled AC-3:

  ```sh
  ffmpeg -f lavfi -i testsrc2=duration=2:size=352x288:rate=25          -f lavfi -i sine=frequency=440:duration=2          -f lavfi -i sine=frequency=880:duration=2          -map 0:v -map 1:a -map 2:a -c:v mpeg2video          -c:a:0 ac3 -b:a:0 192k -c:a:1 dca -strict -2 -f vob out.ps
  ```

  (ffmpeg 8.1.2; `dca` requires `-strict -2`.) ffprobe reads it as one
  `mpeg2video`, one `ac3` and one `dts` stream; the substream ids are 0x80
  (AC-3) and 0x88 (DTS), the DVD/ATSC assignments.

- `ffmpeg-ac3-dts.a0.ac3` — the AC-3 program of the file above, extracted by
  ffmpeg (`-map 0:a:0 -c copy -f ac3`), the oracle for the AC-3 track's bytes.

- `ffmpeg-ac3-dts-with-fake-ac3-sync.ps` — **derived** from `ffmpeg-ac3-dts.ps`
  by stamping an AC-3 syncword (`0B 77`) into one DTS packet's payload at the
  position its `first_access_unit_pointer` names. Nothing else is changed. It
  exists so a test can prove the substream_id range check, not just the syncword
  probe, is what keeps a DTS substream out of the AC-3 track. Released under the
  workspace licence.
