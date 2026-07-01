# Codec-config fixture oracle

Real fixtures (ffmpeg-encoded real bitstreams) for the SPS/PPS/VPS-decode + RFC 6381
work (#425) and the AC-3/E-AC-3 fMP4 work (#426). Oracle values are the authoritative
SPS/VPS fields dumped by `ffmpeg -bsf:v trace_headers` (H.264/H.265) and `ffprobe`
(dims/pixfmt/Dolby). Every value a decoder produces must match its row here.

Regenerate: see `fixtures/ts/GENERATE.md`.

## H.264 profile matrix (`fixtures/ts/h264/`, all 320×240 unless noted)

The high-profile branch (profile_idc ∈ {100,110,122,244,…}) carries extra SPS syntax
(`chroma_format_idc`, `bit_depth_*_minus8`, `seq_scaling_matrix_present_flag`) that
Baseline/Main omit — each fixture exercises a distinct decode path.

| fixture | profile | profile_idc | constraint set0/1 | level_idc | chroma_format_idc | bit_depth luma/chroma (−8) | width_mbs−1 | height_map−1 | frame_mbs_only | crop | coded W×H | rfc6381 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `baseline.ts` | Constrained Baseline | 66 | 1 / 1 | 13 | (n/a → 1) | (n/a → 0/0) | 19 | 14 | 1 | — | 320×240 | `avc1.42C00D` |
| `main.ts` | Main | 77 | 0 / 1 | 13 | (n/a → 1) | (n/a → 0/0) | 19 | 14 | 1 | — | 320×240 | `avc1.4D400D` |
| `high.ts` | High | 100 | 0 / 0 | 13 | 1 | 0 / 0 | 19 | 14 | 1 | — | 320×240 | `avc1.64000D` |
| `high10.ts` | High 10 | 110 | 0 / 0 | 13 | 1 | 2 / 2 | 19 | 14 | 1 | — | 320×240 | `avc1.6E000D` |
| `high422.ts` | High 4:2:2 | 122 | 0 / 0 | 13 | 2 | 0 / 0 | 19 | 14 | 1 | — | 320×240 | `avc1.7A000D` |
| `high444.ts` | High 4:4:4 Predictive | 244 | 0 / 0 | 13 | 3 | 0 / 0 | 19 | 14 | 1 | — | 320×240 | `avc1.F4000D` |
| `interlaced.ts` | High | 100 | 0 / 0 | 30 | 1 | 0 / 0 | 44 | 17 | **0** (MBAFF) | — | 720×576 | `avc1.64001E` |
| `high_1080_cropped.ts` | High | 100 | 0 / 0 | — | 1 | 0 / 0 | 119 | 67 | 1 | bottom=4 | **1920×1080** | — |

Coded dimensions:
- Width  = `(pic_width_in_mbs_minus1 + 1) × 16` − chroma-scaled `(crop_left+crop_right)`.
- Height = `(2 − frame_mbs_only_flag) × (pic_height_in_map_units_minus1 + 1) × 16`
  − chroma-scaled `(crop_top+crop_bottom)`.
  - `interlaced.ts`: `(2−0) × (17+1) × 16 = 576`.
  - `high_1080_cropped.ts`: `1088 − CropUnitY(=2 for 4:2:0, frame_mbs_only=1) × 4 = 1080`.

The `seq_scaling_matrix_present_flag=1` branch (SPS-embedded scaling lists) is **not**
emitted by common encoders (x264 puts custom matrices in the PPS), so it has no real
fixture — cover it with a spec-vector unit test that forces the flag and asserts the
decoder skips the scaling lists and still reaches the correct dimensions.

`h264_aac.ts` (repo root, already committed): Main profile, level 1.3, 320×240,
progressive, 4:2:0 8-bit → `avc1.4D000D`; AAC-LC 44100 Hz 1ch.

## HEVC (`fixtures/ts/hevc/`, 320×240)

| fixture | profile | pix_fmt | bit depth | notes |
|---|---|---|---|---|
| `main.ts` | Main | yuv420p | 8 | general_profile_idc=1 |
| `main10.ts` | Main 10 | yuv420p10le | 10 | general_profile_idc=2 |

VPS/SPS/`profile_tier_level` fields (general_profile_space/tier_flag/profile_idc,
compatibility flags, constraint bytes, level_idc, chroma_format_idc, bit-depths,
conformance-window-cropped dims) → dump with `trace_headers` when implementing the
HEVC path; the `hvc1.…` RFC 6381 string is built from them.

## Dolby (`fixtures/ts/dolby/`) — for #426

| fixture | codec | sample_rate | channels | bitrate |
|---|---|---|---|---|
| `ac3.ts` | AC-3 | 44100 | 1 | 192 kbps |
| `eac3.ts` | E-AC-3 | 44100 | 1 | ~192 kbps |

`dac3`/`dec3` field oracle (fscod/bsid/bsmod/acmod/lfeon; E-AC-3 substream layout)
comes from ETSI TS 102 366 Annex F when implementing #426.
