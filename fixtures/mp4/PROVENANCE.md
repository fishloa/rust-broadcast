# PROVENANCE


## fMP4 gap oracle

# fMP4 gap-tier fixture oracle (#429–#437)

Real ffmpeg-authored MP4s + the **byte-exact config-box bodies** ffmpeg writes.
Each gate builds its box from the source and must reproduce the body byte-for-byte
(same technique that verified esds/dac3/dec3 in 0.4.1). Bodies below are the bytes
**after** the 8-byte box header; truncated to 64 hex where long.

Regenerate: see the "Regeneration" section below.

## avcC / hvcC value-verification (the #425 follow-up — clean reference)

The vendored ISO/IEC 14496-15:2017 is an image-only scan, so its syntax is read from the rendered pages (`private/specs-verified/mpeg/isobmff/iso-iec-14496-15-2017-avc-hevc-config.md`). The clean
reference is ffmpeg's own muxer (movenc.c) — the byte oracle here, read
against that 2017 transcription.

| box | source mp4 | body (hex) |
|---|---|---|
| `avcC` | `h264-high.mp4` | `0164000dffe100196764000dacd94141fb011000000300100000030320f14299…` |
| `hvcC` | `hevc-main.mp4` | `0101600000009000000000003cf000fcfdf8f800000f0320…` |
| `pasp` | (all video) | present (pixel aspect ratio) |

`avcC[0..4]` = configurationVersion(1), AVCProfileIndication(0x64=100),
profile_compatibility(0), AVCLevelIndication(0x0d=13) — matches the SPS.

## #429 CENC (`cenc.mp4`, scheme cenc-aes-ctr)

| box | len | body (hex) | notes |
|---|---|---|---|
| `tenc` | 32 | `0000000000000108 a7e61c373e219033c21091fa607bf3b8` | default_isProtected=1, IV_size=8, default_KID |
| `schm` | 20 | `00000000 63656e63 00010000` | scheme_type='cenc', version 1.0 |
| `frma` | 12 | `61766331` | original format 'avc1' |
| `senc` | 364 | `00000002 0000000f 298f062baa9886ea …` | sample_count, per-sample IV + subsample ranges |
| `saiz` | 32 | `00000000 00000000 0f 281616…` | default_sample_info_size + per-sample sizes |
| `saio` | 20 | `00000000 00000001 00005da6` | offset to senc aux data |

(No `pssh` — ffmpeg emits it only for a specific DRM system; pssh is
system-specific init data, hand-built from a spec vector.)

## #430 captions

| box | source | notes |
|---|---|---|
| `stpp` | `stpp.mp4` (TTML) | ✅ real fixture (ffmpeg `ttml` encoder) |
| `wvtt`/`vttC` | — | ⚠️ ffmpeg mp4 muxer can't write WebVTT-in-ISOBMFF; no GPAC installed → **spec vector** from ISO/IEC 14496-30 |

## #434 colr / pasp / clap (`colr-hdr.mp4`)

| box | len | body (hex) | notes |
|---|---|---|---|
| `colr` | 19 | `6e636c78 0002 0002 0009 00` | 'nclx' primaries/transfer/matrix + full_range flag |
| `pasp` | — | present | |

## #435 prft / sbgp+sgpd / subs

| box | source | len | body (hex) | notes |
|---|---|---|---|---|
| `prft` | `prft.mp4` | 32 | `01 000018 00000001 edefe3e3a7ae147a 0000000000001c20` | v1, ref_track_ID=1, ntp_timestamp, media_time |
| `sgpd` | `aac-sgpd.mp4` / `opus.mp4` | 26 | `01 000000 726f6c6c 00000002 00000001 ffff` | grouping='roll', roll_distance=-1 |
| `sbgp` | (same) | — | present | sample→group map |

## #436 AV1 (`av1.mp4`)

| box | len | body (hex) |
|---|---|---|
| `av01` | — | sample entry present |
| `av1C` | 25 | `81000c000a0b000000043cffbc02f80040` |

## #437 Opus / FLAC / VP9

