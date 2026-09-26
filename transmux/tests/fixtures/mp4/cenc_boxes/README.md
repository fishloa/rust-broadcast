# Fixture provenance — `tests/fixtures/mp4/cenc_boxes/`

Real, tool-generated fixtures for the W4 box-level spec-misread fixes
(issue #1131 theme T3 — no hand-made bytes, no self-round-trip-only oracle).
All three are generated from synthetic input (ffmpeg test tones), not
copied third-party media, and released under the workspace licence.

## `aac_cenc.mp4`

CENC AES-CTR (scheme `cenc`) encrypted AAC-LC audio, whole-sample protected
(8-byte per-sample IV, no subsamples) — the common case that hits `saiz`'s
uniform-size form (issue #1013) and the `enca`/`mp4a` four-CC bug (issue
#1017).

Generated with ffmpeg 8.1.2 + GPAC/MP4Box 26.07-revrelease:

```bash
ffmpeg -y -f lavfi -i "sine=frequency=1000:duration=2" -c:a aac -b:a 96k \
  -movflags cmaf+frag_keyframe+empty_moov+default_base_moof \
  -f mp4 aac_clear.mp4

cat > crypt_cenc.xml <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<GPACDRM type="CENC AES-CTR">
 <CrypTrack trackID="1" IsEncrypted="1" IV_size="8" first_IV="0x0000000000000001">
  <key KID="0xA7E61C373E219033C21091FA607BF3B8" value="0x76A6C65C5EA762046BD749A2E632CCBB"/>
 </CrypTrack>
</GPACDRM>
EOF

MP4Box -crypt crypt_cenc.xml aac_clear.mp4 -out aac_cenc.mp4
```

Verified independently with `MP4Box -diso aac_cenc.mp4` (GPAC's own ISOBMFF
dumper, never this crate's parser): the file is re-muxed by MP4Box to a
progressive (non-fragmented) layout with `saiz` inside `stbl`:

```
<SchemeTypeBox ... scheme_type="cenc" scheme_version="65536">
<TrackEncryptionBox ... isEncrypted="1" IV_size="8" KID="0xA7E61C373E219033C21091FA607BF3B8">
<AudioSampleDescriptionBox ... Type="enca" ...>
<SampleAuxiliaryInfoSizeBox Size="17" Type="saiz" ... default_sample_info_size="8" sample_count="88">
<SampleEncryptionBox Size="720" Type="senc" ... sampleCount="88">
```

`sample_count="88"` is the independent oracle used by
`tests/cenc_saiz_stsz_v1_boxes.rs`: this crate's `SampleAuxInfoSizesBox::parse_box`
must report the same 88, and the sample entry's four-CC must survive as
`enca` (not `mp4a`) through a parse → serialize round trip.

## `pcm_clear.mp4`

Uncompressed 16-bit PCM audio (`ipcm`/`sowt`), one 2-byte sample per PCM
sample at 8 kHz for 1 second — a genuinely constant-size track, i.e. the
real-world case `stsz`'s uniform `sample_size` field exists for (issue
#1018).

Generated with ffmpeg 8.1.2:

```bash
ffmpeg -y -f lavfi -i "sine=frequency=1000:duration=1" -c:a pcm_s16le -ar 8000 -ac 1 \
  -f mp4 pcm_clear.mp4
```

Verified independently with `MP4Box -diso pcm_clear.mp4`:

```
<SampleSizeBox Size="20" Type="stsz" ... SampleCount="8000" ConstantSampleSize="2">
```

`SampleCount="8000"` with `ConstantSampleSize="2"` (no per-sample table —
the 20-byte box size proves it) is the independent oracle: this crate's
`stsz` parser must report `sample_count == 8000` even though
`sample_info_sizes`/`entries` stays empty for the uniform form.

## `v1_mvhd.mp4`

A raw H.264 elementary stream muxed with an artificially large **movie
timescale** (2,000,000,000 Hz) so the 3-second duration in ticks
(6,000,000,000) exceeds `u32::MAX`, forcing GPAC to emit **version-1**
`mvhd`/`tkhd` boxes (64-bit `creation_time`/`modification_time`/`duration`)
— the real-world trigger for issues #1015 (`mvhd` v1 wrong size/offset) and
#1016 (`tkhd` v1 wrong offsets), which normally only appears with a
multi-hour recording or very large timescale.

Generated with ffmpeg 8.1.2 (raw H.264 elementary stream) + GPAC/MP4Box
26.07-revrelease (import + remux with a forced movie timescale):

```bash
ffmpeg -y -f lavfi -i "testsrc2=size=64x64:rate=2:duration=3" -c:v libx264 \
  -preset ultrafast -pix_fmt yuv420p -g 2 -f h264 raw.h264

MP4Box -add raw.h264:fps=2 -timescale 2000000000 -new v1_mvhd.mp4
```

Verified independently with `MP4Box -diso v1_mvhd.mp4`:

```
<MovieHeaderBox Size="120" Type="mvhd" Version="1" ... TimeScale="2000000000" Duration="6000000000" NextTrackID="2">
<TrackHeaderBox Size="104" Type="tkhd" Version="1" ... Duration="6000000000" Width="64.00" Height="64.00">
```

`mvhd` `Size="120"` (not 124) and `tkhd` `Size="104"` are the independent
oracle for the exact box lengths/offsets this crate's v1 parsers must use.
