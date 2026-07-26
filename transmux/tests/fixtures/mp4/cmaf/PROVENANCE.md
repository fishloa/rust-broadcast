# Fixture provenance — `tests/fixtures/mp4/cmaf/`

## `av_subtitle_frag.mp4`

Media plane step 2d (`CodecConfig::Subtitle` coverage): a real, fragmented
CMAF file carrying an AVC video track plus a TTML/IMSC (`stpp`, ISO/IEC
14496-30 §7.2) subtitle track, generated locally with ffmpeg 8.1.2 — not
hand-built/fabricated bytes. Used by `tests/subtitle_demux.rs`'s ordering-guard
test (`Fmp4Demux` must demux the `stpp` track as `CodecConfig::Subtitle`, not
drop it).

Regenerate with:

```bash
cat > subs.srt <<'EOF'
1
00:00:00,000 --> 00:00:01,000
Hello

2
00:00:01,000 --> 00:00:02,000
World
EOF

ffmpeg -y -f lavfi -i "testsrc=size=64x64:rate=2:duration=2" -i subs.srt \
  -map 0:v -map 1:s -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 2 \
  -c:s ttml -movflags cmaf+frag_keyframe+empty_moov+default_base_moof \
  -f mp4 av_subtitle_frag.mp4
```

Verified structure (top-level boxes, via a manual byte walk): `ftyp` / `moov`
(2 `trak`: `avc1` + `stpp`) / `moof`+`mdat` (fragment 1) / `moof`+`mdat`
(fragment 2) / `mfra`. `ffprobe` confirms stream 0 = `h264`/`avc1`, stream 1 =
`ttml`/`stpp`.

ffmpeg's `mov` muxer does not support muxing a `wvtt` (WebVTT) sample entry
into an ISOBMFF file (`Could not find tag for codec webvtt in stream`, tried
2026-07-26) and no other locally-available tool (no `MP4Box`/GPAC) could
produce one either, so WebVTT demux coverage is instead exercised as an
in-crate unit test (`src/media.rs`, `wvtt_sample_entry_demuxes_to_subtitle_webvtt`)
that synthesises a `wvtt` sample entry directly via the crate's own
`WvttSampleEntry::new` builder (already covered by
`subtitle_entries::tests::wvtt_sample_entry_round_trip`) and feeds it to the
same `codec_config_from_entry` reconstruction path this fixture exercises for
`stpp`.