| box | source | len | body (hex) | notes |
|---|---|---|---|---|
| `dOps` | `opus.mp4` | 19 | `00010138 0000bb80 000000…` | v1, channels=1, pre_skip=0x0138, input_sr=0xbb80=48000 |
| `dfLa` | `flac.mp4` | 50 | `00000000 80000022 1200120000…` | FLAC STREAMINFO metadata block |
| `vpcC` | `vp9.mp4` | 20 | `01000000 0014820202020000` | v1, profile/level/bit-depth/chroma |

## #431 AC-4 — UNBLOCKED via ../rust-ac4

Real fixtures from the sibling `rust-ac4` codec repo:
- `./fixtures/mp4/ac4/ac4_2ch.mp4` — real Dolby AC-4 init segment (moov + `ac-4` sample
  entry + `dac4`); the **dac4 oracle**.
- `fixtures/ts/ac4/ac4-channel.ac4` — real AC-4 elementary stream (syncframes/TOC).

| box | len | body (hex) |
|---|---|---|
| `dac4` (AC4SpecificBox = ac4_dsi_v1) | 29 | `20a401400000001fffffffe0010ff88000004200000250100000030080` |

Spec md: `ac4-dac4.md` (ETSI TS 103 190-2 §E.5). The `dac4` body is the `ac4_dsi`
derived from the AC-4 TOC; `rust-ac4`'s `ac4-si` crate parses that TOC (reference).
transmux (samples-in) can take the `dac4` as caller-supplied config (like avcC/esds)
and wrap it in the `ac-4` sample entry; gate = build the entry, re-parse, dac4 body
== oracle byte-for-byte.

## Still blocked (no fixture path)

- **HE-AAC #432** — no `libfdk_aac` in this ffmpeg (default `aac` won't emit SBR/PS).
- **MPEG-H #433** — no encoder and ISO/IEC 23008-3 not vendored.



## Regeneration

# Regenerating the fMP4 gap-tier fixtures

Real ffmpeg-authored MP4s carrying the config boxes the gap-tier features build.
Requires ffmpeg with libx264/libx265/libsvtav1/libopus/flac/libvpx-vp9 + the mp4
muxer's `-encryption_scheme`.

