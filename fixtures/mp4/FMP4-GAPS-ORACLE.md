# fMP4 gap-tier fixture oracle (#429–#437)

Real ffmpeg-authored MP4s + the **byte-exact config-box bodies** ffmpeg writes.
Each gate builds its box from the source and must reproduce the body byte-for-byte
(same technique that verified esds/dac3/dec3 in 0.4.1). Bodies below are the bytes
**after** the 8-byte box header; truncated to 64 hex where long.

Regenerate: `fixtures/mp4/GENERATE.md`.

## avcC / hvcC value-verification (the #425 follow-up — clean reference)

The vendored ISO/IEC 14496-15 is image-only **scanned** (unusable). The clean
reference is ffmpeg's own muxer (movenc.c) — the byte oracle here + a text-layer
14496-15 fetched into `docs/codec/`.

| box | source mp4 | body (hex) |
|---|---|---|
| `avcC` | `h264_high.mp4` | `0164000dffe100196764000dacd94141fb011000000300100000030320f14299…` |
| `hvcC` | `hevc_main.mp4` | `0101600000009000000000003cf000fcfdf8f800000f0320…` |
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

## #434 colr / pasp / clap (`colr_hdr.mp4`)

| box | len | body (hex) | notes |
|---|---|---|---|
| `colr` | 19 | `6e636c78 0002 0002 0009 00` | 'nclx' primaries/transfer/matrix + full_range flag |
| `pasp` | — | present | |

## #435 prft / sbgp+sgpd / subs

| box | source | len | body (hex) | notes |
|---|---|---|---|---|
| `prft` | `prft.mp4` | 32 | `01 000018 00000001 edefe3e3a7ae147a 0000000000001c20` | v1, ref_track_ID=1, ntp_timestamp, media_time |
| `sgpd` | `aac_sgpd.mp4` / `opus.mp4` | 26 | `01 000000 726f6c6c 00000002 00000001 ffff` | grouping='roll', roll_distance=-1 |
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

## Blocked (no fixture path here)

- **AC-4 #431** — spec vendored (ETSI TS 103 190-2) but **no AC-4 encoder in ffmpeg**. Needs a real DVB capture or a spec-reference bitstream.
- **HE-AAC #432** — no `libfdk_aac` in this ffmpeg (default `aac` won't emit SBR/PS).
- **MPEG-H #433** — no encoder and ISO/IEC 23008-3 not vendored.
