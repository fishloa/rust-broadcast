# Shared test fixtures

Real captures and locally generated (ffmpeg) reference streams shared across
crates (`transmux`, `ts-fix`, `media-doctor`, `dvb-si`). Crate-local fixtures
stay under `<crate>/tests/fixtures/`; anything used by more than one crate —
or staged for upcoming work — lives here.

Every file below is identity-verified with `ffprobe` before commit; entries
list the verified codec/profile so a gate can assert against it.

## `ts/` — MPEG-2 TS

| File | Content (ffprobe-verified) | Provenance / intended gate |
|---|---|---|
| `m6-single.ts` | Real French DTT capture (M6 mux), single service | dvb-si / ts-fix baseline |
| `m6-discontinuity.ts` | m6 capture with continuity-counter gaps | ts-fix CC repair |
| `m6-duplicate.ts` | m6 capture with 5 true legal duplicates (identical payload, same CC) | ts-fix duplicate handling (must NOT "repair") |
| `france-pcr-discontinuity.ts` | Real French DTT capture containing a PCR discontinuity | ts-fix PCR restamp / media-doctor PcrCheck |
| `h264_aac.ts` | H.264 Main + AAC-LC mux | transmux TS→IR A/V demux gate |
| `h264/baseline.ts` | H.264 Constrained Baseline 320×240 | SPS/profile matrix |
| `h264/main.ts` | H.264 Main 320×240 | SPS/profile matrix |
| `h264/high.ts` | H.264 High 320×240 | SPS/profile matrix |
| `h264/high10.ts` | H.264 High 10 320×240 | SPS/profile matrix (bit_depth > 8) |
| `h264/high422.ts` | H.264 High 4:2:2 320×240 | SPS/profile matrix (chroma_format_idc 2) |
| `h264/high444.ts` | H.264 High 4:4:4 Predictive 320×240 | SPS/profile matrix (chroma_format_idc 3) |
| `h264/high_1080_cropped.ts` | H.264 High 1920×1080 (frame_cropping) | SPS crop-offset decode |
| `h264/interlaced.ts` | H.264 High 720×576 interlaced (top-field-first) | SPS frame_mbs_only_flag=0 / interlace handling |
| `hevc/main.ts` | HEVC Main 320×240 | HEVC SPS/profile matrix |
| `hevc/main10.ts` | HEVC Main 10 320×240 | HEVC SPS/profile matrix (10-bit) |
| `dolby/ac3.ts` | AC-3 audio-only mux, 44.1 kHz mono, ~2 s | transmux AC-3 spoke (ETSI TS 102 366) |
| `dolby/eac3.ts` | E-AC-3 audio-only mux, 44.1 kHz mono, ~2 s | transmux E-AC-3 spoke (ETSI TS 102 366) |

## `transmux/` — ISO BMFF

| File | Content (ffprobe-verified) | Provenance / intended gate |
|---|---|---|
| `h264_aac_frag.mp4` | Fragmented MP4, H.264 + AAC-LC | transmux fMP4 demux gate |
| `hevc_frag.mp4` | Fragmented MP4, HEVC (hvc1/hvcC) | transmux hvcC gate |
| `h264_aac_prog.mp4` | Progressive (non-fragmented) MP4, H.264 High + AAC-LC | transmux progressive-MP4 demux spoke |
| `h264_sidx.mp4` | Fragmented MP4 with `sidx` segment index, H.264 High | transmux sidx parse / DASH on-demand |