```bash
# avcC / hvcC (from the SPS-fixture TS)
ffmpeg -y -i fixtures/ts/h264/high.ts -c:v copy -f mp4 fixtures/mp4/h264-high.mp4
ffmpeg -y -i fixtures/ts/hevc/main.ts -c:v copy -f mp4 fixtures/mp4/hevc-main.mp4

# #429 CENC (cenc-aes-ctr)
ffmpeg -y -i fixtures/ts/h264/main.ts -c:v copy \
  -encryption_scheme cenc-aes-ctr \
  -encryption_key 76a6c65c5ea762046bd749a2e632ccbb \
  -encryption_kid a7e61c373e219033c21091fa607bf3b8 -f mp4 fixtures/mp4/cenc.mp4

# #1080 audit r05-W6 CENC with a protected *audio* track as well (`encv` +
# `enca`), the shape a real CENC asset has and which `CencDecryptor::demux`
# used to reject outright. ffmpeg's own `-encryption_scheme` encrypts every
# track (key/KID as for cenc.mp4 above), so a single pass produces both, with
# the standard in-`stbl` `senc` layout:
ffmpeg -y -f lavfi -i "testsrc2=size=64x64:rate=10:duration=1" \
  -f lavfi -i "sine=frequency=440:duration=1" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 5 -c:a aac -b:a 64k \
  -movflags cmaf+frag_keyframe+empty_moov+default_base_moof av_clear.mp4
ffmpeg -y -i av_clear.mp4 -c copy -encryption_scheme cenc-aes-ctr \
  -encryption_key 76a6c65c5ea762046bd749a2e632ccbb \
  -encryption_kid a7e61c373e219033c21091fa607bf3b8 \
  -f mp4 fixtures/mp4/cenc-av-enca.mp4

# #1080 audit r05-W6 follow-up: a mixed file — one protected track (`encv`) and
# one UNPROTECTED track (`mp4a`) — to test that the unprotected neighbour is
# reported in `Media::skipped`, not dropped silently. `cenc-av-enca.mp4` has
# both tracks protected, so its audio sample entry is rewritten to clear: the
# `enca` four-CC becomes `mp4a` and the audio `sinf` box's type becomes `free`
# (size unchanged, so every other offset is untouched).
python3 - <<'EOF'
d = bytearray(open('cenc-av-enca.mp4', 'rb').read())
i = bytes(d).find(b'enca')
j = bytes(d).find(b'sinf', i)
d[i:i+4] = b'mp4a'
d[j:j+4] = b'free'
open('fixtures/mp4/cenc-mixed-tracks.mp4', 'wb').write(bytes(d))
EOF

# #430 captions (TTML → stpp; wvtt needs GPAC MP4Box, absent → spec vector)
printf 'WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nHello CMAF\n' > /tmp/cap.vtt
ffmpeg -y -i /tmp/cap.vtt -c:s ttml -f mp4 fixtures/mp4/stpp.mp4

# #434 colr(nclx) / pasp
ffmpeg -y -f lavfi -i testsrc2=size=320x240:rate=25 -frames:v 8 -c:v libx264 -pix_fmt yuv420p \
  -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc -color_range tv \
  -movflags +write_colr -f mp4 fixtures/mp4/colr-hdr.mp4

# #435 prft (fragmented + wallclock) / sgpd+sbgp (roll)
ffmpeg -y -i fixtures/ts/h264-aac.ts -map 0:v:0 -c:v copy \
  -movflags +frag_keyframe+empty_moov+separate_moof -frag_duration 1000000 \
  -write_prft wallclock -f mp4 fixtures/mp4/prft.mp4
ffmpeg -y -i fixtures/ts/h264-aac.ts -map 0:a:0 -c:a copy -f mp4 fixtures/mp4/aac-sgpd.mp4

# #436 AV1
ffmpeg -y -f lavfi -i testsrc2=size=320x240:rate=25 -frames:v 12 -c:v libsvtav1 -f mp4 fixtures/mp4/av1.mp4

# #437 Opus / FLAC / VP9
ffmpeg -y -f lavfi -i "sine=frequency=440:duration=1" -c:a libopus -f mp4 fixtures/mp4/opus.mp4
ffmpeg -y -f lavfi -i "sine=frequency=440:duration=1" -c:a flac   -f mp4 fixtures/mp4/flac.mp4
# vp9.mp4: `-t 0.4` (10 frames at 25 fps). Its `vpcC` box body is the vpcC
# oracle used by transmux/tests/it/codecs_new.rs::vpcc_round_trip_and_fields and
# transmux/src/vp9.rs::version_1_ffmpeg_oracle_parses_and_round_trips:
#   010000000014820202020000
# (FullBox version 1; profile 0, level 20, bitDepth 8, chromaSubsampling 1,
#  CICP 2/2/2, codecInitializationDataSize 0).
ffmpeg -y -f lavfi -i testsrc2=size=320x240:rate=25 -t 0.4 -c:v libvpx-vp9 -f mp4 fixtures/mp4/vp9.mp4
```

Extract a config-box body: find the fourcc, read the big-endian `u32` size 4 bytes
before it, slice `[i-4 .. i-4+size]`, body = `[8..]`. See the "fMP4 gap oracle" section above.

# #1010 (C9) sidx — real MP4Box -dash onDemand-profile output
ffmpeg -y -f lavfi -i testsrc2=duration=2:size=160x120:rate=25 -preset ultrafast -crf 40 \
  -g 25 -pix_fmt yuv420p -c:v libx264 in.mp4
MP4Box -dash 500 -rap -frag-rap -profile onDemand in.mp4 -out out_dash.mpd
# -> in_dashinit.mp4 carries one full sidx box (2 references). Extract with:
#   MP4Box -diso in_dashinit.mp4 -out in_dashinit.xml   # oracle: SegmentIndexBox attrs
#   python3: find b'sidx', read the big-endian u32 size 4 bytes before it,
#   slice [i-4 .. i-4+size] -> fixtures/mp4/sidx-real.bin (the full sidx box,
#   header included). GPAC's own MP4Box (not this crate) is the independent
#   oracle: reference_ID=1, timescale=12800, 2 references
#   (size=11401/dur=12800/SAP_type=1, size=10306/dur=12800/SAP_type=1).

## AUD-less AVC/HEVC (issue #1080, audit r05-W17)

H.222.0 §2.14.1/§2.17.1 require every AVC/HEVC access unit carried in a TS to
begin with an access unit delimiter. `libx264`/`libx265` do not emit one by
default (`aud=0`), so an MP4 muxed from them carries none — the exact input a
`Fmp4Demux → TsMux` remux receives. Kept tiny (15 frames, 320×240, one GOP).

```bash
ffmpeg -y -v error -f lavfi -i "testsrc2=size=320x240:rate=25" -frames:v 15 \
  -c:v libx264 -profile:v high -pix_fmt yuv420p \
  -x264-params "keyint=15:scenecut=0:aud=0" -an -f mp4 fixtures/mp4/noaud/h264-noaud.mp4

ffmpeg -y -v error -f lavfi -i "testsrc2=size=320x240:rate=25" -frames:v 15 \
  -c:v libx265 -profile:v main -pix_fmt yuv420p \
  -x265-params "keyint=15:scenecut=0:aud=0" -an -f mp4 fixtures/mp4/noaud/hevc-noaud.mp4
```

## HEVC VUI timing / frame-rate fixtures (`hevc_vui_*.mp4`)

Real `libx265` streams whose SPS VUI carries specific timing syntax, for the HLS
`FRAME-RATE` derivation (`private/specs-verified/mpeg/transmux/codec/vui-timing-frame-rate.md`). 25 frames,
320x240, source `lavfi testsrc2` (generated, no third-party content). The `field`
files only set the SPS flags (`field_seq_flag`, `frame_field_info_present_flag`);
their pictures are NOT really interlaced. SPS flags verified with
`ffmpeg -v trace ... -bsf:v trace_headers`:

| File | field_seq | frame_field_info | vui_num_units_in_tick / vui_time_scale | HRD | ffprobe avg_frame_rate |
|---|---|---|---|---|---|
| `hevc-vui-hrd.mp4` | 0 | 0 | 1 / 25 | present, `fixed_pic_rate_general_flag[0]=1`, `elemental_duration_in_tc_minus1[0]=0` | 25/1 |
| `hevc-vui-ntsc.mp4` | 0 | 0 | 1001 / 30000 | absent | 30000/1001 |
| `hevc-vui-field.mp4` | 1 | 1 | 1 / 25 | absent | 25/1 |
| `hevc-vui-field-hrd.mp4` | 1 | 1 | 1 / 25 | present (as above) | 25/1 |

```bash
g() { ffmpeg -y -v error -f lavfi -i "testsrc2=size=320x240:rate=$2" -frames:v 25 \
  -c:v libx265 -profile:v main -pix_fmt yuv420p \
  -x265-params "keyint=25:scenecut=0:$3log-level=error" -an -f mp4 fixtures/mp4/$1; }
g hevc-vui-hrd.mp4        25        "hrd=1:vbv-maxrate=2000:vbv-bufsize=2000:"
g hevc-vui-ntsc.mp4       30000/1001 ""
g hevc-vui-field.mp4      25        "field=1:interlace=tff:"
g hevc-vui-field-hrd.mp4  25        "field=1:interlace=tff:hrd=1:vbv-maxrate=2000:vbv-bufsize=2000:"
```

### `hevc-vps-timing.mp4` — real VPS timing (`vps_timing_info_present_flag = 1`)

libx265 never writes VPS timing, so ffmpeg's `hevc_metadata` bitstream filter ("Set VPS and VUI tick
rate") adds it to a real libx265 stream. Verified with `trace_headers` on the output file: VPS
`vps_timing_info_present_flag=1`, `vps_num_units_in_tick=1001`, `vps_time_scale=30000`, `vps_num_hrd_parameters=0`;
SPS VUI `vui_timing_info_present_flag=1` with the same pair (the spec requires them equal). The filter
sets both, so **no real stream with timing in the VPS only exists here** (a VPS-only case is covered by
pure-function tests, `private/specs-verified/mpeg/transmux/codec/hevc-vps-timing.md` §4). The mp4 timebase (`-r`) is irrelevant
to the SPS/VPS content.

```bash
ffmpeg -y -f lavfi -i "testsrc2=size=320x240:rate=25" -frames:v 25 -c:v libx265 -profile:v main -pix_fmt yuv420p \
  -x265-params "keyint=25:scenecut=0:log-level=error" -an -f hevc /tmp/src.hevc
ffmpeg -y -i /tmp/src.hevc -c copy -bsf:v "hevc_metadata=tick_rate=30000/1001" -f hevc /tmp/vps.hevc
ffmpeg -y -r 30000/1001 -i /tmp/vps.hevc -c copy -f mp4 fixtures/mp4/hevc-vps-timing.mp4
```



## Independent oracles

# Independent oracle fixtures (audit r05 W1 / W2 / W7)

"Independent" = a tool that shares no code with this crate (ffmpeg 8.1.2, Bento4 1.6.0.0, pycryptodome 3.23.0).
Every dir has `gen.sh` (deterministic: run twice, byte-identical), tool versions in its PROVENANCE.md, and a recorded `proof.log`.

| # | Fixture (dir) | Claim it proves | Generator | Independent tool(s) | Result |
|---|---|---|---|---|---|
| A1 | `crypto/sample-aes-h264/enc-chained.*` vs `enc-reset.*` | H.264 Sample-AES CBC **chains across the encrypted blocks of a NAL**; IV reset only per NAL | `gen.sh`, `sample_aes_ref.py` | ffmpeg decrypt | chained **10/10** frames = clear; per-block reset **0/10** |
| A2 | `crypto/sample-aes-h264/bento4-sample-aes/clear/` | an independent ENCRYPTOR writes the chained form | `gen-bento4.sh` | Bento4 `mp4hls` SAMPLE-AES (encrypt), ffmpeg (decrypt) | ES byte-identical to python reference (40/40 slice NALs); ffmpeg 10/10 |
| A3 | `crypto/sample-aes-h264/tailprobe-*` | tail rule: block encrypted only if **>16 bytes remain**; exactly-16 tail stays clear | `prep_clear.py` | ffmpeg, Bento4 | `>16`: 10/10 (Bento4 40/40 identical); `>=16`: 0/10 |
| A4 | `crypto/sample-aes-h264/escprobe-*` | encrypted output gets 0x03 emulation prevention where ciphertext has `00 00 0[0-3]`; NAL is unescaped before encrypt | `prep_clear.py` | Bento4 (wire-byte identical), ffmpeg | identical incl. the inserted 03; ffmpeg 10/10 |
| A5 | `crypto/sample-aes-h264/epclear-*` | EP bytes inside the **clear** regions of a slice | - | Bento4, ffmpeg | **NO oracle**: Bento4 double-escapes, ffmpeg emits unescaped NALs; python reference (Apple text) only |
| B1 | `crypto/sample-aes-eac3/enc-reset.*` vs `enc-carried.*` | E-AC-3 (independent syncframes): 16-byte clear leader, whole 16-byte blocks encrypted, partial tail clear, **IV reset every syncframe** | `gen.sh`, `sample_aes_eac3_ref.py` | ffmpeg decrypt (TS) | reset **9/9** audio frames = clear; carried **1/9** |
| B2 | `crypto/sample-aes-eac3/bento4/{ts,packed}/` | independent ENCRYPTOR agrees | `gen-bento4.sh` | Bento4 `mp4hls` (packed + ts) | ES byte-identical to reset reference (13 syncframes); ffmpeg 9/9 on Bento4 TS |
| B3 | (none) | IV carried across an independent + **dependent** syncframes of one access unit | - | none obtainable | **NO oracle**; spec-text (Apple §2.3.1.3) only. Note ffmpeg/Bento4 reset per *independent* syncframe |
| B4 | (none) | AC-3 (non-E) Sample-AES | - | ffmpeg packed path fails on encrypted ac3/ec3 | not built |
| C1 | `mp4/cenc-layouts/enc-default-*` | CENC decrypt with tfhd `base_data_offset` (single, explicit-2-trun, implicit-2-trun) | `gen.sh`, `relayout.py` | Bento4 mp4encrypt (encrypt), mp4decrypt, ffmpeg | mp4decrypt CORRECT 3/3; ffmpeg 115/115 (single, explicit), fails implicit (its limitation) |
| C2 | `mp4/cenc-layouts/enc-base-moof-*` | same with default-base-is-moof | same | same | mp4decrypt CORRECT 3/3; ffmpeg same as C1 |
| C3 | `mp4/cenc-layouts/enc-omit-*` | omit-tfhd-offset: traf 2 base = end of traf 1 data (single / explicit / implicit multi-trun) | same | mp4decrypt, ffmpeg | **NO independent decryptor is correct** (mp4decrypt WRONG audio; ffmpeg wrong audio). Oracle = plaintext == `clear.mp4` (`clear-samples.txt`); layout offsets identical to ffmpeg's own muxer output; ffmpeg reads clear omit 115/115 |
| C4 | `mp4/cenc-layouts/clear-ffmpeg-{default,base-moof,omit}.mp4` | the three addressing forms as a real muxer writes them | `gen.sh` | ffmpeg (writer + reader), mp4dump | all read 115/115; dumps in `layouts.txt` |
| C5 | `mp4/cenc-layouts/*multitrun*`, `enc-*-{explicit,implicit}` | multiple truns per traf, explicit and implicit data offset | `relayout.py` (**hand-split; no tool emits it**) | mp4dump validated; mp4decrypt (default/base_moof only); spec-derived `frag_samples.py` | see C1-C3; implicit unreadable by ffmpeg even in clear |
| C6 | (rejected) ffmpeg `-encryption_scheme` output | - | - | ffmpeg muxer | audio of fragments >= 2 does not decrypt in ffmpeg **or** mp4decrypt; not used as an oracle |

**Claims with NO independent oracle:** A5 (EP inside clear regions), B3 (IV carry across dependent syncframes), B4 (AC-3),
C3 (any decryptor of omit_tfhd_offset+CENC), implicit-trun with ffmpeg, and `saio` semantics for omit (moof-relative,
matching ffmpeg's muxer only).
Corrections to existing repo claims: `crypto/sample-aes/PROVENANCE.md` / `sample_aes::tests::eac3_multi_syncframe_carries_iv`
carry the IV across two **independent** syncframes; ffmpeg and Bento4 reset it there (see B1/B2/B3).



## Notes

# transmux test fixtures

Independent-oracle fixtures for the crypto fixes (audit r05 W1/W2/W7) are indexed in the table above (paths relative to `fixtures/`):
`crypto/sample-aes-h264/`, `crypto/sample-aes-eac3/`, `mp4/cenc-layouts/`, each with a `gen.sh` and a PROVENANCE.md recording tool versions
and evidence. (`crypto/sample-aes/` holds the older E-AC-3 5.1 fixture used by a src unit test.)


## `piff-sample-encrypted.mp4`

A real PIFF-CBC sample-encrypted, fragmented MP4 (audit run 7, W15) —
generated from this workspace's own `fixtures/ts/h264-aac.ts` with Bento4:

```bash
ffmpeg -y -i fixtures/ts/h264-aac.ts -c copy /tmp/base.mp4
mp4fragment /tmp/base.mp4 /tmp/frag.mp4
mp4encrypt --method PIFF-CBC \
  --key 1:0123456789abcdef0123456789abcdef:0123456789abcdef \
  /tmp/frag.mp4 fixtures/mp4/piff-sample-encrypted.mp4
```

Track 1 (video) is PIFF-CBC encrypted: the fragment's `traf` carries the PIFF
"UUID Sample Encryption Box" (`A2394F52-5A9B-4F14-A244-6C427C648DF4`), which
`fragment_looks_encrypted` must recognise. The key/IV are a throwaway
test vector, not secret. Own work — MIT OR Apache-2.0.
